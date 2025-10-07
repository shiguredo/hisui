use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;

use crate::{Error, ReleaseGuard, ensure_cuda_initialized, sys};

/// プリセット
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset(sys::GUID);

impl Preset {
    /// P1プリセット（最高速）
    pub const P1: Self = Self(sys::NV_ENC_PRESET_P1_GUID);

    /// P2プリセット
    pub const P2: Self = Self(sys::NV_ENC_PRESET_P2_GUID);

    /// P3プリセット
    pub const P3: Self = Self(sys::NV_ENC_PRESET_P3_GUID);

    /// P4プリセット（バランス型）
    pub const P4: Self = Self(sys::NV_ENC_PRESET_P4_GUID);

    /// P5プリセット
    pub const P5: Self = Self(sys::NV_ENC_PRESET_P5_GUID);

    /// P6プリセット
    pub const P6: Self = Self(sys::NV_ENC_PRESET_P6_GUID);

    /// P7プリセット（最高品質）
    pub const P7: Self = Self(sys::NV_ENC_PRESET_P7_GUID);

    fn to_sys(self) -> sys::GUID {
        self.0
    }
}

/// チューニング情報
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuningInfo(sys::NV_ENC_TUNING_INFO);

impl TuningInfo {
    /// 高品質
    pub const HIGH_QUALITY: Self = Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_HIGH_QUALITY);

    /// 低遅延
    pub const LOW_LATENCY: Self = Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOW_LATENCY);

    /// 超低遅延
    pub const ULTRA_LOW_LATENCY: Self =
        Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY);

    /// ロスレス
    pub const LOSSLESS: Self = Self(sys::NV_ENC_TUNING_INFO_NV_ENC_TUNING_INFO_LOSSLESS);

    fn to_sys(self) -> sys::NV_ENC_TUNING_INFO {
        self.0
    }
}

/// プロファイル
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile(sys::GUID);

impl Profile {
    /// 自動選択プロファイル
    pub const AUTO_SELECT: Self = Self(sys::NV_ENC_CODEC_PROFILE_AUTOSELECT_GUID);

    // H.264 プロファイル
    /// H.264 Baseline プロファイル
    pub const H264_BASELINE: Self = Self(sys::NV_ENC_H264_PROFILE_BASELINE_GUID);
    /// H.264 Main プロファイル
    pub const H264_MAIN: Self = Self(sys::NV_ENC_H264_PROFILE_MAIN_GUID);
    /// H.264 High プロファイル
    pub const H264_HIGH: Self = Self(sys::NV_ENC_H264_PROFILE_HIGH_GUID);
    /// H.264 High 10 プロファイル
    pub const H264_HIGH_10: Self = Self(sys::NV_ENC_H264_PROFILE_HIGH_10_GUID);
    /// H.264 High 422 プロファイル
    pub const H264_HIGH_422: Self = Self(sys::NV_ENC_H264_PROFILE_HIGH_422_GUID);
    /// H.264 High 444 プロファイル
    pub const H264_HIGH_444: Self = Self(sys::NV_ENC_H264_PROFILE_HIGH_444_GUID);
    /// H.264 Stereo プロファイル
    pub const H264_STEREO: Self = Self(sys::NV_ENC_H264_PROFILE_STEREO_GUID);
    /// H.264 Progressive High プロファイル
    pub const H264_PROGRESSIVE_HIGH: Self = Self(sys::NV_ENC_H264_PROFILE_PROGRESSIVE_HIGH_GUID);
    /// H.264 Constrained High プロファイル
    pub const H264_CONSTRAINED_HIGH: Self = Self(sys::NV_ENC_H264_PROFILE_CONSTRAINED_HIGH_GUID);

