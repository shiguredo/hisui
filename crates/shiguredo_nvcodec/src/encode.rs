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
                return Err(Error::new(
                    status,
                    "cuCtxCreate_v2",
                    "Failed to create CUDA context",
                ));
            }

            // Activate CUDA context for NVENC operations
            let status = sys::cuCtxPushCurrent_v2(ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuCtxDestroy_v2(ctx);
                return Err(Error::new(
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
                return Err(Error::new(
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
                return Err(Error::new(
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
                return Err(Error::new(
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
                sys::NV_ENC_CODEC_HEVC_GUID,
                sys::NV_ENC_PRESET_P4_GUID,
                sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY, // TODO: make configurable
                &mut preset_config,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
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
            init_params.encodeGUID = sys::NV_ENC_CODEC_HEVC_GUID;
            init_params.presetGUID = sys::NV_ENC_PRESET_P4_GUID;
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
            config.profileGUID = sys::NV_ENC_HEVC_PROFILE_MAIN_GUID;
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
                return Err(Error::new(
                    status,
                    "nvEncInitializeEncoder",
                    "Failed to initialize encoder",
                ));
            }

            Ok(())
        }
    }

    /// Encode a single frame in NV12 format
    pub fn encode_frame(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        let state = self.state.lock().unwrap();
        let expected_size = (state.width * state.height * 3 / 2) as usize;

        if nv12_data.len() != expected_size {
            return Err(Error::new(
                sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
                "encode_frame",
                "Invalid NV12 data size",
            ));
        }

        unsafe {
            // Push CUDA context
            let status = sys::cuCtxPushCurrent_v2(self.ctx);
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(Error::new(
                    status,
                    "cuCtxPushCurrent_v2",
                    "Failed to push CUDA context",
                ));
            }

            // Allocate device memory for input
            let mut device_input = 0u64;
            let status = sys::cuMemAlloc_v2(&mut device_input, nv12_data.len());
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "cuMemAlloc_v2",
                    "Failed to allocate device memory",
                ));
            }

            // Copy data to device
            let status = sys::cuMemcpyHtoD_v2(
                device_input,
                nv12_data.as_ptr() as *const c_void,
                nv12_data.len(),
            );
            if status != sys::cudaError_enum_CUDA_SUCCESS {
                sys::cuMemFree_v2(device_input);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "cuMemcpyHtoD_v2",
                    "Failed to copy data to device",
                ));
            }

            // Register the CUDA device memory as an input resource
            let mut register_resource: sys::NV_ENC_REGISTER_RESOURCE = std::mem::zeroed();
            register_resource.version = sys::NV_ENC_REGISTER_RESOURCE_VER;
            register_resource.resourceType =
                sys::_NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR;
            register_resource.resourceToRegister = device_input as *mut c_void;
            register_resource.width = state.width;
            register_resource.height = state.height;
            register_resource.pitch = state.width;
            register_resource.bufferFormat = state.buffer_format;
            register_resource.bufferUsage = sys::_NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE;

            let status = (self.encoder.nvEncRegisterResource.unwrap())(
                self.h_encoder,
                &mut register_resource,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                sys::cuMemFree_v2(device_input);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "nvEncRegisterResource",
                    "Failed to register input resource",
                ));
            }

            let registered_resource = register_resource.registeredResource;

            // Map the registered resource
            let mut map_input_resource: sys::NV_ENC_MAP_INPUT_RESOURCE = std::mem::zeroed();
            map_input_resource.version = sys::NV_ENC_MAP_INPUT_RESOURCE_VER;
            map_input_resource.registeredResource = registered_resource;

            let status = (self.encoder.nvEncMapInputResource.unwrap())(
                self.h_encoder,
                &mut map_input_resource,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                (self.encoder.nvEncUnregisterResource.unwrap())(
                    self.h_encoder,
                    registered_resource,
                );
                sys::cuMemFree_v2(device_input);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "nvEncMapInputResource",
                    "Failed to map input resource",
                ));
            }

            let mapped_resource = map_input_resource.mappedResource;

            // Allocate output bitstream buffer
            let mut create_bitstream: sys::NV_ENC_CREATE_BITSTREAM_BUFFER = std::mem::zeroed();
            create_bitstream.version = sys::NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

            let status = (self.encoder.nvEncCreateBitstreamBuffer.unwrap())(
                self.h_encoder,
                &mut create_bitstream,
            );
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
                (self.encoder.nvEncUnregisterResource.unwrap())(
                    self.h_encoder,
                    registered_resource,
                );
                sys::cuMemFree_v2(device_input);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "nvEncCreateBitstreamBuffer",
                    "Failed to create bitstream buffer",
                ));
            }
            let output_buffer = create_bitstream.bitstreamBuffer;

            // Setup encode picture parameters
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.inputWidth = state.width;
            pic_params.inputHeight = state.height;
            pic_params.inputPitch = state.width;
            pic_params.inputBuffer = mapped_resource; // Use mapped resource, not device pointer!
            pic_params.outputBitstream = output_buffer;
            pic_params.bufferFmt = state.buffer_format;
            pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;

            drop(state); // Release lock before encoding

            // Encode picture
            let status =
                (self.encoder.nvEncEncodePicture.unwrap())(self.h_encoder, &mut pic_params);

            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
                (self.encoder.nvEncUnregisterResource.unwrap())(
                    self.h_encoder,
                    registered_resource,
                );
                (self.encoder.nvEncDestroyBitstreamBuffer.unwrap())(self.h_encoder, output_buffer);
                sys::cuMemFree_v2(device_input);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "nvEncEncodePicture",
                    "Failed to encode picture",
                ));
            }

            // Lock bitstream to read encoded data
            let mut lock_bitstream: sys::NV_ENC_LOCK_BITSTREAM = std::mem::zeroed();
            lock_bitstream.version = sys::NV_ENC_LOCK_BITSTREAM_VER;
            lock_bitstream.outputBitstream = output_buffer;

            let status =
                (self.encoder.nvEncLockBitstream.unwrap())(self.h_encoder, &mut lock_bitstream);
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
                (self.encoder.nvEncUnregisterResource.unwrap())(
                    self.h_encoder,
                    registered_resource,
                );
                (self.encoder.nvEncDestroyBitstreamBuffer.unwrap())(self.h_encoder, output_buffer);
                sys::cuMemFree_v2(device_input);
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                return Err(Error::new(
                    status,
                    "nvEncLockBitstream",
                    "Failed to lock bitstream",
                ));
            }

            // Copy encoded data
            let encoded_data = std::slice::from_raw_parts(
                lock_bitstream.bitstreamBufferPtr as *const u8,
                lock_bitstream.bitstreamSizeInBytes as usize,
            )
            .to_vec();

            // Unlock bitstream
            (self.encoder.nvEncUnlockBitstream.unwrap())(
                self.h_encoder,
                lock_bitstream.outputBitstream,
            );

            // Unmap input resource
            (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);

            // Unregister resource
            (self.encoder.nvEncUnregisterResource.unwrap())(self.h_encoder, registered_resource);

            // Destroy bitstream buffer
            (self.encoder.nvEncDestroyBitstreamBuffer.unwrap())(self.h_encoder, output_buffer);

            // Free device memory
            sys::cuMemFree_v2(device_input);

            // Pop context
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            // Store encoded packet
            let mut state = self.state.lock().unwrap();
            state.encoded_packets.push(EncodedPacket {
                data: encoded_data,
                timestamp: lock_bitstream.outputTimeStamp,
                picture_type: lock_bitstream.pictureType,
            });

            Ok(())
        }
    }

    /// Flush encoder and get remaining packets
    pub fn flush(&mut self) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS;

            let status =
                (self.encoder.nvEncEncodePicture.unwrap())(self.h_encoder, &mut pic_params);
            if status != sys::_NVENCSTATUS_NV_ENC_SUCCESS {
                return Err(Error::new(
                    status,
                    "nvEncEncodePicture",
                    "Failed to flush encoder",
                ));
            }

            Ok(())
        }
    }

    /// Get all encoded packets
    pub fn get_encoded_packets(&mut self) -> Vec<EncodedPacket> {
        let mut state = self.state.lock().unwrap();
        std::mem::take(&mut state.encoded_packets)
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

    #[test]
    fn test_encode_black_frame() {
        let width = 640;
        let height = 480;

        // Create encoder
        let mut encoder = Encoder::new_hevc(width, height).expect("Failed to create HEVC encoder");

        // Prepare black frame in NV12 format
        // Y plane: 16 (black in YUV)
        // UV plane: 128 (neutral chroma)
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;

        let mut frame_data = vec![16u8; y_size + uv_size];
        // Set UV plane to 128 (neutral chroma)
        frame_data[y_size..].fill(128);

        // Encode the frame
        encoder
            .encode_frame(&frame_data)
            .expect("Failed to encode black frame");

        // Flush encoder
        encoder.flush().expect("Failed to flush encoder");

        // Get encoded packets
        let packets = encoder.get_encoded_packets();

        // Verify we got at least one packet
        assert!(!packets.is_empty(), "No encoded packets received");

        // Verify the first packet is a keyframe (IDR)
        let first_packet = &packets[0];
        assert!(
            matches!(
                first_packet.picture_type,
                sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_I | sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_IDR
            ),
            "First frame should be a keyframe"
        );

        // Verify packet has data
        assert!(
            !first_packet.data.is_empty(),
            "Encoded packet should have data"
        );

        println!(
            "Successfully encoded black frame: {} packets, first packet size: {} bytes",
            packets.len(),
            first_packet.data.len()
        );
    }
}
