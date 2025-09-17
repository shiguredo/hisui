use std::{num::NonZeroUsize, str::FromStr, time::Duration};

use orfail::OrFail;
use shiguredo_mp4::boxes::{SampleEntry, VisualSampleEntryFields};

use crate::{
    metadata::SourceId,
    types::{CodecName, EvenUsize},
};

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub source_id: Option<SourceId>,
    pub data: Vec<u8>,
    pub format: VideoFormat,
    pub keyframe: bool,
    pub width: usize,
    pub height: usize,
    pub timestamp: Duration,
    pub duration: Duration,
    pub sample_entry: Option<SampleEntry>,
}

impl VideoFrame {
    pub fn from_bgr_data(
        bgr_data: &[u8],
        width: EvenUsize,
        height: EvenUsize,
        timestamp: Duration,
        duration: Duration,
    ) -> orfail::Result<Self> {
        let width_val = width.get();
        let height_val = height.get();

        let expected_size = width_val * height_val * 3;
        if bgr_data.len() != expected_size {
            return Err(orfail::Failure::new(format!(
                "BGR data size mismatch: expected {}, got {}",
                expected_size,
                bgr_data.len()
            )));
        }

        let y_size = width_val * height_val;
        let uv_size = (width_val / 2) * (height_val / 2);
        let mut yuv_data = Vec::with_capacity(y_size + uv_size * 2);

        let mut y_plane = vec![0u8; y_size];
        let mut u_plane = vec![0u8; uv_size];
        let mut v_plane = vec![0u8; uv_size];

        for y in 0..height_val {
            for x in 0..width_val {
                let bgr_idx = (y * width_val + x) * 3;
                let b = bgr_data[bgr_idx] as f32;
                let g = bgr_data[bgr_idx + 1] as f32;
                let r = bgr_data[bgr_idx + 2] as f32;

                // ITU-R BT.601 standard RGB to YUV conversion
                let y_val = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                let u_val = ((-0.169 * r - 0.331 * g + 0.500 * b) + 128.0) as u8;
                let v_val = ((0.500 * r - 0.419 * g - 0.081 * b) + 128.0) as u8;

                y_plane[y * width_val + x] = y_val;

                // U and V are subsampled (4:2:0)
                if y % 2 == 0 && x % 2 == 0 {
                    let uv_idx = (y / 2) * (width_val / 2) + (x / 2);
                    u_plane[uv_idx] = u_val;
                    v_plane[uv_idx] = v_val;
                }
            }
        }

        yuv_data.extend_from_slice(&y_plane);
        yuv_data.extend_from_slice(&u_plane);
        yuv_data.extend_from_slice(&v_plane);

        Ok(Self {
            source_id: None,
            data: yuv_data,
            format: VideoFormat::I420,
            keyframe: true,
            width: width.get(),
            height: height.get(),
            timestamp,
            duration,
            sample_entry: None,
        })
    }

    pub fn to_stripped(&self) -> Self {
        Self {
            source_id: self.source_id.clone(),
            data: Vec::new(),
            format: self.format,
            keyframe: self.keyframe,
            width: self.width,
            height: self.height,
            timestamp: self.timestamp,
            duration: self.duration,
            sample_entry: None,
        }
    }

