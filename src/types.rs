//! 雑多な型定義をまとめたモジュール
use std::str::FromStr;

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
            "Opus" => Ok(Self::Opus), // コマンドライン引数パース時には以前の Hisui に合わせた名前にする
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

/// エンジン名
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EngineName {
    AudioToolbox,
    Dav1d,
    FdkAac,
    Libvpx,
    Openh264,
    Opus,
    SvtAv1,
    VideoToolbox,
}

impl nojson::DisplayJson for EngineName {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.as_str())
    }
}

impl EngineName {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineName::AudioToolbox => "audio_toolbox",
            EngineName::Dav1d => "dav1d",
            EngineName::FdkAac => "fdk_aac",
            EngineName::Libvpx => "libvpx",
            EngineName::Openh264 => "openh264",
            EngineName::Opus => "opus",
            EngineName::SvtAv1 => "svt_av1",
            EngineName::VideoToolbox => "video_toolbox",
        }
    }
}

/// 画像内でのピクセル位置を表現するための構造体
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelPosition {
    pub x: EvenUsize,
    pub y: EvenUsize,
}

/// YUV (I420) 画像のサイズや位置を表現するための構造体
///
/// 通常の usize と同様だが、I420 に合わせて奇数が表現できないようになっている
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EvenUsize(usize);

impl EvenUsize {
    pub const MIN_CELL_SIZE: Self = Self(16);

    pub const fn new(n: usize) -> Option<Self> {
        if n % 2 == 0 { Some(Self(n)) } else { None }
    }

    pub const fn truncating_new(n: usize) -> Self {
        if n % 2 == 0 { Self(n) } else { Self(n - 1) }
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
