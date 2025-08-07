use std::{num::NonZeroUsize, str::FromStr, time::Duration};

use orfail::OrFail;
use shiguredo_mp4::boxes::{SampleEntry, VisualSampleEntryFields};

use crate::{
    metadata::SourceId,
    types::{CodecName, EvenUsize},
};

pub type VideoFrameSyncSender = crate::channel::SyncSender<VideoFrame>;
pub type VideoFrameReceiver = crate::channel::Receiver<VideoFrame>;

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub source_id: Option<SourceId>,
    pub data: Vec<u8>,
    pub format: VideoFormat,
    pub keyframe: bool,
    pub width: EvenUsize,
    pub height: EvenUsize,
    pub timestamp: Duration,
    pub duration: Duration,
    pub sample_entry: Option<SampleEntry>,
}

impl VideoFrame {
    pub fn new_i420(
        input_frame: Self,
        width: EvenUsize,
        height: EvenUsize,
        y_plane: &[u8],
        u_plane: &[u8],
        v_plane: &[u8],
        y_stride: usize,
        u_stride: usize,
        v_stride: usize,
    ) -> Self {
        let y_size = width.get() * height.get();
        let uv_size = width.get() / 2 * height.get() / 2;
        let mut data = Vec::with_capacity(y_size + uv_size * 2);

        // ストライドを考慮して YUV 成分をコピーする
        if width.get() == y_stride {
            // ストライドと横幅が同じならパディングバイトの考慮が不要
            data.extend_from_slice(y_plane);
        } else {
            for i in 0..height.get() {
                let offset = y_stride * i;
                data.extend_from_slice(&y_plane[offset..][..width.get()]);
            }
        }
        if width.get() / 2 == u_stride {
            data.extend_from_slice(u_plane);
        } else {
            for i in 0..height.get() / 2 {
                let offset = u_stride * i;
                data.extend_from_slice(&u_plane[offset..][..width.get() / 2]);
            }
        }
        if width.get() / 2 == v_stride {
            data.extend_from_slice(v_plane);
        } else {
            for i in 0..height.get() / 2 {
                let offset = v_stride * i;
                data.extend_from_slice(&v_plane[offset..][..width.get() / 2]);
            }
        }

        Self {
            source_id: input_frame.source_id,
            sample_entry: None, // 生データにはサンプルエントリは存在しない
            data,
            format: VideoFormat::I420,
            keyframe: true, // 生データは全てキーフレーム扱い
            width,
            height,
            timestamp: input_frame.timestamp,
            duration: input_frame.duration,
        }
    }

    pub fn mono_color(rgb: [u8; 3], width: EvenUsize, height: EvenUsize) -> Self {
        if rgb == [0, 0, 0] {
            // 典型的なユースケースでは最適化された実装を使う
            return Self::black(width, height);
        }

        // RGB から YUV に変換
        let r = rgb[0] as f32;
        let g = rgb[1] as f32;
        let b = rgb[2] as f32;

        // ITU-R BT.601 標準を使用した RGB から YUV への変換
        let y = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
        let u = ((-0.169 * r - 0.331 * g + 0.500 * b) + 128.0) as u8;
        let v = ((0.500 * r - 0.419 * g - 0.081 * b) + 128.0) as u8;

        let y_plane_size = width.get() * height.get();
        let u_plane_size = (width.get() / 2) * (height.get() / 2);
        let v_plane_size = u_plane_size;
        let total_size = y_plane_size + u_plane_size + v_plane_size;

        let mut data = Vec::with_capacity(total_size);

        // Y プレーンを埋める
        data.resize(y_plane_size, y);

        // U プレーンを埋める
        data.resize(y_plane_size + u_plane_size, u);

        // V プレーンを埋める
        data.resize(total_size, v);

        Self {
            source_id: None,
            data,
            format: VideoFormat::I420,
            keyframe: true,
            width,
            height,
            timestamp: Duration::ZERO,
            duration: Duration::ZERO,
            sample_entry: None,
        }
    }

