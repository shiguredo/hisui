//! [Hisui] 用の [Video Toolbox] デコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [Video Toolbox]: https://developer.apple.com/documentation/videotoolbox/
#![warn(missing_docs)]

use std::{
    collections::HashMap,
    ffi::{c_int, c_void},
    marker::PhantomData,
    mem::MaybeUninit,
    num::NonZeroUsize,
    time::Duration,
};

use sys::VTCompressionSessionCreate;

mod sys;

/// エラー
#[derive(Debug)]
pub struct Error {
    status: i32,
    function: &'static str,
}

impl Error {
    fn check(status: i32, function: &'static str) -> Result<(), Self> {
        if status == 0 {
            return Ok(());
        }
        Err(Self { status, function })
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}() failed: status={}",
            env!("CARGO_PKG_NAME"),
            self.function,
            self.status
        )
    }
}

impl std::error::Error for Error {}

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// 入出力画像の幅
    pub width: usize,

    /// 入出力画像の高さ
    pub height: usize,

    /// エンコードビットレート (bps 単位)
    pub target_bitrate: usize,

    /// FPS の分子
    pub fps_numerator: usize,

    /// FPS の分母
    pub fps_denominator: usize,

    /// 速度優先モード (true: 速度優先, false: 品質優先)
    pub prioritize_speed_over_quality: bool,

    /// リアルタイムモード (false: オフライン処理)
    pub real_time: bool,

    /// 電力効率最大化 (true: バックグラウンド処理時)
    pub maximize_power_efficiency: bool,

    /// フレーム再順序付けを許可 (false: B-frame無効で高速化)
    pub allow_frame_reordering: bool,

    /// Open GOP を許可 (false: 高速化、H.265のみ)
    pub allow_open_gop: bool,

    /// 時間的圧縮を許可 (false: キーフレームのみで高速化)
    pub allow_temporal_compression: bool,

    /// キーフレーム間隔 (フレーム数、小さいほど高速)
    pub max_key_frame_interval: Option<NonZeroUsize>,

    /// キーフレーム間隔 (小さいほど高速)
    pub max_key_frame_interval_duration: Option<Duration>,

    /// プロファイルレベル設定
    pub profile_level: ProfileLevel,

    /// H.264エントロピー符号化モード (CAVLC: 高速, CABAC: 高品質)
    pub h264_entropy_mode: H264EntropyMode,

    /// フレーム遅延制限 (小さいほど高速)
    pub max_frame_delay_count: Option<NonZeroUsize>,

    /// 並列処理を使用
    pub use_parallelization: bool,
}

/// プロファイルレベル設定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileLevel {
    /// H.264 Baseline (最高速)
    H264Baseline,
    /// H.264 Main
    H264Main,
    /// H.264 High (高品質)
    H264High,
    /// H.265 Main (デフォルト)
    H265Main,
    /// H.265 Main10
    H265Main10,
}

/// H.264エントロピー符号化モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264EntropyMode {
    /// CAVLC (高速)
    Cavlc,
    /// CABAC (高品質)
    Cabac,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,

            // B フレームは基本無効
            allow_frame_reordering: false,

            // デフォルトはバランス設定
            prioritize_speed_over_quality: false,
            real_time: false,
            maximize_power_efficiency: false,
            allow_open_gop: true,
            allow_temporal_compression: true,
            max_key_frame_interval: None,
            max_key_frame_interval_duration: None,
            profile_level: ProfileLevel::H264Main,
            h264_entropy_mode: H264EntropyMode::Cabac,
            max_frame_delay_count: None,
            use_parallelization: false,
        }
    }
}

impl EncoderConfig {
    /// 最高速度優先の設定 (オフライン処理用)
    pub fn fastest_offline() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,
            allow_frame_reordering: false,

            // 最高速設定
            prioritize_speed_over_quality: true,
            real_time: false,
            maximize_power_efficiency: false,
            allow_open_gop: false,
            allow_temporal_compression: true,
            max_key_frame_interval: Some(NonZeroUsize::MIN.saturating_add(29)), // 短い間隔
            max_key_frame_interval_duration: None,
            profile_level: ProfileLevel::H264Baseline, // 最軽量
            h264_entropy_mode: H264EntropyMode::Cavlc, // 高速
            max_frame_delay_count: Some(NonZeroUsize::MIN), // 最小遅延
            use_parallelization: true,
        }
    }

    /// 高速オフライン設定 (品質とのバランス)
    pub fn fast_offline() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,
            allow_frame_reordering: false,

            // 高速設定
            prioritize_speed_over_quality: true,
            real_time: false,
            maximize_power_efficiency: false,
            allow_open_gop: false,
            allow_temporal_compression: true,
            max_key_frame_interval: Some(NonZeroUsize::MIN.saturating_add(59)),
            max_key_frame_interval_duration: None,
            profile_level: ProfileLevel::H264Main,
            h264_entropy_mode: H264EntropyMode::Cavlc,
            max_frame_delay_count: Some(NonZeroUsize::MIN.saturating_add(3)),
            use_parallelization: true,
        }
    }

    /// バックグラウンド処理用設定
    pub fn background_processing() -> Self {
        Self {
            prioritize_speed_over_quality: false,
            real_time: false,
            maximize_power_efficiency: true, // 電力効率優先
            ..Self::fast_offline()
        }
    }

    /// 品質重視設定
    pub fn high_quality() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,
            allow_frame_reordering: false,

            // 品質重視設定
            prioritize_speed_over_quality: false,
            real_time: false,
            maximize_power_efficiency: false,
            allow_open_gop: true,
            allow_temporal_compression: true,
            max_key_frame_interval: Some(NonZeroUsize::MIN.saturating_add(119)), // 長い間隔
            max_key_frame_interval_duration: None,
            profile_level: ProfileLevel::H264High,
            h264_entropy_mode: H264EntropyMode::Cabac, // 高品質
            max_frame_delay_count: None,               // 制限なし
            use_parallelization: false,
        }
    }

    /// H.265用の高速設定
    pub fn h265_fast() -> Self {
        Self {
            profile_level: ProfileLevel::H265Main,
            allow_open_gop: false, // H.265でOpen GOP無効
            ..Self::fast_offline()
        }
    }
}

