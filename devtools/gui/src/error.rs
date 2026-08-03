//! エラー型。
//!
//! hisui 本体の `src/error.rs` と同じ設計。任意のエラー型から変換可能にするために
//! 意図的に [`std::error::Error`] を実装していない。

use std::backtrace::{Backtrace, BacktraceStatus};
use std::panic::Location;

/// エラー型
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

    /// エラー理由のみの文字列表現を返す
    ///
    /// `Display` を実装していないため、互換用途で明示的に提供する。
    pub fn display(&self) -> String {
        self.reason.clone()
    }
}

impl std::fmt::Debug for Error {
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

/// このクレートのエラー結果型
pub type Result<T> = std::result::Result<T, Error>;

impl From<std::io::Error> for Error {
    #[track_caller]
    fn from(e: std::io::Error) -> Self {
        Self::new(e.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for Error {
    #[track_caller]
    fn from(e: tokio::time::error::Elapsed) -> Self {
        Self::new(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_returns_reason() {
        let err = Error::new("reason");
        assert_eq!(err.display(), "reason");
    }
}
