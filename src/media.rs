use std::sync::Arc;
use std::time::Duration;

use crate::audio::AudioFrame;
use crate::text::TextFrame;
use crate::video::VideoFrame;

#[derive(Debug, Clone)]
pub enum MediaFrame {
    Audio(Arc<AudioFrame>),
    Video(Arc<VideoFrame>),
    Text(Arc<TextFrame>),
}

impl MediaFrame {
    pub fn new_audio(frame: AudioFrame) -> Self {
        Self::Audio(Arc::new(frame))
    }

    pub fn new_video(frame: VideoFrame) -> Self {
        Self::Video(Arc::new(frame))
    }

    pub fn new_text(frame: TextFrame) -> Self {
        Self::Text(Arc::new(frame))
    }

    pub fn timestamp(&self) -> Duration {
        match self {
            Self::Audio(x) => x.timestamp,
            Self::Video(x) => x.timestamp,
            Self::Text(x) => x.start,
        }
    }

    /// バリアント名を文字列で返す ("audio" / "video" / "text")。
    /// エラーメッセージで実バリアント名を埋め込むために使う。
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Audio(_) => "audio",
            Self::Video(_) => "video",
            Self::Text(_) => "text",
        }
    }

    pub fn expect_audio(self) -> crate::Result<Arc<AudioFrame>> {
        let actual = self.kind_name();
        match self {
            Self::Audio(frame) => Ok(frame),
            _ => Err(crate::Error::new(format!(
                "expected audio sample, but got {actual} sample"
            ))),
        }
    }

    pub fn expect_video(self) -> crate::Result<Arc<VideoFrame>> {
        let actual = self.kind_name();
        match self {
            Self::Video(frame) => Ok(frame),
            _ => Err(crate::Error::new(format!(
                "expected video sample, but got {actual} sample"
            ))),
        }
    }

    pub fn expect_text(self) -> crate::Result<Arc<TextFrame>> {
        let actual = self.kind_name();
        match self {
            Self::Text(frame) => Ok(frame),
            _ => Err(crate::Error::new(format!(
                "expected text sample, but got {actual} sample"
            ))),
        }
    }

    pub fn audio(frame: AudioFrame) -> Self {
        Self::Audio(Arc::new(frame))
    }

    pub fn video(frame: VideoFrame) -> Self {
        Self::Video(Arc::new(frame))
    }
}