/// H.264 / H.265 エンコーダー
#[derive(Debug)]
pub struct Encoder {
    session: sys::VTCompressionSessionRef,
    config: EncoderConfig,
    next_input_pts: i64,
    next_output_pts: i64,
    output_frames: HashMap<i64, EncodedFrame>, // キーは pts
    encoded_frame_rx: std::sync::mpsc::Receiver<EncodedFrame>,

    // 以下のフィールドは Video Toolbox スレッドが呼び出すコールバック関数内でのみ使用されている。
    // 実質的には使われていても、Rust としてはそれを認識できずに警告が出るので expect で許容している。
    #[expect(dead_code)]
    encoded_frame_tx: Box<std::sync::mpsc::Sender<EncodedFrame>>,
}

impl Encoder {
    /// H.264 エンコーダーのインスタンスを生成する
    pub fn new_h264(config: &EncoderConfig) -> Result<Self, Error> {
        Self::new_encoder(config, false) // false = H.264
    }

    /// H.265 エンコーダーのインスタンスを生成する
    pub fn new_h265(config: &EncoderConfig) -> Result<Self, Error> {
        Self::new_encoder(config, true) // true = H.265
    }

    fn new_encoder(config: &EncoderConfig, is_h265: bool) -> Result<Self, Error> {
        unsafe {
            let (tx, rx) = std::sync::mpsc::channel();
            let tx = Box::new(tx);
            let mut session = std::ptr::null_mut();

            let (codec_fourcc, callback, profile_level) = if is_h265 {
                let profile_level = match config.profile_level {
                    ProfileLevel::H265Main => sys::kVTProfileLevel_HEVC_Main_AutoLevel,
                    ProfileLevel::H265Main10 => sys::kVTProfileLevel_HEVC_Main10_AutoLevel,
                    ProfileLevel::H264Baseline
                    | ProfileLevel::H264Main
                    | ProfileLevel::H264High => {
                        return Err(Error {
                            status: -1,
                            function: "new_encoder: invalid profile for H.265",
                        });
                    }
                };
                (
                    u32::from_be_bytes(*b"hvc1"),
                    Self::output_callback_h265 as unsafe extern "C" fn(_, _, _, _, _),
                    profile_level,
                )
            } else {
                let profile_level = match config.profile_level {
                    ProfileLevel::H264Baseline => sys::kVTProfileLevel_H264_Baseline_3_1,
                    ProfileLevel::H264Main => sys::kVTProfileLevel_H264_Main_3_1,
                    ProfileLevel::H264High => sys::kVTProfileLevel_H264_High_3_1,
                    ProfileLevel::H265Main | ProfileLevel::H265Main10 => {
                        return Err(Error {
                            status: -1,
                            function: "new_encoder: invalid profile for H.264",
                        });
                    }
                };
                (
                    u32::from_be_bytes(*b"avc1"),
                    Self::output_callback_h264 as unsafe extern "C" fn(_, _, _, _, _),
                    profile_level,
                )
            };

            let status = VTCompressionSessionCreate(
                std::ptr::null_mut(),
                config.width as i32,
                config.height as i32,
                codec_fourcc,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                Some(callback),
                (&*tx as *const std::sync::mpsc::Sender<EncodedFrame>)
                    .cast::<c_void>()
                    .cast_mut(),
                &mut session,
            );
            Error::check(status, "VTCompressionSessionCreate")?;

            // 共通のプロパティ設定
            let mut properties = Vec::new();
            Self::add_common_properties(&mut properties, config)?;

            // プロファイルレベル設定
            properties.push((
                sys::kVTCompressionPropertyKey_ProfileLevel,
                profile_level.cast(),
            ));

            // コーデック固有の設定
            if is_h265 {
                Self::add_h265_specific_properties(&mut properties, config)?;
            } else {
                Self::add_h264_specific_properties(&mut properties, config)?;
            }

            let properties_dict = cf_dictionary(&properties);
            let status = sys::VTSessionSetProperties(session.cast(), properties_dict);
            Error::check(status, "VTSessionSetProperties")?;

            Ok(Self {
                session,
                config: config.clone(),
                next_input_pts: 0,
                next_output_pts: 0,
                output_frames: HashMap::new(),
                encoded_frame_tx: tx,
                encoded_frame_rx: rx,
            })
        }
    }

