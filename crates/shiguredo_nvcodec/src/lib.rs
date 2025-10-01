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
        if status != sys::cudaError_enum_CUDA_SUCCESS {
            return Err(Error::new(
                status,
                "cuCtxPushCurrent_v2",
                "failed to push CUDA context",
            ));
        }

        let result = f();

        sys::cuCtxPopCurrent_v2(std::ptr::null_mut());

        result
    }
}

/// ドロップ時に自動でクリーンアップ処理を実行する構造体
struct OwnedWithCleanup<T, F: FnOnce(T)> {
    value: Option<T>,
    cleanup: Option<F>,
}

impl<T, F: FnOnce(T)> OwnedWithCleanup<T, F> {
    /// 新しい OwnedWithCleanup を作成する
    fn new(value: T, cleanup: F) -> Self {
        Self {
            value: Some(value),
            cleanup: Some(cleanup),
        }
    }

    /// 値への参照を取得する
    fn get(&self) -> &T {
        self.value.as_ref().expect("value should be present")
    }

    /// 値への可変参照を取得する
    fn get_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("value should be present")
    }

    /// クリーンアップ処理をキャンセルし、値の所有権を取得する
    /// （リソースの所有権が移転した場合などに使用）
    fn into_inner(mut self) -> T {
        let value = self.value.take().expect("value should be present");
        self.cleanup = None;
        value
    }
}

impl<T, F: FnOnce(T)> Drop for OwnedWithCleanup<T, F> {
    fn drop(&mut self) {
        if let (Some(value), Some(cleanup)) = (self.value.take(), self.cleanup.take()) {
            cleanup(value);
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