    #[expect(clippy::too_many_arguments)]
    pub fn new_i420(
        input_frame: Self,
        width: usize,
        height: usize,
        y_plane: &[u8],
        u_plane: &[u8],
        v_plane: &[u8],
        y_stride: usize,
        u_stride: usize,
        v_stride: usize,
    ) -> Self {
        let y_size = width * height;
        // 奇数の場合は切り上げ除算を使用してUVプレーンのサイズを計算
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height;
        let mut data = Vec::with_capacity(y_size + uv_size * 2);

        // ストライドを考慮して YUV 成分をコピーする
        if width == y_stride {
            // ストライドと横幅が同じならパディングバイトの考慮が不要
            data.extend_from_slice(&y_plane[..y_size]);
        } else {
            for i in 0..height {
                let offset = y_stride * i;
                data.extend_from_slice(&y_plane[offset..][..width]);
            }
        }

        if uv_width == u_stride {
            data.extend_from_slice(&u_plane[..uv_size]);
        } else {
            for i in 0..uv_height {
                let offset = u_stride * i;
                data.extend_from_slice(&u_plane[offset..][..uv_width]);
            }
        }

        if uv_width == v_stride {
            data.extend_from_slice(&v_plane[..uv_size]);
        } else {
            for i in 0..uv_height {
                let offset = v_stride * i;
                data.extend_from_slice(&v_plane[offset..][..uv_width]);
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
            width: width.get(),
            height: height.get(),
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
        for b in data.iter_mut().take(total_size).skip(y_plane_size) {
            *b = 128;
        }

        Self {
            source_id: None,
            data,
            format: VideoFormat::I420,
            keyframe: true,
            width: width.get(),
            height: height.get(),
            timestamp: Duration::ZERO,
            duration: Duration::ZERO,
            sample_entry: None,
        }
    }

    pub fn ceiling_width(&self) -> EvenUsize {
        EvenUsize::ceiling_new(self.width)
    }

    pub fn ceiling_height(&self) -> EvenUsize {
        EvenUsize::ceiling_new(self.height)
    }

    pub fn as_yuv_planes(&self) -> Option<(&[u8], &[u8], &[u8])> {
        if self.format != VideoFormat::I420 {
            return None;
        }

        let y_size = self.ceiling_width().get() * self.ceiling_height().get();

        let uv_width = self.width.div_ceil(2);
        let uv_height = self.height.div_ceil(2);
        let uv_size = uv_width * uv_height;

        let y_plane = &self.data[..y_size];
        let u_plane = &self.data[y_size..][..uv_size];
        let v_plane = &self.data[y_size + uv_size..][..uv_size];

        Some((y_plane, u_plane, v_plane))
    }

    pub fn end_timestamp(&self) -> Duration {
        self.timestamp + self.duration
    }

    /// ボックスフィルターアルゴリズムで YUV(I420) 画像をリサイズする
    pub fn resize(
        &self,
        new_width: EvenUsize,
        new_height: EvenUsize,
    ) -> orfail::Result<Option<Self>> {
        (self.format == VideoFormat::I420).or_fail()?;

        let width = self.width;
        let height = self.height;
        if width == new_width.get() && height == new_height.get() {
            // リサイズ不要
            return Ok(None);
        }

        // 新しい YUV バッファを作成
        let new_y_size = new_width.get() * new_height.get();
        let new_uv_size = (new_width.get() / 2) * (new_height.get() / 2);
        let mut new_data = vec![0; new_y_size + new_uv_size * 2];

        // 元のデータのサイズを計算（奇数の解像度を考慮）
        let y_size = width * height;
        let uv_width = width.div_ceil(2); // 奇数幅の場合は切り上げ
        let uv_height = height.div_ceil(2); // 奇数高さの場合は切り上げ

        // Y 平面のリサイズ
        let x_ratio = width as f64 / new_width.get() as f64;
        let y_ratio = height as f64 / new_height.get() as f64;
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

                for box_y in y_start..y_end.min(height) {
                    for box_x in x_start..x_end.min(width) {
                        let i = box_y * width + box_x;
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

        let resized = Self {
            source_id: self.source_id.clone(),
            data: new_data,
            format: self.format,
            keyframe: self.keyframe,
            width: new_width.get(),
            height: new_height.get(),
            timestamp: self.timestamp,
            duration: self.duration,
            sample_entry: self.sample_entry.clone(),
        };
        Ok(Some(resized))
    }

    pub fn to_bgr_data(&self) -> orfail::Result<Vec<u8>> {
        (self.format == VideoFormat::I420).or_fail()?;

        // 実際の解像度（出力に使用）
        let actual_width = self.width;
        let actual_height = self.height;

        // YUV プレーンを取得（奇数解像度の場合はパディングを含む）
        let (y_plane, u_plane, v_plane) = self.as_yuv_planes().or_fail()?;

        // パディングされた解像度を計算（内部データアクセス用）
        let padded_width = self.ceiling_width().get();
        let padded_uv_width = padded_width / 2;

        // 出力 BGR データは実際の解像度のみを含む
        let mut bgr_data = Vec::with_capacity(actual_width * actual_height * 3);

        for y in 0..actual_height {
            for x in 0..actual_width {
                // Y プレーンのインデックス（パディング幅をストライドとして使用）
                let y_idx = y * padded_width + x;

                // UV プレーンのインデックス（パディングUV幅をストライドとして使用）
                let uv_y = y / 2;
                let uv_x = x / 2;
                let uv_idx = uv_y * padded_uv_width + uv_x;

                let y_val = y_plane[y_idx] as f32;
                let u_val = u_plane[uv_idx] as f32 - 128.0;
                let v_val = v_plane[uv_idx] as f32 - 128.0;

                // ITU-R BT.601 標準 YUV から RGB への変換
                let r = y_val + 1.402 * v_val;
                let g = y_val - 0.344 * u_val - 0.714 * v_val;
                let b = y_val + 1.772 * u_val;

                // 値を 0-255 の範囲にクランプ
                let r = r.clamp(0.0, 255.0) as u8;
                let g = g.clamp(0.0, 255.0) as u8;
                let b = b.clamp(0.0, 255.0) as u8;

                bgr_data.push(b);
                bgr_data.push(g);
                bgr_data.push(r);
            }
        }

        Ok(bgr_data)
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

pub fn sample_entry_visual_fields(width: usize, height: usize) -> VisualSampleEntryFields {
    VisualSampleEntryFields {
        data_reference_index: VisualSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
        width: width as u16,
        height: height as u16,
        horizresolution: VisualSampleEntryFields::DEFAULT_HORIZRESOLUTION,
        vertresolution: VisualSampleEntryFields::DEFAULT_VERTRESOLUTION,
        frame_count: VisualSampleEntryFields::DEFAULT_FRAME_COUNT,
        compressorname: VisualSampleEntryFields::NULL_COMPRESSORNAME,
        depth: VisualSampleEntryFields::DEFAULT_DEPTH,
    }
}