    /// 共通のプロパティを追加
    fn add_common_properties(
        properties: &mut Vec<(sys::CFStringRef, *const c_void)>,
        config: &EncoderConfig,
    ) -> Result<(), Error> {
        unsafe {
            // 基本設定
            let target_bitrate = cf_number_i32(config.target_bitrate as i32);
            let fps = cf_number_i32(config.fps_numerator.div_ceil(config.fps_denominator) as i32);
            let pixel_format =
                cf_number_i32(sys::kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange as i32);

            properties.push((
                sys::kVTCompressionPropertyKey_AverageBitRate,
                target_bitrate.0,
            ));
            properties.push((sys::kVTCompressionPropertyKey_ExpectedFrameRate, fps.0));
            properties.push((
                sys::kVTCompressionPropertyKey_PixelTransferProperties,
                pixel_format.0,
            ));

            // リアルタイムモード
            properties.push((
                sys::kVTCompressionPropertyKey_RealTime,
                if config.real_time {
                    sys::kCFBooleanTrue
                } else {
                    sys::kCFBooleanFalse
                }
                .cast(),
            ));

            // フレーム再順序付け
            properties.push((
                sys::kVTCompressionPropertyKey_AllowFrameReordering,
                if config.allow_frame_reordering {
                    sys::kCFBooleanTrue
                } else {
                    sys::kCFBooleanFalse
                }
                .cast(),
            ));

            // 時間的圧縮
            properties.push((
                sys::kVTCompressionPropertyKey_AllowTemporalCompression,
                if config.allow_temporal_compression {
                    sys::kCFBooleanTrue
                } else {
                    sys::kCFBooleanFalse
                }
                .cast(),
            ));

            // キーフレーム間隔（フレーム数）
            if let Some(interval) = config.max_key_frame_interval {
                let interval_value = cf_number_i32(interval.get() as i32);
                properties.push((
                    sys::kVTCompressionPropertyKey_MaxKeyFrameInterval,
                    interval_value.0,
                ));
            }

            // キーフレーム間隔（秒数）
            if let Some(duration) = config.max_key_frame_interval_duration {
                let duration_value = cf_number_f64(duration.as_secs_f64());
                properties.push((
                    sys::kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration,
                    duration_value.0,
                ));
            }

            // フレーム遅延制限
            if let Some(delay_count) = config.max_frame_delay_count {
                let delay_value = cf_number_i32(delay_count.get() as i32);
                properties.push((
                    sys::kVTCompressionPropertyKey_MaxFrameDelayCount,
                    delay_value.0,
                ));
            }

            // 速度優先モード
            if config.prioritize_speed_over_quality {
                properties.push((
                    sys::kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                    sys::kCFBooleanTrue.cast(),
                ));
            }

            // 電力効率最大化
            if config.maximize_power_efficiency {
                properties.push((
                    sys::kVTCompressionPropertyKey_MaximizePowerEfficiency,
                    sys::kCFBooleanTrue.cast(),
                ));
            }
        }
        Ok(())
    }

    /// H.264固有のプロパティを追加
    fn add_h264_specific_properties(
        properties: &mut Vec<(sys::CFStringRef, *const c_void)>,
        config: &EncoderConfig,
    ) -> Result<(), Error> {
        unsafe {
            // H.264エントロピー符号化モード
            let entropy_mode = match config.h264_entropy_mode {
                H264EntropyMode::Cavlc => sys::kVTH264EntropyMode_CAVLC,
                H264EntropyMode::Cabac => sys::kVTH264EntropyMode_CABAC,
            };
            properties.push((
                sys::kVTCompressionPropertyKey_H264EntropyMode,
                entropy_mode.cast(),
            ));
        }
        Ok(())
    }

    /// H.265固有のプロパティを追加
    fn add_h265_specific_properties(
        properties: &mut Vec<(sys::CFStringRef, *const c_void)>,
        config: &EncoderConfig,
    ) -> Result<(), Error> {
        unsafe {
            // Open GOP設定（H.265のみ）
            if !config.allow_open_gop {
                properties.push((
                    sys::kVTCompressionPropertyKey_AllowOpenGOP,
                    sys::kCFBooleanFalse.cast(),
                ));
            }
        }
        Ok(())
    }

