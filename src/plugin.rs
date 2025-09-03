use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

use orfail::OrFail;

use crate::json::JsonObject;
use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{
    MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
};
use crate::stats::ProcessorStats;

#[derive(Debug)]
pub struct PluginCommand {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub input_stream_ids: Vec<MediaStreamId>,
}

impl PluginCommand {
    pub fn start(&self) -> orfail::Result<PluginCommandProcessor> {
        let mut process = std::process::Command::new(&self.command)
            .args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .or_fail_with(|e| format!("failed to start plugin command: {e}"))?;

        let stdin = process
            .stdin
            .take()
            .or_fail_with(|()| "failed to get stdin handle".to_owned())?;

        let stdout = process
            .stdout
            .take()
            .or_fail_with(|()| "failed to get stdout handle".to_owned())?;

        Ok(PluginCommandProcessor {
            process,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            input_stream_ids: self.input_stream_ids.clone(),
            next_request_id: 0,
        })
    }
}

#[derive(Debug)]
pub struct PluginCommandProcessor {
    process: std::process::Child,
    stdin: BufWriter<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    input_stream_ids: Vec<MediaStreamId>,
    next_request_id: u64,
}

impl PluginCommandProcessor {
    fn cast<T>(
        &mut self,
        notification: &JsonRpcRequest<T>,
        payload: Option<&[u8]>,
    ) -> orfail::Result<()>
    where
        T: nojson::DisplayJson,
    {
        let notification = nojson::Json(notification).to_string();
        writeln!(self.stdin, "Content-Length: {}", notification.len()).or_fail()?;
        writeln!(self.stdin, "Content-Type: application/json").or_fail()?;
        writeln!(self.stdin).or_fail()?;
        write!(self.stdin, "{notification}").or_fail()?;

        if let Some(payload) = payload {
            writeln!(self.stdin, "Content-Length: {}", payload.len()).or_fail()?;
            writeln!(self.stdin, "Content-Type: application/octet-stream").or_fail()?;
            writeln!(self.stdin).or_fail()?;
            self.stdin.write_all(payload).or_fail()?;
        }

        self.stdin.flush().or_fail()?;
        Ok(())
    }

    fn call<T, U>(&mut self, request: &JsonRpcRequest<T>) -> orfail::Result<U>
    where
        T: nojson::DisplayJson,
        U: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>>,
    {
        let request = nojson::Json(request).to_string();
        writeln!(self.stdin, "Content-Length: {}", request.len()).or_fail()?;
        writeln!(self.stdin, "Content-Type: application/json").or_fail()?;
        writeln!(self.stdin).or_fail()?;
        write!(self.stdin, "{request}").or_fail()?;
        self.stdin.flush().or_fail()?;

        // Read headers to get content length
        let mut content_length = None;
        let mut line = String::new();

        loop {
            line.clear();
            self.stdout.read_line(&mut line).or_fail()?;

            if line.trim().is_empty() {
                // Empty line indicates end of headers
                break;
            }

            if let Some(header_value) = line.strip_prefix("Content-Length: ") {
                content_length = Some(
                    header_value
                        .trim()
                        .parse::<usize>()
                        .or_fail_with(|e| format!("invalid content length: {e}"))?,
                );
            }
        }

        let content_length = content_length
            .or_fail_with(|()| "missing Content-Length header in response".to_owned())?;

        // Read the JSON response body
        let mut response_buffer = vec![0u8; content_length];
        self.stdout.read_exact(&mut response_buffer).or_fail()?;

        let response_text = std::str::from_utf8(&response_buffer)
            .or_fail_with(|e| format!("invalid UTF-8 in response: {e}"))?;

        // Parse the JSON-RPC response
        let json = nojson::RawJson::parse(response_text)
            .or_fail_with(|e| format!("failed to parse JSON response: {e}"))?;

        // Extract the result field from the JSON-RPC response
        if let Some(error) = json.value().to_member("error").or_fail()?.get() {
            return Err(orfail::Failure::new(format!("JSON-RPC error: {error}",)));
        }

        let result = json
            .value()
            .to_member("result")
            .or_fail()?
            .required()
            .or_fail()?;
        U::try_from(result).map_err(|_| {
            orfail::Failure::new("failed to convert response to expected type".to_owned())
        })
    }
}