    pub fn black(width: EvenUsize, height: EvenUsize) -> Self {
        let y_plane_size = width.get() * height.get();
        let u_plane_size = (width.get() / 2) * (height.get() / 2);
        let v_plane_size = u_plane_size;
        let total_size = y_plane_size + u_plane_size + v_plane_size;

        let mut data = vec![0; total_size];
        for i in y_plane_size..total_size {
            data[i] = 128;
        }

        Self {
            source_id: None,
            data,
            format: VideoFormat::I420,
            keyframe: true,
            width,
            height,
            timestamp: Duration::ZERO,
            duration: Duration::ZERO,
            sample_entry: None,
        }
    }

    pub fn as_yuv_planes(&self) -> Option<(&[u8], &[u8], &[u8])> {
        if self.format != VideoFormat::I420 {
            return None;
        }

        let y_size = self.width.get() * self.height.get();
        let uv_size = y_size / 4;

        let y_plane = &self.data[..y_size];
        let u_plane = &self.data[y_size..][..uv_size];
        let v_plane = &self.data[y_size + uv_size..][..uv_size];

        Some((y_plane, u_plane, v_plane))
    }

    pub fn end_timestamp(&self) -> Duration {
        self.timestamp + self.duration
    }

    /// ボックスフィルターアルゴリズムで YUV(I420) 画像をリサイズする
    pub fn resize(&mut self, new_width: EvenUsize, new_height: EvenUsize) -> orfail::Result<()> {
        (self.format == VideoFormat::I420).or_fail()?;

        let width = self.width;
        let height = self.height;
        if width == new_width && height == new_height {
            // リサイズ不要
            return Ok(());
        }

        // 新しい YUV バッファを作成
        let new_y_size = new_width.get() * new_height.get();
        let new_uv_size = (new_width.get() / 2) * (new_height.get() / 2);
        let mut new_data = vec![0; new_y_size + new_uv_size * 2];

        // 元のデータのサイズを計算
        let y_size = width.get() * height.get();
        let uv_width = width.get() / 2;
        let uv_height = height.get() / 2;

        // Y 平面のリサイズ
        let x_ratio = width.get() as f64 / new_width.get() as f64;
        let y_ratio = height.get() as f64 / new_height.get() as f64;
        for y in 0..new_height.get() {
            for x in 0..new_width.get() {
                // ボックスフィルターの開始位置
                let x_start = (x as f64 * x_ratio) as usize;
                let y_start = (y as f64 * y_ratio) as usize;

                // ボックスフィルターの終了位置
                let x_end = (((x as f64 + 1.0) * x_ratio) as usize).max(x_start + 1);
                let y_end = (((y as f64 + 1.0) * y_ratio) as usize).max(y_start + 1);

                // ボックス領域のピクセルの値を累積する
                let mut y_acc = 0u32;
                let mut count = 0u32;

                for box_y in y_start..y_end.min(height.get()) {
                    for box_x in x_start..x_end.min(width.get()) {
                        let i = box_y * width.get() + box_x;
                        y_acc += self.data[i] as u32;
                        count += 1;
                    }
                }

                // 新しいピクセル値を平均値で求める
                let i = y * new_width.get() + x;
                new_data[i] = (y_acc / count) as u8;
            }
        }

        // U平面のリサイズ
        let new_uv_width = new_width.get() / 2;
        let new_uv_height = new_height.get() / 2;
        let x_ratio_uv = uv_width as f64 / new_uv_width as f64;
        let y_ratio_uv = uv_height as f64 / new_uv_height as f64;
        for y in 0..new_uv_height {
            for x in 0..new_uv_width {
                // ボックスフィルターの開始位置
                let x_start = (x as f64 * x_ratio_uv) as usize;
                let y_start = (y as f64 * y_ratio_uv) as usize;

                // ボックスフィルターの終了位置
                let x_end = (((x as f64 + 1.0) * x_ratio_uv) as usize).max(x_start + 1);
                let y_end = (((y as f64 + 1.0) * y_ratio_uv) as usize).max(y_start + 1);

                // ボックス領域のピクセルの値を累積する
                let mut u_acc = 0u32;
                let mut count = 0u32;

                for box_y in y_start..y_end.min(uv_height) {
                    for box_x in x_start..x_end.min(uv_width) {
                        let i = y_size + box_y * uv_width + box_x;
                        u_acc += self.data[i] as u32;
                        count += 1;
                    }
                }

                // 新しいピクセル値を平均値で求める
                let i = new_y_size + y * new_uv_width + x;
                new_data[i] = (u_acc / count) as u8;
            }
        }

        // V平面のリサイズ
        for y in 0..new_uv_height {
            for x in 0..new_uv_width {
                // ボックスフィルターの開始位置
                let x_start = (x as f64 * x_ratio_uv) as usize;
                let y_start = (y as f64 * y_ratio_uv) as usize;

                // ボックスフィルターの終了位置
                let x_end = (((x as f64 + 1.0) * x_ratio_uv) as usize).max(x_start + 1);
                let y_end = (((y as f64 + 1.0) * y_ratio_uv) as usize).max(y_start + 1);

                // ボックス領域のピクセルの値を累積する
                let mut v_acc = 0u32;
                let mut count = 0u32;

                for box_y in y_start..y_end.min(uv_height) {
                    for box_x in x_start..x_end.min(uv_width) {
                        let i = y_size + (uv_width * uv_height) + box_y * uv_width + box_x;
                        v_acc += self.data[i] as u32;
                        count += 1;
                    }
                }

                // 新しいピクセル値を平均値で求める
                let i = new_y_size + new_uv_size + y * new_uv_width + x;
                new_data[i] = (v_acc / count) as u8;
            }
        }

        self.width = new_width;
        self.height = new_height;
        self.data = new_data;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFormat {
    I420,
    H264,
    H264AnnexB,
    H265,
    Vp8,
    Vp9,
    Av1,
}

impl VideoFormat {
    pub fn codec_name(self) -> Option<CodecName> {
        match self {
            VideoFormat::I420 => None,
            VideoFormat::H264 => Some(CodecName::H264),
            VideoFormat::H264AnnexB => Some(CodecName::H264),
            VideoFormat::H265 => Some(CodecName::H265),
            VideoFormat::Vp8 => Some(CodecName::Vp8),
            VideoFormat::Vp9 => Some(CodecName::Vp9),
            VideoFormat::Av1 => Some(CodecName::Av1),
        }
    }
}

impl std::fmt::Display for VideoFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoFormat::I420 => write!(f, "I420"),
            _ => {
                let name = self.codec_name().expect("infallible");
                write!(f, "{}", name.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: NonZeroUsize,
    pub denumerator: NonZeroUsize,
}

impl FrameRate {
    pub const FPS_1: Self = Self {
        numerator: NonZeroUsize::MIN,
        denumerator: NonZeroUsize::MIN,
    };

    pub const FPS_25: Self = Self {
        numerator: NonZeroUsize::MIN.saturating_add(24),
        denumerator: NonZeroUsize::MIN,
    };
}

impl FromStr for FrameRate {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((integer, fraction)) = s.split_once('/') {
            // 分数表記
            let integer = NonZeroUsize::from_str(integer).map_err(|_| {
                format!("the integer part of {s:?} is not a valid positive integer")
            })?;
            let fraction = NonZeroUsize::from_str(fraction).map_err(|_| {
                format!("the fraction part of {s:?} is not a valid positive integer")
            })?;
            Ok(Self {
                numerator: integer,
                denumerator: fraction,
            })
        } else {
            // 整数表記
            let integer = NonZeroUsize::from_str(s)
                .map_err(|_| format!("{s:?} is not a valid positive integer"))?;
            Ok(Self {
                numerator: integer,
                denumerator: NonZeroUsize::MIN,
            })
        }
    }
}

impl nojson::DisplayJson for FrameRate {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        if self.denumerator.get() == 1 {
            f.value(self.numerator.get())
        } else {
            f.string(format!("{}/{}", self.numerator, self.denumerator))
        }
    }
}

pub fn sample_entry_visual_fields(width: EvenUsize, height: EvenUsize) -> VisualSampleEntryFields {
    VisualSampleEntryFields {
        data_reference_index: VisualSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
        width: width.get() as u16,
        height: height.get() as u16,
        horizresolution: VisualSampleEntryFields::DEFAULT_HORIZRESOLUTION,
        vertresolution: VisualSampleEntryFields::DEFAULT_VERTRESOLUTION,
        frame_count: VisualSampleEntryFields::DEFAULT_FRAME_COUNT,
        compressorname: VisualSampleEntryFields::NULL_COMPRESSORNAME,
        depth: VisualSampleEntryFields::DEFAULT_DEPTH,
    }
}