    /// I420 形式の画像データをエンコードする
    ///
    /// エンコード結果は [`Encoder::next_frame()`] で取得できる
    ///
    /// なお `y` のストライドは入力フレームの幅と等しいことが前提
    ///
    /// また B フレームは扱わない前提（つまり入力フレームと出力フレームの順番が一致する）
    pub fn encode(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> Result<(), Error> {
        let width = self.config.width;
        let height = self.config.height;

        unsafe {
            let mut image_buffer = std::ptr::null_mut();
            let status = sys::CVPixelBufferCreateWithPlanarBytes(
                std::ptr::null_mut(),
                width,
                height,
                u32::from_be_bytes(*b"y420"),
                std::ptr::null_mut(),
                0,
                3,
                [
                    y.as_ptr().cast::<c_void>().cast_mut(),
                    u.as_ptr().cast::<c_void>().cast_mut(),
                    v.as_ptr().cast::<c_void>().cast_mut(),
                ]
                .as_mut_ptr(),
                [width, width.div_ceil(2), width.div_ceil(2)].as_mut_ptr(),
                [height, height.div_ceil(2), height.div_ceil(2)].as_mut_ptr(),
                [width, width.div_ceil(2), width.div_ceil(2)].as_mut_ptr(),
                None,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut image_buffer,
            );
            Error::check(status, "CVPixelBufferCreateWithPlanarBytes")?;

            let image_buffer = CfPtrMut(image_buffer);
            let status = sys::VTCompressionSessionEncodeFrame(
                self.session,
                image_buffer.0,
                sys::CMTimeMake(self.next_input_pts, self.config.fps_numerator as i32),
                sys::kCMTimeInvalid,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            Error::check(status, "VTCompressionSessionEncodeFrame")?;

            self.next_input_pts += self.config.fps_denominator as i64;

            Ok(())
        }
    }

    /// これ以上データが来ないことをエンコーダーに伝える
    ///
    /// 残りのエンコード結果は [`Encoder::next_frame()`] で取得できる
    pub fn finish(&mut self) -> Result<(), Error> {
        unsafe {
            let status = sys::VTCompressionSessionCompleteFrames(self.session, sys::kCMTimeInvalid);
            Error::check(status, "VTCompressionSessionCompleteFrames")?;
        }
        Ok(())
    }

    /// エンコード済みのフレームを取り出す
    pub fn next_frame(&mut self) -> Option<EncodedFrame> {
        let Ok(frame) = self.encoded_frame_rx.try_recv() else {
            return None;
        };
        self.output_frames.insert(frame.pts, frame);
        self.output_frames
            .remove(&self.next_output_pts)
            .inspect(|_| {
                self.next_output_pts += self.config.fps_denominator as i64;
            })
    }

    unsafe extern "C" fn output_callback_h264(
        output_callback_ref_con: *mut c_void,
        _source_frame_ref_con: *mut c_void,
        status: i32,
        _info_flags: sys::VTEncodeInfoFlags,
        sample_buffer: sys::CMSampleBufferRef,
    ) {
        if let Err(e) = Error::check(status, "output_callback_h264") {
            log::error!("{e}");
            return;
        }

        unsafe {
            let data_buffer = sys::CMSampleBufferGetDataBuffer(sample_buffer);
            let mut data_pointer = std::ptr::null_mut();
            let mut data_pointer_len = 0;
            let status = sys::CMBlockBufferGetDataPointer(
                data_buffer,
                0,
                &mut data_pointer_len,
                std::ptr::null_mut(),
                &mut data_pointer,
            );
            if let Err(e) = Error::check(status, "CMBlockBufferGetDataPointer") {
                log::error!("{e}");
                return;
            }

            // PTS を取得
            let pts = sys::CMSampleBufferGetPresentationTimeStamp(sample_buffer);

            // キーフレーム、SPS、PPS の情報を取得する
            let description = sys::CMSampleBufferGetFormatDescription(sample_buffer);
            let mut nalu_header_length = 0;
            let status = sys::CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                description,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut nalu_header_length,
            );
            if let Err(e) =
                Error::check(status, "CMVideoFormatDescriptionGetH264ParameterSetAtIndex")
            {
                log::error!("{e}");
                return;
            }
            if nalu_header_length != 4 {
                // 現実的には 4 以外になることはないはず（そうではないならハンドリングを追加する）
                log::error!("unexpected NAL unit header length: {nalu_header_length}");
                return;
            }

            let data =
                std::slice::from_raw_parts(data_pointer as *const u8, data_pointer_len).to_vec();
            let keyframe = is_keyframe(sample_buffer);
            let mut sps_list = Vec::new();
            let mut pps_list = Vec::new();
            if keyframe {
                let mut sps_ptr = std::ptr::null();
                let mut pps_ptr = std::ptr::null();
                let mut sps_size = 0;
                let mut pps_size = 0;

                for (i, (ps_ptr, ps_size)) in
                    [(&mut sps_ptr, &mut sps_size), (&mut pps_ptr, &mut pps_size)]
                        .into_iter()
                        .enumerate()
                {
                    let status = sys::CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                        description,
                        i,
                        ps_ptr,
                        ps_size,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if let Err(e) =
                        Error::check(status, "CMVideoFormatDescriptionGetH264ParameterSetAtIndex")
                    {
                        log::error!("{e}");
                        return;
                    }
                }
                sps_list.push(std::slice::from_raw_parts(sps_ptr, sps_size).to_vec());
                pps_list.push(std::slice::from_raw_parts(pps_ptr, pps_size).to_vec());
            }

            let frame = EncodedFrame {
                keyframe,
                sps_list,
                pps_list,
                vps_list: Vec::new(), // H.264 には VPS は存在しない
                data,
                pts: pts.value,
            };

            // 呼び出しもとスレッドに結果を伝える
            // (Sender は Send を実装しているので、複数スレッドで参照を共有しても問題ない)
            let tx = &*(output_callback_ref_con as *mut std::sync::mpsc::Sender<EncodedFrame>);
            let _ = tx.send(frame);
        }
    }

    unsafe extern "C" fn output_callback_h265(
        output_callback_ref_con: *mut c_void,
        _source_frame_ref_con: *mut c_void,
        status: i32,
        _info_flags: sys::VTEncodeInfoFlags,
        sample_buffer: sys::CMSampleBufferRef,
    ) {
        if let Err(e) = Error::check(status, "output_callback_h265") {
            log::error!("{e}");
            return;
        }

        unsafe {
            let data_buffer = sys::CMSampleBufferGetDataBuffer(sample_buffer);
            let mut data_pointer = std::ptr::null_mut();
            let mut data_pointer_len = 0;
            let status = sys::CMBlockBufferGetDataPointer(
                data_buffer,
                0,
                &mut data_pointer_len,
                std::ptr::null_mut(),
                &mut data_pointer,
            );
            if let Err(e) = Error::check(status, "CMBlockBufferGetDataPointer") {
                log::error!("{e}");
                return;
            }

            // PTS を取得
            let pts = sys::CMSampleBufferGetPresentationTimeStamp(sample_buffer);

            // キーフレーム、SPS、PPS の情報を取得する
            let description = sys::CMSampleBufferGetFormatDescription(sample_buffer);
            let mut nalu_header_length = 0;
            let status = sys::CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(
                description,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut nalu_header_length,
            );
            if let Err(e) =
                Error::check(status, "CMVideoFormatDescriptionGetHEVCParameterSetAtIndex")
            {
                log::error!("{e}");
                return;
            }
            if nalu_header_length != 4 {
                // 現実的には 4 以外になることはないはず（そうではないならハンドリングを追加する）
                log::error!("unexpected NAL unit header length: {nalu_header_length}");
                return;
            }

            let data =
                std::slice::from_raw_parts(data_pointer as *const u8, data_pointer_len).to_vec();
            let keyframe = is_keyframe(sample_buffer);
            let mut vps_list = Vec::new();
            let mut sps_list = Vec::new();
            let mut pps_list = Vec::new();
            if keyframe {
                let mut vps_ptr = std::ptr::null();
                let mut sps_ptr = std::ptr::null();
                let mut pps_ptr = std::ptr::null();
                let mut vps_size = 0;
                let mut sps_size = 0;
                let mut pps_size = 0;

                for (i, (ps_ptr, ps_size)) in [
                    (&mut vps_ptr, &mut vps_size),
                    (&mut sps_ptr, &mut sps_size),
                    (&mut pps_ptr, &mut pps_size),
                ]
                .into_iter()
                .enumerate()
                {
                    let status = sys::CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(
                        description,
                        i,
                        ps_ptr,
                        ps_size,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                    if let Err(e) =
                        Error::check(status, "CMVideoFormatDescriptionGetHEVCParameterSetAtIndex")
                    {
                        log::error!("{e}");
                        return;
                    }
                }
                vps_list.push(std::slice::from_raw_parts(vps_ptr, vps_size).to_vec());
                sps_list.push(std::slice::from_raw_parts(sps_ptr, sps_size).to_vec());
                pps_list.push(std::slice::from_raw_parts(pps_ptr, pps_size).to_vec());
            }

            let frame = EncodedFrame {
                keyframe,
                sps_list,
                pps_list,
                vps_list,
                data,
                pts: pts.value,
            };

            // 呼び出しもとスレッドに結果を伝える
            // (Sender は Send を実装しているので、複数スレッドで参照を共有しても問題ない)
            let tx = &*(output_callback_ref_con as *mut std::sync::mpsc::Sender<EncodedFrame>);
            let _ = tx.send(frame);
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            sys::VTCompressionSessionInvalidate(self.session);
            sys::CFRelease(self.session as *const c_void);
        }
    }
}

unsafe impl Send for Encoder {}

/// エンコードされた映像フレーム (AVCC 形式)
#[derive(Debug)]
pub struct EncodedFrame {
    /// キーフレームかどうか
    pub keyframe: bool,

    /// SPS
    pub sps_list: Vec<Vec<u8>>,

    /// PPS
    pub pps_list: Vec<Vec<u8>>,

    /// VPS (H.265 only)
    pub vps_list: Vec<Vec<u8>>,

    /// 圧縮データ
    pub data: Vec<u8>,

    pts: i64,
}

fn is_keyframe(sample_buffer: sys::CMSampleBufferRef) -> bool {
    unsafe {
        let attachments = sys::CMSampleBufferGetSampleAttachmentsArray(sample_buffer, 1);
        if attachments.is_null() {
            return false;
        }

        let attachment = sys::CFArrayGetValueAtIndex(attachments, 0);
        if attachment.is_null() {
            return false;
        }

        let not_sync = sys::CFDictionaryGetValue(
            attachment as *mut _,
            sys::kCMSampleAttachmentKey_NotSync as *const c_void,
        );
        not_sync != sys::kCFBooleanTrue as *const c_void
    }
}

/// H.264 / H.265 デコーダー
#[derive(Debug)]
pub struct Decoder {
    description: sys::CMVideoFormatDescriptionRef,
    session: sys::VTDecompressionSessionRef,
}

impl Decoder {
    /// H.264 デコーダーのインスタンスを生成する
    pub fn new_h264(sps: &[u8], pps: &[u8], nalu_len_bytes: usize) -> Result<Self, Error> {
        unsafe {
            let mut description: sys::CMVideoFormatDescriptionRef = std::ptr::null_mut();
            let status = sys::CMVideoFormatDescriptionCreateFromH264ParameterSets(
                std::ptr::null_mut(),
                2,
                [sps.as_ptr(), pps.as_ptr()].as_ptr(),
                [sps.len(), pps.len()].as_ptr(),
                nalu_len_bytes as c_int,
                &mut description,
            );
            Error::check(
                status,
                "CMVideoFormatDescriptionCreateFromH264ParameterSets",
            )?;

            let mut session: sys::VTDecompressionSessionRef = std::ptr::null_mut();
            let mut callback =
                MaybeUninit::<sys::VTDecompressionOutputCallbackRecord>::zeroed().assume_init();
            callback.decompressionOutputCallback = Some(Self::output_callback);

            let pixel_format = cf_number_i32(sys::kCVPixelFormatType_420YpCbCr8Planar as i32);
            let status = sys::VTDecompressionSessionCreate(
                std::ptr::null_mut(),
                description,
                std::ptr::null_mut(),
                cf_dictionary(&[(sys::kCVPixelBufferPixelFormatTypeKey, pixel_format.0)]),
                &callback,
                &mut session,
            );
            Error::check(status, "VTDecompressionSessionCreate")?;

            Ok(Self {
                description,
                session,
            })
        }
    }

    /// H.265 デコーダーのインスタンスを生成する
    pub fn new_h265(
        vps: &[u8],
        sps: &[u8],
        pps: &[u8],
        nalu_len_bytes: usize,
    ) -> Result<Self, Error> {
        unsafe {
            let mut description: sys::CMVideoFormatDescriptionRef = std::ptr::null_mut();
            let status = sys::CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                std::ptr::null_mut(),
                3,
                [vps.as_ptr(), sps.as_ptr(), pps.as_ptr()].as_ptr(),
                [vps.len(), sps.len(), pps.len()].as_ptr(),
                nalu_len_bytes as c_int,
                std::ptr::null_mut(),
                &mut description,
            );
            Error::check(
                status,
                "CMVideoFormatDescriptionCreateFromHEVCParameterSets",
            )?;

            let mut session: sys::VTDecompressionSessionRef = std::ptr::null_mut();
            let mut callback =
                MaybeUninit::<sys::VTDecompressionOutputCallbackRecord>::zeroed().assume_init();
            callback.decompressionOutputCallback = Some(Self::output_callback);

            let pixel_format = cf_number_i32(sys::kCVPixelFormatType_420YpCbCr8Planar as i32);
            let status = sys::VTDecompressionSessionCreate(
                std::ptr::null_mut(),
                description,
                std::ptr::null_mut(),
                cf_dictionary(&[(sys::kCVPixelBufferPixelFormatTypeKey, pixel_format.0)]),
                &callback,
                &mut session,
            );
            Error::check(status, "VTDecompressionSessionCreate")?;

            Ok(Self {
                description,
                session,
            })
        }
    }

    /// 圧縮された映像フレーム（AVCC 形式）をデコードする
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedFrame>, Error> {
        unsafe {
            let mut block_buffer = std::ptr::null_mut();
            let status = sys::CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null_mut(),
                data.as_ptr().cast_mut().cast(),
                data.len(),
                sys::kCFAllocatorNull, // data の自動解放を Video Toolbox 側で行わないようにする
                std::ptr::null(),
                0,
                data.len(),
                0,
                &mut block_buffer,
            );
            Error::check(status, "CMBlockBufferCreateWithMemoryBlock")?;
            let block_buffer = CfPtrMut(block_buffer);

            let mut sample_buffer = std::ptr::null_mut();
            let status = sys::CMSampleBufferCreateReady(
                std::ptr::null_mut(),
                block_buffer.0,
                self.description,
                1,
                0,
                [].as_ptr(),
                0,
                [].as_ptr(),
                &mut sample_buffer,
            );
            Error::check(status, "CMSampleBufferCreateReadyWithPacketDescriptions")?;
            let sample_buffer = CfPtrMut(sample_buffer);

            let decode_flags = 0;
            let mut info_flags = 0;
            let mut image_buffer: sys::CVImageBufferRef = std::ptr::null_mut();
            let status = sys::VTDecompressionSessionDecodeFrame(
                self.session,
                sample_buffer.0,
                decode_flags,
                ((&mut image_buffer) as *mut sys::CVImageBufferRef).cast(),
                &mut info_flags,
            );
            Error::check(status, "VTDecompressionSessionDecodeFrame")?;

            if image_buffer.is_null() {
                return Ok(None);
            }

            let image_buffer = CfPtrMut(image_buffer);
            let flags_readonly = 1;
            let status = sys::CVPixelBufferLockBaseAddress(image_buffer.0, flags_readonly);
            Error::check(status, "CVPixelBufferLockBaseAddress")?;

            Ok(Some(DecodedFrame {
                inner: image_buffer,
                _lifetime: PhantomData,
            }))
        }
    }

