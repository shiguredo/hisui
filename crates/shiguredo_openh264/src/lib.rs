//! [Hisui] 用の [openh264] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [openh264]: https://github.com/cisco/openh264
#![warn(missing_docs)]

use std::{
    ffi::{c_int, c_longlong, c_uint, c_ushort},
    marker::PhantomData,
    mem::MaybeUninit,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use libloading::{Library, Symbol};

mod sys;

/// ビルド時に参照した OpenH264 のバージョン（タグ）
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// ビルド時に参照した OpenH264 のリポジトリ URL
pub const BUILD_REPOSITORY: &str = sys::BUILD_METADATA_REPOSITORY;

// Hisui でのエンコード時のレベル
const LEVEL: sys::ELevelIdc = sys::ELevelIdc_LEVEL_3_1;

// Hisui でのエンコード時のプロファイル
const PROFILE: sys::EProfileIdc = sys::EProfileIdc_PRO_BASELINE;

/// エラー
#[derive(Debug)]
#[allow(missing_docs)]
pub enum Error {
    /// 共有ライブラリ関連のエラー
    SharedLibraryError(libloading::Error),

    /// openh264 関連のエラー
    Openh264Error { code: c_int, function: &'static str },

    /// openh264 の仮想テーブル (vtbl) のメソッドが None だった場合のエラー
    UnavailableMethod(&'static str),

    /// デコード結果が I420 以外だった
    UnsupportedFormat { format: sys::EVideoFormatType },

    /// エンコード時の入力 YUV のサイズが不正だった
    InvalidYuvSize,
}

impl Error {
    fn check(code: c_int, function: &'static str) -> Result<(), Error> {
        match code {
            0 => Ok(()),
            _ => Err(Self::Openh264Error { code, function }),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SharedLibraryError(error) => write!(f, "{error}"),
            Error::Openh264Error { code, function } => {
                write!(f, "{function}() failed: code={code}")
            }
            Error::UnavailableMethod(name) => write!(f, "unavailable method: name={name}"),
            Error::UnsupportedFormat { format } => {
                write!(f, "unsupported video format (not I420): format={format}")
            }
            Error::InvalidYuvSize => write!(f, "invalid input YUV size"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::SharedLibraryError(error) => Some(error),
            _ => None,
        }
    }
}

impl From<libloading::Error> for Error {
    fn from(value: libloading::Error) -> Self {
        Self::SharedLibraryError(value)
    }
}

/// openh264 用の共有ライブラリを管理するための構造体
#[derive(Debug, Clone)]
pub struct Openh264Library {
    lib: Arc<Library>,
    path: PathBuf,
    version: sys::OpenH264Version,
}

impl Openh264Library {
    /// 指定のパスにある動的ライブラリをロードする
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        unsafe {
            let lib = Library::new(path.as_ref().as_os_str())?;
            let version = lib
                .get::<unsafe extern "C" fn() -> sys::OpenH264Version>(b"WelsGetCodecVersion")?(
            );
            let this = Self {
                lib: Arc::new(lib),
                path: path.as_ref().to_path_buf(),
                version,
            };
            this.check_version();
            Ok(this)
        }
    }

    /// 共有ライブラリのパスを取得する
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 共有ライブラリのバージョンを取得する
    pub fn runtime_version(&self) -> String {
        format!(
            "v{}.{}.{}",
            self.version.uMajor, self.version.uMinor, self.version.uRevision
        )
    }

    fn check_version(&self) {
        let runtime_version = self.runtime_version();
        if runtime_version != BUILD_VERSION {
            // バージョンが不一致になったからといって、必ずしも利用可能とは限らないので、
            // とりあえずは警告ログを出しておくに留めて、処理自体は継続する
            log::warn!(
                concat!(
                    "OpenH264 version mismatch: build-time version is '{}', ",
                    "but runtime version is '{}'. ",
                    "This may cause compatibility issues."
                ),
                BUILD_VERSION,
                runtime_version
            );
        }
    }

    fn call<F, T, U>(&self, symbol: &str, f: F) -> Result<U, Error>
    where
        F: FnOnce(Symbol<T>) -> U,
    {
        let external_function = unsafe { self.lib.get(symbol.as_bytes())? };
        Ok(f(external_function))
    }
}

// 以下は共有ライブラリからの取得されるそれぞれの関数の型定義。
// Rust では関数から直接その型をコンパイル時に取得する方法がないので、自前で定義している。
type WelsCreateSVCEncoder = unsafe extern "C" fn(pp_encoder: *mut *mut sys::ISVCEncoder) -> c_int;
type WelsDestroySVCEncoder = unsafe extern "C" fn(p_encoder: *mut sys::ISVCEncoder);
type WelsCreateDecoder = unsafe extern "C" fn(pp_decoder: *mut *mut sys::ISVCDecoder) -> c_int;
type WelsDestroyDecoder = unsafe extern "C" fn(p_decoder: *mut sys::ISVCDecoder);

/// H.264 デコーダー
#[derive(Debug)]
pub struct Decoder {
    lib: Openh264Library,
    inner: *mut sys::ISVCDecoder,
}

impl Decoder {
    /// デコーダーインスタンスを生成する
    pub fn new(lib: Openh264Library) -> Result<Self, Error> {
        let mut inner = std::ptr::null_mut();
        let param = MaybeUninit::<sys::SDecodingParam>::zeroed();
        unsafe {
            let name = "WelsCreateDecoder";
            let code = lib.call(name, |f: Symbol<WelsCreateDecoder>| f(&mut inner))?;
            Error::check(code, name)?;

            let mut param = param.assume_init();
            param.pFileNameRestructed = std::ptr::null_mut();
            param.uiTargetDqLayer = 1;
            param.eEcActiveIdc = sys::ERROR_CON_IDC_ERROR_CON_DISABLE;
            param.bParseOnly = false;
            param.sVideoProperty.eVideoBsType = sys::VIDEO_BITSTREAM_TYPE_VIDEO_BITSTREAM_AVC;

            let name = "ISVCEncoder.GetDefaultParams";
            let code = (**inner).Initialize.ok_or(Error::UnavailableMethod(name))?(inner, &param);
            Error::check(code as c_int, name)?;

            Ok(Self { lib, inner })
        }
    }

    /// 圧縮された映像フレーム（Annex.B 形式）をデコードする
    ///
    /// B フレームは存在しない前提（つまり入力と出力の順番が一致する）
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedFrame>, Error> {
        let mut info = MaybeUninit::<sys::SBufferInfo>::zeroed();
        unsafe {
            let mut yuv = [std::ptr::null_mut(); 3];
            let name = "ISVCDecoder.DecodeFrameNoDelay";
            let code = (**self.inner)
                .DecodeFrameNoDelay
                .ok_or(Error::UnavailableMethod(name))?(
                self.inner,
                data.as_ptr(),
                data.len() as c_int,
                yuv.as_mut_ptr(),
                info.as_mut_ptr(),
            );
            Error::check(code as c_int, name)?;

            let info = info.assume_init();
            if info.iBufferStatus != 1 {
                // ステータスが 1 以外ならまだデコード結果は存在しない。
                // B フレームを扱っていない場合でも、そもそも `data` に映像フレームを含まない NAL ユニットを
                // 指定することはできるので、ここに来る可能性はある。
                return Ok(None);
            }

            if info.UsrData.sSystemBuffer.iFormat != sys::EVideoFormatType_videoFormatI420 as c_int
            {
                // I420 以外は想定外
                return Err(Error::UnsupportedFormat {
                    format: info.UsrData.sSystemBuffer.iFormat as sys::EVideoFormatType,
                });
            }

            Ok(Some(DecodedFrame {
                info,
                _lifetime: PhantomData,
            }))
        }
    }

    /// これ以上データが来ないことをデコーダーに伝えて残りの結果を取得する
    pub fn finish(&mut self) -> Result<Option<DecodedFrame>, Error> {
        let mut info = MaybeUninit::<sys::SBufferInfo>::zeroed();
        unsafe {
            let mut yuv = [std::ptr::null_mut(); 3];
            let name = "ISVCDecoder.FlushFrame";
            let code = (**self.inner)
                .FlushFrame
                .ok_or(Error::UnavailableMethod(name))?(
                self.inner,
                yuv.as_mut_ptr(),
                info.as_mut_ptr(),
            );
            Error::check(code as c_int, name)?;

            let info = info.assume_init();
            if info.iBufferStatus != 1 {
                // ステータスが 1 以外ならデコード結果は存在しない。
                return Ok(None);
            }

            if info.UsrData.sSystemBuffer.iFormat != sys::EVideoFormatType_videoFormatI420 as c_int
            {
                // I420 以外は想定外
                return Err(Error::UnsupportedFormat {
                    format: info.UsrData.sSystemBuffer.iFormat as sys::EVideoFormatType,
                });
            }

            Ok(Some(DecodedFrame {
                info,
                _lifetime: PhantomData,
            }))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if let Some(uninitialize) = (**self.inner).Uninitialize {
                uninitialize(self.inner);
            }
            let _ = self
                .lib
                .call("WelsDestroyDecoder", |f: Symbol<WelsDestroyDecoder>| {
                    f(self.inner)
                });
        }
    }
}

unsafe impl Send for Decoder {}

/// デコードされた映像フレーム (I420 形式)
pub struct DecodedFrame<'a> {
    info: sys::SBufferInfo,

    // info の中には openh264 が返した一時的なデータへの参照も含まれているので、
    // このライフタイムで利用側での使用範囲を制限する。
    _lifetime: PhantomData<&'a ()>,
}

impl DecodedFrame<'_> {
    /// フレームの Y 成分のデータを返す
    pub fn y_plane(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.info.pDst[0], self.height() * self.y_stride()) }
    }

    /// フレームの U 成分のデータを返す
    pub fn u_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.info.pDst[1],
                self.height().div_ceil(2) * self.u_stride(),
            )
        }
    }

    /// フレームの V 成分のデータを返す
    pub fn v_plane(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.info.pDst[2],
                self.height().div_ceil(2) * self.v_stride(),
            )
        }
    }

    /// フレームの Y 成分のストライドを返す
    pub fn y_stride(&self) -> usize {
        unsafe { self.info.UsrData.sSystemBuffer.iStride[0] as usize }
    }

    /// フレームの U 成分のストライドを返す
    pub fn u_stride(&self) -> usize {
        unsafe { self.info.UsrData.sSystemBuffer.iStride[1] as usize }
    }

    /// フレームの V 成分のストライドを返す
    pub fn v_stride(&self) -> usize {
        // U と V のストライドは等しい
        unsafe { self.info.UsrData.sSystemBuffer.iStride[1] as usize }
    }

    /// フレームの幅を返す
    pub fn width(&self) -> usize {
        unsafe { self.info.UsrData.sSystemBuffer.iWidth as usize }
    }

    /// フレームの高さを返す
    pub fn height(&self) -> usize {
        unsafe { self.info.UsrData.sSystemBuffer.iHeight as usize }
    }
}

