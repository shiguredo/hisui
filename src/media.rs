use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::Duration;

use orfail::OrFail;

use crate::audio::AudioData;
use crate::metadata::SourceId;
use crate::video::VideoFrame;

#[derive(Debug, Default)]
pub struct MediaStreamNameRegistry {
    name_to_id: HashMap<MediaStreamName, MediaStreamId>,
    next_id: MediaStreamId,
}

impl MediaStreamNameRegistry {
    pub fn new() -> Self {
        Self {
            name_to_id: HashMap::new(),
            next_id: MediaStreamId::new(0),
        }
    }

    pub fn get_id(&self, name: &MediaStreamName) -> orfail::Result<MediaStreamId> {
        self.name_to_id
            .get(name)
            .copied()
            .or_fail_with(|()| format!("unknown stream name: {:?}", name.get()))
    }

    pub fn register_name(&mut self, name: MediaStreamName) -> orfail::Result<MediaStreamId> {
        (!self.name_to_id.contains_key(&name))
            .or_fail_with(|()| format!("duplicate stream name: {:?}", name.get()))?;

        let id = self.next_id.fetch_add(1);
        self.name_to_id.insert(name, id);
        Ok(id)
    }
}

// 設定ファイルなどで使われる外部用の名前
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MediaStreamName(String);

impl MediaStreamName {
    pub fn new(name: &str) -> Self {
        Self(name.to_owned())
    }

    pub fn get(&self) -> &str {
        &self.0
    }

    pub fn to_source_id(&self) -> SourceId {
        SourceId::new(&self.0)
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for MediaStreamName {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        value.try_into().map(Self)
    }
}

// 内部用の識別子
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MediaStreamId(u64);

impl MediaStreamId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn fetch_add(&mut self, n: u64) -> Self {
        let id = *self;
        self.0 += n;
        id
    }
}

#[derive(Debug, Clone)]
pub enum MediaSample {
    Audio(Arc<AudioData>),
    Video(Arc<VideoFrame>),
}

impl MediaSample {
    pub fn timestamp(&self) -> Duration {
        match self {
            Self::Audio(x) => x.timestamp,
            Self::Video(x) => x.timestamp,
        }
    }

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
