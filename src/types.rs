//! 雑多な型定義をまとめたモジュール
use std::{path::Path, str::FromStr, sync::OnceLock, time::Duration};

/// コーデック名
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CodecName {
    // Audio
    Aac,
    Opus,

    // Video
    H264,
    H265,
    Vp8,
    Vp9,
    Av1,
}

impl nojson::DisplayJson for CodecName {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.as_str())
    }
}

impl CodecName {
    pub fn as_str(self) -> &'static str {
        match self {
            CodecName::Opus => "OPUS",
            CodecName::Aac => "AAC",
            CodecName::H264 => "H264",
            CodecName::H265 => "H265",
            CodecName::Vp8 => "VP8",
            CodecName::Vp9 => "VP9",
            CodecName::Av1 => "AV1",
        }
    }

    pub fn parse_audio(s: &str) -> Result<Self, String> {
        match s {
            "OPUS" => Ok(Self::Opus),
            "AAC" => Ok(Self::Aac),
            _ => Err(format!("unknown audio codec name: {s}")),
        }
    }

    pub fn parse_video(s: &str) -> Result<Self, String> {
        let codec = s.parse()?;
        if matches!(
            codec,
            Self::H264 | Self::H265 | Self::Vp8 | Self::Vp9 | Self::Av1
        ) {
            Ok(codec)
        } else {
            Err(format!("{s} is not a video codec"))
        }
    }
}

impl FromStr for CodecName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "OPUS" => Ok(Self::Opus),
            "AAC" => Ok(Self::Aac),
            "H264" => Ok(Self::H264),
            "H265" => Ok(Self::H265),
            "VP8" => Ok(Self::Vp8),
            "VP9" => Ok(Self::Vp9),
            "AV1" => Ok(Self::Av1),
            _ => Err(format!("unknown codec name: {s}")),
        }
    }
}

/// コンテナー形式
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFormat {
    #[default]
    Webm,
    Mp4,
}

impl ContainerFormat {
    pub fn from_path<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let ext = path.as_ref().extension().ok_or_else(|| {
            crate::Error::new(format!(
                "no media file extension: {}",
                path.as_ref().display()
            ))
        })?;
        if ext == "mp4" {
            Ok(Self::Mp4)
        } else if ext == "webm" {
            Ok(Self::Webm)
        } else {
            Err(crate::Error::new(format!(
                "unexpected media file extension: {}",
                path.as_ref().display()
            )))
        }
    }
}

impl nojson::DisplayJson for ContainerFormat {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            ContainerFormat::Webm => f.string("webm"),
            ContainerFormat::Mp4 => f.string("mp4"),
        }
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ContainerFormat {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        match value.to_unquoted_string_str()?.as_ref() {
            "webm" => Ok(Self::Webm),
            "mp4" => Ok(Self::Mp4),
            v => Err(value.invalid(format!("unknown container format: {v}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EngineName {
    AudioToolbox,
    Dav1d,
    FdkAac,
    Libvpx,
    Nvcodec,
    Openh264,
    Opus,
    SvtAv1,
    VideoToolbox,
}

// ハードウェア対応状況はプロセス実行中に変化しないため、初回の結果をキャッシュして
// エンジン選択のたびに各クレートの supported_codecs() を呼ぶオーバーヘッドを避ける

#[cfg(target_os = "macos")]
fn video_toolbox_supported_codecs() -> &'static [shiguredo_video_toolbox::CodecInfo] {
    static CACHE: OnceLock<Vec<shiguredo_video_toolbox::CodecInfo>> = OnceLock::new();
    CACHE.get_or_init(shiguredo_video_toolbox::supported_codecs)
}

// ソフトウェアコーデックの結果は常に同じだが、VideoToolbox と統一したパターンとしてキャッシュする
fn libvpx_supported_codecs() -> &'static [shiguredo_libvpx::CodecInfo] {
    static CACHE: OnceLock<Vec<shiguredo_libvpx::CodecInfo>> = OnceLock::new();
    CACHE.get_or_init(shiguredo_libvpx::supported_codecs)
}

fn dav1d_supported_codecs() -> &'static [shiguredo_dav1d::CodecInfo] {
    static CACHE: OnceLock<Vec<shiguredo_dav1d::CodecInfo>> = OnceLock::new();
    CACHE.get_or_init(shiguredo_dav1d::supported_codecs)
}

fn svt_av1_supported_codecs() -> &'static [shiguredo_svt_av1::CodecInfo] {
    static CACHE: OnceLock<Vec<shiguredo_svt_av1::CodecInfo>> = OnceLock::new();
    CACHE.get_or_init(shiguredo_svt_av1::supported_codecs)
}