    // HEVC プロファイル
    /// HEVC Main プロファイル
    pub const HEVC_MAIN: Self = Self(sys::NV_ENC_HEVC_PROFILE_MAIN_GUID);
    /// HEVC Main10 プロファイル
    pub const HEVC_MAIN10: Self = Self(sys::NV_ENC_HEVC_PROFILE_MAIN10_GUID);
    /// HEVC FREXT プロファイル (Main 422/444 8/10 bit)
    pub const HEVC_FREXT: Self = Self(sys::NV_ENC_HEVC_PROFILE_FREXT_GUID);

    // AV1 プロファイル
    /// AV1 Main プロファイル
    pub const AV1_MAIN: Self = Self(sys::NV_ENC_AV1_PROFILE_MAIN_GUID);

    fn to_sys(self) -> sys::GUID {
        self.0
    }
}

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// 入出力画像の幅
    pub width: u32,

    /// 入出力画像の高さ
    pub height: u32,

    /// 最大エンコード幅（動的解像度変更用）
    /// None の場合は width と同じ値が使用される
    pub max_encode_width: Option<u32>,

    /// 最大エンコード高さ（動的解像度変更用）
    /// None の場合は height と同じ値が使用される
    pub max_encode_height: Option<u32>,

    /// FPS の分子
    pub fps_numerator: u32,

    /// FPS の分母
    pub fps_denominator: u32,

    /// ビットレート (bps 単位)
    /// None の場合はレート制御モードが ConstQp である必要がある
    pub target_bitrate: Option<u32>,

    /// プリセット GUID (品質と速度のバランス)
    pub preset: Preset,

    /// チューニング情報
    pub tuning_info: TuningInfo,

    /// レート制御モード
    pub rate_control_mode: RateControlMode,

    /// GOP長
    /// None の場合は無限GOP (NVENC_INFINITE_GOPLENGTH) が使用される
    pub gop_length: Option<u32>,

    /// IDRフレーム間隔
    /// None の場合は gop_length と同じ値が使用される
    pub idr_period: Option<u32>,

    /// Pフレーム間隔
    pub frame_interval_p: u32,

    /// プロファイル
    /// None の場合は自動選択またはコーデックのデフォルトプロファイルが使用される
    pub profile: Option<Profile>,

    /// デバイスID (使用するGPU)
    pub device_id: i32,
}

/// レート制御モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    /// Constant QP mode
    ConstQp,

    /// Variable bitrate mode
    Vbr,

    /// Constant bitrate mode
    Cbr,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            max_encode_width: None,
            max_encode_height: None,
            fps_numerator: 30,
            fps_denominator: 1,
            target_bitrate: Some(5_000_000), // 5 Mbps
            preset: Preset::P4,              // バランスの良いプリセット
            tuning_info: TuningInfo::LOW_LATENCY,
            rate_control_mode: RateControlMode::Vbr,
            gop_length: None, // 無限GOP
            idr_period: None, // gop_length と同じ
            frame_interval_p: 1,
            profile: None,
            device_id: 0, // プライマリGPU
        }
    }
}

impl RateControlMode {
    fn to_sys(self) -> sys::NV_ENC_PARAMS_RC_MODE {
        match self {
            RateControlMode::ConstQp => sys::_NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_CONSTQP,
            RateControlMode::Vbr => sys::_NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_VBR,
            RateControlMode::Cbr => sys::_NV_ENC_PARAMS_RC_MODE_NV_ENC_PARAMS_RC_CBR,
        }
    }
}

/// エンコーダー
pub struct Encoder {
    ctx: sys::CUcontext,
    encoder: sys::NV_ENCODE_API_FUNCTION_LIST,
    h_encoder: *mut c_void,
    width: u32,
    height: u32,
    buffer_format: sys::NV_ENC_BUFFER_FORMAT,
    encoded_frames: VecDeque<EncodedFrame>,
    fps_denominator: u64,
    frame_count: u64,
}

impl Encoder {
    /// H.264 エンコーダーインスタンスを生成する
    pub fn new_h264(config: EncoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(
            config,
            sys::NV_ENC_CODEC_H264_GUID,
            sys::NV_ENC_H264_PROFILE_MAIN_GUID,
        )
    }

