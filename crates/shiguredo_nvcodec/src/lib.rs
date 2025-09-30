//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/nvidia-video-codec-sdk
#![warn(missing_docs)]

use std::sync::Once;

mod decode;
mod encode;
mod sys;

pub use decode::Encoder;
pub use decode::{DecodedFrame, Decoder};

// ビルド時に参照したリポジトリのバージョン
// Note: sys module doesn't export BUILD_METADATA_VERSION, so this is commented out
// pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// CUDA ドライバーの初期化（プロセスごとに1回だけ実行される）
static CUDA_INIT: Once = Once::new();
static mut CUDA_INIT_RESULT: Option<Result<(), Error>> = None;

/// CUDA ドライバーを初期化する（内部使用）
fn ensure_cuda_initialized() -> Result<(), Error> {
    unsafe {
        CUDA_INIT.call_once(|| {
            let status = sys::cuInit(0);
            CUDA_INIT_RESULT = Some(if status == sys::cudaError_enum_CUDA_SUCCESS {
                Ok(())
            } else {
                Err(Error::with_reason(
                    status,
                    "cuInit",
                    "Failed to initialize CUDA driver",
                ))
            });
        });

        // CUDA_INIT_RESULT は call_once の中で必ず初期化されるため unwrap は安全
        // Use raw pointer instead of reference to avoid static_mut_refs lint
        std::ptr::addr_of!(CUDA_INIT_RESULT)
            .read()
            .as_ref()
            .unwrap()
            .clone()
    }
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