impl std::fmt::Debug for DecodedFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecodedFrame").finish_non_exhaustive()
    }
}

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

    /// 複雑度モード (LOW_COMPLEXITY, MEDIUM_COMPLEXITY, HIGH_COMPLEXITY)
    pub complexity_mode: ComplexityMode,

    /// エントロピー符号化モード (false: CAVLC, true: CABAC)
    pub entropy_coding: bool,

    /// 参照フレーム数 (1で最高速)
    pub ref_frame_count: NonZeroUsize,

    /// マルチスレッド数 (None なら自動)
    pub thread_count: Option<NonZeroUsize>,

    /// 空間レイヤー数 (1で最高速)
    pub spatial_layers: NonZeroUsize,

    /// 時間レイヤー数 (1で最高速)
    pub temporal_layers: NonZeroUsize,

    /// Intra フレーム間隔
    pub intra_period: Option<usize>,

    /// レート制御モード
    pub rate_control_mode: RateControlMode,

    /// 最大QP値
    pub max_qp: usize,

    /// 最小QP値
    pub min_qp: usize,

    /// ノイズ除去機能
    pub denoise: bool,

    /// 背景検出機能
    pub background_detection: bool,

    /// 適応量子化機能
    pub adaptive_quantization: bool,

    /// シーン変化検出機能
    pub scene_change_detection: bool,

    /// デブロッキングフィルタ
    pub deblocking_filter: bool,

    /// 長期参照フレーム機能
    pub long_term_reference: bool,

    /// スライスモード
    pub slice_mode: SliceMode,
}

