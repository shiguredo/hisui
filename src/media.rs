use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};

use crate::audio::AudioData;
use crate::video::VideoFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MediaStreamId(u64);

#[derive(Debug, Clone)]
pub enum MediaSample {
    Audio(AudioData),
    Video(VideoFrame),
}

pub type SharedMediaSample = Arc<MediaSample>;
pub type MediaStreamReceiver = Receiver<SharedMediaSample>;
pub type MediaStreamSyncSender = SyncSender<SharedMediaSample>;