    // [NOTE] このコールバック関数は VTDecompressionSessionDecodeFrame() の処理中に呼び出される
    //        (指定したフラグによって挙動は変わるがデフォルトでは）
    unsafe extern "C" fn output_callback(
        _decompression_output_ref_con: *mut c_void,
        source_frame_ref_con: *mut c_void,
        status: i32,
        _info_flags: sys::VTDecodeInfoFlags,
        image_buffer: sys::CVImageBufferRef,
        _presentation_time_stamp: sys::CMTime,
        _presentation_duration: sys::CMTime,
    ) {
        if let Err(e) = Error::check(status, "output_callback") {
            log::error!("{e}");
            return;
        }

        let output = source_frame_ref_con.cast();
        unsafe {
            *output = sys::CFRetain(image_buffer.cast());
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            sys::VTDecompressionSessionInvalidate(self.session);
            sys::CFRelease(self.session as *const c_void);
            sys::CFRelease(self.description as *const c_void);
        }
    }
}

unsafe impl Send for Decoder {}

/// デコードされた映像フレーム (I420 形式)
#[derive(Debug)]
pub struct DecodedFrame<'a> {
    inner: CfPtrMut<sys::__CVBuffer>,

    // inner の中には Video Toolbox が返した一時的なデータへの参照も含まれているので、
    // このライフタイムで利用側での使用範囲を制限する。
    _lifetime: PhantomData<&'a ()>,
}