/// CodecName から shiguredo_video_toolbox の VideoCodecType へのマッピング
#[cfg(target_os = "macos")]
fn to_video_toolbox_codec(codec: CodecName) -> Option<shiguredo_video_toolbox::VideoCodecType> {
    match codec {
        CodecName::H264 => Some(shiguredo_video_toolbox::VideoCodecType::H264),
        CodecName::H265 => Some(shiguredo_video_toolbox::VideoCodecType::Hevc),
        CodecName::Vp9 => Some(shiguredo_video_toolbox::VideoCodecType::Vp9),
        CodecName::Av1 => Some(shiguredo_video_toolbox::VideoCodecType::Av1),
        _ => None,
    }
}

/// CodecName から shiguredo_libvpx の VideoCodecType へのマッピング
fn to_libvpx_codec(codec: CodecName) -> Option<shiguredo_libvpx::VideoCodecType> {
    match codec {
        CodecName::Vp8 => Some(shiguredo_libvpx::VideoCodecType::Vp8),
        CodecName::Vp9 => Some(shiguredo_libvpx::VideoCodecType::Vp9),
        _ => None,
    }
}

/// CodecName から shiguredo_dav1d の VideoCodecType へのマッピング
fn to_dav1d_codec(codec: CodecName) -> Option<shiguredo_dav1d::VideoCodecType> {
    match codec {
        CodecName::Av1 => Some(shiguredo_dav1d::VideoCodecType::Av1),
        _ => None,
    }
}

/// CodecName から shiguredo_svt_av1 の VideoCodecType へのマッピング
fn to_svt_av1_codec(codec: CodecName) -> Option<shiguredo_svt_av1::VideoCodecType> {
    match codec {
        CodecName::Av1 => Some(shiguredo_svt_av1::VideoCodecType::Av1),
        _ => None,
    }
}

impl EngineName {
    // NOTE: 先頭の方が優先順位が高い
    pub fn default_video_decoders(is_openh264_enabled: bool) -> Vec<Self> {
        let mut engines = Vec::new();

        if is_openh264_enabled {
            engines.push(Self::Openh264);
        }
        #[cfg(feature = "nvcodec")]
        if shiguredo_nvcodec::is_cuda_library_available() {
            engines.push(Self::Nvcodec);
        }
        #[cfg(target_os = "macos")]
        {
            engines.push(Self::VideoToolbox);
        }
        engines.push(Self::Dav1d);
        engines.push(Self::Libvpx);

        engines
    }

    pub fn is_available_video_decode_codec(self, codec: CodecName) -> bool {
        match self {
            EngineName::Libvpx => to_libvpx_codec(codec).is_some_and(|c| {
                libvpx_supported_codecs()
                    .iter()
                    .any(|info| info.codec == c && info.decoding.supported)
            }),
            #[cfg(feature = "nvcodec")]
            EngineName::Nvcodec => {
                matches!(
                    codec,
                    CodecName::H264
                        | CodecName::H265
                        | CodecName::Vp8
                        | CodecName::Vp9
                        | CodecName::Av1
                )
            }
            EngineName::Openh264 => matches!(codec, CodecName::H264),
            EngineName::Dav1d => to_dav1d_codec(codec).is_some_and(|c| {
                dav1d_supported_codecs()
                    .iter()
                    .any(|info| info.codec == c && info.decoding.supported)
            }),
            #[cfg(target_os = "macos")]
            EngineName::VideoToolbox => to_video_toolbox_codec(codec).is_some_and(|c| {
                video_toolbox_supported_codecs()
                    .iter()
                    .any(|info| info.codec == c && info.decoding.supported)
            }),
            _ => false,
        }
    }

    // NOTE: 先頭の方が優先順位が高い
    pub fn default_video_encoders(is_openh264_enabled: bool) -> Vec<Self> {
        let mut engines = Vec::new();

        if is_openh264_enabled {
            engines.push(Self::Openh264);
        }
        #[cfg(feature = "nvcodec")]
        if shiguredo_nvcodec::is_cuda_library_available() {
            engines.push(Self::Nvcodec);
        }
        #[cfg(target_os = "macos")]
        {
            engines.push(Self::VideoToolbox);
        }
        engines.push(Self::SvtAv1);
        engines.push(Self::Libvpx);

        engines
    }