    /// H.265 エンコーダーインスタンスを生成する
    pub fn new_h265(config: EncoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(
            config,
            sys::NV_ENC_CODEC_HEVC_GUID,
            sys::NV_ENC_HEVC_PROFILE_MAIN_GUID,
        )
    }

    /// AV1 エンコーダーインスタンスを生成する
    pub fn new_av1(config: EncoderConfig) -> Result<Self, Error> {
        Self::new_with_codec(
            config,
            sys::NV_ENC_CODEC_AV1_GUID,
            sys::NV_ENC_AV1_PROFILE_MAIN_GUID,
        )
    }

    /// 指定されたコーデックタイプでエンコーダーインスタンスを生成する
    fn new_with_codec(
        config: EncoderConfig,
        codec_guid: sys::GUID,
        profile_guid: sys::GUID,
    ) -> Result<Self, Error> {
        // CUDA ドライバーの初期化（プロセスごとに1回だけ実行される）
        ensure_cuda_initialized()?;

        unsafe {
            let mut ctx = ptr::null_mut();

            // CUDA context の初期化
            let ctx_flags = 0; // デフォルトのコンテキストフラグ
            let status = sys::cuCtxCreate_v2(&mut ctx, ctx_flags, config.device_id);
            Error::check(status, "cuCtxCreate_v2", "failed to create CUDA context")?;

            let ctx_guard = ReleaseGuard::new(|| {
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
                width: config.width,
                height: config.height,
                buffer_format: sys::_NV_ENC_BUFFER_FORMAT_NV_ENC_BUFFER_FORMAT_NV12,
                encoded_frames: VecDeque::new(),
                fps_denominator: config.fps_denominator as u64,
                frame_count: 0,
            };

            // デフォルトパラメータでエンコーダーを初期化
            crate::with_cuda_context(ctx, || {
                encoder.initialize_encoder(&config, codec_guid, profile_guid)
            })?;

            Ok(encoder)
        }
    }

    fn initialize_encoder(
        &mut self,
        config: &EncoderConfig,
        codec_guid: sys::GUID,
        default_profile: sys::GUID,
    ) -> Result<(), Error> {
        unsafe {
            // プロファイルの決定: 指定されたプロファイルを優先、なければデフォルトを使用
            let profile = config
                .profile
                .map(|p| p.to_sys())
                .unwrap_or(default_profile);

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
                        codec_guid,
                        config.preset.to_sys(),
                        config.tuning_info.to_sys(),
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
            let mut encode_config: sys::NV_ENC_CONFIG = preset_config.presetCfg;

            init_params.version = sys::NV_ENC_INITIALIZE_PARAMS_VER;
            init_params.encodeGUID = codec_guid;
            init_params.presetGUID = config.preset.to_sys();
            init_params.encodeWidth = config.width;
            init_params.encodeHeight = config.height;
            init_params.darWidth = config.width;
            init_params.darHeight = config.height;
            init_params.frameRateNum = config.fps_numerator;
            init_params.frameRateDen = config.fps_denominator;
            init_params.enablePTD = 1;
            init_params.encodeConfig = &mut encode_config;
            init_params.maxEncodeWidth = config.max_encode_width.unwrap_or(config.width);
            init_params.maxEncodeHeight = config.max_encode_height.unwrap_or(config.height);
            init_params.tuningInfo = config.tuning_info.to_sys();

            encode_config.version = sys::NV_ENC_CONFIG_VER;
            encode_config.profileGUID = profile;
            encode_config.gopLength = config.gop_length.unwrap_or(sys::NVENC_INFINITE_GOPLENGTH);
            encode_config.frameIntervalP = config.frame_interval_p as i32;
            encode_config.rcParams.rateControlMode = config.rate_control_mode.to_sys();

            // ビットレート設定
            if config.rate_control_mode != RateControlMode::ConstQp {
                let bitrate = config.target_bitrate.ok_or_else(|| {
                    Error::new(
                        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
                        "initialize_encoder",
                        "target_bitrate must be specified when not using ConstQp mode",
                    )
                })?;
                encode_config.rcParams.averageBitRate = bitrate;
                encode_config.rcParams.maxBitRate = bitrate;
            }

            let idr_period = config
                .idr_period
                .unwrap_or_else(|| config.gop_length.unwrap_or(sys::NVENC_INFINITE_GOPLENGTH));

            // コーデック固有の設定
            match codec_guid {
                sys::NV_ENC_CODEC_HEVC_GUID => {
                    encode_config.encodeCodecConfig.hevcConfig.idrPeriod = idr_period;
                }
                sys::NV_ENC_CODEC_H264_GUID => {
                    encode_config.encodeCodecConfig.h264Config.idrPeriod = idr_period;
                }
                sys::NV_ENC_CODEC_AV1_GUID => {
                    encode_config.encodeCodecConfig.av1Config.idrPeriod = idr_period;
                }
                _ => {
                    return Err(Error::new(
                        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
                        "initialize_encoder",
                        "unsupported codec GUID",
                    ));
                }
            }

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

    /// シーケンスパラメータ（SPS/PPS または Sequence Header OBU）を取得する
    ///
    /// H.264/HEVC の場合は SPS/PPS、AV1 の場合は Sequence Header OBU を取得します。
    pub fn get_sequence_params(&mut self) -> Result<Vec<u8>, Error> {
        crate::with_cuda_context(self.ctx, || self.get_sequence_params_inner())
    }

    fn get_sequence_params_inner(&mut self) -> Result<Vec<u8>, Error> {
        unsafe {
            // シーケンスパラメータを格納するバッファを確保
            let mut payload_buffer = vec![0u8; sys::NV_MAX_SEQ_HDR_LEN as usize];
            let mut out_size: u32 = 0; // 実際のサイズを受け取る変数

            let mut seq_params: sys::NV_ENC_SEQUENCE_PARAM_PAYLOAD = std::mem::zeroed();
            seq_params.version = sys::NV_ENC_SEQUENCE_PARAM_PAYLOAD_VER;
            seq_params.spsppsBuffer = payload_buffer.as_mut_ptr() as *mut std::ffi::c_void;
            seq_params.inBufferSize = sys::NV_MAX_SEQ_HDR_LEN;
            seq_params.outSPSPPSPayloadSize = &mut out_size;

            let status = self
                .encoder
                .nvEncGetSequenceParams
                .map(|f| f(self.h_encoder, &mut seq_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);

            Error::check(
                status,
                "nvEncGetSequenceParams",
                "failed to get sequence parameters",
            )?;

            // 実際に書き込まれたサイズに合わせてバッファをリサイズ
            payload_buffer.truncate(out_size as usize);

            Ok(payload_buffer)
        }
    }

    /// NV12 形式のフレームをエンコードする
    pub fn encode(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        let expected_size = (self.width * self.height * 3 / 2) as usize;

        if nv12_data.len() != expected_size {
            return Err(Error::new(
                sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM,
                "encode",
                "invalid NV12 data size",
            ));
        }

        crate::with_cuda_context(self.ctx, || self.encode_inner(nv12_data))
    }

    fn encode_inner(&mut self, nv12_data: &[u8]) -> Result<(), Error> {
        // 入力データをデバイスにコピー
        let (device_input, _device_guard) = self.copy_input_data_to_device(nv12_data)?;

        // CUDA デバイスメモリを入力リソースとして登録
        let (registered_resource, _registered_guard) =
            self.register_input_resource(device_input)?;

        // 登録したリソースをマップ
        let (mapped_resource, _mapped_guard) = self.map_input_resource(registered_resource)?;

        // 出力ビットストリームバッファを割り当て
        let (output_buffer, _bitstream_guard) = self.create_output_bitstream_buffer()?;

        // ピクチャをエンコード
        self.encode_picture(mapped_resource, output_buffer)?;

        // ビットストリームをロックしてエンコード済みデータをコピー
        let encoded_frame = self.lock_and_copy_bitstream(output_buffer)?;

        // エンコード済みフレームを保存
        self.encoded_frames.push_back(encoded_frame);

        Ok(())
    }

    fn copy_input_data_to_device(
        &mut self,
        nv12_data: &[u8],
    ) -> Result<(sys::CUdeviceptr, ReleaseGuard<impl FnOnce() + use<>>), Error> {
        unsafe {
            let mut device_input: sys::CUdeviceptr = 0;
            let status = sys::cuMemAlloc_v2(&mut device_input, nv12_data.len());
            Error::check(status, "cuMemAlloc_v2", "failed to allocate device memory")?;

            let device_guard = ReleaseGuard::new(move || {
                sys::cuMemFree_v2(device_input);
            });

            let status =
                sys::cuMemcpyHtoD_v2(device_input, nv12_data.as_ptr().cast(), nv12_data.len());
            Error::check(status, "cuMemcpyHtoD_v2", "failed to copy data to device")?;

            Ok((device_input, device_guard))
        }
    }

    fn register_input_resource(
        &mut self,
        device_input: sys::CUdeviceptr,
    ) -> Result<
        (
            sys::NV_ENC_REGISTERED_PTR,
            ReleaseGuard<impl FnOnce() + use<>>,
        ),
        Error,
    > {
        unsafe {
            let mut register_resource: sys::NV_ENC_REGISTER_RESOURCE = std::mem::zeroed();
            register_resource.version = sys::NV_ENC_REGISTER_RESOURCE_VER;
            register_resource.resourceType =
                sys::_NV_ENC_INPUT_RESOURCE_TYPE_NV_ENC_INPUT_RESOURCE_TYPE_CUDADEVICEPTR;
            register_resource.resourceToRegister = device_input as *mut c_void;
            register_resource.width = self.width;
            register_resource.height = self.height;
            register_resource.pitch = self.width;
            register_resource.bufferFormat = self.buffer_format;
            register_resource.bufferUsage = sys::_NV_ENC_BUFFER_USAGE_NV_ENC_INPUT_IMAGE;

            let status = self
                .encoder
                .nvEncRegisterResource
                .map(|f| f(self.h_encoder, &mut register_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(
                status,
                "nvEncRegisterResource",
                "failed to register input resource",
            )?;

            let registered_resource = register_resource.registeredResource;

            let unregister = self.encoder.nvEncUnregisterResource;
            let h_encoder = self.h_encoder;
            let registered_guard = ReleaseGuard::new(move || {
                unregister.map(|f| f(h_encoder, registered_resource));
            });

            Ok((registered_resource, registered_guard))
        }
    }

    fn map_input_resource(
        &mut self,
        registered_resource: sys::NV_ENC_REGISTERED_PTR,
    ) -> Result<(sys::NV_ENC_INPUT_PTR, ReleaseGuard<impl FnOnce() + use<>>), Error> {
        unsafe {
            let mut map_input_resource: sys::NV_ENC_MAP_INPUT_RESOURCE = std::mem::zeroed();
            map_input_resource.version = sys::NV_ENC_MAP_INPUT_RESOURCE_VER;
            map_input_resource.registeredResource = registered_resource;

            let status = self
                .encoder
                .nvEncMapInputResource
                .map(|f| f(self.h_encoder, &mut map_input_resource))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(
                status,
                "nvEncMapInputResource",
                "failed to map input resource",
            )?;

            let mapped_resource = map_input_resource.mappedResource;

            let unmap = self.encoder.nvEncUnmapInputResource;
            let h_encoder = self.h_encoder;
            let mapped_guard = ReleaseGuard::new(move || {
                unmap.map(|f| f(h_encoder, mapped_resource));
            });

            Ok((mapped_resource, mapped_guard))
        }
    }

    fn create_output_bitstream_buffer(
        &mut self,
    ) -> Result<(sys::NV_ENC_OUTPUT_PTR, ReleaseGuard<impl FnOnce() + use<>>), Error> {
        unsafe {
            let mut create_bitstream: sys::NV_ENC_CREATE_BITSTREAM_BUFFER = std::mem::zeroed();
            create_bitstream.version = sys::NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

            let status = self
                .encoder
                .nvEncCreateBitstreamBuffer
                .map(|f| f(self.h_encoder, &mut create_bitstream))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(
                status,
                "nvEncCreateBitstreamBuffer",
                "failed to create bitstream buffer",
            )?;

            let output_buffer = create_bitstream.bitstreamBuffer;

            let destroy = self.encoder.nvEncDestroyBitstreamBuffer;
            let h_encoder = self.h_encoder;
            let bitstream_guard = ReleaseGuard::new(move || {
                destroy.map(|f| f(h_encoder, output_buffer));
            });

            Ok((output_buffer, bitstream_guard))
        }
    }

    fn encode_picture(
        &mut self,
        mapped_resource: sys::NV_ENC_INPUT_PTR,
        output_buffer: sys::NV_ENC_OUTPUT_PTR,
    ) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.inputWidth = self.width;
            pic_params.inputHeight = self.height;
            pic_params.inputPitch = self.width;
            pic_params.inputBuffer = mapped_resource;
            pic_params.outputBitstream = output_buffer;
            pic_params.bufferFmt = self.buffer_format;
            pic_params.pictureStruct = sys::_NV_ENC_PIC_STRUCT_NV_ENC_PIC_STRUCT_FRAME;
            pic_params.inputTimeStamp = self.frame_count * self.fps_denominator;

            self.frame_count += 1;

            let status = self
                .encoder
                .nvEncEncodePicture
                .map(|f| f(self.h_encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(status, "nvEncEncodePicture", "failed to encode picture")?;

            Ok(())
        }
    }

    fn lock_and_copy_bitstream(
        &mut self,
        output_buffer: sys::NV_ENC_OUTPUT_PTR,
    ) -> Result<EncodedFrame, Error> {
        unsafe {
            let mut lock_bitstream: sys::NV_ENC_LOCK_BITSTREAM = std::mem::zeroed();
            lock_bitstream.version = sys::NV_ENC_LOCK_BITSTREAM_VER;
            lock_bitstream.outputBitstream = output_buffer;

            let status = self
                .encoder
                .nvEncLockBitstream
                .map(|f| f(self.h_encoder, &mut lock_bitstream))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(status, "nvEncLockBitstream", "failed to lock bitstream")?;

            // ビットストリームがロックされている間にエンコード済みデータをコピー
            let encoded_data = std::slice::from_raw_parts(
                lock_bitstream.bitstreamBufferPtr as *const u8,
                lock_bitstream.bitstreamSizeInBytes as usize,
            )
            .to_vec();

            let status = self
                .encoder
                .nvEncUnlockBitstream
                .map(|f| f(self.h_encoder, lock_bitstream.outputBitstream));
            if let Some(status) = status {
                Error::check(status, "nvEncUnlockBitstream", "failed to unlock bitstream")?;
            }

            let timestamp = lock_bitstream.outputTimeStamp;
            let picture_type = PictureType::new(lock_bitstream.pictureType);

            Ok(EncodedFrame {
                data: encoded_data,
                timestamp,
                picture_type,
            })
        }
    }

    /// エンコーダーを終了し、残りのフレームを取得する
    pub fn finish(&mut self) -> Result<(), Error> {
        unsafe {
            let mut pic_params: sys::NV_ENC_PIC_PARAMS = std::mem::zeroed();
            pic_params.version = sys::NV_ENC_PIC_PARAMS_VER;
            pic_params.encodePicFlags = sys::NV_ENC_PIC_FLAG_EOS;
            pic_params.inputTimeStamp = self.frame_count;

            let status = self
                .encoder
                .nvEncEncodePicture
                .map(|f| f(self.h_encoder, &mut pic_params))
                .unwrap_or(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR);
            Error::check(status, "nvEncEncodePicture", "failed to finish encoder")?;

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
            .field("frame_count", &self.frame_count)
            .finish()
    }
}

unsafe impl Send for Encoder {}

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

impl PictureType {
    fn new(pic_type: sys::NV_ENC_PIC_TYPE) -> Self {
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

    /// エンコードされたデータを取得する（所有権を移動）
    pub fn into_data(self) -> Vec<u8> {
        self.data
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
        let _encoder =
            Encoder::new_h265(EncoderConfig::default()).expect("failed to initialize h265 encoder");
        println!("h265 encoder initialized successfully");
    }

    #[test]
    fn init_h264_encoder() {
        let _encoder =
            Encoder::new_h264(EncoderConfig::default()).expect("failed to initialize h264 encoder");
        println!("h264 encoder initialized successfully");
    }

    #[test]
    fn init_av1_encoder() {
        let _encoder =
            Encoder::new_av1(EncoderConfig::default()).expect("failed to initialize av1 encoder");
        println!("av1 encoder initialized successfully");
    }

    #[test]
    fn test_get_sequence_params_h264() {
        // H.264 エンコーダーを作成
        let mut encoder =
            Encoder::new_h264(EncoderConfig::default()).expect("failed to create h264 encoder");

        // シーケンスパラメータを取得
        let seq_params = encoder
            .get_sequence_params()
            .expect("failed to get sequence parameters");

        // シーケンスパラメータが空でないことを確認
        assert!(
            !seq_params.is_empty(),
            "Sequence parameters should not be empty"
        );

        println!("H.264 sequence parameters size: {} bytes", seq_params.len());
    }

    #[test]
    fn test_get_sequence_params_h265() {
        // H.265 エンコーダーを作成
        let mut encoder =
            Encoder::new_h265(EncoderConfig::default()).expect("failed to create h265 encoder");

        // シーケンスパラメータを取得
        let seq_params = encoder
            .get_sequence_params()
            .expect("failed to get sequence parameters");

        // シーケンスパラメータが空でないことを確認
        assert!(
            !seq_params.is_empty(),
            "Sequence parameters should not be empty"
        );

        println!("H.265 sequence parameters size: {} bytes", seq_params.len());
    }

    #[test]
    fn test_get_sequence_params_av1() {
        // AV1 エンコーダーを作成
        let mut encoder =
            Encoder::new_av1(EncoderConfig::default()).expect("failed to create av1 encoder");

        // シーケンスパラメータを取得
        let seq_params = encoder
            .get_sequence_params()
            .expect("failed to get sequence parameters");

        // シーケンスパラメータが空でないことを確認
        assert!(
            !seq_params.is_empty(),
            "Sequence parameters should not be empty"
        );

        println!("AV1 sequence header size: {} bytes", seq_params.len());
    }

    #[test]
    fn test_encode_h265_black_frame() {
        let config = EncoderConfig::default();
        let width = config.width;
        let height = config.height;

        // エンコーダーを作成
        let mut encoder = Encoder::new_h265(config).expect("failed to create h265 encoder");

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
            .encode(&frame_data)
            .expect("failed to encode black frame");
        encoder.finish().expect("failed to finish encoder");

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

    #[test]
    fn test_encode_h264_black_frame() {
        let config = EncoderConfig::default();
        let width = config.width;
        let height = config.height;

        // エンコーダーを作成
        let mut encoder = Encoder::new_h264(config).expect("failed to create h264 encoder");

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
            .encode(&frame_data)
            .expect("failed to encode black frame");
        encoder.finish().expect("failed to finish encoder");

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

    #[test]
    fn test_encode_av1_black_frame() {
        let config = EncoderConfig::default();
        let width = config.width;
        let height = config.height;

        // エンコーダーを作成
        let mut encoder = Encoder::new_av1(config).expect("failed to create av1 encoder");

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
            .encode(&frame_data)
            .expect("failed to encode black frame");
        encoder.finish().expect("failed to finish encoder");

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
