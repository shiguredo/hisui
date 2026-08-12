use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};

use super::OutputSink;
use crate::{
    video::h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS, H264AnnexBNalUnits, NALU_HEADER_LENGTH},
    video::h265::{H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct VideoToolboxDecoder {
    inner: shiguredo_video_toolbox::Decoder,
    sink: OutputSink,

    // デコーダーの再初期化が必要かどうかの判定に使うフィールド
    //
    // H264/H265: VPS/SPS/PPS の変化で判定（resolution は未使用）
    // VP9/AV1: 解像度の変化で判定（vps/sps/pps は未使用）
    vps: Vec<u8>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    resolution: Option<(u32, u32)>,
}

impl VideoToolboxDecoder {
    pub fn new_h264(frame: &VideoFrame, sink: OutputSink) -> crate::Result<Self> {
        let (sps, pps) = get_h264_sps_pps(frame)?;
        tracing::debug!("Initialize H.264 decoder: sps={sps:?}, pps={pps:?}");

        let inner =
            shiguredo_video_toolbox::Decoder::new(shiguredo_video_toolbox::DecoderConfig {
                codec: shiguredo_video_toolbox::DecoderCodec::H264 {
                    sps: &sps,
                    pps: &pps,
                    nalu_len_bytes: NALU_HEADER_LENGTH as u32,
                },
                pixel_format: shiguredo_video_toolbox::PixelFormat::I420,
            })?;
        Ok(Self {
            inner,
            sink,
            vps: Vec::new(),
            sps,
            pps,
            resolution: None,
        })
    }

    pub fn new_h265(frame: &VideoFrame, sink: OutputSink) -> crate::Result<Self> {
        let (vps, sps, pps) = get_h265_vps_sps_pps(frame)?;
        tracing::debug!("Initialize H.265 decoder: vps={vps:?}, sps={sps:?}, pps={pps:?}");

        let inner =
            shiguredo_video_toolbox::Decoder::new(shiguredo_video_toolbox::DecoderConfig {
                codec: shiguredo_video_toolbox::DecoderCodec::Hevc {
                    vps,
                    sps,
                    pps,
                    nalu_len_bytes: NALU_HEADER_LENGTH as u32,
                },
                pixel_format: shiguredo_video_toolbox::PixelFormat::I420,
            })?;
        Ok(Self {
            inner,
            sink,
            vps: vps.to_vec(),
            sps: sps.to_vec(),
            pps: pps.to_vec(),
            resolution: None,
        })
    }

    pub fn new_vp9(frame: &VideoFrame, sink: OutputSink) -> crate::Result<Self> {
        let (width, height) = get_frame_resolution(frame, "VP9")?;
        tracing::debug!("Initialize VP9 decoder: width={width}, height={height}");
        Self::new_raw_codec(
            shiguredo_video_toolbox::DecoderCodec::Vp9 { width, height },
            width,
            height,
            sink,
        )
    }

    pub fn new_av1(frame: &VideoFrame, sink: OutputSink) -> crate::Result<Self> {
        let (width, height) = get_frame_resolution(frame, "AV1")?;
        tracing::debug!("Initialize AV1 decoder: width={width}, height={height}");
        Self::new_raw_codec(
            shiguredo_video_toolbox::DecoderCodec::Av1 { width, height },
            width,
            height,
            sink,
        )
    }

    /// VP9/AV1 共通のデコーダー生成
    fn new_raw_codec(
        codec: shiguredo_video_toolbox::DecoderCodec<'_>,
        width: u32,
        height: u32,
        sink: OutputSink,
    ) -> crate::Result<Self> {
        let inner =
            shiguredo_video_toolbox::Decoder::new(shiguredo_video_toolbox::DecoderConfig {
                codec,
                pixel_format: shiguredo_video_toolbox::PixelFormat::I420,
            })?;
        Ok(Self {
            inner,
            sink,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
            resolution: Some((width, height)),
        })
    }

    // デコーダーの再初期化が必要かどうかを判定し、必要であれば再初期化する
    //
    // H264/H265: VPS/SPS/PPS の変化で判定
    // VP9/AV1: 解像度の変化で判定
    //
    // [NOTE] WebM 対応削除によりサンプルエントリーの変更を見て判定できるようになったが、
    // 現状は上記の組み合わせ判定のままにしている
    fn reinitialize_if_need(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !frame.keyframe {
            // 切り替わりが発生するのは必ずキーフレーム
            return Ok(());
        }

        match frame.format {
            VideoFormat::H265 => {
                // [NOTE] VPS / SPS / PPS が存在しない場合には、デコード情報が変わっていないと判断して何もしない
                if let Ok((vps, sps, pps)) = get_h265_vps_sps_pps(frame) {
                    if vps == self.vps && sps == self.sps && pps == self.pps {
                        return Ok(());
                    }

                    // シンクを引き継ぐ (元の `VideoDecoder` の受信側 `rx` に届き続ける必要があるため)。
                    // 未消費フレームは送信側 (`Sender`) 経由で即時に emit 済のため、
                    // 再初期化で喪失するリスクはない。
                    let sink = self.sink.clone();
                    *self = Self::new_h265(frame, sink)?;
                }
            }
            VideoFormat::H264 | VideoFormat::H264AnnexB => {
                // [NOTE] SPS / PPS が存在しない場合には、デコード情報が変わっていないと判断して何もしない
                if let Ok((sps, pps)) = get_h264_sps_pps(frame) {
                    if sps == self.sps && pps == self.pps {
                        return Ok(());
                    }

                    let sink = self.sink.clone();
                    *self = Self::new_h264(frame, sink)?;
                }
            }
            VideoFormat::Vp9 => {
                self.reinitialize_raw_codec_if_need(frame, "VP9", Self::new_vp9)?;
            }
            VideoFormat::Av1 => {
                self.reinitialize_raw_codec_if_need(frame, "AV1", Self::new_av1)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// VP9/AV1 の解像度変更時にデコーダーを再作成する
    fn reinitialize_raw_codec_if_need(
        &mut self,
        frame: &VideoFrame,
        codec_name: &str,
        constructor: fn(&VideoFrame, OutputSink) -> crate::Result<Self>,
    ) -> crate::Result<()> {
        let (new_width, new_height) = get_frame_resolution(frame, codec_name)?;
        if Some((new_width, new_height)) == self.resolution {
            return Ok(());
        }

        // シンクを引き継ぐ (上記 H264/H265 経路と同じ理由)
        let sink = self.sink.clone();
        *self = constructor(frame, sink)?;
        Ok(())
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(
            frame.format,
            VideoFormat::H264
                | VideoFormat::H264AnnexB
                | VideoFormat::H265
                | VideoFormat::Vp9
                | VideoFormat::Av1
        ) {
            return Err(crate::Error::new(format!(
                "unsupported input format for VideoToolbox decoder: {:?}",
                frame.format
            )));
        }

        self.reinitialize_if_need(frame)?;

        let decoded = if matches!(frame.format, VideoFormat::H264AnnexB) {
            // AVC 形式に変換する
            let mut data = Vec::new();
            for nalu in H264AnnexBNalUnits::new(&frame.data) {
                let nalu = nalu?;
                data.extend_from_slice(&(nalu.data.len() as u32).to_be_bytes());
                data.extend_from_slice(nalu.data);
            }
            self.inner.decode(&data)?
        } else {
            // VP9/AV1 はデータをそのまま渡す（NALU 変換不要）
            self.inner.decode(&frame.data)?
        };
        let Some(decoded) = decoded else {
            return Ok(());
        };

        let shiguredo_video_toolbox::DecodedFrame::I420(decoded) = decoded else {
            return Err(crate::Error::new(
                "VideoToolbox decoder returned unsupported pixel format",
            ));
        };

        self.sink.emit_ok(VideoFrame::new_i420(
            frame.to_stripped(),
            decoded.width(),
            decoded.height(),
            decoded.y_plane(),
            decoded.u_plane(),
            decoded.v_plane(),
            decoded.y_stride(),
            decoded.u_stride(),
            decoded.v_stride(),
        ));
        Ok(())
    }
}

fn get_h264_sps_pps(frame: &VideoFrame) -> crate::Result<(Vec<u8>, Vec<u8>)> {
    if !matches!(frame.format, VideoFormat::H264 | VideoFormat::H264AnnexB) {
        return Err(crate::Error::new(format!(
            "expected H264 or H264AnnexB format, got {:?}",
            frame.format
        )));
    }

    let mut sps = Vec::new();
    let mut pps = Vec::new();
    match frame.format {
        VideoFormat::H264AnnexB => {
            for nal in H264AnnexBNalUnits::new(&frame.data) {
                let nal = nal?;
                match nal.ty {
                    H264_NALU_TYPE_SPS => sps = nal.data.to_vec(),
                    H264_NALU_TYPE_PPS => pps = nal.data.to_vec(),
                    _ => {}
                }
            }
        }
        VideoFormat::H264 => {
            // フレームデータ (AVCC 形式) 内の SPS / PPS を優先し、無ければ sample_entry にフォールバックする。
            // 単一 stsd + ビットストリーム内パラメータセット変化の入力では、キーフレームの
            // フレームデータ内に in-band の SPS / PPS が含まれるため、sample_entry だけでなく
            // フレームデータも参照して再初期化を判定する。
            let in_frame = extract_h264_sps_pps_from_avcc(&frame.data)?;
            let Some(SampleEntry::Avc1(Avc1Box {
                avcc_box: AvccBox {
                    sps_list, pps_list, ..
                },
                ..
            })) = frame.sample_entry.as_ref().map(|e| e.get())
            else {
                return Err(crate::Error::new(
                    "missing sample entry for H.264 first frame",
                ));
            };
            sps = match in_frame.sps {
                Some(sps) => sps.to_vec(),
                None => sps_list
                    .first()
                    .ok_or_else(|| crate::Error::new("missing H.264 SPS in sample entry"))?
                    .to_vec(),
            };
            pps = match in_frame.pps {
                Some(pps) => pps.to_vec(),
                None => pps_list
                    .first()
                    .ok_or_else(|| crate::Error::new("missing H.264 PPS in sample entry"))?
                    .to_vec(),
            };
        }
        _ => unreachable!(),
    }
    if sps.is_empty() {
        return Err(crate::Error::new("missing H.264 SPS"));
    }
    if pps.is_empty() {
        return Err(crate::Error::new("missing H.264 PPS"));
    }

    Ok((sps, pps))
}

/// VP9/AV1 フレームから解像度を取得する
fn get_frame_resolution(frame: &VideoFrame, codec_name: &str) -> crate::Result<(u32, u32)> {
    let size = frame.size.ok_or_else(|| {
        crate::Error::new(format!(
            "{codec_name} frame size is required for VideoToolbox decoder"
        ))
    })?;
    let width = u32::try_from(size.width).expect("frame width exceeds u32::MAX");
    let height = u32::try_from(size.height).expect("frame height exceeds u32::MAX");
    Ok((width, height))
}

/// AVCC 形式のフレームデータから抽出した H.264 の SPS / PPS NALU
///
/// フレーム内に該当 NALU が無い場合は `None` となる。
#[derive(Debug)]
struct H264SpsPpsFromAvcc<'a> {
    sps: Option<&'a [u8]>,
    pps: Option<&'a [u8]>,
}

/// AVCC 形式のフレームデータから抽出した H.265 の VPS / SPS / PPS NALU
///
/// フレーム内に該当 NALU が無い場合はそれぞれ `None` となる。
#[derive(Debug)]
struct H265VpsSpsPpsFromAvcc<'a> {
    vps: Option<&'a [u8]>,
    sps: Option<&'a [u8]>,
    pps: Option<&'a [u8]>,
}

/// AVCC 形式の H.264 フレームデータから SPS / PPS NALU を抽出する
///
/// NALU 長プレフィックスは 4 バイト固定 (ISO/IEC 14496-15 §5.3.3.1 の
/// `lengthSizeMinusOne` が 3 の場合。既存デコーダーも 4 バイト固定で扱う) で、
/// フレーム内に SPS / PPS が無い場合は `None` を返す。
/// NAL unit type は ITU-T H.264 仕様 7.4.1 の nal_unit_type (下位 5 ビット) で判定する。
/// フレームデータが壊れている場合 (長さプレフィックスがデータ末尾を超える) は Err を返す。
fn extract_h264_sps_pps_from_avcc(data: &[u8]) -> crate::Result<H264SpsPpsFromAvcc<'_>> {
    let mut sps = None;
    let mut pps = None;
    let mut offset = 0;
    while offset < data.len() {
        if offset + NALU_HEADER_LENGTH > data.len() {
            return Err(crate::Error::new(format!(
                "invalid H264 AVCC payload: NALU length header is truncated (remaining={})",
                data.len() - offset
            )));
        }
        let nalu_len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += NALU_HEADER_LENGTH;

        if offset + nalu_len > data.len() {
            return Err(crate::Error::new(format!(
                "invalid H264 AVCC payload: NALU length {nalu_len} exceeds remaining data {} at offset {}",
                data.len() - offset,
                offset
            )));
        }

        let nalu = &data[offset..offset + nalu_len];
        match nalu.first().map(|b| b & 0x1F) {
            Some(H264_NALU_TYPE_SPS) => sps = Some(nalu),
            Some(H264_NALU_TYPE_PPS) => pps = Some(nalu),
            _ => {}
        }
        offset += nalu_len;
    }
    Ok(H264SpsPpsFromAvcc { sps, pps })
}

/// AVCC 形式の H.265 フレームデータから VPS / SPS / PPS NALU を抽出する
///
/// NALU 長プレフィックスは 4 バイト固定 (ISO/IEC 14496-15 §8.3.3.1 の
/// `lengthSizeMinusOne` が 3 の場合。既存デコーダーも 4 バイト固定で扱う) で、
/// フレーム内に VPS / SPS / PPS が無い場合はそれぞれ `None` を返す。
/// NAL unit type は ITU-T H.265 仕様 7.3.1.2 の nal_unit_type (第 1 バイトの bit 1-6) で判定する。
/// フレームデータが壊れている場合 (長さプレフィックスがデータ末尾を超える) は Err を返す。
fn extract_h265_vps_sps_pps_from_avcc(data: &[u8]) -> crate::Result<H265VpsSpsPpsFromAvcc<'_>> {
    let mut vps = None;
    let mut sps = None;
    let mut pps = None;
    let mut offset = 0;
    while offset < data.len() {
        if offset + NALU_HEADER_LENGTH > data.len() {
            return Err(crate::Error::new(format!(
                "invalid H265 AVCC payload: NALU length header is truncated (remaining={})",
                data.len() - offset
            )));
        }
        let nalu_len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += NALU_HEADER_LENGTH;

        if offset + nalu_len > data.len() {
            return Err(crate::Error::new(format!(
                "invalid H265 AVCC payload: NALU length {nalu_len} exceeds remaining data {} at offset {}",
                data.len() - offset,
                offset
            )));
        }

        let nalu = &data[offset..offset + nalu_len];
        // H.265 の NAL unit type は NAL ヘッダ第 1 バイトの bit 1-6 (H265AnnexBNalUnits と同じ抽出方法)
        match nalu.first().map(|b| (b >> 1) & 0x3F) {
            Some(H265_NALU_TYPE_VPS) => vps = Some(nalu),
            Some(H265_NALU_TYPE_SPS) => sps = Some(nalu),
            Some(H265_NALU_TYPE_PPS) => pps = Some(nalu),
            _ => {}
        }
        offset += nalu_len;
    }
    Ok(H265VpsSpsPpsFromAvcc { vps, sps, pps })
}

