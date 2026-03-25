use std::sync::Arc;
use std::time::Duration;

use crate::audio::AudioFrame;
use crate::video::VideoFrame;

#[derive(Debug, Clone)]
pub enum MediaFrame {
    Audio(Arc<AudioFrame>),
    Video(Arc<VideoFrame>),
}

impl MediaFrame {
    pub fn new_audio(frame: AudioFrame) -> Self {
        Self::Audio(Arc::new(frame))
    }

    pub fn new_video(frame: VideoFrame) -> Self {
        Self::Video(Arc::new(frame))
    }

    pub fn timestamp(&self) -> Duration {
        match self {
            Self::Audio(x) => x.timestamp,
            Self::Video(x) => x.timestamp,
        }
    }

    pub fn expect_audio(self) -> crate::Result<Arc<AudioFrame>> {
        if let Self::Audio(frame) = self {
            Ok(frame)
        } else {
            Err(crate::Error::new(
                "expected an audio sample, but got a video sample",
            ))
        }
    }

    pub fn expect_video(self) -> crate::Result<Arc<VideoFrame>> {
        if let Self::Video(frame) = self {
            Ok(frame)
        } else {
            Err(crate::Error::new(
                "expected a video sample, but got an audio sample",
            ))
        }
    }

    pub fn audio(frame: AudioFrame) -> Self {
        Self::Audio(Arc::new(frame))
    }

    pub fn video(frame: VideoFrame) -> Self {
        Self::Video(Arc::new(frame))
    }
}
