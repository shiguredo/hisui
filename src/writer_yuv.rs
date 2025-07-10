use std::{fs::File, io::Write, path::Path};

use orfail::OrFail;

use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct YuvWriter {
    file: File,
}

impl YuvWriter {
    pub fn new<P: AsRef<Path>>(path: P) -> orfail::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .or_fail_with(|e| format!("{e}: {}", path.as_ref().display()))?;
        Ok(Self { file })
    }

    pub fn append(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        matches!(frame.format, VideoFormat::I420).or_fail()?;
        self.file.write_all(&frame.data).or_fail()?;
        Ok(())
    }
}
