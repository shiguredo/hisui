use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::Duration;

use crate::audio::AudioData;
use crate::video::VideoFrame;

#[derive(Debug, Clone)]
pub enum MediaSample {
    Audio(Arc<AudioData>),
    Video(Arc<VideoFrame>),
}

impl MediaSample {
    pub fn new_audio(data: AudioData) -> Self {
        Self::Audio(Arc::new(data))
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

    pub fn expect_audio_data(self) -> crate::Result<Arc<AudioData>> {
        if let Self::Audio(sample) = self {
            Ok(sample)
        } else {
            Err(crate::Error::new(
                "expected an audio sample, but got a video sample",
            ))
        }
    }

    pub fn expect_video_frame(self) -> crate::Result<Arc<VideoFrame>> {
        if let Self::Video(frame) = self {
            Ok(frame)
        } else {
            Err(crate::Error::new(
                "expected a video sample, but got an audio sample",
            ))
        }
    }

    pub fn audio_data(data: AudioData) -> Self {
        Self::Audio(Arc::new(data))
    }

    pub fn video_frame(frame: VideoFrame) -> Self {
        Self::Video(Arc::new(frame))
    }
}

pub type MediaStreamReceiver = Receiver<MediaSample>;
pub type MediaStreamSyncSender = SyncSender<MediaSample>;