impl DecodedFrame<'_> {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                sys::CVPixelBufferGetBaseAddressOfPlane(self.inner.0, 0) as *const u8,
                self.height() * self.y_stride(),
            )
        }
    }

    /// フレームの U 成分のデータを返す
    pub fn u_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                sys::CVPixelBufferGetBaseAddressOfPlane(self.inner.0, 1) as *const u8,
                self.height().div_ceil(2) * self.u_stride(),
            )
        }
    }

    /// フレームの V 成分のデータを返す
    pub fn v_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                sys::CVPixelBufferGetBaseAddressOfPlane(self.inner.0, 2) as *const u8,
                self.height().div_ceil(2) * self.v_stride(),
            )
        }
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        unsafe { sys::CVPixelBufferGetBytesPerRowOfPlane(self.inner.0, 0) }
    }

    /// フレームの U 成分のストライドを返す
    pub fn u_stride(&self) -> usize {
        unsafe { sys::CVPixelBufferGetBytesPerRowOfPlane(self.inner.0, 1) }
    }

    /// フレームの V 成分のストライドを返す
    pub fn v_stride(&self) -> usize {
        unsafe { sys::CVPixelBufferGetBytesPerRowOfPlane(self.inner.0, 2) }
    }

    /// フレームの幅を返す
    pub fn width(&self) -> usize {
        unsafe { sys::CVPixelBufferGetWidth(self.inner.0) }
    }

    /// フレームの高さを返す
    pub fn height(&self) -> usize {
        unsafe { sys::CVPixelBufferGetHeight(self.inner.0) }
    }
}

