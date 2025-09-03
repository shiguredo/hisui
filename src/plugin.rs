use std::path::PathBuf;

use crate::media::MediaStreamId;

#[derive(Debug)]
pub struct PluginCommand {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub input_stream_ids: Vec<MediaStreamId>,
}

#[derive(Debug)]
pub struct PluginCommandProcessor {
    process: std::process::Child,
}

impl Drop for PluginCommandProcessor {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
