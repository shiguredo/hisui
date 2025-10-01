//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/video-codec-sdk
#![warn(missing_docs)]

use std::sync::LazyLock;

mod decode;
mod encode;
mod sys;

pub use decode::{DecodedFrame, Decoder};
pub use encode::Encoder;

/// ビルド時に参照したバージョン
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// CUDA ドライバーをプロセスごとに1回だけ初期化する
fn ensure_cuda_initialized() -> Result<(), Error> {
    static CUDA_INIT_RESULT: LazyLock<Result<(), Error>> = LazyLock::new(|| {
        let flags = 0;
        let status = unsafe { sys::cuInit(flags) };
        if status == sys::cudaError_enum_CUDA_SUCCESS {
            Ok(())
        } else {
            Err(Error::with_reason(
                status,
                "cuInit",
                "failed to initialize CUDA driver",
            ))
        }
    });

    CUDA_INIT_RESULT.clone()
}

/// エラー
#[derive(Debug, Clone)]
pub struct Error {
    status: u32, // NVENCSTATUS は u32 型
    function: &'static str,
    reason: Option<&'static str>,
    detail: Option<String>,
}

impl Error {
    fn with_reason(status: u32, function: &'static str, reason: &'static str) -> Self {
        Self {
            status,
            function,
            reason: Some(reason),
            detail: None,
        }
    }

    fn reason(&self) -> &str {
        self.reason.unwrap_or("Unknown NVCODEC error")
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: status={}", self.function, self.status)?;
        write!(f, ", reason={}", self.reason())?;
        if let Some(detail) = &self.detail {
            write!(f, ", detail={detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = Error::with_reason(
            sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
            "test_function",
            "test error",
        );
        let error_string = format!("{}", e);
        assert!(error_string.contains("test_function"));
        assert!(error_string.contains("test error"));
    }
}