/// 複雑度モード
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityMode {
    /// 最低複雑度 (最高速)
    #[default]
    Low,
    /// 中程度複雑度
    Medium,
    /// 高複雑度 (最高品質)
    High,
}

/// レート制御モード
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    /// レート制御無効 (最高速)
    Off,
    /// 品質モード
    #[default]
    Quality,
    /// ビットレートモード
    Bitrate,
    /// タイムスタンプモード
    Timestamp,
}

/// スライスモード
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SliceMode {
    /// 単一スライス (最高速)
    #[default]
    Single,
    /// 固定スライス数
    FixedCount(usize),
    /// サイズ制限スライス
    SizeConstrained(usize),
}

impl EncoderConfig {
    /// 最高速度優先の設定
    pub fn fastest() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,

            // 高速化設定
            complexity_mode: ComplexityMode::Low,
            entropy_coding: false, // CAVLC
            ref_frame_count: NonZeroUsize::MIN,
            thread_count: None, // 自動
            spatial_layers: NonZeroUsize::MIN,
            temporal_layers: NonZeroUsize::MIN,
            intra_period: Some(30),
            rate_control_mode: RateControlMode::Off,
            max_qp: 51,
            min_qp: 0,

            // 前処理機能を全て無効化
            denoise: false,
            background_detection: false,
            adaptive_quantization: false,
            scene_change_detection: false,
            deblocking_filter: false,
            long_term_reference: false,
            slice_mode: SliceMode::Single,
        }
    }

    /// 高速オフライン用設定
    pub fn fast_offline() -> Self {
        Self {
            width: 1920,
            height: 1080,
            target_bitrate: 2_000_000,
            fps_numerator: 30,
            fps_denominator: 1,

            // バランス重視の高速化設定
            complexity_mode: ComplexityMode::Low,
            entropy_coding: false, // CAVLC
            ref_frame_count: NonZeroUsize::MIN,
            thread_count: None, // 自動
            spatial_layers: NonZeroUsize::MIN,
            temporal_layers: NonZeroUsize::MIN,
            intra_period: Some(60),
            rate_control_mode: RateControlMode::Bitrate,
            max_qp: 40,
            min_qp: 10,

            // 最低限の前処理機能のみ有効
            denoise: false,
            background_detection: false,
            adaptive_quantization: false,
            scene_change_detection: false,
            deblocking_filter: true, // 品質維持のため有効
            long_term_reference: false,
            slice_mode: SliceMode::Single,
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

            // 品質重視設定
            complexity_mode: ComplexityMode::High,
            entropy_coding: true, // CABAC
            ref_frame_count: NonZeroUsize::MIN.saturating_add(3),
            thread_count: None, // 自動
            spatial_layers: NonZeroUsize::MIN,
            temporal_layers: NonZeroUsize::MIN,
            intra_period: Some(120),
            rate_control_mode: RateControlMode::Quality,
            max_qp: 30,
            min_qp: 5,

            // 前処理機能を有効活用
            denoise: true,
            background_detection: true,
            adaptive_quantization: true,
            scene_change_detection: true,
            deblocking_filter: true,
            long_term_reference: true,
            slice_mode: SliceMode::Single,
        }
    }
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self::fast_offline()
    }
}

