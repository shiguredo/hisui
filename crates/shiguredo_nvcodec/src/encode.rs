use std::ffi::c_void;
use std::ptr;

use crate::{Error, ensure_cuda_initialized, sys};

/// エンコーダー
pub struct Encoder {
    ctx: sys::CUcontext,
    encoder: sys::NV_ENCODE_API_FUNCTION_LIST,
    h_encoder: *mut c_void,
    state: EncoderState,
}

struct EncoderState {
    width: u32,
    height: u32,
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
        // CUDA ドライバーの初期化（プロセスごとに1回だけ実行）
        ensure_cuda_initialized()?;

        unsafe {
            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let status = sys::cuCtxCreate_v2(&mut ctx, 0, 0);
            Error::check(status, "cuCtxCreate_v2", "Failed to create CUDA context")?;

            // NVENC 操作のために CUDA context をアクティブ化
            let status = sys::cuCtxPushCurrent_v2(ctx);
            Error::check(status, "cuCtxPushCurrent_v2", "Failed to push CUDA context")
                .inspect_err(|_| {
                    sys::cuCtxDestroy_v2(ctx);
                })?;

            // NVENC API をロード
            let mut encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
            encoder_api.version = sys::NV_ENCODE_API_FUNCTION_LIST_VER;

            let status = sys::NvEncodeAPICreateInstance(&mut encoder_api);
            Error::check(
                status,
                "NvEncodeAPICreateInstance",
                "Failed to create NVENC API instance",
            )
            .inspect_err(|_| {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                sys::cuCtxDestroy_v2(ctx);
            })?;

            // エンコードセッションを開く
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
            Error::check(
                status,
                "nvEncOpenEncodeSessionEx",
                "Failed to open encode session",
            )
            .inspect_err(|_| {
                sys::cuCtxPopCurrent_v2(ptr::null_mut());
                sys::cuCtxDestroy_v2(ctx);
            })?;

            // 初期化後に context を pop
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            let state = EncoderState {
                width,
                height,
                buffer_format: sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_NV12,
                encoded_packets: Vec::new(),
            };

            let mut encoder = Self {
                ctx,
                encoder: encoder_api,
                h_encoder,
                state,
            };

            // デフォルトパラメータでエンコーダーを初期化
            encoder.initialize_encoder()?;

            Ok(encoder)
        }
    }

    unsafe fn initialize_encoder(&mut self) -> Result<(), Error> {
        // CUDA context を push
        let status = unsafe { sys::cuCtxPushCurrent_v2(self.ctx) };
        Error::check(status, "cuCtxPushCurrent_v2", "Failed to push CUDA context")?;

        let result = (|| {
            // プリセット設定を取得
            let mut preset_config: sys::NV_ENC_PRESET_CONFIG = unsafe { std::mem::zeroed() };
            preset_config.version = sys::NV_ENC_PRESET_CONFIG_VER;
            preset_config.presetCfg.version = sys::NV_ENC_CONFIG_VER;

            let status = unsafe {
                (self.encoder.nvEncGetEncodePresetConfigEx.unwrap())(
                    self.h_encoder,
                    sys::NV_ENC_CODEC_HEVC_GUID,
                    sys::NV_ENC_PRESET_P4_GUID,
                    sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY,
                    &mut preset_config,
                )
            };
            Error::check(
                status,
                "nvEncGetEncodePresetConfigEx",
                "Failed to get preset configuration",
            )?;

            // エンコーダーパラメータを初期化
            let mut init_params: sys::NV_ENC_INITIALIZE_PARAMS = unsafe { std::mem::zeroed() };
            let mut config: sys::NV_ENC_CONFIG = preset_config.presetCfg;

            init_params.version = sys::NV_ENC_INITIALIZE_PARAMS_VER;
            init_params.encodeGUID = sys::NV_ENC_CODEC_HEVC_GUID;
            init_params.presetGUID = sys::NV_ENC_PRESET_P4_GUID;
            init_params.encodeWidth = self.state.width;
            init_params.encodeHeight = self.state.height;
            init_params.darWidth = self.state.width;
            init_params.darHeight = self.state.height;
            init_params.frameRateNum = 30;
            init_params.frameRateDen = 1;
            init_params.enablePTD = 1;
            init_params.encodeConfig = &mut config;
            init_params.maxEncodeWidth = self.state.width;
            init_params.maxEncodeHeight = self.state.height;
            init_params.tuningInfo = sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY;

            config.version = sys::NV_ENC_CONFIG_VER;
            config.profileGUID = sys::NV_ENC_HEVC_PROFILE_MAIN_GUID;
            config.gopLength = sys::NVENC_INFINITE_GOPLENGTH;
            config.frameIntervalP = 1;

            // HEVC 固有の設定
            config.encodeCodecConfig.hevcConfig.idrPeriod = config.gopLength;

            // エンコーダーを初期化
            let status = unsafe {
                (self.encoder.nvEncInitializeEncoder.unwrap())(self.h_encoder, &mut init_params)
            };
            Error::check(
                status,
                "nvEncInitializeEncoder",
                "Failed to initialize encoder",
            )?;

            Ok(())
        })();

        // 初期化後に context を pop
        unsafe { sys::cuCtxPopCurrent_v2(ptr::null_mut()) };

        result
    }

    /// NV12 形式の1フレームをエンコードする
    pub fn encode_frame(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        let expected_size = (self.state.width * self.state.height * 3 / 2) as usize;

        if nv12_data.len() != expected_size {
            return Err(Error::new(
                sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
                "encode_frame",
                "Invalid NV12 data size",
            ));
        }

        unsafe {
            // CUDA context を push
            let status = sys::cuCtxPushCurrent_v2(self.ctx);
            Error::check(status, "cuCtxPushCurrent_v2", "Failed to push CUDA context")?;

            let result = self.encode_frame_inner(nv12_data);

            // context を pop
            sys::cuCtxPopCurrent_v2(ptr::null_mut());

            result
        }
    }

    unsafe fn encode_frame_inner(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        // 入力用のデバイスメモリを割り当て
        let mut device_input = 0u64;
        let status = unsafe { sys::cuMemAlloc_v2(&mut device_input, nv12_data.len()) };
        Error::check(status, "cuMemAlloc_v2", "Failed to allocate device memory")?;

        // データをデバイスにコピー
        let status = unsafe {
            sys::cuMemcpyHtoD_v2(
                device_input,
                nv12_data.as_ptr() as *const c_void,
                nv12_data.len(),
            )
        };
        Error::check(status, "cuMemcpyHtoD_v2", "Failed to copy data to device").inspect_err(
            |_| unsafe {
                sys::cuMemFree_v2(device_input);
            },
        )?;

        // CUDA デバイスメモリを入力リソースとして登録
        let mut register_resource: sys::NV_ENC_REGISTER_RESOURCE = unsafe { std::mem::zeroed() };
        register_resource.version = sys::NV_ENC_REGISTER_RESOURCE_VER;
        register_resource.resourceType =
            sys::_NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR;
        register_resource.resourceToRegister = device_input as *mut c_void;
        register_resource.width = self.state.width;
        register_resource.height = self.state.height;
        register_resource.pitch = self.state.width;
        register_resource.bufferFormat = self.state.buffer_format;
        register_resource.bufferUsage = sys::_NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE;

        let status = unsafe {
            (self.encoder.nvEncRegisterResource.unwrap())(self.h_encoder, &mut register_resource)
        };
        Error::check(
            status,
            "nvEncRegisterResource",
            "Failed to register input resource",
        )
        .inspect_err(|_| unsafe {
            sys::cuMemFree_v2(device_input);
        })?;

        let registered_resource = register_resource.registeredResource;

        // 登録したリソースをマップ
        let mut map_input_resource: sys::NV_ENC_MAP_INPUT_RESOURCE = unsafe { std::mem::zeroed() };
        map_input_resource.version = sys::NV_ENC_MAP_INPUT_RESOURCE_VER;
        map_input_resource.registeredResource = registered_resource;

        let status = unsafe {
            (self.encoder.nvEncMapInputResource.unwrap())(self.h_encoder, &mut map_input_resource)
        };
        Error::check(
            status,
            "nvEncMapInputResource",
            "Failed to map input resource",
        )
        .inspect_err(|_| unsafe {
            (self.encoder.nvEncUnregisterResource.unwrap())(self.h_encoder, registered_resource);
            sys::cuMemFree_v2(device_input);
        })?;

        let mapped_resource = map_input_resource.mappedResource;

        // 出力ビットストリームバッファを割り当て
        let mut create_bitstream: sys::NV_ENC_CREATE_BITSTREAM_BUFFER =
            unsafe { std::mem::zeroed() };
        create_bitstream.version = sys::NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

        let status = unsafe {
            (self.encoder.nvEncCreateBitstreamBuffer.unwrap())(
                self.h_encoder,
                &mut create_bitstream,
            )
        };
        Error::check(
            status,
            "nvEncCreateBitstreamBuffer",
            "Failed to create bitstream buffer",
        )
        .inspect_err(|_| unsafe {
            (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
            (self.encoder.nvEncUnregisterResource.unwrap())(self.h_encoder, registered_resource);
            sys::cuMemFree_v2(device_input);
        })?;

        let output_buffer = create_bitstream.bitstreamBuffer;

        // エンコードピクチャパラメータを設定
        let mut pic_params: sys::NV_ENC_PIC_PARAMS = unsafe { std::mem::zeroed() };
        pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
        pic_params.inputWidth = self.state.width;
        pic_params.inputHeight = self.state.height;
        pic_params.inputPitch = self.state.width;
        pic_params.inputBuffer = mapped_resource;
        pic_params.outputBitstream = output_buffer;
        pic_params.bufferFmt = self.state.buffer_format;
        pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;

        // ピクチャをエンコード
        let status =
            unsafe { (self.encoder.nvEncEncodePicture.unwrap())(self.h_encoder, &mut pic_params) };
        Error::check(status, "nvEncEncodePicture", "Failed to encode picture").inspect_err(
            |_| unsafe {
                (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
                (self.encoder.nvEncUnregisterResource.unwrap())(
                    self.h_encoder,
                    registered_resource,
                );
                (self.encoder.nvEncDestroyBitstreamBuffer.unwrap())(self.h_encoder, output_buffer);
                sys::cuMemFree_v2(device_input);
            },
        )?;

        // ビットストリームをロックしてエンコード済みデータを読み取る
        let mut lock_bitstream: sys::NV_ENC_LOCK_BITSTREAM = unsafe { std::mem::zeroed() };
        lock_bitstream.version = sys::NV_ENC_LOCK_BITSTREAM_VER;
        lock_bitstream.outputBitstream = output_buffer;

        let status = unsafe {
            (self.encoder.nvEncLockBitstream.unwrap())(self.h_encoder, &mut lock_bitstream)
        };
        Error::check(status, "nvEncLockBitstream", "Failed to lock bitstream").inspect_err(
            |_| unsafe {
                (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
                (self.encoder.nvEncUnregisterResource.unwrap())(
                    self.h_encoder,
                    registered_resource,
                );
                (self.encoder.nvEncDestroyBitstreamBuffer.unwrap())(self.h_encoder, output_buffer);
                sys::cuMemFree_v2(device_input);
            },
        )?;

        // エンコード済みデータをコピー
        let encoded_data = unsafe {
            std::slice::from_raw_parts(
                lock_bitstream.bitstreamBufferPtr as *const u8,
                lock_bitstream.bitstreamSizeInBytes as usize,
            )
            .to_vec()
        };

        let timestamp = lock_bitstream.outputTimeStamp;
        let picture_type = lock_bitstream.pictureType;

        // ビットストリームをアンロック
        unsafe {
            (self.encoder.nvEncUnlockBitstream.unwrap())(
                self.h_encoder,
                lock_bitstream.outputBitstream,
            );
        }

        // 入力リソースをアンマップ
        unsafe {
            (self.encoder.nvEncUnmapInputResource.unwrap())(self.h_encoder, mapped_resource);
        }

        // リソースを登録解除
        unsafe {
            (self.encoder.nvEncUnregisterResource.unwrap())(self.h_encoder, registered_resource);
        }

        // ビットストリームバッファを破棄
        unsafe {
            (self.encoder.nvEncDestroyBitstreamBuffer.unwrap())(self.h_encoder, output_buffer);
        }

        // デバイスメモリを解放
        unsafe {
            sys::cuMemFree_v2(device_input);
        }

        // エンコード済みパケットを保存
        self.state.encoded_packets.push(EncodedPacket {
            data: encoded_data,
            timestamp,
            picture_type,
        });

        Ok(())
    }

    /// エンコーダーをフラッシュし、残りのパケットを取得する
    pub fn flush(&mut self) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS;

            let status =
                (self.encoder.nvEncEncodePicture.unwrap())(self.h_encoder, &mut pic_params);
            Error::check(status, "nvEncEncodePicture", "Failed to flush encoder")?;

            Ok(())
        }
    }

    /// すべてのエンコード済みパケットを取得する
    pub fn get_encoded_packets(&mut self) -> Vec<EncodedPacket> {
        std::mem::take(&mut self.state.encoded_packets)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            if !self.h_encoder.is_null() {
                // クリーンアップ前に context をアクティブ化
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

        // エンコーダーを作成
        let mut encoder = Encoder::new_hevc(width, height).expect("Failed to create HEVC encoder");

        // NV12 形式の黒フレームを準備
        // Y プレーン: 16 (YUV での黒)
        // UV プレーン: 128 (ニュートラルなクロマ)
        let y_size = (width * height) as usize;
        let uv_size = (width * height / 2) as usize;

        let mut frame_data = vec![16u8; y_size + uv_size];
        // UV プレーンを 128 に設定（ニュートラルなクロマ）
        frame_data[y_size..].fill(128);

        // フレームをエンコード
        encoder
            .encode_frame(&frame_data)
            .expect("Failed to encode black frame");

        // エンコーダーをフラッシュ
        encoder.flush().expect("Failed to flush encoder");

        // エンコード済みパケットを取得
        let packets = encoder.get_encoded_packets();

        // 少なくとも1つのパケットを取得したことを確認
        assert!(!packets.is_empty(), "No encoded packets received");

        // 最初のパケットがキーフレーム（IDR）であることを確認
        let first_packet = &packets[0];
        assert!(
            matches!(
                first_packet.picture_type,
                sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_I | sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_IDR
            ),
            "First frame should be a keyframe"
        );

        // パケットにデータがあることを確認
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