    pub fn is_available_video_encode_codec(self, codec: CodecName) -> bool {
        match self {
            EngineName::Libvpx => to_libvpx_codec(codec).is_some_and(|c| {
                libvpx_supported_codecs()
                    .iter()
                    .any(|info| info.codec == c && info.encoding.supported)
            }),
            #[cfg(feature = "nvcodec")]
            EngineName::Nvcodec => {
                matches!(codec, CodecName::H264 | CodecName::H265 | CodecName::Av1)
            }
            EngineName::Openh264 => matches!(codec, CodecName::H264),
            EngineName::SvtAv1 => to_svt_av1_codec(codec).is_some_and(|c| {
                svt_av1_supported_codecs()
                    .iter()
                    .any(|info| info.codec == c && info.encoding.supported)
            }),
            #[cfg(target_os = "macos")]
            EngineName::VideoToolbox => to_video_toolbox_codec(codec).is_some_and(|c| {
                video_toolbox_supported_codecs()
                    .iter()
                    .any(|info| info.codec == c && info.encoding.supported)
            }),
            _ => false,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EngineName::AudioToolbox => "audio_toolbox",
            EngineName::Dav1d => "dav1d",
            EngineName::FdkAac => "fdk_aac",
            EngineName::Libvpx => "libvpx",
            EngineName::Nvcodec => "nvcodec",
            EngineName::Openh264 => "openh264",
            EngineName::Opus => "opus",
            EngineName::SvtAv1 => "svt_av1",
            EngineName::VideoToolbox => "video_toolbox",
        }
    }

    pub fn parse_video_encoder(
        value: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let s = value.to_unquoted_string_str()?;
        match s.as_ref() {
            "libvpx" => Ok(Self::Libvpx),
            "nvcodec" => {
                #[cfg(feature = "nvcodec")]
                {
                    Ok(Self::Nvcodec)
                }
                #[cfg(not(feature = "nvcodec"))]
                {
                    Err(value.invalid("nvcodec feature is not enabled"))
                }
            }
            "openh264" => Ok(Self::Openh264),
            "svt_av1" => Ok(Self::SvtAv1),
            "video_toolbox" => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::VideoToolbox)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(value.invalid("video_toolbox is only available on macOS"))
                }
            }
            "audio_toolbox" | "dav1d" | "fdk_aac" | "opus" => {
                Err(value.invalid(format!("{s} is not a video encoder")))
            }
            _ => Err(value.invalid(format!("unknown video encoder: {s}"))),
        }
    }

    pub fn parse_video_decoder(
        value: nojson::RawJsonValue<'_, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        let s = value.to_unquoted_string_str()?;
        match s.as_ref() {
            "libvpx" => Ok(Self::Libvpx),
            "nvcodec" => {
                #[cfg(feature = "nvcodec")]
                {
                    Ok(Self::Nvcodec)
                }
                #[cfg(not(feature = "nvcodec"))]
                {
                    Err(value.invalid("nvcodec feature is not enabled"))
                }
            }
            "openh264" => Ok(Self::Openh264),
            "dav1d" => Ok(Self::Dav1d),
            "video_toolbox" => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::VideoToolbox)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(value.invalid("video_toolbox is only available on macOS"))
                }
            }
            "audio_toolbox" | "fdk_aac" | "opus" | "svt_av1" => {
                Err(value.invalid(format!("{s} is not a video decoder")))
            }
            _ => Err(value.invalid(format!("unknown video decoder: {s}"))),
        }
    }
}

impl nojson::DisplayJson for EngineName {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.as_str())
    }
}

/// 画像内でのピクセル位置を表現するための構造体
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelPosition {
    pub x: EvenUsize,
    pub y: EvenUsize,
}

/// 奇数が表現できない usize のための構造体
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EvenUsize(usize);

impl EvenUsize {
    pub const ZERO: Self = Self(0);

    pub const MIN_CELL_SIZE: Self = Self(16);

    pub const fn new(n: usize) -> Option<Self> {
        if n.is_multiple_of(2) {
            Some(Self(n))
        } else {
            None
        }
    }

