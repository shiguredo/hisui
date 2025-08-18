use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};

use crate::audio::AudioData;
use crate::video::VideoFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MediaStreamId(u64);

#[derive(Debug)]
pub struct MediaStreamIdGenerator(MediaStreamId);

impl MediaStreamIdGenerator {
    pub fn new() -> Self {
        Self(MediaStreamId(0))
    }

    pub fn next_media_stream_id(&mut self) -> MediaStreamId {
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
    pub fn audio_data(data: AudioData) -> Self {
        Self::Audio(Arc::new(data))
    }

    pub fn video_frame(frame: VideoFrame) -> Self {
        Self::Video(Arc::new(frame))
    }
}

pub type MediaStreamReceiver = Receiver<MediaSample>;
pub type MediaStreamSyncSender = SyncSender<MediaSample>;
