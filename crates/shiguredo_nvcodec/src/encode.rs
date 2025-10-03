use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;

use crate::{Error, ensure_cuda_initialized, sys};

/// エンコーダー
pub struct Encoder {
    ctx: sys::CUcontext,
    encoder: sys::NV_ENCODE_API_FUNCTION_LIST,
    h_encoder: *mut c_void,
    width: u32,
    height: u32,
    buffer_format: sys::NV_ENC_BUFFER_FORMAT,
    encoded_frames: VecDeque<EncodedFrame>,
}

impl Encoder {
    /// H.265 エンコーダーインスタンスを生成する
    pub fn new_h265(width: u32, height: u32) -> Result<Self, Error> {
        // CUDA ドライバーの初期化（プロセスごとに1回だけ実行される）
        ensure_cuda_initialized()?;

        unsafe {
            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            let device_id = 0; // プライマリGPUデバイスを使用 // TODO(atode): make configurable
            let status = sys::cuCtxCreate_v2(&mut ctx, ctx_flags, device_id);
            Error::check(status, "cuCtxCreate_v2", "failed to create CUDA context")?;

            let ctx_guard = crate::ReleaseGuard::new(|| {
                sys::cuCtxDestroy_v2(ctx);
            });

            // NVENC 操作のために CUDA context をアクティブ化し、エンコードセッションを開く
            let (encoder_api, h_encoder) = crate::with_cuda_context(ctx, || {
                // NVENC API をロード
                let mut encoder_api: sys::NV_ENCODE_API_FUNCTION_LIST = std::mem::zeroed();
                encoder_api.version = sys::NV_ENCODE_API_FUNCTION_LIST_VER;

                let status = sys::NvEncodeAPICreateInstance(&mut encoder_api);
                Error::check(
                    status,
                    "NvEncodeAPICreateInstance",
                    "failed to create NVENC API instance",
                )?;

                // エンコードセッションを開く
                let mut open_session_params: sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS =
                    std::mem::zeroed();
                open_session_params.version = sys::NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
                open_session_params.deviceType = sys::_NV_ENC_DEVICE_TYPE_NV_ENC_DEVICE_TYPE_CUDA;
                open_session_params.device = ctx.cast();
                open_session_params.apiVersion = sys::NVENCAPI_VERSION;

                let mut h_encoder = ptr::null_mut();
                let status = encoder_api
                    .nvEncOpenEncodeSessionEx
                    .map(|f| f(&mut open_session_params, &mut h_encoder))
                    .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
                Error::check(
                    status,
                    "nvEncOpenEncodeSessionEx",
                    "failed to open encode session",
                )?;

                Ok((encoder_api, h_encoder))
            })?;

            // ここまで成功したらクリーンアップをキャンセル（あとはDrop に任せる）
            ctx_guard.cancel();

            let mut encoder = Self {
                ctx,
                encoder: encoder_api,
                h_encoder,
                width,
                height,
                buffer_format: sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_NV12,
                encoded_frames: VecDeque::new(),
            };

            // デフォルトパラメータでエンコーダーを初期化
            crate::with_cuda_context(ctx, || encoder.initialize_encoder())?;

            Ok(encoder)
        }
    }

