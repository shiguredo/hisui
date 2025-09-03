use std::path::PathBuf;

use orfail::OrFail;

use crate::media::MediaStreamId;
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
            stdin,
            stdout,
            input_stream_ids: self.input_stream_ids.clone(),
        })
    }
}

#[derive(Debug)]
pub struct PluginCommandProcessor {
    process: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::process::ChildStdout,
    input_stream_ids: Vec<MediaStreamId>,
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
        todo!()
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
