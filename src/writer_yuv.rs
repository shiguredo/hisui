use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Duration,
};

use orfail::OrFail;

use crate::video::{VideoFormat, VideoFrameReceiver};

/// 合成結果を含んだ YUV ファイルを書き出すための構造体
#[derive(Debug)]
pub struct YuvWriter {
    file: BufWriter<File>,
    input_video_rx: VideoFrameReceiver,
}

impl YuvWriter {
    pub fn new<P: AsRef<Path>>(
        path: P,
        input_video_rx: VideoFrameReceiver,
    ) -> orfail::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .or_fail()?;

        Ok(Self {
            file: BufWriter::new(file),
            input_video_rx,
        })
    }

    pub fn poll(&mut self) -> orfail::Result<Option<Duration>> {
        if let Some(frame) = self.input_video_rx.recv() {
            matches!(frame.format, VideoFormat::I420).or_fail()?;
            self.file.write_all(&frame.data).or_fail()?;
            Ok(Some(frame.timestamp))
        } else {
            self.file.flush().or_fail()?;
            Ok(None)
        }
    }
}
