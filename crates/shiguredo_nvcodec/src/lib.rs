//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/video-codec-sdk
#![warn(missing_docs)]

use std::sync::LazyLock;

mod decode;
mod encode;
mod sys;

pub use decode::{DecodedFrame, Decoder, DecoderConfig};
pub use encode::{EncodedFrame, Encoder, EncoderConfig, PictureType};

/// ビルド時に参照したバージョン
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// エラー
#[derive(Debug, Clone)]
pub struct Error {
    status: u32, // NVENCSTATUS は u32 型
    function: &'static str,
    reason: &'static str,
}

impl Error {
    fn new(status: u32, function: &'static str, reason: &'static str) -> Self {
        Self {
            status,
            function,
            reason,
        }
    }

    fn check(status: u32, function: &'static str, reason: &'static str) -> Result<(), Error> {
        if status == sys::cudaError_enum_CUDA_SUCCESS {
            Ok(())
        } else {
            Err(Self::new(status, function, reason))
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}() failed: status={}, reason={}",
            self.function, self.status, self.reason
        )?;
        Ok(())
    }
}

impl std::error::Error for Error {}

/// CUDA ドライバーをプロセスごとに1回だけ初期化する
fn ensure_cuda_initialized() -> Result<(), Error> {
    static CUDA_INIT_RESULT: LazyLock<Result<(), Error>> = LazyLock::new(|| {
        let flags = 0;
        let status = unsafe { sys::cuInit(flags) };
        if status == sys::cudaError_enum_CUDA_SUCCESS {
            Ok(())
        } else {
            Err(Error::new(
                status,
                "cuInit",
                "failed to initialize CUDA driver",
            ))
        }
    });

    CUDA_INIT_RESULT.clone()
}

// CUDA context を push して、クロージャを実行し、自動的に pop する
fn with_cuda_context<F, R>(ctx: sys::CUcontext, f: F) -> Result<R, Error>
where
    F: FnOnce() -> Result<R, Error>,
{
    unsafe {
        let status = sys::cuCtxPushCurrent_v2(ctx);
        Error::check(status, "cuCtxPushCurrent_v2", "failed to push CUDA context")?;

        let result = f();

        let status = sys::cuCtxPopCurrent_v2(std::ptr::null_mut());
        Error::check(status, "cuCtxPopCurrent_v2", "failed to pop CUDA context")?;

        result
    }
}

/// エラー時にリソースを確実に解放するための構造体
struct ReleaseGuard<F: FnOnce()> {
    cleanup: Option<F>,
}

impl<F: FnOnce()> ReleaseGuard<F> {
    /// 新しい ReleaseGuard を作成する
    fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }

    /// クリーンアップ処理をキャンセルする（リソースの所有権が移転した場合などに使用）
    fn cancel(mut self) {
        self.cleanup = None;
    }
}

impl<F: FnOnce()> Drop for ReleaseGuard<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = Error::new(
            sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
            "test_function",
            "test error",
        );
        let error_string = format!("{}", e);
        assert!(error_string.contains("test_function"));
        assert!(error_string.contains("test error"));
    }
}
