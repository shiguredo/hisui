//! 雑多な型定義をまとめたモジュール
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

/// コーデック名
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
// TODO: #[serde(rename_all = "UPPERCASE")]
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
// TODO: #[serde(rename_all = "snake_case")]
pub enum EngineName {
    AudioToobox,
    Dav1d,
    FdkAac,
    Libvpx,
    Openh264,
    Opus,
    SvtAv1,
    VideoToolbox,
}

impl EngineName {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineName::AudioToobox => "audio_toolbox",
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
        if n % 2 == 0 {
            Some(Self(n))
        } else {
            None
        }
    }

    pub const fn truncating_new(n: usize) -> Self {
        if n % 2 == 0 {
            Self(n)
        } else {
            Self(n - 1)
        }
    }

    pub const fn get(self) -> usize {
        self.0
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

#[derive(Debug, Default)]
pub struct CodecEngines(BTreeMap<CodecName, Engines>);

impl CodecEngines {
    pub fn insert_decoder(&mut self, codec: CodecName, engine: EngineName) {
        self.0.entry(codec).or_default().decoders.insert(engine);
    }

    pub fn insert_encoder(&mut self, codec: CodecName, engine: EngineName) {
        self.0.entry(codec).or_default().encoders.insert(engine);
    }
}

impl nojson::DisplayJson for CodecEngines {
    fn fmt(&self, _f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        todo!()
    }
}

#[derive(Debug, Default)]
pub struct Engines {
    pub encoders: BTreeSet<EngineName>,
    pub decoders: BTreeSet<EngineName>,
}
