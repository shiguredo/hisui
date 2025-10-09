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
pub use encode::{
    EncodedFrame, Encoder, EncoderConfig, PictureType, Preset, Profile, RateControlMode, TuningInfo,
};

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

/// CUDA ライブラリのラッパー構造体
#[derive(Debug, Clone)]
struct CudaLibrary {
    lib: Arc<libloading::Library>,
}

impl CudaLibrary {
    /// CUDA ライブラリをロードし、必要な関数が利用可能かチェックする
    fn load() -> Result<Self, Error> {
        let lib = load_cuda_library()?;

        // 必要な関数が存在するか確認
        unsafe {
            // コンテキスト管理関連
            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32> =
                lib.get(b"cuCtxCreate_v2")
                    .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxCreate_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = lib
                .get(b"cuCtxDestroy_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxDestroy_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = lib
                .get(b"cuCtxPushCurrent_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxPushCurrent_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext) -> u32> = lib
                .get(b"cuCtxPopCurrent_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxPopCurrent_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib
                .get(b"cuCtxSynchronize")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxSynchronize not found"))?;

            // メモリ管理関連
            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32> =
                lib.get(b"cuMemAlloc_v2")
                    .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemAlloc_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUdeviceptr) -> u32> = lib
                .get(b"cuMemFree_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemFree_v2 not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32,
            > = lib
                .get(b"cuMemcpyHtoD_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemcpyHtoD_v2 not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32,
            > = lib
                .get(b"cuMemcpyDtoH_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemcpyDtoH_v2 not found"))?;
        }

        Ok(Self { lib })
    }

    /// CUDA コンテキストを作成する
    unsafe fn cu_ctx_create(
        &self,
        ctx: *mut sys::CUcontext,
        flags: u32,
        device: i32,
    ) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32> =
            self.lib
                .get(b"cuCtxCreate_v2")
                .expect("cuCtxCreate_v2 should exist (checked in load())");
        let status = f(ctx, flags, device);
        Error::check(status, "cuCtxCreate_v2", "failed to create CUDA context")
    }

    /// CUDA コンテキストを破棄する
    unsafe fn cu_ctx_destroy(&self, ctx: sys::CUcontext) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = self
            .lib
            .get(b"cuCtxDestroy_v2")
            .expect("cuCtxDestroy_v2 should exist (checked in load())");
        let status = f(ctx);
        Error::check(status, "cuCtxDestroy_v2", "failed to destroy CUDA context")
    }

    /// CUDA コンテキストをスタックにプッシュする
    unsafe fn cu_ctx_push_current(&self, ctx: sys::CUcontext) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = self
            .lib
            .get(b"cuCtxPushCurrent_v2")
            .expect("cuCtxPushCurrent_v2 should exist (checked in load())");
        let status = f(ctx);
        Error::check(status, "cuCtxPushCurrent_v2", "failed to push CUDA context")
    }

    /// CUDA コンテキストをスタックからポップする
    unsafe fn cu_ctx_pop_current(&self, ctx: *mut sys::CUcontext) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext) -> u32> = self
            .lib
            .get(b"cuCtxPopCurrent_v2")
            .expect("cuCtxPopCurrent_v2 should exist (checked in load())");
        let status = f(ctx);
        Error::check(status, "cuCtxPopCurrent_v2", "failed to pop CUDA context")
    }

    /// CUDA コンテキストを同期する
    unsafe fn cu_ctx_synchronize(&self) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn() -> u32> = self
            .lib
            .get(b"cuCtxSynchronize")
            .expect("cuCtxSynchronize should exist (checked in load())");
        let status = f();
        Error::check(
            status,
            "cuCtxSynchronize",
            "failed to synchronize CUDA context",
        )
    }

    /// デバイスメモリを割り当てる
    unsafe fn cu_mem_alloc(
        &self,
        dptr: *mut sys::CUdeviceptr,
        bytesize: usize,
    ) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32> = self
            .lib
            .get(b"cuMemAlloc_v2")
            .expect("cuMemAlloc_v2 should exist (checked in load())");
        let status = f(dptr, bytesize);
        Error::check(status, "cuMemAlloc_v2", "failed to allocate device memory")
    }

    /// デバイスメモリを解放する
    unsafe fn cu_mem_free(&self, dptr: sys::CUdeviceptr) -> Result<(), Error> {
        let f: libloading::Symbol<unsafe extern "C" fn(sys::CUdeviceptr) -> u32> = self
            .lib
            .get(b"cuMemFree_v2")
            .expect("cuMemFree_v2 should exist (checked in load())");
        let status = f(dptr);
        Error::check(status, "cuMemFree_v2", "failed to free device memory")
    }

    /// ホストからデバイスへメモリをコピーする
    unsafe fn cu_memcpy_h_to_d(
        &self,
        dst_device: sys::CUdeviceptr,
        src_host: *const c_void,
        byte_count: usize,
    ) -> Result<(), Error> {
        let f: libloading::Symbol<
            unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32,
        > = self
            .lib
            .get(b"cuMemcpyHtoD_v2")
            .expect("cuMemcpyHtoD_v2 should exist (checked in load())");
        let status = f(dst_device, src_host, byte_count);
        Error::check(
            status,
            "cuMemcpyHtoD_v2",
            "failed to copy memory from host to device",
        )
    }

    /// デバイスからホストへメモリをコピーする
    unsafe fn cu_memcpy_d_to_h(
        &self,
        dst_host: *mut c_void,
        src_device: sys::CUdeviceptr,
        byte_count: usize,
    ) -> Result<(), Error> {
        let f: libloading::Symbol<
            unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32,
        > = self
            .lib
            .get(b"cuMemcpyDtoH_v2")
            .expect("cuMemcpyDtoH_v2 should exist (checked in load())");
        let status = f(dst_host, src_device, byte_count);
        Error::check(
            status,
            "cuMemcpyDtoH_v2",
            "failed to copy memory from device to host",
        )
    }
}

/// CUDA ライブラリを動的にロードする
fn load_cuda_library() -> Result<Arc<libloading::Library>, Error> {
    static CUDA_LIB: LazyLock<Result<Arc<libloading::Library>, Error>> = LazyLock::new(|| unsafe {
        // ライブラリをロード
        let lib = libloading::Library::new("libcuda.so.1")
            .map(Arc::new)
            .map_err(|_| {
                Error::new(
                    sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                    "load_cuda_library",
                    "failed to load CUDA library (libcuda.so.1 not found)",
                )
            })?;

        // cuInit を呼び出して CUDA ドライバーを初期化
        let cu_init: libloading::Symbol<unsafe extern "C" fn(u32) -> u32> =
            lib.get(b"cuInit").map_err(|_| {
                Error::new(
                    sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                    "load_cuda_library",
                    "cuInit not found",
                )
            })?;
        let flags = 0;
        let status = cu_init(flags);
        Error::check(status, "cuInit", "failed to initialize CUDA driver")?;

        Ok(lib)
    });

    CUDA_LIB.clone()
}

/// CUDA ライブラリがロード可能かチェックする
pub fn is_cuda_available() -> bool {
    load_cuda_library().is_ok()
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