    pub const fn truncating_new(n: usize) -> Self {
        if n.is_multiple_of(2) {
            Self(n)
        } else {
            Self(n - 1)
        }
    }

    pub const fn ceiling_new(n: usize) -> Self {
        if n.is_multiple_of(2) {
            Self(n)
        } else {
            Self(n + 1)
        }
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

impl nojson::DisplayJson for EvenUsize {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.0)
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for EvenUsize {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let n = value.try_into()?;
        Self::new(n).ok_or_else(|| value.invalid(format!("expected even number, got {n}")))
    }
}

impl std::str::FromStr for EvenUsize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n: usize = s.parse().map_err(|e| format!("{e}"))?;
        Self::new(n).ok_or_else(|| format!("expected even number, got {n}"))
    }
}

impl std::ops::Add for EvenUsize {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for EvenUsize {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl std::ops::Sub for EvenUsize {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::Mul for EvenUsize {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl std::ops::Mul<usize> for EvenUsize {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self::Output {
        Self(self.0 * rhs)
    }
}

/// 正の有限な f64 のための構造体
///
/// 正の有限値のみを保持するため、NaN や無限が存在せず全順序が成立する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositiveFiniteF64(f64);

// 正の有限値のみを保持するため、PartialEq の反射律が成立する
impl Eq for PositiveFiniteF64 {}

impl Ord for PositiveFiniteF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for PositiveFiniteF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PositiveFiniteF64 {
    pub const ONE: Self = Self(1.0);

    pub fn new(n: f64) -> Option<Self> {
        if n.is_finite() && n > 0.0 {
            Some(Self(n))
        } else {
            None
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl nojson::DisplayJson for PositiveFiniteF64 {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.0)
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for PositiveFiniteF64 {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let n: f64 = value.try_into()?;
        Self::new(n).ok_or_else(|| value.invalid("expected positive finite number"))
    }
}

impl std::fmt::Display for PositiveFiniteF64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 0.0 以上の有限な f64 値。
///
/// 非負の有限値のみを保持するため、NaN や無限が存在せず全順序が成立する。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NonNegFiniteF64(f64);

impl Eq for NonNegFiniteF64 {}

impl Ord for NonNegFiniteF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for NonNegFiniteF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl NonNegFiniteF64 {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    pub fn new(n: f64) -> Option<Self> {
        if n.is_finite() && n >= 0.0 {
            Some(Self(n))
        } else {
            None
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl nojson::DisplayJson for NonNegFiniteF64 {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.0)
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for NonNegFiniteF64 {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let n: f64 = value.try_into()?;
        Self::new(n).ok_or_else(|| value.invalid("expected non-negative finite number"))
    }
}

impl std::fmt::Display for NonNegFiniteF64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// タイムオフセット
//
// フォーマット:
// - 数値 (単位: 秒)
// - "時:分:秒[.小数秒]" 形式の文字列
#[derive(Debug, Default, Clone, Copy)]
pub struct TimeOffset(Duration);

impl TimeOffset {
    pub fn get(self) -> Duration {
        self.0
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for TimeOffset {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        if let Ok(n) = value.as_number_str() {
            let secs = n
                .parse()
                .map_err(|_| value.invalid("not a non negative finite number"))?;
            Ok(Self(Duration::from_secs_f64(secs)))
        } else if let Ok(s) = value.to_unquoted_string_str() {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() != 3 {
                return Err(value.invalid("time string must be in format HH:MM:SS[.fraction]"));
            }

            let hours: u64 = parts[0]
                .parse()
                .map_err(|_| value.invalid("invalid hour value"))?;
            let minutes: u64 = parts[1]
                .parse()
                .map_err(|_| value.invalid("invalid minute value"))?;
            let seconds: f64 = parts[2]
                .parse()
                .map_err(|_| value.invalid("invalid second value"))?;

            if minutes >= 60 {
                return Err(value.invalid("minutes must be less than 60"));
            }
            if seconds >= 60.0 {
                return Err(value.invalid("seconds must be less than 60"));
            }

            let total_duration =
                Duration::from_secs(hours * 3600 + minutes * 60) + Duration::from_secs_f64(seconds);
            Ok(Self(total_duration))
        } else {
            Err(value.invalid("expected number or time string in format HH:MM:SS[.fraction]"))
        }
    }
}