/// H.264 エンコーダー
#[derive(Debug)]
pub struct Encoder {
    lib: Openh264Library,
    inner: *mut sys::ISVCEncoder,
    pic: sys::SSourcePicture,
    frames: usize,
    fps_numerator: usize,
    fps_denominator: usize,
}

impl Encoder {
    /// エンコーダーインスタンスを生成する
    pub fn new(lib: Openh264Library, config: &EncoderConfig) -> Result<Self, Error> {
        let mut inner = std::ptr::null_mut();
        let mut param = MaybeUninit::<sys::SEncParamExt>::zeroed();
        let pic = MaybeUninit::<sys::SSourcePicture>::zeroed();
        unsafe {
            let name = "WelsCreateSVCEncoder";
            let code = lib.call(name, |f: Symbol<WelsCreateSVCEncoder>| f(&mut inner))?;
            Error::check(code, name)?;

            let name = "ISVCEncoder.GetDefaultParams";
            let code = (**inner)
                .GetDefaultParams
                .ok_or(Error::UnavailableMethod(name))?(
                inner, param.as_mut_ptr()
            );
            Error::check(code, name)?;

            let mut param = param.assume_init();

            // 基本設定
            param.iUsageType = sys::EUsageType_CAMERA_VIDEO_REAL_TIME;
            param.fMaxFrameRate = config.fps_numerator as f32 / config.fps_denominator as f32;
            param.iPicWidth = config.width as c_int;
            param.iPicHeight = config.height as c_int;
            param.iTargetBitrate = config.target_bitrate as c_int;

            // 複雑度モード設定
            param.iComplexityMode = match config.complexity_mode {
                ComplexityMode::Low => sys::ECOMPLEXITY_MODE_LOW_COMPLEXITY,
                ComplexityMode::Medium => sys::ECOMPLEXITY_MODE_MEDIUM_COMPLEXITY,
                ComplexityMode::High => sys::ECOMPLEXITY_MODE_HIGH_COMPLEXITY,
            };

            // エントロピー符号化設定
            param.iEntropyCodingModeFlag = if config.entropy_coding { 1 } else { 0 };

            // 参照フレーム数設定
            param.iNumRefFrame = config.ref_frame_count.get() as c_int;

            // マルチスレッド設定
            param.iMultipleThreadIdc = config.thread_count.map_or(0, |v| v.get()) as c_ushort;

            // 空間レイヤー数設定
            param.iSpatialLayerNum = config.spatial_layers.get() as c_int;

            // 時間レイヤー数設定
            param.iTemporalLayerNum = config.temporal_layers.get() as c_int;

            // Intra期間設定
            if let Some(intra) = config.intra_period {
                param.uiIntraPeriod = intra as c_uint;
            }

            // レート制御モード設定
            param.iRCMode = match config.rate_control_mode {
                RateControlMode::Off => sys::RC_MODES_RC_OFF_MODE,
                RateControlMode::Quality => sys::RC_MODES_RC_QUALITY_MODE,
                RateControlMode::Bitrate => sys::RC_MODES_RC_BITRATE_MODE,
                RateControlMode::Timestamp => sys::RC_MODES_RC_TIMESTAMP_MODE,
            };

            // QP設定
            param.iMaxQp = config.max_qp as i32;
            param.iMinQp = config.min_qp as i32;

            // 前処理機能設定
            param.bEnableDenoise = config.denoise;
            param.bEnableBackgroundDetection = config.background_detection;
            param.bEnableAdaptiveQuant = config.adaptive_quantization;
            param.bEnableSceneChangeDetect = config.scene_change_detection;

            // デブロッキングフィルタ設定
            param.iLoopFilterDisableIdc = if config.deblocking_filter { 0 } else { 1 };

            // 長期参照フレーム設定
            if config.long_term_reference {
                param.bEnableLongTermReference = true;
                param.iLTRRefNum = 1; // 基本的に1つの長期参照フレーム
            } else {
                param.bEnableLongTermReference = false;
                param.iLTRRefNum = 0;
            }

            // 空間レイヤー設定
            for layer in &mut param.sSpatialLayers[..param.iSpatialLayerNum as usize] {
                layer.uiLevelIdc = LEVEL;
                layer.uiProfileIdc = PROFILE;
                layer.iVideoWidth = config.width as c_int;
                layer.iVideoHeight = config.height as c_int;
                layer.fFrameRate = param.fMaxFrameRate;
                layer.iSpatialBitrate = config.target_bitrate as c_int;
                layer.iMaxSpatialBitrate = (config.target_bitrate * 2) as c_int; // 2倍をmax値として設定

                // スライスモード設定
                match config.slice_mode {
                    SliceMode::Single => {
                        layer.sSliceArgument.uiSliceMode = sys::SliceModeEnum_SM_SINGLE_SLICE;
                    }
                    SliceMode::FixedCount(count) => {
                        layer.sSliceArgument.uiSliceMode = sys::SliceModeEnum_SM_FIXEDSLCNUM_SLICE;
                        layer.sSliceArgument.uiSliceNum = count as c_uint;
                    }
                    SliceMode::SizeConstrained(size) => {
                        layer.sSliceArgument.uiSliceMode = sys::SliceModeEnum_SM_SIZELIMITED_SLICE;
                        layer.sSliceArgument.uiSliceSizeConstraint = size as c_uint;
                    }
                }
            }

            let name = "ISVCEncoder.InitializeExt";
            let code = (**inner)
                .InitializeExt
                .ok_or(Error::UnavailableMethod(name))?(inner, &param);
            Error::check(code, name)?;

            // I420 フォーマット設定
            let mut i420 = sys::EVideoFormatType_videoFormatI420;
            let name = "ISVCEncoder.SetOption";
            let code = (**inner).SetOption.ok_or(Error::UnavailableMethod(name))?(
                inner,
                sys::ENCODER_OPTION_ENCODER_OPTION_DATAFORMAT,
                ((&mut i420) as *mut u32).cast(),
            );
            Error::check(code, name)?;

            // 画像設定
            let mut pic = pic.assume_init();
            pic.iPicWidth = config.width as c_int;
            pic.iPicHeight = config.height as c_int;
            pic.iColorFormat = i420 as c_int;
            pic.iStride[0] = pic.iPicWidth;
            pic.iStride[1] = config.width.div_ceil(2) as c_int;
            pic.iStride[2] = config.width.div_ceil(2) as c_int;

            Ok(Self {
                lib,
                inner,
                pic,
                frames: 0,
                fps_numerator: config.fps_numerator,
                fps_denominator: config.fps_denominator,
            })
        }
    }

