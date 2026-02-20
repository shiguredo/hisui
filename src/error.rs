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
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
}