impl Drop for DecodedFrame<'_> {
    fn drop(&mut self) {
        unsafe {
            let flags_readonly = 1;
            sys::CVPixelBufferUnlockBaseAddress(self.inner.0, flags_readonly);
        }
    }
}

// ドロップ時に確実に sys::CFRelease() を呼び出すようにするためのラッパー
#[derive(Debug)]
struct CfPtrMut<T>(*mut T);

impl<T> Drop for CfPtrMut<T> {
    fn drop(&mut self) {
        unsafe { sys::CFRelease(self.0.cast()) }
    }
}

#[derive(Debug)]
struct CfPtr<T>(*const T);

impl<T> Drop for CfPtr<T> {
    fn drop(&mut self) {
        unsafe { sys::CFRelease(self.0.cast()) }
    }
}

fn cf_dictionary(kvs: &[(sys::CFStringRef, *const c_void)]) -> sys::CFDictionaryRef {
    let mut keys = kvs.iter().map(|(k, _)| k.cast()).collect::<Vec<_>>();
    let mut values = kvs.iter().map(|(_, v)| *v).collect::<Vec<_>>();
    unsafe {
        sys::CFDictionaryCreate(
            std::ptr::null_mut(),
            keys.as_mut_ptr(),
            values.as_mut_ptr(),
            kvs.len() as sys::CFIndex,
            &sys::kCFTypeDictionaryKeyCallBacks,
            &sys::kCFTypeDictionaryValueCallBacks,
        )
    }
}

fn cf_number_i32(n: i32) -> CfPtr<c_void> {
    let ptr = unsafe {
        sys::CFNumberCreate(
            std::ptr::null_mut(),
            sys::kCFNumberSInt32Type as sys::CFNumberType,
            ((&n) as *const i32).cast(),
        )
    };
    CfPtr(ptr.cast())
}