    /// I420 形式の画像データをエンコードする
    ///
    /// なお `y` のストライドは入力フレームの幅と等しいことが前提
    ///
    /// また B フレームは扱わない前提（つまり入力フレームと出力フレームの順番が一致する）
    pub fn encode(&mut self, y: &[u8], u: &[u8], v: &[u8]) -> Result<Option<EncodedFrame>, Error> {
        let height = self.pic.iPicHeight as usize;
        let y_size = height * self.pic.iStride[0] as usize;
        let u_size = height.div_ceil(2) * self.pic.iStride[1] as usize;
        let v_size = u_size;
        if y.len() != y_size || u.len() != u_size || v.len() != v_size {
            return Err(Error::InvalidYuvSize);
        }

        unsafe {
            self.pic.pData[0] = y.as_ptr().cast_mut();
            self.pic.pData[1] = u.as_ptr().cast_mut();
            self.pic.pData[2] = v.as_ptr().cast_mut();

            let timestamp = Duration::from_secs((self.frames * self.fps_denominator) as u64)
                / self.fps_numerator as u32;
            self.pic.uiTimeStamp = timestamp.as_millis() as c_longlong; // openh264 はミリ秒固定
            self.frames += 1;

            let mut info = MaybeUninit::<sys::SFrameBSInfo>::zeroed();
            let name = "ISVCEncoder.InitializeExt";
            let code = (**self.inner)
                .EncodeFrame
                .ok_or(Error::UnavailableMethod(name))?(
                self.inner,
                &mut self.pic,
                info.as_mut_ptr(),
            );
            Error::check(code, name)?;

            let info = info.assume_init();
            if info.eFrameType == sys::EVideoFrameType_videoFrameTypeSkip {
                return Ok(None);
            }

            let mut data = Vec::new();
            for layer_info in &info.sLayerInfo[..info.iLayerNum as usize] {
                if layer_info.iNalCount == 0 {
                    // カウントがゼロの場合には、環境によっては、
                    // pNalLengthInByte が不正なアドレスを指していて from_raw_parts() がクラッシュする
                    // 可能性があるので明示的にハンドリングする
                    continue;
                }

                let data_size = std::slice::from_raw_parts(
                    layer_info.pNalLengthInByte,
                    layer_info.iNalCount as usize,
                )
                .iter()
                .map(|n| *n as usize)
                .sum::<usize>();
                data.extend_from_slice(std::slice::from_raw_parts(layer_info.pBsBuf, data_size));
            }

            Ok(Some(EncodedFrame {
                keyframe: info.eFrameType == sys::EVideoFrameType_videoFrameTypeIDR,
                data,
            }))
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            if let Some(uninitialize) = (**self.inner).Uninitialize {
                uninitialize(self.inner);
            }
            let _ = self.lib.call(
                "WelsDestroySVCEncoder",
                |f: Symbol<WelsDestroySVCEncoder>| f(self.inner),
            );
        }
    }
}

