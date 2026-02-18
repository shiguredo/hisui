use std::{fs::File, io::Write, path::Path};

use crate::{Error, MediaSample, Message, ProcessorHandle, Result, TrackId, video::VideoFormat};

#[derive(Debug)]
pub struct YuvWriter {
    file: File,
}

impl YuvWriter {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::new(format!("{e}: {}", path.as_ref().display())))?;
        Ok(Self { file })
    }

    pub async fn run(mut self, handle: ProcessorHandle, input_track_id: TrackId) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id.clone());
        handle.notify_ready();

        loop {
            match input_rx.recv().await {
                Message::Media(MediaSample::Video(frame)) => {
                    if frame.format != VideoFormat::I420 {
                        return Err(Error::new(format!(
                            "expected I420 video sample on track {}, but got {}",
                            input_track_id.get(),
                            frame.format
                        )));
                    }
                    self.file.write_all(&frame.data)?;
                }
                Message::Media(MediaSample::Audio(_)) => {
                    return Err(Error::new(format!(
                        "expected a video sample on track {}, but got an audio sample",
                        input_track_id.get()
                    )));
                }
                Message::Eos => break,
                Message::Syn(_) => {}
            }
        }

        Ok(())
    }
}
