//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/video-codec-sdk
#![warn(missing_docs)]

use std::ffi::c_void;
use std::sync::{Arc, LazyLock};

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

    /// CUDA エラーをチェックする
    fn check_cuda(status: u32, function: &'static str, reason: &'static str) -> Result<(), Error> {
        if status == sys::cudaError_enum_CUDA_SUCCESS {
            Ok(())
        } else {
            Err(Self::new(status, function, reason))
        }
    }

    /// NVENC エラーをチェックする
    fn check_nvenc(status: u32, function: &'static str, reason: &'static str) -> Result<(), Error> {
        if status == sys::_NVENCSTATUS_NV_ENC_SUCCESS {
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
    cuda_lib: Arc<libloading::Library>,
    nvcuvid_lib: Arc<libloading::Library>,
    nvenc_lib: Arc<libloading::Library>,
}

impl CudaLibrary {
    /// CUDA ライブラリをロードし、必要な関数が利用可能かチェックする
    fn load() -> Result<Self, Error> {
        type Libraries = (
            Arc<libloading::Library>,
            Arc<libloading::Library>,
            Arc<libloading::Library>,
        );
        static LIBS: LazyLock<Result<Libraries, Error>> = LazyLock::new(|| unsafe {
            // CUDA ドライバーライブラリをロード
            let cuda_lib = libloading::Library::new("libcuda.so.1")
                .map(Arc::new)
                .map_err(|_| {
                    Error::new(
                        sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                        "CudaLibrary::load",
                        "failed to load CUDA library (libcuda.so.1 not found)",
                    )
                })?;

            // cuInit を呼び出して CUDA ドライバーを初期化
            let cu_init: libloading::Symbol<unsafe extern "C" fn(u32) -> u32> =
                cuda_lib.get(b"cuInit").map_err(|_| {
                    Error::new(
                        sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                        "CudaLibrary::load",
                        "cuInit not found",
                    )
                })?;
            let flags = 0;
            let status = cu_init(flags);
            Error::check_cuda(status, "cuInit", "failed to initialize CUDA driver")?;

            // NVCUVID ライブラリをロード（デコード用）
            let nvcuvid_lib = libloading::Library::new("libnvcuvid.so.1")
                .map(Arc::new)
                .map_err(|_| {
                    Error::new(
                        sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                        "CudaLibrary::load",
                        "failed to load NVCUVID library (libnvcuvid.so.1 not found)",
                    )
                })?;

            // NVENC ライブラリをロード（エンコード用）
            let nvenc_lib = libloading::Library::new("libnvidia-encode.so.1")
                .map(Arc::new)
                .map_err(|_| {
                    Error::new(
                        sys::cudaError_enum_CUDA_ERROR_UNKNOWN,
                        "CudaLibrary::load",
                        "failed to load NVENC library (libnvidia-encode.so.1 not found)",
                    )
                })?;

            Ok((cuda_lib, nvcuvid_lib, nvenc_lib))
        });

        let (cuda_lib, nvcuvid_lib, nvenc_lib) = LIBS.clone()?;

        // 必要な関数が存在するか確認
        unsafe {
            // コンテキスト管理関連
            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32> =
                cuda_lib
                    .get(b"cuCtxCreate_v2")
                    .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxCreate_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = cuda_lib
                .get(b"cuCtxDestroy_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxDestroy_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = cuda_lib
                .get(b"cuCtxPushCurrent_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxPushCurrent_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext) -> u32> = cuda_lib
                .get(b"cuCtxPopCurrent_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxPopCurrent_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn() -> u32> = cuda_lib
                .get(b"cuCtxSynchronize")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuCtxSynchronize not found"))?;

            // メモリ管理関連
            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32> =
                cuda_lib
                    .get(b"cuMemAlloc_v2")
                    .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemAlloc_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUdeviceptr) -> u32> = cuda_lib
                .get(b"cuMemFree_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemFree_v2 not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32,
            > = cuda_lib
                .get(b"cuMemcpyHtoD_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemcpyHtoD_v2 not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32,
            > = cuda_lib
                .get(b"cuMemcpyDtoH_v2")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuMemcpyDtoH_v2 not found"))?;

            // NVCUVID 関連
            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::CUvideoctxlock, sys::CUcontext) -> u32,
            > = nvcuvid_lib
                .get(b"cuvidCtxLockCreate")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuvidCtxLockCreate not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideoctxlock) -> u32> =
                nvcuvid_lib.get(b"cuvidCtxLockDestroy").map_err(|_| {
                    Error::new(0, "CudaLibrary::load", "cuvidCtxLockDestroy not found")
                })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::CUvideoparser, *mut sys::CUVIDPARSERPARAMS) -> u32,
            > = nvcuvid_lib.get(b"cuvidCreateVideoParser").map_err(|_| {
                Error::new(0, "CudaLibrary::load", "cuvidCreateVideoParser not found")
            })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideoparser) -> u32> =
                nvcuvid_lib.get(b"cuvidDestroyVideoParser").map_err(|_| {
                    Error::new(0, "CudaLibrary::load", "cuvidDestroyVideoParser not found")
                })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUvideoparser, *mut sys::CUVIDSOURCEDATAPACKET) -> u32,
            > = nvcuvid_lib
                .get(b"cuvidParseVideoData")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuvidParseVideoData not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(
                    *mut sys::CUvideodecoder,
                    *mut sys::CUVIDDECODECREATEINFO,
                ) -> u32,
            > = nvcuvid_lib
                .get(b"cuvidCreateDecoder")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuvidCreateDecoder not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideodecoder) -> u32> =
                nvcuvid_lib.get(b"cuvidDestroyDecoder").map_err(|_| {
                    Error::new(0, "CudaLibrary::load", "cuvidDestroyDecoder not found")
                })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUvideodecoder, *mut sys::CUVIDPICPARAMS) -> u32,
            > = nvcuvid_lib
                .get(b"cuvidDecodePicture")
                .map_err(|_| Error::new(0, "CudaLibrary::load", "cuvidDecodePicture not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(
                    sys::CUvideodecoder,
                    i32,
                    *mut u64,
                    *mut u32,
                    *mut sys::CUVIDPROCPARAMS,
                ) -> u32,
            > = nvcuvid_lib.get(b"cuvidMapVideoFrame64").map_err(|_| {
                Error::new(0, "CudaLibrary::load", "cuvidMapVideoFrame64 not found")
            })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideodecoder, u64) -> u32> =
                nvcuvid_lib.get(b"cuvidUnmapVideoFrame64").map_err(|_| {
                    Error::new(0, "CudaLibrary::load", "cuvidUnmapVideoFrame64 not found")
                })?;

            // NVENC 関連
            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::NV_ENCODE_API_FUNCTION_LIST) -> u32,
            > = nvenc_lib.get(b"NvEncodeAPICreateInstance").map_err(|_| {
                Error::new(
                    0,
                    "CudaLibrary::load",
                    "NvEncodeAPICreateInstance not found",
                )
            })?;
        }

        Ok(Self {
            cuda_lib,
            nvcuvid_lib,
            nvenc_lib,
        })
    }

    /// CUDA コンテキストを作成する
    fn cu_ctx_create(
        &self,
        ctx: *mut sys::CUcontext,
        flags: u32,
        device: i32,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32> =
                self.cuda_lib
                    .get(b"cuCtxCreate_v2")
                    .expect("cuCtxCreate_v2 should exist (checked in load())");
            let status = f(ctx, flags, device);
            Error::check_cuda(status, "cuCtxCreate_v2", "failed to create CUDA context")
        }
    }

    /// CUDA コンテキストを破棄する
    fn cu_ctx_destroy(&self, ctx: sys::CUcontext) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = self
                .cuda_lib
                .get(b"cuCtxDestroy_v2")
                .expect("cuCtxDestroy_v2 should exist (checked in load())");
            let status = f(ctx);
            Error::check_cuda(status, "cuCtxDestroy_v2", "failed to destroy CUDA context")
        }
    }

    /// CUDA コンテキストをスタックにプッシュする
    fn cu_ctx_push_current(&self, ctx: sys::CUcontext) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = self
                .cuda_lib
                .get(b"cuCtxPushCurrent_v2")
                .expect("cuCtxPushCurrent_v2 should exist (checked in load())");
            let status = f(ctx);
            Error::check_cuda(status, "cuCtxPushCurrent_v2", "failed to push CUDA context")
        }
    }

    /// CUDA コンテキストをスタックからポップする
    fn cu_ctx_pop_current(&self, ctx: *mut sys::CUcontext) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext) -> u32> = self
                .cuda_lib
                .get(b"cuCtxPopCurrent_v2")
                .expect("cuCtxPopCurrent_v2 should exist (checked in load())");
            let status = f(ctx);
            Error::check_cuda(status, "cuCtxPopCurrent_v2", "failed to pop CUDA context")
        }
    }

    /// CUDA コンテキストを同期する
    fn cu_ctx_synchronize(&self) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn() -> u32> = self
                .cuda_lib
                .get(b"cuCtxSynchronize")
                .expect("cuCtxSynchronize should exist (checked in load())");
            let status = f();
            Error::check_cuda(
                status,
                "cuCtxSynchronize",
                "failed to synchronize CUDA context",
            )
        }
    }

    /// デバイスメモリを割り当てる
    fn cu_mem_alloc(&self, dptr: *mut sys::CUdeviceptr, bytesize: usize) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32> =
                self.cuda_lib
                    .get(b"cuMemAlloc_v2")
                    .expect("cuMemAlloc_v2 should exist (checked in load())");
            let status = f(dptr, bytesize);
            Error::check_cuda(status, "cuMemAlloc_v2", "failed to allocate device memory")
        }
    }

    /// デバイスメモリを解放する
    fn cu_mem_free(&self, dptr: sys::CUdeviceptr) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUdeviceptr) -> u32> = self
                .cuda_lib
                .get(b"cuMemFree_v2")
                .expect("cuMemFree_v2 should exist (checked in load())");
            let status = f(dptr);
            Error::check_cuda(status, "cuMemFree_v2", "failed to free device memory")
        }
    }

    /// ホストからデバイスへメモリをコピーする
    fn cu_memcpy_h_to_d(
        &self,
        dst_device: sys::CUdeviceptr,
        src_host: *const c_void,
        byte_count: usize,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32,
            > = self
                .cuda_lib
                .get(b"cuMemcpyHtoD_v2")
                .expect("cuMemcpyHtoD_v2 should exist (checked in load())");
            let status = f(dst_device, src_host, byte_count);
            Error::check_cuda(
                status,
                "cuMemcpyHtoD_v2",
                "failed to copy memory from host to device",
            )
        }
    }

    /// デバイスからホストへメモリをコピーする
    fn cu_memcpy_d_to_h(
        &self,
        dst_host: *mut c_void,
        src_device: sys::CUdeviceptr,
        byte_count: usize,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32,
            > = self
                .cuda_lib
                .get(b"cuMemcpyDtoH_v2")
                .expect("cuMemcpyDtoH_v2 should exist (checked in load())");
            let status = f(dst_host, src_device, byte_count);
            Error::check_cuda(
                status,
                "cuMemcpyDtoH_v2",
                "failed to copy memory from device to host",
            )
        }
    }

    /// NvEncodeAPICreateInstance を呼び出す
    fn nvenc_create_api_instance(
        &self,
        function_list: *mut sys::NV_ENCODE_API_FUNCTION_LIST,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::NV_ENCODE_API_FUNCTION_LIST) -> u32,
            > = self
                .nvenc_lib
                .get(b"NvEncodeAPICreateInstance")
                .expect("NvEncodeAPICreateInstance should exist (checked in load())");
            let status = f(function_list);
            Error::check_nvenc(
                status,
                "NvEncodeAPICreateInstance",
                "failed to create NVENC API instance",
            )
        }
    }

    /// cuvidCtxLockCreate を呼び出す
    fn cuvid_ctx_lock_create(
        &self,
        lock: *mut sys::CUvideoctxlock,
        ctx: sys::CUcontext,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::CUvideoctxlock, sys::CUcontext) -> u32,
            > = self
                .nvcuvid_lib
                .get(b"cuvidCtxLockCreate")
                .expect("cuvidCtxLockCreate should exist (checked in load())");
            let status = f(lock, ctx);
            Error::check_cuda(
                status,
                "cuvidCtxLockCreate",
                "failed to create context lock",
            )
        }
    }

    /// cuvidCtxLockDestroy を呼び出す
    fn cuvid_ctx_lock_destroy(&self, lock: sys::CUvideoctxlock) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUvideoctxlock) -> u32> = self
                .nvcuvid_lib
                .get(b"cuvidCtxLockDestroy")
                .expect("cuvidCtxLockDestroy should exist (checked in load())");
            let status = f(lock);
            Error::check_cuda(
                status,
                "cuvidCtxLockDestroy",
                "failed to destroy context lock",
            )
        }
    }

    /// cuvidCreateVideoParser を呼び出す
    fn cuvid_create_video_parser(
        &self,
        parser: *mut sys::CUvideoparser,
        params: *mut sys::CUVIDPARSERPARAMS,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::CUvideoparser, *mut sys::CUVIDPARSERPARAMS) -> u32,
            > = self
                .nvcuvid_lib
                .get(b"cuvidCreateVideoParser")
                .expect("cuvidCreateVideoParser should exist (checked in load())");
            let status = f(parser, params);
            Error::check_cuda(
                status,
                "cuvidCreateVideoParser",
                "failed to create video parser",
            )
        }
    }

    /// cuvidDestroyVideoParser を呼び出す
    fn cuvid_destroy_video_parser(&self, parser: sys::CUvideoparser) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUvideoparser) -> u32> = self
                .nvcuvid_lib
                .get(b"cuvidDestroyVideoParser")
                .expect("cuvidDestroyVideoParser should exist (checked in load())");
            let status = f(parser);
            Error::check_cuda(
                status,
                "cuvidDestroyVideoParser",
                "failed to destroy video parser",
            )
        }
    }

    /// cuvidParseVideoData を呼び出す
    fn cuvid_parse_video_data(
        &self,
        parser: sys::CUvideoparser,
        packet: *mut sys::CUVIDSOURCEDATAPACKET,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(sys::CUvideoparser, *mut sys::CUVIDSOURCEDATAPACKET) -> u32,
            > = self
                .nvcuvid_lib
                .get(b"cuvidParseVideoData")
                .expect("cuvidParseVideoData should exist (checked in load())");
            let status = f(parser, packet);
            Error::check_cuda(status, "cuvidParseVideoData", "failed to parse video data")
        }
    }

    /// cuvidCreateDecoder を呼び出す
    fn cuvid_create_decoder(
        &self,
        decoder: *mut sys::CUvideodecoder,
        create_info: *mut sys::CUVIDDECODECREATEINFO,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(
                    *mut sys::CUvideodecoder,
                    *mut sys::CUVIDDECODECREATEINFO,
                ) -> u32,
            > = self
                .nvcuvid_lib
                .get(b"cuvidCreateDecoder")
                .expect("cuvidCreateDecoder should exist (checked in load())");
            let status = f(decoder, create_info);
            Error::check_cuda(status, "cuvidCreateDecoder", "failed to create decoder")
        }
    }

    /// cuvidDestroyDecoder を呼び出す
    fn cuvid_destroy_decoder(&self, decoder: sys::CUvideodecoder) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUvideodecoder) -> u32> = self
                .nvcuvid_lib
                .get(b"cuvidDestroyDecoder")
                .expect("cuvidDestroyDecoder should exist (checked in load())");
            let status = f(decoder);
            Error::check_cuda(status, "cuvidDestroyDecoder", "failed to destroy decoder")
        }
    }

    /// cuvidDecodePicture を呼び出す
    fn cuvid_decode_picture(
        &self,
        decoder: sys::CUvideodecoder,
        pic_params: *mut sys::CUVIDPICPARAMS,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(sys::CUvideodecoder, *mut sys::CUVIDPICPARAMS) -> u32,
            > = self
                .nvcuvid_lib
                .get(b"cuvidDecodePicture")
                .expect("cuvidDecodePicture should exist (checked in load())");
            let status = f(decoder, pic_params);
            Error::check_cuda(status, "cuvidDecodePicture", "failed to decode picture")
        }
    }

    /// cuvidMapVideoFrame64 を呼び出す
    fn cuvid_map_video_frame(
        &self,
        decoder: sys::CUvideodecoder,
        picture_index: i32,
        device_ptr: *mut u64,
        pitch: *mut u32,
        proc_params: *mut sys::CUVIDPROCPARAMS,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<
                unsafe extern "C" fn(
                    sys::CUvideodecoder,
                    i32,
                    *mut u64,
                    *mut u32,
                    *mut sys::CUVIDPROCPARAMS,
                ) -> u32,
            > = self
                .nvcuvid_lib
                .get(b"cuvidMapVideoFrame64")
                .expect("cuvidMapVideoFrame64 should exist (checked in load())");
            let status = f(decoder, picture_index, device_ptr, pitch, proc_params);
            Error::check_cuda(status, "cuvidMapVideoFrame64", "failed to map video frame")
        }
    }

    /// cuvidUnmapVideoFrame64 を呼び出す
    fn cuvid_unmap_video_frame(
        &self,
        decoder: sys::CUvideodecoder,
        device_ptr: u64,
    ) -> Result<(), Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(sys::CUvideodecoder, u64) -> u32> = self
                .nvcuvid_lib
                .get(b"cuvidUnmapVideoFrame64")
                .expect("cuvidUnmapVideoFrame64 should exist (checked in load())");
            let status = f(decoder, device_ptr);
            Error::check_cuda(
                status,
                "cuvidUnmapVideoFrame64",
                "failed to unmap video frame",
            )
        }
    }

    /// CUDA context を push して、クロージャを実行し、自動的に pop する
    fn with_context<F, R>(&self, ctx: sys::CUcontext, f: F) -> Result<R, Error>
    where
        F: FnOnce() -> Result<R, Error>,
    {
        self.cu_ctx_push_current(ctx)?;

        let result = f();

        let mut popped_ctx = std::ptr::null_mut();
        self.cu_ctx_pop_current(&mut popped_ctx)?;

        result
    }
}

/// CUDA ライブラリがロード可能かチェックする
pub fn is_cuda_available() -> bool {
    CudaLibrary::load().is_ok()
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
