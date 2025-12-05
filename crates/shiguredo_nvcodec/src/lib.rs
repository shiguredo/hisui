//! [Hisui] 用の [NVCODEC] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [NVCODEC]: https://developer.nvidia.com/video-codec-sdk
#![warn(missing_docs)]

use std::ffi::c_void;
use std::sync::{Arc, LazyLock};

mod decode;
mod encode;
mod error;
mod sys;

pub use decode::{DecodedFrame, Decoder, DecoderConfig};
pub use encode::{
    EncodedFrame, Encoder, EncoderConfig, PictureType, Preset, Profile, RateControlMode, TuningInfo,
};
pub use error::Error;

/// ビルド時に参照したバージョン
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

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
                    Error::new_custom(
                        "CudaLibrary::load",
                        "failed to load CUDA library (libcuda.so.1 not found)",
                    )
                })?;

            // cuInit を呼び出して CUDA ドライバーを初期化
            let cu_init: libloading::Symbol<unsafe extern "C" fn(u32) -> u32> = cuda_lib
                .get(b"cuInit")
                .map_err(|_| Error::new_custom("CudaLibrary::load", "cuInit not found"))?;
            let flags = 0;
            let status = cu_init(flags);
            Error::check_cuda(status, "cuInit")?;

            // NVCUVID ライブラリをロード（デコード用）
            let nvcuvid_lib = libloading::Library::new("libnvcuvid.so.1")
                .map(Arc::new)
                .map_err(|_| {
                    Error::new_custom(
                        "CudaLibrary::load",
                        "failed to load NVCUVID library (libnvcuvid.so.1 not found)",
                    )
                })?;

            // NVENC ライブラリをロード（エンコード用）
            let nvenc_lib = libloading::Library::new("libnvidia-encode.so.1")
                .map(Arc::new)
                .map_err(|_| {
                    Error::new_custom(
                        "CudaLibrary::load",
                        "failed to load NVENC library (libnvidia-encode.so.1 not found)",
                    )
                })?;

            Ok((cuda_lib, nvcuvid_lib, nvenc_lib))
        });

        let (cuda_lib, nvcuvid_lib, nvenc_lib) = LIBS.clone()?;

        // 必要な関数が存在するか確認
        unsafe {
            // エラー関連
            let _: libloading::Symbol<unsafe extern "C" fn(u32, *mut *const u8) -> u32> = cuda_lib
                .get(b"cuGetErrorName")
                .map_err(|_| Error::new_custom("CudaLibrary::load", "cuGetErrorName not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(u32, *mut *const u8) -> u32> =
                cuda_lib.get(b"cuGetErrorString").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuGetErrorString not found")
                })?;

            // コンテキスト管理関連
            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext, u32, i32) -> u32> =
                cuda_lib.get(b"cuCtxCreate_v2").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuCtxCreate_v2 not found")
                })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> = cuda_lib
                .get(b"cuCtxDestroy_v2")
                .map_err(|_| Error::new_custom("CudaLibrary::load", "cuCtxDestroy_v2 not found"))?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUcontext) -> u32> =
                cuda_lib.get(b"cuCtxPushCurrent_v2").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuCtxPushCurrent_v2 not found")
                })?;

            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUcontext) -> u32> =
                cuda_lib.get(b"cuCtxPopCurrent_v2").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuCtxPopCurrent_v2 not found")
                })?;

            let _: libloading::Symbol<unsafe extern "C" fn() -> u32> =
                cuda_lib.get(b"cuCtxSynchronize").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuCtxSynchronize not found")
                })?;

            // メモリ管理関連
            let _: libloading::Symbol<unsafe extern "C" fn(*mut sys::CUdeviceptr, usize) -> u32> =
                cuda_lib.get(b"cuMemAlloc_v2").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuMemAlloc_v2 not found")
                })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUdeviceptr) -> u32> = cuda_lib
                .get(b"cuMemFree_v2")
                .map_err(|_| Error::new_custom("CudaLibrary::load", "cuMemFree_v2 not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUdeviceptr, *const c_void, usize) -> u32,
            > = cuda_lib
                .get(b"cuMemcpyHtoD_v2")
                .map_err(|_| Error::new_custom("CudaLibrary::load", "cuMemcpyHtoD_v2 not found"))?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut c_void, sys::CUdeviceptr, usize) -> u32,
            > = cuda_lib
                .get(b"cuMemcpyDtoH_v2")
                .map_err(|_| Error::new_custom("CudaLibrary::load", "cuMemcpyDtoH_v2 not found"))?;

            // NVCUVID 関連
            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::CUvideoctxlock, sys::CUcontext) -> u32,
            > = nvcuvid_lib.get(b"cuvidCtxLockCreate").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "cuvidCtxLockCreate not found")
            })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideoctxlock) -> u32> =
                nvcuvid_lib.get(b"cuvidCtxLockDestroy").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidCtxLockDestroy not found")
                })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::CUvideoparser, *mut sys::CUVIDPARSERPARAMS) -> u32,
            > = nvcuvid_lib.get(b"cuvidCreateVideoParser").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "cuvidCreateVideoParser not found")
            })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideoparser) -> u32> =
                nvcuvid_lib.get(b"cuvidDestroyVideoParser").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidDestroyVideoParser not found")
                })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUvideoparser, *mut sys::CUVIDSOURCEDATAPACKET) -> u32,
            > = nvcuvid_lib.get(b"cuvidParseVideoData").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "cuvidParseVideoData not found")
            })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(
                    *mut sys::CUvideodecoder,
                    *mut sys::CUVIDDECODECREATEINFO,
                ) -> u32,
            > = nvcuvid_lib.get(b"cuvidCreateDecoder").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "cuvidCreateDecoder not found")
            })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideodecoder) -> u32> =
                nvcuvid_lib.get(b"cuvidDestroyDecoder").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidDestroyDecoder not found")
                })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(sys::CUvideodecoder, *mut sys::CUVIDPICPARAMS) -> u32,
            > = nvcuvid_lib.get(b"cuvidDecodePicture").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "cuvidDecodePicture not found")
            })?;

            let _: libloading::Symbol<
                unsafe extern "C" fn(
                    sys::CUvideodecoder,
                    i32,
                    *mut u64,
                    *mut u32,
                    *mut sys::CUVIDPROCPARAMS,
                ) -> u32,
            > = nvcuvid_lib.get(b"cuvidMapVideoFrame64").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "cuvidMapVideoFrame64 not found")
            })?;

            let _: libloading::Symbol<unsafe extern "C" fn(sys::CUvideodecoder, u64) -> u32> =
                nvcuvid_lib.get(b"cuvidUnmapVideoFrame64").map_err(|_| {
                    Error::new_custom("CudaLibrary::load", "cuvidUnmapVideoFrame64 not found")
                })?;

            // NVENC 関連
            let _: libloading::Symbol<
                unsafe extern "C" fn(*mut sys::NV_ENCODE_API_FUNCTION_LIST) -> u32,
            > = nvenc_lib.get(b"NvEncodeAPICreateInstance").map_err(|_| {
                Error::new_custom("CudaLibrary::load", "NvEncodeAPICreateInstance not found")
            })?;
        }

        Ok(Self {
            cuda_lib,
            nvcuvid_lib,
            nvenc_lib,
        })
    }

    /// エラーコードに対応する名前を取得する
    fn cu_get_error_name(&self, code: u32) -> Result<String, Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(u32, *mut *const u8) -> u32> = self
                .cuda_lib
                .get(b"cuGetErrorName")
                .expect("cuGetErrorName should exist (checked in load())");

            let mut error_name: *const u8 = std::ptr::null();
            let status = f(code, &mut error_name);
            Error::check_cuda(status, "cuGetErrorName")?;

            let error_str = std::ffi::CStr::from_ptr(error_name as *const i8)
                .to_string_lossy()
                .into_owned();
            Ok(error_str)
        }
    }

    /// エラーコードに対応するメッセージを取得する
    fn cu_get_error_string(&self, code: u32) -> Result<String, Error> {
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(u32, *mut *const u8) -> u32> = self
                .cuda_lib
                .get(b"cuGetErrorString")
                .expect("cuGetErrorString should exist (checked in load())");

            let mut error_msg: *const u8 = std::ptr::null();
            let status = f(code, &mut error_msg);
            Error::check_cuda(status, "cuGetErrorString")?;

            let error_str = std::ffi::CStr::from_ptr(error_msg as *const i8)
                .to_string_lossy()
                .into_owned();
            Ok(error_str)
        }
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
            Error::check_cuda(status, "cuCtxCreate_v2")
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
            Error::check_cuda(status, "cuCtxDestroy_v2")
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
            Error::check_cuda(status, "cuCtxPushCurrent_v2")
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
            Error::check_cuda(status, "cuCtxPopCurrent_v2")
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
            Error::check_cuda(status, "cuCtxSynchronize")
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
            Error::check_cuda(status, "cuMemAlloc_v2")
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
            Error::check_cuda(status, "cuMemFree_v2")
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
            Error::check_cuda(status, "cuMemcpyHtoD_v2")
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
            Error::check_cuda(status, "cuMemcpyDtoH_v2")
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
            Error::check_nvenc(status, "NvEncodeAPICreateInstance")
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
            Error::check_cuda(status, "cuvidCtxLockCreate")
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
            Error::check_cuda(status, "cuvidCtxLockDestroy")
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
            Error::check_cuda(status, "cuvidCreateVideoParser")
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
            Error::check_cuda(status, "cuvidDestroyVideoParser")
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
            Error::check_cuda(status, "cuvidParseVideoData")
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
            Error::check_cuda(status, "cuvidCreateDecoder")
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
            Error::check_cuda(status, "cuvidDestroyDecoder")
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
            Error::check_cuda(status, "cuvidDecodePicture")
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
            Error::check_cuda(status, "cuvidMapVideoFrame64")
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
            Error::check_cuda(status, "cuvidUnmapVideoFrame64")
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
///
/// NOTE:
/// この関数がチェックするのは、あくまでも .so などが読み込めるかどうか、までで
/// その環境で実際に CUDA が利用可能かどうかまでは確認していない
pub fn is_cuda_library_available() -> bool {
    unsafe { libloading::Library::new("libcuda.so.1").is_ok() }
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
