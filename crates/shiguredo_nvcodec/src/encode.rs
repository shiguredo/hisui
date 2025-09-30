use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Mutex};

use crate::{Error, ensure_cuda_initialized, sys};

/// H.265 エンコーダー
pub struct Encoder {
    ctx: sys::CUcontext,
    encoder: sys::NV_ENCODE_API_FUNCTION_LIST,
    h_encoder: *mut c_void,
    state: Arc<Mutex<EncoderState>>,
}

struct EncoderState {
    width: u32,
    height: u32,
    #[allow(dead_code)]
    input_buffers: Vec<sys::NV_ENC_REGISTERED_PTR>,
    #[allow(dead_code)]
    output_buffers: Vec<sys::NV_ENC_OUTPUT_PTR>,
    #[allow(dead_code)]
    buffer_format: sys::NV_ENC_BUFFER_FORMAT,
    encoded_packets: Vec<EncodedPacket>,
}

/// エンコード済みパケット
pub struct EncodedPacket {
    /// エンコードされたデータ
    pub data: Vec<u8>,
    /// タイムスタンプ
    pub timestamp: u64,
    /// ピクチャータイプ
    pub picture_type: sys::NV_ENC_PIC_TYPE,
}

impl Encoder {
    /// H.265 エンコーダーインスタンスを生成する
    pub fn new_hevc(width: u32, height: u32) -> Result<Self, Error> {
        // CUDA ドライバーの初期化
        ensure_cuda_initialized()?;

        unsafe {
            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let status = sys::cuCtxCreate_v2(&mut ctx, 0, 0);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxCreate_v2",
                    "Failed to create CUDA context",
                ));
            }

            // Activate CUDA context for NVENC operations
            let status = sys::cuCtxPushCurrent_v2(ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            // Load NVENC API
            let mut encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
            encoder_api.version = sys::NV_ENCODE_API_FUNCTION_LIST_VER;

            let status = sys::NvEncodeAPICreateInstance(&mut encoder_api);
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "NvEncodeAPICreateInstance",
                    "Failed to create NVENC API instance",
                ));
            }

            // Open encode session
            let mut open_session_params: sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS =
                std::mem::zeroed();
            open_session_params.version = sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
            open_session_params.deviceType = sys::_NV_ENC_DEVICE_TYPE_NV_ENC_DEVICE_TYPE_CUDA;
            open_session_params.device = ctx as *mut c_void;
            open_session_params.apiVersion = sys::NVENCAPI_VERSION;

            let mut h_encoder = ptr::null_mut();
            let status = (encoder_api.nvEncOpenEncodeSessionEx.unwrap())(
                &mut open_session_params,
                &mut h_encoder,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "nvEncOpenEncodeSessionEx",
                    "Failed to open encode session",
                ));
            }

            // Pop context after initialization
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            let state = Arc::new(Mutex::new(EncoderState {
                width,
                height,
                input_buffers: Vec::new(),
                output_buffers: Vec::new(),
                buffer_format: sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_NV12,
                encoded_packets: Vec::new(),
            }));

            let mut encoder = Self {
                ctx,
                encoder: encoder_api,
                h_encoder,
                state,
            };

            // Initialize encoder with default parameters
            encoder.initialize_encoder()?;

            Ok(encoder)
        }
    }

    unsafe fn initialize_encoder(&mut self) -> Result<(), Error> {
        unsafe {
            // Push CUDA context
            let status = sys::cuCtxPushCurrent_v2(self.ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            // Get preset configuration first
            let mut preset_config: sys::NV_ENC_PRESET_CONFIG = std::mem::zeroed();
            preset_config.version = sys::NV_ENC_PRESET_CONFIG_VER;
            preset_config.presetCfg.version = sys::NV_ENC_CONFIG_VER;

            let status = (self.encoder.nvEncGetEncodePresetConfigEx.unwrap())(
                self.h_encoder,
                crate::guid::NV_ENC_CODEC_HEVC_GUID,
                crate::guid::NV_ENC_PRESET_P4_GUID,
                sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY, // TODO: make configurable
                &mut preset_config,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::with_reason(
                    status,
                    "nvEncGetEncodePresetConfigEx",
                    "Failed to get preset configuration",
                ));
            }

            // Initialize encoder parameters
            let mut init_params: sys::NV_ENC_INITIALIZE_PARAMS = std::mem::zeroed();
            let mut config: sys::NV_ENC_CONFIG = preset_config.presetCfg;

            let state = self.state.lock().unwrap();

            init_params.version = sys::NV_ENC_INITIALIZE_PARAMS_VER;
            init_params.encodeGUID = crate::guid::NV_ENC_CODEC_HEVC_GUID;
            init_params.presetGUID = crate::guid::NV_ENC_PRESET_P4_GUID;
            init_params.encodeWidth = state.width;
            init_params.encodeHeight = state.height;
            init_params.darWidth = state.width;
            init_params.darHeight = state.height;
            init_params.frameRateNum = 30;
            init_params.frameRateDen = 1;
            init_params.enablePTD = 1;
            init_params.encodeConfig = &mut config;
            init_params.maxEncodeWidth = state.width;
            init_params.maxEncodeHeight = state.height;
            init_params.tuningInfo = sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY; // TODO: make configurable

            config.version = sys::NV_ENC_CONFIG_VER;
            config.profileGUID = crate::guid::NV_ENC_HEVC_PROFILE_MAIN_GUID;
            config.gopLength = sys::NVENC_INFINITE_GOPLENGTH;
            config.frameIntervalP = 1;

            // Set HEVC-specific configuration
            config.encodeCodecConfig.hevcConfig.idrPeriod = config.gopLength;

            drop(state); // Release lock before calling encoder

            // Initialize encoder
            let status =
                (self.encoder.nvEncInitializeEncoder.unwrap())(self.h_encoder, &mut init_params);

            // Pop context after initialization
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                return Err(Error::with_reason(
                    status,
                    "nvEncInitializeEncoder",
                    "Failed to initialize encoder",
                ));
            }

            Ok(())
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            if !self.h_encoder.is_null() {
                // Activate context before cleanup
                let _ = sys::cuCtxPushCurrent_v2(self.ctx);

                if let Some(destroy_fn) = self.encoder.nvEncDestroyEncoder {
                    destroy_fn(self.h_encoder);
                }

                sys::cuCtxPopCurrent_v2(ptr::null_mut());
            }

            if !self.ctx.is_null() {
                sys::cuCtxDestroy_v2(self.ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_hevc_encoder() {
        let _encoder = Encoder::new_hevc(640, 480).expect("Failed to initialize HEVC encoder");
        println!("HEVC encoder initialized successfully");
    }
}