fn get_h265_vps_sps_pps(frame: &VideoFrame) -> crate::Result<(&[u8], &[u8], &[u8])> {
    if !matches!(frame.format, VideoFormat::H265) {
        return Err(crate::Error::new(format!(
            "expected H265 format, got {:?}",
            frame.format
        )));
    }

    // フレームデータ (AVCC 形式) 内の VPS / SPS / PPS を優先し、無ければ sample_entry に
    // フォールバックする。hev1 は in-band パラメータセット変化が仕様準拠の正規パターンで、
    // キーフレームのフレームデータ内に VPS / SPS / PPS が含まれるため、sample_entry だけでなく
    // フレームデータも参照して再初期化を判定する。
    let in_frame = extract_h265_vps_sps_pps_from_avcc(&frame.data)?;

    let hvcc = match frame.sample_entry.as_ref().map(|e| e.get()) {
        Some(SampleEntry::Hev1(b)) => &b.hvcc_box,
        Some(SampleEntry::Hvc1(b)) => &b.hvcc_box,
        _ => return Err(crate::Error::new("no H.265 sample entry")),
    };

    let mut vps = &[][..];
    let mut sps = &[][..];
    let mut pps = &[][..];
    for arrays in &hvcc.nalu_arrays {
        if arrays.nalus.is_empty() {
            continue;
        }

        match arrays.nal_unit_type.get() {
            H265_NALU_TYPE_VPS => vps = arrays.nalus[0].as_slice(),
            H265_NALU_TYPE_SPS => sps = arrays.nalus[0].as_slice(),
            H265_NALU_TYPE_PPS => pps = arrays.nalus[0].as_slice(),
            _ => {}
        }
    }

    if let Some(v) = in_frame.vps {
        vps = v;
    }
    if let Some(s) = in_frame.sps {
        sps = s;
    }
    if let Some(p) = in_frame.pps {
        pps = p;
    }

    if vps.is_empty() {
        return Err(crate::Error::new("missing H.265 VPS"));
    }
    if sps.is_empty() {
        return Err(crate::Error::new("missing H.265 SPS"));
    }
    if pps.is_empty() {
        return Err(crate::Error::new("missing H.265 PPS"));
    }

    Ok((vps, sps, pps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::VideoFrameSize;

    // AVCC 形式のフレームデータを構築するヘルパー
    // 各 NALU を 4 バイト長プレフィックス付きで連結する
    fn avcc_data(nalus: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        for nalu in nalus {
            data.extend_from_slice(&(nalu.len() as u32).to_be_bytes());
            data.extend_from_slice(nalu);
        }
        data
    }

    // ffmpeg + libx265 で生成した実機 H.265 ストリームから抽出した VPS / SPS / PPS
    // 生成コマンド: `ffmpeg -f lavfi -i color=c=blue:s=640x480:d=1:r=25 -c:v libx265 -x265-params repeat-headers=1 out.mp4`
    const H265_VPS_640X480: &[u8] = &[
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x03, 0x00, 0x5a, 0x95, 0x94, 0x09,
    ];
    const H265_SPS_640X480: &[u8] = &[
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x5a, 0xa0, 0x05, 0x02, 0x01, 0xe1, 0x65, 0x95, 0x95, 0x29, 0x30, 0xbc, 0x05,
        0xa0, 0x20, 0x00, 0x00, 0x03, 0x00, 0x20, 0x00, 0x00, 0x03, 0x03, 0x21,
    ];
    const H265_PPS_640X480: &[u8] = &[0x44, 0x01, 0xc0, 0x73, 0xc1, 0x89];

    // ffmpeg + libx265 で生成した実機 H.265 ストリームから抽出した VPS / SPS / PPS (320x320)
    // 生成コマンド: `ffmpeg -f lavfi -i color=c=red:s=320x320:d=1:r=25 -c:v libx265 -qp 40 -x265-params repeat-headers=1:bframes=0 out.mp4`
    // PPS は QP 違い (init_qp_minus26) で 640x480 側と差別化する
    const H265_VPS_320X320: &[u8] = &[
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x03, 0x00, 0x3c, 0xba, 0x02, 0x40,
    ];
    const H265_SPS_320X320: &[u8] = &[
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x3c, 0xa0, 0x0a, 0x08, 0x05, 0x05, 0x96, 0xe9, 0x29, 0x30, 0xbc, 0x05, 0xa0,
        0x20, 0x00, 0x00, 0x03, 0x00, 0x20, 0x00, 0x00, 0x03, 0x03, 0x21,
    ];
    const H265_PPS_320X320: &[u8] = &[0x44, 0x01, 0xc0, 0x71, 0x83, 0x12];

    // ffmpeg + libx264 で生成した実機 H.264 ストリームから抽出した SPS / PPS
    // 生成コマンド: `ffmpeg -f lavfi -i color=c=blue:s=320x240:d=1:r=25 -c:v libx264 -profile:v baseline -qp 20 -f h264 out.h264`
    // PPS は QP 違い (pic_init_qp_minus26) で sample_entry 側 (0x68 0xce 0x06 0xe2) と差別化する
    const H264_SPS_320X240: &[u8] = &crate::video::h264::tests::SPS_320X240;
    const H264_PPS_320X240: &[u8] = &[0x68, 0xce, 0x06, 0xf2];

    fn make_video_frame(
        format: VideoFormat,
        data: Vec<u8>,
        sample_entry: Option<crate::sample_entry::SharedSampleEntry>,
    ) -> VideoFrame {
        VideoFrame {
            data,
            format,
            keyframe: true,
            size: Some(VideoFrameSize {
                width: 320,
                height: 240,
            }),
            timestamp: std::time::Duration::ZERO,
            sample_entry,
        }
    }

    #[test]
    fn extract_h264_sps_pps_from_avcc_extracts_sps_and_pps() -> crate::Result<()> {
        // SEI が先行するフレームデータから SPS / PPS を抽出できること
        let data = avcc_data(&[
            &[0x06, 0x01, 0x02, 0x03],
            H264_SPS_320X240,
            H264_PPS_320X240,
            &[0x65, 0x88, 0x84],
        ]);
        let in_frame = extract_h264_sps_pps_from_avcc(&data)?;
        assert_eq!(in_frame.sps, Some(H264_SPS_320X240), "SPS が抽出されること");
        assert_eq!(in_frame.pps, Some(H264_PPS_320X240), "PPS が抽出されること");
        Ok(())
    }

    #[test]
    fn extract_h264_sps_pps_from_avcc_returns_none_when_missing() -> crate::Result<()> {
        // SPS / PPS を含まないフレームデータでは None を返すこと
        let data = avcc_data(&[&[0x65, 0x88, 0x84]]);
        let in_frame = extract_h264_sps_pps_from_avcc(&data)?;
        assert_eq!(in_frame.sps, None, "SPS が無い場合は None");
        assert_eq!(in_frame.pps, None, "PPS が無い場合は None");
        Ok(())
    }

    #[test]
    fn extract_h264_sps_pps_from_avcc_returns_err_on_truncated_length_header() {
        // 長さプレフィックスがデータ末尾を超える場合は Err を返すこと
        let data = vec![0x00, 0x00];
        assert!(extract_h264_sps_pps_from_avcc(&data).is_err());
    }

    #[test]
    fn extract_h264_sps_pps_from_avcc_returns_err_on_truncated_nalu() {
        // NALU 長がデータ末尾を超える場合は Err を返すこと
        let data = avcc_data(&[H264_SPS_320X240, &[0x65, 0x88]]);
        let truncated = [&data[..], &[0xffu8][..]].concat();
        let mut data = Vec::new();
        // 本来の SPS 長より大きい NALU 長を持つ不正データを構築する
        data.extend_from_slice(&(H264_SPS_320X240.len() as u32 + 10).to_be_bytes());
        data.extend_from_slice(H264_SPS_320X240);
        assert!(extract_h264_sps_pps_from_avcc(&data).is_err());
        // 上記の truncated データ自体は末尾 1 バイトが不完全な長さヘッダになる
        assert!(extract_h264_sps_pps_from_avcc(&truncated).is_err());
    }

    #[test]
    fn extract_h265_vps_sps_pps_from_avcc_extracts_vps_sps_pps() -> crate::Result<()> {
        // SEI が先行するフレームデータから VPS / SPS / PPS を抽出できること
        let data = avcc_data(&[
            &[0x4e, 0x01, 0x02, 0x03],
            H265_VPS_640X480,
            H265_SPS_640X480,
            H265_PPS_640X480,
            &[0x26, 0x01, 0xaf, 0x04],
        ]);
        let in_frame = extract_h265_vps_sps_pps_from_avcc(&data)?;
        assert_eq!(in_frame.vps, Some(H265_VPS_640X480), "VPS が抽出されること");
        assert_eq!(in_frame.sps, Some(H265_SPS_640X480), "SPS が抽出されること");
        assert_eq!(in_frame.pps, Some(H265_PPS_640X480), "PPS が抽出されること");
        Ok(())
    }

    #[test]
    fn extract_h265_vps_sps_pps_from_avcc_returns_none_when_missing() -> crate::Result<()> {
        // VPS / SPS / PPS を含まないフレームデータでは None を返すこと
        let data = avcc_data(&[&[0x26, 0x01, 0xaf, 0x04]]);
        let in_frame = extract_h265_vps_sps_pps_from_avcc(&data)?;
        assert_eq!(in_frame.vps, None, "VPS が無い場合は None");
        assert_eq!(in_frame.sps, None, "SPS が無い場合は None");
        assert_eq!(in_frame.pps, None, "PPS が無い場合は None");
        Ok(())
    }

    #[test]
    fn extract_h265_vps_sps_pps_from_avcc_returns_err_on_truncated_nalu() {
        // NALU 長がデータ末尾を超える場合は Err を返すこと
        let mut data = Vec::new();
        data.extend_from_slice(&(H265_VPS_640X480.len() as u32 + 10).to_be_bytes());
        data.extend_from_slice(H265_VPS_640X480);
        assert!(extract_h265_vps_sps_pps_from_avcc(&data).is_err());
    }

    #[test]
    fn get_h264_sps_pps_prefers_sps_pps_in_frame() -> crate::Result<()> {
        // フレームデータ内の SPS / PPS が sample_entry の avcC より優先されること
        let sample_entry = crate::video::h264::h264_sample_entry_from_annexb(
            &[
                &crate::video::h264::tests::SPS_1920X1080_ANNEXB[..],
                &[0, 0, 0, 1, 0x68, 0xce, 0x06, 0xe2][..],
            ]
            .concat(),
        )?;
        let frame = make_video_frame(
            VideoFormat::H264,
            avcc_data(&[H264_SPS_320X240, H264_PPS_320X240, &[0x65, 0x88]]),
            Some(crate::sample_entry::SharedSampleEntry::new(sample_entry)),
        );
        let (sps, pps) = get_h264_sps_pps(&frame)?;
        assert_eq!(sps, H264_SPS_320X240, "フレーム内の SPS が優先されること");
        assert_eq!(pps, H264_PPS_320X240, "フレーム内の PPS が優先されること");
        Ok(())
    }

    #[test]
    fn get_h264_sps_pps_falls_back_to_sample_entry() -> crate::Result<()> {
        // フレームデータ内に SPS / PPS が無い場合は sample_entry の avcC にフォールバックすること
        let sample_entry = crate::video::h264::h264_sample_entry_from_annexb(
            &[
                &crate::video::h264::tests::SPS_320X240_ANNEXB[..],
                &[0, 0, 0, 1, 0x68, 0xce, 0x06, 0xe2][..],
            ]
            .concat(),
        )?;
        let frame = make_video_frame(
            VideoFormat::H264,
            avcc_data(&[&[0x65, 0x88]]),
            Some(crate::sample_entry::SharedSampleEntry::new(sample_entry)),
        );
        let (sps, pps) = get_h264_sps_pps(&frame)?;
        assert_eq!(
            sps,
            crate::video::h264::tests::SPS_320X240,
            "sample_entry の SPS にフォールバックすること"
        );
        assert_eq!(
            pps,
            &[0x68, 0xce, 0x06, 0xe2][..],
            "sample_entry の PPS にフォールバックすること"
        );
        Ok(())
    }

    #[test]
    fn get_h265_vps_sps_pps_prefers_parameter_sets_in_frame() -> crate::Result<()> {
        // フレームデータ内の VPS / SPS / PPS が sample_entry の hvcc より優先されること
        // sample_entry には 640x480、フレームデータ内には 320x320 (QP 違いで PPS も差別化) を入れ、
        // 優先順位を値の違いで検出できるようにする
        let sample_entry = crate::video::h265::h265_sample_entry_from_annexb(
            &[
                &[0, 0, 0, 1][..],
                H265_VPS_640X480,
                &[0, 0, 0, 1][..],
                H265_SPS_640X480,
                &[0, 0, 0, 1][..],
                H265_PPS_640X480,
            ]
            .concat(),
            crate::video::FrameRate::FPS_30,
        )?;
        let frame = make_video_frame(
            VideoFormat::H265,
            avcc_data(&[
                H265_VPS_320X320,
                H265_SPS_320X320,
                H265_PPS_320X320,
                &[0x26, 0x01],
            ]),
            Some(crate::sample_entry::SharedSampleEntry::new(sample_entry)),
        );
        let (vps, sps, pps) = get_h265_vps_sps_pps(&frame)?;
        assert_eq!(vps, H265_VPS_320X320, "フレーム内の VPS が優先されること");
        assert_eq!(sps, H265_SPS_320X320, "フレーム内の SPS が優先されること");
        assert_eq!(pps, H265_PPS_320X320, "フレーム内の PPS が優先されること");
        Ok(())
    }

    #[test]
    fn get_h265_vps_sps_pps_falls_back_to_sample_entry() -> crate::Result<()> {
        // フレームデータ内に VPS / SPS / PPS が無い場合は sample_entry の hvcc にフォールバックすること
        let sample_entry = crate::video::h265::h265_sample_entry_from_annexb(
            &[
                &[0, 0, 0, 1][..],
                H265_VPS_640X480,
                &[0, 0, 0, 1][..],
                H265_SPS_640X480,
                &[0, 0, 0, 1][..],
                H265_PPS_640X480,
            ]
            .concat(),
            crate::video::FrameRate::FPS_30,
        )?;
        let frame = make_video_frame(
            VideoFormat::H265,
            avcc_data(&[&[0x26, 0x01]]),
            Some(crate::sample_entry::SharedSampleEntry::new(sample_entry)),
        );
        let (vps, sps, pps) = get_h265_vps_sps_pps(&frame)?;
        assert_eq!(
            vps, H265_VPS_640X480,
            "sample_entry の VPS にフォールバックすること"
        );
        assert_eq!(
            sps, H265_SPS_640X480,
            "sample_entry の SPS にフォールバックすること"
        );
        assert_eq!(
            pps, H265_PPS_640X480,
            "sample_entry の PPS にフォールバックすること"
        );
        Ok(())
    }
}