unsafe impl Send for Encoder {}

/// エンコードされた映像フレーム
#[derive(Debug)]
pub struct EncodedFrame {
    /// キーフレームかどうか
    pub keyframe: bool,

    /// 圧縮データ
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_library() {
        let Ok(path) = std::env::var("OPENH264_PATH") else {
            panic!("OPENH264_PATH env var is not found");
        };

        assert!(Openh264Library::load(path).is_ok());
    }

    #[test]
    fn init_decoder() {
        let Ok(path) = std::env::var("OPENH264_PATH") else {
            panic!("OPENH264_PATH env var is not found");
        };

        let lib = Openh264Library::load(path).expect("load library error");
        assert!(Decoder::new(lib).is_ok());
    }

    #[test]
    fn decode_black() {
        let Ok(path) = std::env::var("OPENH264_PATH") else {
            panic!("OPENH264_PATH env var is not found");
        };

        let lib = Openh264Library::load(path).expect("load library error");
        let mut decoder = Decoder::new(lib).expect("create decoder error");
        let mut decoded_count = 0;

        let data = [
            // SPS
            0, 0, 0, 1, 103, 100, 0, 30, 172, 217, 64, 160, 61, 176, 17, 0, 0, 3, 0, 1, 0, 0, 3, 0,
            50, 15, 22, 45, 150, //
            // PPS
            0, 0, 0, 1, 104, 235, 227, 203, 34, 192, //
            // 映像データ
            0, 0, 0, 1, 101, 136, 132, 0, 43, 255, 254, 246, 115, 124, 10, 107, 109, 176, 149, 46,
            5, 118, 247, 102, 163, 229, 208, 146, 229, 251, 16, 96, 250, 208, 0, 0, 3, 0, 0, 3, 0,
            0, 16, 15, 210, 222, 245, 204, 98, 91, 229, 32, 0, 0, 9, 216, 2, 56, 13, 16, 118, 133,
            116, 69, 196, 32, 71, 6, 120, 150, 16, 161, 210, 50, 128, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0,
            0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 0, 3, 0, 37, 225,
        ];
        decoded_count += decoder.decode(&data).expect("decode error").is_some() as usize;
        decoded_count += decoder.finish().expect("decode error").is_some() as usize;
        assert_eq!(decoded_count, 1);
    }

    #[test]
    fn init_encoder() {
        let Ok(path) = std::env::var("OPENH264_PATH") else {
            panic!("OPENH264_PATH env var is not found");
        };

        let lib = Openh264Library::load(path).expect("load library error");
        let config = EncoderConfig {
            fps_denominator: 1,
            fps_numerator: 1,
            width: 64,
            height: 64,
            target_bitrate: 100_000,
            ..Default::default()
        };
        assert!(Encoder::new(lib, &config).is_ok());
    }

    #[test]
    fn encode_black() {
        let Ok(path) = std::env::var("OPENH264_PATH") else {
            panic!("OPENH264_PATH env var is not found");
        };

        let lib = Openh264Library::load(path).expect("load library error");
        let config = EncoderConfig {
            fps_denominator: 1,
            fps_numerator: 1,
            width: 64,
            height: 64,
            target_bitrate: 100_000,
            ..Default::default()
        };
        let mut encoder = Encoder::new(lib, &config).expect("create encoder error");
        let encoded = encoder
            .encode(&[0; 64 * 64], &[0; 32 * 32], &[0; 32 * 32])
            .expect("encode error");
        assert!(encoded.is_some());
    }
}
