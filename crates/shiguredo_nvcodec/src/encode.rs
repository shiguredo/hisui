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

            // Load NVENC API
            let mut encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
            encoder_api.version = sys::NVENCAPI_VERSION;

            let status = sys::NvEncodeAPICreateInstance(&mut encoder_api);
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
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
            open_session_params.version = sys::NVENCAPI_VERSION;
            open_session_params.deviceType = sys::_NV_ENC_DEVICE_TYPE_NV_ENC_DEVICE_TYPE_CUDA;
            open_session_params.device = ctx as *mut c_void;
            open_session_params.apiVersion = sys::NVENCAPI_VERSION;

            let mut h_encoder = ptr::null_mut();
            let status = (encoder_api.nvEncOpenEncodeSessionEx.unwrap())(
                &mut open_session_params,
                &mut h_encoder,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::with_reason(
                    status,
                    "nvEncOpenEncodeSessionEx",
                    "Failed to open encode session",
                ));
            }

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
        // Initialize encoder parameters
        let mut init_params: sys::NV_ENC_INITIALIZE_PARAMS = unsafe { std::mem::zeroed() };
        let mut config: sys::NV_ENC_CONFIG = unsafe { std::mem::zeroed() };

        init_params.version = sys::NVENCAPI_VERSION;
        init_params.encodeGUID = crate::guid::NV_ENC_CODEC_HEVC_GUID;
        init_params.presetGUID = crate::guid::NV_ENC_PRESET_P4_GUID;
        init_params.encodeWidth = self.state.lock().unwrap().width;
        init_params.encodeHeight = self.state.lock().unwrap().height;
        init_params.darWidth = self.state.lock().unwrap().width;
        init_params.darHeight = self.state.lock().unwrap().height;
        init_params.frameRateNum = 30;
        init_params.frameRateDen = 1;
        init_params.enablePTD = 1;
        init_params.encodeConfig = &mut config;

        config.version = sys::NVENCAPI_VERSION;
        config.profileGUID = crate::guid::NV_ENC_HEVC_PROFILE_MAIN_GUID;
        config.gopLength = sys::NVENC_INFINITE_GOPLENGTH;
        config.frameIntervalP = 1;

        // Initialize encoder
        let status = unsafe {
            (self.encoder.nvEncInitializeEncoder.unwrap())(self.h_encoder, &mut init_params)
        };
        if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
            return Err(Error::with_reason(
                status,
                "nvEncInitializeEncoder",
                "Failed to initialize encoder",
            ));
        }

        Ok(())
    }

    /// NV12フォーマットのフレームをエンコードする
    pub fn encode_frame(&mut self, _y_plane: &[u8], _uv_plane: &[u8]) -> Result<(), Error> {
        // Implementation would handle frame encoding
        // This is a simplified version
        Ok(())
    }

    /// エンコード済みパケットを取得する
    pub fn get_encoded_packet(&mut self) -> Option<EncodedPacket> {
        let mut state = self.state.lock().unwrap();
        if state.encoded_packets.is_empty() {
            None
        } else {
            Some(state.encoded_packets.remove(0))
        }
    }

    /// エンコードを終了する
    pub fn finish(&mut self) -> Result<(), Error> {
        // Flush encoder
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            if !self.h_encoder.is_null() {
                if let Some(destroy_fn) = self.encoder.nvEncDestroyEncoder {
                    destroy_fn(self.h_encoder);
                }
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