impl MediaProcessor for PluginCommandProcessor {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_stream_ids.clone(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("plugin_command"),
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        match input.sample {
            None => {
                self.input_stream_ids.retain(|id| *id != input.stream_id);

                let req = JsonRpcRequest::notification(
                    "notify_eos",
                    nojson::object(|f| f.member("stream_id", input.stream_id)),
                );
                self.cast(&req, None).or_fail()?;
            }
            Some(MediaSample::Audio(data)) => {
                let req = JsonRpcRequest::notification(
                    "notify_audio",
                    nojson::object(|f| {
                        f.member("stream_id", input.stream_id)?;
                        f.member("stereo", data.stereo)?;
                        f.member("sample_rate", data.sample_rate)?;
                        f.member("timestamp_us", data.timestamp.as_micros())?;
                        f.member("duration_us", data.duration.as_micros())?;
                        Ok(())
                    }),
                );
                self.cast(&req, Some(&data.data)).or_fail()?;
            }
            Some(MediaSample::Video(frame)) => {
                let req = JsonRpcRequest::notification(
                    "notify_video",
                    nojson::object(|f| {
                        f.member("stream_id", input.stream_id)?;
                        f.member("width", frame.width.get())?;
                        f.member("height", frame.height.get())?;
                        f.member("timestamp_us", frame.timestamp.as_micros())?;
                        f.member("duration_us", frame.duration.as_micros())?;
                        Ok(())
                    }),
                );
                let rgb_data = frame.to_rgb_data().or_fail()?;
                self.cast(&req, Some(&rgb_data)).or_fail()?;
            }
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        let id = self.next_request_id;
        self.next_request_id += 1;

        let req = JsonRpcRequest::request("poll_output", id, ());
        let res: PollOutputResponse = self.call(&req).or_fail()?;

        let output = match res {
            PollOutputResponse::WaitingInputAny => MediaProcessorOutput::awaiting_any(),
            PollOutputResponse::WaitingInput { stream_id } => {
                MediaProcessorOutput::pending(stream_id)
            }
            PollOutputResponse::Finished => MediaProcessorOutput::Finished,
        };
        Ok(output)
    }
}

impl Drop for PluginCommandProcessor {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[derive(Debug)]
pub struct JsonRpcRequest<'a, T> {
    method: &'a str,
    id: Option<u64>,
    params: T,
}

impl<'a, T> JsonRpcRequest<'a, T> {
    pub fn notification(method: &'a str, params: T) -> Self {
        Self {
            method,
            id: None,
            params,
        }
    }

    pub fn request(method: &'a str, id: u64, params: T) -> Self {
        Self {
            method,
            id: Some(id),
            params,
        }
    }
}

impl<'a, T> nojson::DisplayJson for JsonRpcRequest<'a, T>
where
    T: nojson::DisplayJson,
{
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("jsonrpc", "2.0")?;
            f.member("method", self.method)?;
            if let Some(id) = self.id {
                f.member("id", id)?;
            }
            f.member("params", &self.params)?;
            Ok(())
        })
    }
}

#[derive(Debug)]
pub enum PollOutputResponse {
    WaitingInputAny,
    WaitingInput { stream_id: MediaStreamId },
    Finished,
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for PollOutputResponse {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let obj = JsonObject::new(value)?;
        let response_type: String = obj.get_required("type")?;

        match response_type.as_str() {
            "waiting_input_any" => Ok(Self::WaitingInputAny),
            "waiting_input" => {
                let stream_id = obj.get_required("stream_id")?;
                Ok(Self::WaitingInput { stream_id })
            }
            "finished" => Ok(Self::Finished),
            unknown => {
                Err(value.invalid(format!("unknown poll output response type: {unknown:?}")))
            }
        }
    }
}
