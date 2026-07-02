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

    /// TextFrame を Arc に包んで Text バリアントを返す。`timestamp()` は `TextFrame::start` を返す。
    pub fn new_text(frame: TextFrame) -> Self {
        Self::Text(Arc::new(frame))
    }

    /// バリアントごとに以下を返す:
    /// - Audio: `AudioFrame::timestamp`
    /// - Video: `VideoFrame::timestamp`
    /// - Text: `TextFrame::start` (`end` ではない)
    pub fn timestamp(&self) -> Duration {
        match self {
            Self::Audio(x) => x.timestamp,
            Self::Video(x) => x.timestamp,
            Self::Text(x) => x.start,
        }
    }

    /// エラーメッセージで実バリアント名を埋め込むための内部識別子を返す ("audio" / "video" / "text")。
    /// codec 名や外部プロトコルフィールドとは独立した文字列。
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Audio(_) => "audio",
            Self::Video(_) => "video",
            Self::Text(_) => "text",
        }
    }

    pub fn expect_audio(self) -> crate::Result<Arc<AudioFrame>> {
        match self {
            Self::Audio(frame) => Ok(frame),
            other => Err(crate::Error::new(format!(
                "expected audio sample, but got {} sample",
                other.kind_name()
            ))),
        }
    }

    pub fn expect_video(self) -> crate::Result<Arc<VideoFrame>> {
        match self {
            Self::Video(frame) => Ok(frame),
            other => Err(crate::Error::new(format!(
                "expected video sample, but got {} sample",
                other.kind_name()
            ))),
        }
    }

    pub fn expect_text(self) -> crate::Result<Arc<TextFrame>> {
        match self {
            Self::Text(frame) => Ok(frame),
            other => Err(crate::Error::new(format!(
                "expected text sample, but got {} sample",
                other.kind_name()
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
