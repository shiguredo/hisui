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
    /// I420 形式の各プレーンサイズを計算
    fn i420_plane_sizes(width: usize, height: usize) -> (usize, usize, usize) {
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height;
        (y_size, uv_size, uv_size)
    }

    /// I420 形式の総データサイズを計算
    fn i420_total_size(width: usize, height: usize) -> usize {
        let (y_size, u_size, v_size) = Self::i420_plane_sizes(width, height);
        y_size + u_size + v_size
    }

    /// UV プレーンの幅・高さを計算
    fn i420_uv_dimensions(width: usize, height: usize) -> (usize, usize) {
        (width.div_ceil(2), height.div_ceil(2))
    }

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

        let (y_size, u_size, _) = Self::i420_plane_sizes(width_val, height_val);
        let mut yuv_data = Vec::with_capacity(Self::i420_total_size(width_val, height_val));

        let mut y_plane = vec![0u8; y_size];
        let mut u_plane = vec![0u8; u_size];
        let mut v_plane = vec![0u8; u_size];

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
        let (y_size, _, _) = Self::i420_plane_sizes(width, height);
        let (uv_width, uv_height) = Self::i420_uv_dimensions(width, height);
        let uv_size = uv_width * uv_height;
        let mut data = Vec::with_capacity(Self::i420_total_size(width, height));

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

    /// 10 ビット高ビット深度 YUV データから I420 VideoFrame を作成
    /// libvpx は 10-bit 前提のため、10-bit -> 8-bit 変換に特化
    #[expect(clippy::too_many_arguments)]
    pub fn new_i420_from_high_depth(
        input_frame: Self,
        width: usize,
        height: usize,
        y_plane_16: &[u8],
        u_plane_16: &[u8],
        v_plane_16: &[u8],
        y_stride: usize,
        u_stride: usize,
        v_stride: usize,
    ) -> orfail::Result<Self> {
        let (y_size, _, _) = Self::i420_plane_sizes(width, height);
        let (uv_width, uv_height) = Self::i420_uv_dimensions(width, height);
        let uv_size = uv_width * uv_height;
        let mut data = Vec::with_capacity(Self::i420_total_size(width, height));

        // 10-bit (0-1023) から 8-bit (0-255) への変換
        // 正確なスケーリング: (value * 255 + 511) / 1023
        let convert_10bit_to_8bit =
            |value_16: u16| -> u8 { ((value_16 as u32 * 255 + 511) / 1023) as u8 };

        // Y プレーンを 10-bit から 8-bit に変換
        if width * 2 == y_stride {
            // パディングなし、チャンク単位で処理可能
            if y_plane_16.len() < y_size * 2 {
                return Err(orfail::Failure::new(format!(
                    "Y plane data insufficient: expected {} bytes, got {}",
                    y_size * 2,
                    y_plane_16.len()
                )));
            }
            for chunk in y_plane_16[..y_size * 2].chunks_exact(2) {
                let value_16 = u16::from_le_bytes([chunk[0], chunk[1]]);
                let value_8 = convert_10bit_to_8bit(value_16);
                data.push(value_8);
            }
        } else {
            // ストライドにパディングがある場合の処理
            for row in 0..height {
                let row_start = row * y_stride;
                if row_start + width * 2 > y_plane_16.len() {
                    return Err(orfail::Failure::new(format!(
                        "Y plane data insufficient: row {} requires {} bytes but only {} available",
                        row,
                        row_start + width * 2,
                        y_plane_16.len()
                    )));
                }
                let row_data = &y_plane_16[row_start..row_start + width * 2];
                for chunk in row_data.chunks_exact(2) {
                    let value_16 = u16::from_le_bytes([chunk[0], chunk[1]]);
                    let value_8 = convert_10bit_to_8bit(value_16);
                    data.push(value_8);
                }
            }
        }

        // U プレーンを 10-bit から 8-bit に変換
        if uv_width * 2 == u_stride {
            if u_plane_16.len() < uv_size * 2 {
                return Err(orfail::Failure::new(format!(
                    "U plane data insufficient: expected {} bytes, got {}",
                    uv_size * 2,
                    u_plane_16.len()
                )));
            }
            for chunk in u_plane_16[..uv_size * 2].chunks_exact(2) {
                let value_16 = u16::from_le_bytes([chunk[0], chunk[1]]);
                let value_8 = convert_10bit_to_8bit(value_16);
                data.push(value_8);
            }
        } else {
            for row in 0..uv_height {
                let row_start = row * u_stride;
                if row_start + uv_width * 2 > u_plane_16.len() {
                    return Err(orfail::Failure::new(format!(
                        "U plane data insufficient: row {} requires {} bytes but only {} available",
                        row,
                        row_start + uv_width * 2,
                        u_plane_16.len()
                    )));
                }
                let row_data = &u_plane_16[row_start..row_start + uv_width * 2];
                for chunk in row_data.chunks_exact(2) {
                    let value_16 = u16::from_le_bytes([chunk[0], chunk[1]]);
                    let value_8 = convert_10bit_to_8bit(value_16);
                    data.push(value_8);
                }
            }
        }

        // V プレーンを 10-bit から 8-bit に変換
        if uv_width * 2 == v_stride {
            if v_plane_16.len() < uv_size * 2 {
                return Err(orfail::Failure::new(format!(
                    "V plane data insufficient: expected {} bytes, got {}",
                    uv_size * 2,
                    v_plane_16.len()
                )));
            }
            for chunk in v_plane_16[..uv_size * 2].chunks_exact(2) {
                let value_16 = u16::from_le_bytes([chunk[0], chunk[1]]);
                let value_8 = convert_10bit_to_8bit(value_16);
                data.push(value_8);
            }
        } else {
            for row in 0..uv_height {
                let row_start = row * v_stride;
                if row_start + uv_width * 2 > v_plane_16.len() {
                    return Err(orfail::Failure::new(format!(
                        "V plane data insufficient: row {} requires {} bytes but only {} available",
                        row,
                        row_start + uv_width * 2,
                        v_plane_16.len()
                    )));
                }
                let row_data = &v_plane_16[row_start..row_start + uv_width * 2];
                for chunk in row_data.chunks_exact(2) {
                    let value_16 = u16::from_le_bytes([chunk[0], chunk[1]]);
                    let value_8 = convert_10bit_to_8bit(value_16);
                    data.push(value_8);
                }
            }
        }

        Ok(Self {
            source_id: input_frame.source_id,
            sample_entry: None, // 生データにはサンプルエントリは存在しない
            data,
            format: VideoFormat::I420,
            keyframe: true, // 生データは全てキーフレーム扱い
            width,
            height,
            timestamp: input_frame.timestamp,
            duration: input_frame.duration,
        })
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

        let actual_width = width.get();
        let actual_height = height.get();
        let (y_plane_size, u_plane_size, _) = Self::i420_plane_sizes(actual_width, actual_height);
        let total_size = Self::i420_total_size(actual_width, actual_height);

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
            width: actual_width,
            height: actual_height,
            timestamp: Duration::ZERO,
            duration: Duration::ZERO,
            sample_entry: None,
        }
    }

    pub fn black(width: EvenUsize, height: EvenUsize) -> Self {
        let actual_width = width.get();
        let actual_height = height.get();
        let (y_plane_size, _, _) = Self::i420_plane_sizes(actual_width, actual_height);
        let total_size = Self::i420_total_size(actual_width, actual_height);

        let mut data = vec![0; total_size];
        for b in data.iter_mut().take(total_size).skip(y_plane_size) {
            *b = 128;
        }

        Self {
            source_id: None,
            data,
            format: VideoFormat::I420,
            keyframe: true,
            width: actual_width,
            height: actual_height,
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

        let (y_size, uv_size, _) = Self::i420_plane_sizes(self.width, self.height);

        let y_plane = &self.data[..y_size];
        let u_plane = &self.data[y_size..][..uv_size];
        let v_plane = &self.data[y_size + uv_size..][..uv_size];

        Some((y_plane, u_plane, v_plane))
    }

    pub fn end_timestamp(&self) -> Duration {
        self.timestamp + self.duration
    }

    /// libyuv を使った YUV(I420) 画像リサイズ
    pub fn resize(
        &self,
        new_width: EvenUsize,
        new_height: EvenUsize,
        filter_mode: shiguredo_libyuv::FilterMode,
    ) -> orfail::Result<Option<Self>> {
        (self.format == VideoFormat::I420).or_fail()?;

        let width = self.width;
        let height = self.height;
        if width == new_width.get() && height == new_height.get() {
            // リサイズ不要
            return Ok(None);
        }

        // 新しい YUV バッファを作成
        let (new_y_size, new_uv_size, _) =
            Self::i420_plane_sizes(new_width.get(), new_height.get());
        let mut new_data = vec![0; Self::i420_total_size(new_width.get(), new_height.get())];

        // 元のYUVプレーンを取得
        let (src_y, src_u, src_v) = self.as_yuv_planes().or_fail()?;

        // ストライド計算（元画像） - 実際の幅を使用
        let src_width = self.width;
        let (src_uv_width, _) = Self::i420_uv_dimensions(self.width, self.height);

        // ストライド計算（出力画像）
        let dst_width = new_width.get();
        let dst_uv_width = dst_width / 2;

        // 出力バッファを分割
        let (dst_y, rest) = new_data.split_at_mut(new_y_size);
        let (dst_u, dst_v) = rest.split_at_mut(new_uv_size);

        // libyuv でリサイズ実行
        let src = shiguredo_libyuv::I420Planes {
            y: src_y,
            y_stride: src_width, // 実際の幅をストライドとして使用
            u: src_u,
            u_stride: src_uv_width, // U プレーンのストライド
            v: src_v,
            v_stride: src_uv_width, // V プレーンのストライド
        };

        let mut dst = shiguredo_libyuv::I420PlanesMut {
            y: dst_y,
            y_stride: dst_width, // 出力 Y プレーンのストライド
            u: dst_u,
            u_stride: dst_uv_width, // 出力 U プレーンのストライド
            v: dst_v,
            v_stride: dst_uv_width, // 出力 V プレーンのストライド
        };

        shiguredo_libyuv::i420_scale(
            &src,
            shiguredo_libyuv::ImageSize::new(width, height), // 元画像の実際のサイズ
            &mut dst,
            shiguredo_libyuv::ImageSize::new(dst_width, new_height.get()), // 出力画像のサイズ
            filter_mode,
        )
        .or_fail()?;

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

        // YUV プレーンを取得
        let (y_plane, u_plane, v_plane) = self.as_yuv_planes().or_fail()?;

        // ストライドは実際の幅を使用
        let y_stride = actual_width;
        let (uv_stride, _) = Self::i420_uv_dimensions(actual_width, actual_height);

        // 出力 BGR データは実際の解像度のみを含む
        let mut bgr_data = Vec::with_capacity(actual_width * actual_height * 3);

        for y in 0..actual_height {
            for x in 0..actual_width {
                // Y プレーンのインデックス（実際の幅をストライドとして使用）
                let y_idx = y * y_stride + x;

                // UV プレーンのインデックス（実際のUV幅をストライドとして使用）
                let uv_y = y / 2;
                let uv_x = x / 2;
                let uv_idx = uv_y * uv_stride + uv_x;

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
