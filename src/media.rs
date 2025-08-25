use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};

use crate::audio::AudioData;
use crate::video::VideoFrame;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MediaStreamId(u64);

impl MediaStreamId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

// TODO(atode): remove
#[derive(Debug, Default)]
pub struct MediaStreamIdGenerator(MediaStreamId);

impl MediaStreamIdGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_id(&mut self) -> MediaStreamId {
        let id = self.0;
        self.0.0 += 1;
        id
    }
}

#[derive(Debug, Clone)]
pub enum MediaSample {
    Audio(Arc<AudioData>),
    Video(Arc<VideoFrame>),
}

impl MediaSample {
    pub fn expect_audio_data(self) -> orfail::Result<Arc<AudioData>> {
        if let Self::Audio(sample) = self {
            Ok(sample)
        } else {
            Err(orfail::Failure::new(
                "expected an audio sample, but got a video sample",
            ))
        }
    }

    pub fn expect_video_frame(self) -> orfail::Result<Arc<VideoFrame>> {
        if let Self::Video(frame) = self {
            Ok(frame)
        } else {
            Err(orfail::Failure::new(
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