    fn initialize_encoder(&mut self) -> Result<(), Error> {
        unsafe {
            // プリセット設定を取得
            let mut preset_config: sys::NV_ENC_PRESET_CONFIG = std::mem::zeroed();
            preset_config.version = sys::NV_ENC_PRESET_CONFIG_VER;
            preset_config.presetCfg.version = sys::NV_ENC_CONFIG_VER;

            let status = self
                .encoder
                .nvEncGetEncodePresetConfigEx
                .map(|f| {
                    f(
                        self.h_encoder,
                        sys::NV_ENC_CODEC_HEVC_GUID,
                        sys::NV_ENC_PRESET_P4_GUID, // TODO(atode): make configurable
                        sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY, // TODO(atode): make configurable
                        &mut preset_config,
                    )
                })
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(
                status,
                "nvEncGetEncodePresetConfigEx",
                "failed to get preset configuration",
            )?;

            // エンコーダーパラメータを初期化
            let mut init_params: sys::NV_ENC_INITIALIZE_PARAMS = std::mem::zeroed();
            let mut config: sys::NV_ENC_CONFIG = preset_config.presetCfg;

            init_params.version = sys::NV_ENC_INITIALIZE_PARAMS_VER;
            init_params.encodeGUID = sys::NV_ENC_CODEC_HEVC_GUID;
            init_params.presetGUID = sys::NV_ENC_PRESET_P4_GUID; // TODO(atode): make configurable
            init_params.encodeWidth = self.width;
            init_params.encodeHeight = self.height;
            init_params.darWidth = self.width;
            init_params.darHeight = self.height;
            init_params.frameRateNum = 30; // TODO(atode): make configurable
            init_params.frameRateDen = 1; // TODO(atode): make configurable
            init_params.enablePTD = 1;
            init_params.encodeConfig = &mut config;
            init_params.maxEncodeWidth = self.width;
            init_params.maxEncodeHeight = self.height;
            init_params.tuningInfo = sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY;

            config.version = sys::NV_ENC_CONFIG_VER;
            config.profileGUID = sys::NV_ENC_HEVC_PROFILE_MAIN_GUID; // TODO(atode): make configurable
            config.gopLength = sys::NVENC_INFINITE_GOPLENGTH;
            config.frameIntervalP = 1;

            // HEVC 固有の設定
            config.encodeCodecConfig.hevcConfig.idrPeriod = config.gopLength; // TODO(atode): make configurable

            // エンコーダーを初期化
            let status = self
                .encoder
                .nvEncInitializeEncoder
                .map(|f| f(self.h_encoder, &mut init_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(
                status,
                "nvEncInitializeEncoder",
                "failed to initialize encoder",
            )?;

            Ok(())
        }
    }

    /// NV12 形式の1フレームをエンコードする
    pub fn encode_frame(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        let expected_size = (self.width * self.height * 3 / 2) as usize;

        if nv12_data.len() != expected_size {
            return Err(Error::new(
                sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
                "encode_frame",
                "Invalid NV12 data size",
            ));
        }

        crate::with_cuda_context(self.ctx, || self.encode_frame_inner(nv12_data))
    }

    fn encode_frame_inner(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        // 入力用のデバイスメモリを割り当て
        let mut device_input = 0u64;
        let status = unsafe { sys::cuMemAlloc_v2(&mut device_input, nv12_data.len()) };
        Error::check(status, "cuMemAlloc_v2", "failed to allocate device memory")?;

        let _device_guard = crate::ReleaseGuard::new(|| unsafe {
            sys::cuMemFree_v2(device_input);
        });

        // データをデバイスにコピー
        let status = unsafe {
            sys::cuMemcpyHtoD_v2(
                device_input,
                nv12_data.as_ptr() as *const c_void,
                nv12_data.len(),
            )
        };
        Error::check(status, "cuMemcpyHtoD_v2", "failed to copy data to device")?;

        // CUDA デバイスメモリを入力リソースとして登録
        let mut register_resource: sys::NV_ENC_REGISTER_RESOURCE = unsafe { std::mem::zeroed() };
        register_resource.version = sys::NV_ENC_REGISTER_RESOURCE_VER;
        register_resource.resourceType =
            sys::_NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR;
        register_resource.resourceToRegister = device_input as *mut c_void;
        register_resource.width = self.width;
        register_resource.height = self.height;
        register_resource.pitch = self.width;
        register_resource.bufferFormat = self.buffer_format;
        register_resource.bufferUsage = sys::_NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE;

        let status = unsafe {
            self.encoder
                .nvEncRegisterResource
                .map(|f| f(self.h_encoder, &mut register_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR)
        };
        Error::check(
            status,
            "nvEncRegisterResource",
            "failed to register input resource",
        )?;

        let registered_resource = register_resource.registeredResource;

        let _registered_guard = crate::ReleaseGuard::new(|| unsafe {
            self.encoder
                .nvEncUnregisterResource
                .map(|f| f(self.h_encoder, registered_resource));
        });

        // 登録したリソースをマップ
        let mut map_input_resource: sys::NV_ENC_MAP_INPUT_RESOURCE = unsafe { std::mem::zeroed() };
        map_input_resource.version = sys::NV_ENC_MAP_INPUT_RESOURCE_VER;
        map_input_resource.registeredResource = registered_resource;

        let status = unsafe {
            self.encoder
                .nvEncMapInputResource
                .map(|f| f(self.h_encoder, &mut map_input_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR)
        };
        Error::check(
            status,
            "nvEncMapInputResource",
            "failed to map input resource",
        )?;

        let mapped_resource = map_input_resource.mappedResource;

        let _mapped_guard = crate::ReleaseGuard::new(|| unsafe {
            self.encoder
                .nvEncUnmapInputResource
                .map(|f| f(self.h_encoder, mapped_resource));
        });

        // 出力ビットストリームバッファを割り当て
        let mut create_bitstream: sys::NV_ENC_CREATE_BITSTREAM_BUFFER =
            unsafe { std::mem::zeroed() };
        create_bitstream.version = sys::NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

        let status = unsafe {
            self.encoder
                .nvEncCreateBitstreamBuffer
                .map(|f| f(self.h_encoder, &mut create_bitstream))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR)
        };
        Error::check(
            status,
            "nvEncCreateBitstreamBuffer",
            "failed to create bitstream buffer",
        )?;

        let output_buffer = create_bitstream.bitstreamBuffer;

        let _bitstream_guard = crate::ReleaseGuard::new(|| unsafe {
            self.encoder
                .nvEncDestroyBitstreamBuffer
                .map(|f| f(self.h_encoder, output_buffer));
        });

        // エンコードピクチャパラメータを設定
        let mut pic_params: sys::NV_ENC_PIC_PARAMS = unsafe { std::mem::zeroed() };
        pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
        pic_params.inputWidth = self.width;
        pic_params.inputHeight = self.height;
        pic_params.inputPitch = self.width;
        pic_params.inputBuffer = mapped_resource;
        pic_params.outputBitstream = output_buffer;
        pic_params.bufferFmt = self.buffer_format;
        pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;

        // ピクチャをエンコード
        let status = unsafe {
            self.encoder
                .nvEncEncodePicture
                .map(|f| f(self.h_encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR)
        };
        Error::check(status, "nvEncEncodePicture", "failed to encode picture")?;

        // ビットストリームをロックしてエンコード済みデータを読み取る
        let mut lock_bitstream: sys::NV_ENC_LOCK_BITSTREAM = unsafe { std::mem::zeroed() };
        lock_bitstream.version = sys::NV_ENC_LOCK_BITSTREAM_VER;
        lock_bitstream.outputBitstream = output_buffer;

        let status = unsafe {
            self.encoder
                .nvEncLockBitstream
                .map(|f| f(self.h_encoder, &mut lock_bitstream))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR)
        };
        Error::check(status, "nvEncLockBitstream", "failed to lock bitstream")?;

        let _lock_guard = crate::ReleaseGuard::new(|| unsafe {
            self.encoder
                .nvEncUnlockBitstream
                .map(|f| f(self.h_encoder, lock_bitstream.outputBitstream));
        });

        // エンコード済みデータをコピー
        let encoded_data = unsafe {
            std::slice::from_raw_parts(
                lock_bitstream.bitstreamBufferPtr as *const u8,
                lock_bitstream.bitstreamSizeInBytes as usize,
            )
        }
        .to_vec();

        let timestamp = lock_bitstream.outputTimeStamp;
        let picture_type = lock_bitstream.pictureType;

        // エンコード済みフレームを保存
        self.encoded_frames.push_back(EncodedFrame {
            data: encoded_data,
            timestamp,
            picture_type: picture_type.into(),
        });

        Ok(())
    }

    /// エンコーダーをフラッシュし、残りのフレームを取得する
    pub fn flush(&mut self) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS;

            let status = self
                .encoder
                .nvEncEncodePicture
                .map(|f| f(self.h_encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(status, "nvEncEncodePicture", "failed to flush encoder")?;

            Ok(())
        }
    }

    /// 次のエンコード済みフレームを取得する
    pub fn next_frame(&mut self) -> Option<EncodedFrame> {
        self.encoded_frames.pop_front()
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = crate::with_cuda_context(self.ctx, || {
                if let Some(destroy_fn) = self.encoder.nvEncDestroyEncoder {
                    destroy_fn(self.h_encoder);
                }
                Ok(())
            });

            sys::cuCtxDestroy_v2(self.ctx);
        }
    }
}

impl std::fmt::Debug for Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder")
            .field("ctx", &format_args!("{:p}", self.ctx))
            .field("h_encoder", &format_args!("{:p}", self.h_encoder))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("buffer_format", &self.buffer_format)
            .finish()
    }
}

/// ピクチャータイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureType {
    /// P フレーム
    P,
    /// B フレーム
    B,
    /// I フレーム
    I,
    /// IDR フレーム
    Idr,
    /// BI フレーム
    Bi,
    /// スキップされたフレーム
    Skipped,
    /// イントラリフレッシュフレーム
    IntraRefresh,
    /// 非参照 P フレーム
    NonRefP,
    /// スイッチフレーム
    Switch,
    /// 不明なフレームタイプ
    Unknown,
}

impl From<sys::NV_ENC_PIC_TYPE> for PictureType {
    fn from(pic_type: sys::NV_ENC_PIC_TYPE) -> Self {
        match pic_type {
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_P => PictureType::P,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_B => PictureType::B,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_I => PictureType::I,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_IDR => PictureType::Idr,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_BI => PictureType::Bi,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_SKIPPED => PictureType::Skipped,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_INTRA_REFRESH => PictureType::IntraRefresh,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_NONREF_P => PictureType::NonRefP,
            sys::_NV_ENC_PIC_TYPE_NV_ENC_PIC_TYPE_SWITCH => PictureType::Switch,
            _ => PictureType::Unknown,
        }
    }
}

/// エンコード済みフレーム
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    data: Vec<u8>,
    timestamp: u64,
    picture_type: PictureType,
}