fn cf_number_f64(n: f64) -> CfPtr<c_void> {
    let ptr = unsafe {
        sys::CFNumberCreate(
            std::ptr::null_mut(),
            sys::kCFNumberFloat64Type as sys::CFNumberType,
            ((&n) as *const f64).cast(),
        )
    };
    CfPtr(ptr.cast())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 960;
    const HEIGHT: usize = 480;
    const SIZE: usize = WIDTH * HEIGHT;

    #[test]
    fn h264_decoder() -> Result<(), Error> {
        let sps = [
            103, 100, 0, 30, 172, 217, 64, 160, 61, 176, 17, 0, 0, 3, 0, 1, 0, 0, 3, 0, 50, 15, 22,
            45, 150,
        ];
        let pps = [104, 235, 227, 203, 34, 192];
        let mut decoder = Decoder::new_h264(&sps, &pps, 4)?;

        let nal_unit = [
            101, 136, 132, 0, 43, 255, 254, 246, 115, 124, 10, 107, 109, 176, 149, 46, 5, 118, 247,
            102, 163, 229, 208, 146, 229, 251, 16, 96, 250, 208, 0, 0, 3, 0, 0, 3, 0, 0, 16, 15,
            210, 222, 245, 204, 98, 91, 229, 32, 0, 0, 9, 216, 2, 56, 13, 16, 118, 133, 116, 69,
            196, 32, 71, 6, 120, 150, 16, 161, 210, 50, 128, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0,
            0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 37, 225,
        ];
        let mut data = Vec::new();
        data.extend_from_slice(&(nal_unit.len() as u32).to_be_bytes());
        data.extend_from_slice(&nal_unit);
        decoder.decode(&data)?;

        Ok(())
    }

    #[test]
    fn h265_decoder() -> Result<(), Error> {
        let vps = [
            64, 1, 12, 1, 255, 255, 1, 96, 0, 0, 3, 0, 144, 0, 0, 3, 0, 0, 3, 0, 90, 149, 152, 9,
        ];
        let sps = [
            66, 1, 1, 1, 96, 0, 0, 3, 0, 144, 0, 0, 3, 0, 0, 3, 0, 90, 160, 5, 2, 1, 225, 101, 149,
            154, 73, 50, 188, 5, 160, 32, 0, 0, 3, 0, 32, 0, 0, 3, 3, 33,
        ];
        let pps = [68, 1, 193, 114, 180, 98, 64];
        let mut decoder = Decoder::new_h265(&vps, &sps, &pps, 4)?;

        let nal_unit = [
            40, 1, 175, 29, 16, 90, 181, 140, 90, 213, 247, 1, 91, 255, 242, 78, 254, 199, 0, 31,
            209, 50, 148, 21, 162, 38, 146, 0, 0, 3, 1, 203, 169, 113, 202, 5, 24, 129, 39, 128, 0,
            0, 3, 0, 7, 204, 147, 13, 148, 32, 0, 0, 3, 0, 0, 3, 0, 12, 24, 135, 0, 0, 3, 0, 0, 3,
            0, 0, 3, 0, 28, 240, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 8, 104, 0, 0, 3, 0, 0, 3, 0, 0, 3,
            0, 104, 192, 0, 0, 3, 0, 0, 3, 0, 0, 3, 1, 223, 0, 0, 3, 0, 9, 248,
        ];
        let mut data = Vec::new();
        data.extend_from_slice(&(nal_unit.len() as u32).to_be_bytes());
        data.extend_from_slice(&nal_unit);
        decoder.decode(&data)?;

        Ok(())
    }

    #[test]
    fn init_h264_encoder() {
        // OK
        let config = encoder_config(false);
        assert!(Encoder::new_h264(&config).is_ok());

        // NG
        let mut config = encoder_config(false);
        config.width = 0;
        assert!(Encoder::new_h264(&config).is_err());
    }

    #[test]
    fn init_h265_encoder() {
        // OK
        let config = encoder_config(true);
        assert!(Encoder::new_h265(&config).is_ok());

        // NG
        let mut config = encoder_config(true);
        config.width = 0;
        assert!(Encoder::new_h265(&config).is_err());
    }

    #[test]
    fn encode_h264_black() {
        let config = encoder_config(false);
        let mut encoder = Encoder::new_h264(&config).expect("create encoder error");
        let mut count = 0;

        // [NOTE]: encode(&[0; SIZE], ..) の様に変数を経由せずに指定するとエラーになる
        let y = [0; SIZE];
        let u = [0; SIZE / 4];
        let v = [0; SIZE / 4];
        encoder.encode(&y, &u, &v).expect("encode error");

        while encoder.next_frame().is_some() {
            count += 1;
        }

        encoder.finish().expect("finish error");
        while encoder.next_frame().is_some() {
            count += 1;
        }

        assert_eq!(count, 1);
    }

    #[test]
    fn encode_h265_black() {
        let config = encoder_config(true);
        let mut encoder = Encoder::new_h265(&config).expect("create encoder error");
        let mut count = 0;

        // [NOTE]: encode(&[0; SIZE], ..) の様に変数を経由せずに指定するとエラーになる
        let y = [0; SIZE];
        let u = [0; SIZE / 4];
        let v = [0; SIZE / 4];
        encoder.encode(&y, &u, &v).expect("encode error");

        while encoder.next_frame().is_some() {
            count += 1;
        }

        encoder.finish().expect("finish error");
        while encoder.next_frame().is_some() {
            count += 1;
        }

        assert_eq!(count, 1);
    }

    fn encoder_config(is_h265: bool) -> EncoderConfig {
        EncoderConfig {
            width: WIDTH,
            height: HEIGHT,
            target_bitrate: 100_000,
            fps_numerator: 1,
            fps_denominator: 1,
            profile_level: if is_h265 {
                ProfileLevel::H265Main
            } else {
                ProfileLevel::H264Main
            },
            ..Default::default()
        }
    }
}
