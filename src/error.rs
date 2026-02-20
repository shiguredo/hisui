use std::backtrace::{Backtrace, BacktraceStatus};
use std::panic::Location;

/// エラー型
///
/// 任意のエラー型から変換可能にするために意図的に [`std::error::Error`] を実装していない
pub struct Error {
    /// エラーが発生した理由
    pub reason: String,

    /// エラーが作成されたソースコードの場所
    pub location: &'static Location<'static>,

    /// エラー発生箇所を示すバックトレース
    ///
    /// バックトレースは `RUST_BACKTRACE` 環境変数が設定されていない場合には取得されない
    pub backtrace: Backtrace,
}

impl Error {
    /// [`Error`] インスタンスを生成する
    #[track_caller]
    pub fn new<T: Into<String>>(reason: T) -> Self {
        Self {
            reason: reason.into(),
            location: Location::caller(),
            backtrace: Backtrace::capture(),
        }
    }

    /// 既存の [`Error`] に文脈情報を追加する
    pub fn with_context(mut self, context: impl AsRef<str>) -> Self {
        self.reason = format!("{}: {}", context.as_ref(), self.reason);
        self
    }

    /// エラー理由のみの文字列表現を返す
    ///
    /// `Display` を実装していないため、互換用途で明示的に提供する。
    pub fn display(&self) -> String {
        self.reason.clone()
    }

    /// エラー理由のみの文字列表現を返す
    ///
    /// `display()` の互換メソッド。
    pub fn to_string(&self) -> String {
        self.display()
    }

    /// `noargs::Error` に変換する
    pub fn to_noargs_error(self) -> noargs::Error {
        noargs::Error::Other {
            metadata: None,
            error: Box::new(format!("{self:?}")),
        }
    }

    fn fmt_detailed(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason)?;
        write!(f, " (at {}:{})", self.location.file(), self.location.line())?;

        if self.backtrace.status() == BacktraceStatus::Disabled {
            write!(f, " [RUST_BACKTRACE=1 for backtrace]")?;
        }
        if self.backtrace.status() == BacktraceStatus::Captured {
            write!(f, "\n\nBacktrace:\n{}", self.backtrace)?;
        }

        Ok(())
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_detailed(f)
    }
}

impl From<Error> for noargs::Error {
    fn from(error: Error) -> Self {
        error.to_noargs_error()
    }
}

impl From<std::io::Error> for Error {
    #[track_caller]
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<tokio::task::JoinError> for Error {
    #[track_caller]
    fn from(e: tokio::task::JoinError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<nojson::JsonParseError> for Error {
    #[track_caller]
    fn from(e: nojson::JsonParseError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<crate::PipelineTerminated> for Error {
    #[track_caller]
    fn from(e: crate::PipelineTerminated) -> Self {
        Self::new(e.to_string())
    }
}

impl From<crate::PublishTrackError> for Error {
    #[track_caller]
    fn from(e: crate::PublishTrackError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<crate::RegisterProcessorError> for Error {
    #[track_caller]
    fn from(e: crate::RegisterProcessorError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_mp4::demux::DemuxError> for Error {
    #[track_caller]
    fn from(e: shiguredo_mp4::demux::DemuxError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_mp4::mux::MuxError> for Error {
    #[track_caller]
    fn from(e: shiguredo_mp4::mux::MuxError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<std::num::ParseIntError> for Error {
    #[track_caller]
    fn from(e: std::num::ParseIntError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<std::num::TryFromIntError> for Error {
    #[track_caller]
    fn from(e: std::num::TryFromIntError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<std::string::FromUtf8Error> for Error {
    #[track_caller]
    fn from(e: std::string::FromUtf8Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<std::time::SystemTimeError> for Error {
    #[track_caller]
    fn from(e: std::time::SystemTimeError) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_rtmp::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_rtmp::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_dav1d::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_dav1d::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(feature = "libvpx")]
impl From<shiguredo_libvpx::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_libvpx::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_libyuv::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_libyuv::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_openh264::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_openh264::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_opus::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_opus::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<shiguredo_svt_av1::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_svt_av1::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(target_os = "macos")]
impl From<shiguredo_audio_toolbox::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_audio_toolbox::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(target_os = "macos")]
impl From<shiguredo_video_toolbox::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_video_toolbox::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(feature = "fdk-aac")]
impl From<shiguredo_fdk_aac::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_fdk_aac::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(feature = "nvcodec")]
impl From<shiguredo_nvcodec::Error> for Error {
    #[track_caller]
    fn from(e: shiguredo_nvcodec::Error) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_context_adds_prefix() {
        let err = Error::new("inner reason").with_context("outer context");
        assert_eq!(err.reason, "outer context: inner reason");
    }

    #[test]
    fn with_context_preserves_location_and_backtrace_status() {
        let err = Error::new("inner");
        let location = err.location;
        let backtrace_status = err.backtrace.status();

        let err = err.with_context("outer");

        assert_eq!(err.location.file(), location.file());
        assert_eq!(err.location.line(), location.line());
        assert_eq!(err.backtrace.status(), backtrace_status);
    }

    #[test]
    fn to_noargs_error_uses_other_variant() {
        let err = Error::new("reason");
        let noargs_err = err.to_noargs_error();

        match noargs_err {
            noargs::Error::Other { metadata, .. } => {
                assert!(metadata.is_none());
            }
            _ => panic!("expected noargs::Error::Other"),
        }
    }
}