impl EncodedFrame {
    /// エンコードされたデータを取得する
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// タイムスタンプを取得する
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// ピクチャータイプを取得する
    pub fn picture_type(&self) -> PictureType {
        self.picture_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_h265_encoder() {
        let _encoder = Encoder::new_h265(640, 480).expect("failed to initialize h265 encoder");
        println!("h265 encoder initialized successfully");
    }

    #[test]
    fn test_encode_black_frame() {
        let width = 640;
        let height = 480;

        // エンコーダーを作成
        let mut encoder = Encoder::new_h265(width, height).expect("failed to create h265 encoder");

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
            .expect("failed to encode black frame");

        // エンコーダーをフラッシュ
        encoder.flush().expect("failed to flush encoder");

        // エンコード済みフレームを取得
        let mut frames = Vec::new();
        while let Some(frame) = encoder.next_frame() {
            frames.push(frame);
        }

        // 少なくとも1つのフレームを取得したことを確認
        assert!(!frames.is_empty(), "No encoded frames received");

        // 最初のフレームがキーフレーム（IDR）であることを確認
        let first_frame = &frames[0];
        assert!(
            matches!(first_frame.picture_type, PictureType::I | PictureType::Idr),
            "First frame should be a keyframe"
        );

        // フレームにデータがあることを確認
        assert!(
            !first_frame.data.is_empty(),
            "Encoded frame should have data"
        );

        println!(
            "Successfully encoded black frame: {} frames, first frame size: {} bytes",
            frames.len(),
            first_frame.data.len()
        );
    }
}
