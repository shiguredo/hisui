use std::io::{BufWriter, Write};
use std::path::PathBuf;

use orfail::OrFail;

use crate::audio::AudioData;
use crate::media::{MediaSample, MediaStreamId};
use crate::processor::{
    MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
};
use crate::stats::ProcessorStats;
use crate::video::VideoFrame;

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
            stdout,
            input_stream_ids: self.input_stream_ids.clone(),
            next_request_id: 0,
        })
    }
}

#[derive(Debug)]
pub struct PluginCommandProcessor {
    process: std::process::Child,
    stdin: BufWriter<std::process::ChildStdin>,
    stdout: std::process::ChildStdout,
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
            Some(MediaSample::Audio(_)) => {}
            Some(MediaSample::Video(_)) => {}
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        todo!()
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

/*
#[derive(Debug)]
pub enum JsonRpcRequest<'a> {
    NotifyAudioData {
        stream_id: MediaStreamId,
        data: &'a AudioData,
    },
    NotifyVideoFrame {
        stream_id: MediaStreamId,
        frame: &'a VideoFrame,
    },
    NotifyEos {
        stream_id: MediaStreamId,
    },
    PollOutput {
        request_id: u64,
    },
}
*/

#[derive(Debug)]
pub enum PollOutputResponse {
    WaitingInputAny,
    WaitingInput { stream_id: MediaStreamId },
    Finished,
}
