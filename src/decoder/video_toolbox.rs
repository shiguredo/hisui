use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};

use crate::{
    video::h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS, H264AnnexBNalUnits, NALU_HEADER_LENGTH},
    video::h265::{H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct VideoToolboxDecoder {
    inner: shiguredo_video_toolbox::Decoder,
    decoded: Option<VideoFrame>,

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
    pub fn new_h264(frame: &VideoFrame) -> crate::Result<Self> {
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
            decoded: None,
            vps: Vec::new(),
            sps,
            pps,
            resolution: None,
        })
    }

    pub fn new_h265(frame: &VideoFrame) -> crate::Result<Self> {
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
            decoded: None,
            vps: vps.to_vec(),
            sps: sps.to_vec(),
            pps: pps.to_vec(),
            resolution: None,
        })
    }

    pub fn new_vp9(frame: &VideoFrame) -> crate::Result<Self> {
        let (width, height) = get_frame_resolution(frame, "VP9")?;
        tracing::debug!("Initialize VP9 decoder: width={width}, height={height}");
        Self::new_raw_codec(
            shiguredo_video_toolbox::DecoderCodec::Vp9 { width, height },
            width,
            height,
        )
    }

    pub fn new_av1(frame: &VideoFrame) -> crate::Result<Self> {
        let (width, height) = get_frame_resolution(frame, "AV1")?;
        tracing::debug!("Initialize AV1 decoder: width={width}, height={height}");
        Self::new_raw_codec(
            shiguredo_video_toolbox::DecoderCodec::Av1 { width, height },
            width,
            height,
        )
    }

    /// VP9/AV1 共通のデコーダー生成
    fn new_raw_codec(
        codec: shiguredo_video_toolbox::DecoderCodec<'_>,
        width: u32,
        height: u32,
    ) -> crate::Result<Self> {

        let inner =
            shiguredo_video_toolbox::Decoder::new(shiguredo_video_toolbox::DecoderConfig {
                codec,
                pixel_format: shiguredo_video_toolbox::PixelFormat::I420,
            })?;
        Ok(Self {
            inner,
            decoded: None,
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
    // [NOTE] WebM 対応がなくなったら VideoDecoder 側でサンプルエントリーの変更を見てハンドリングできる
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

                    if self.decoded.is_some() {
                        return Err(crate::Error::new(
                            "cannot reinitialize decoder while decoded frame is pending",
                        ));
                    }
                    *self = Self::new_h265(frame)?;
                }
            }
            VideoFormat::H264 | VideoFormat::H264AnnexB => {
                // [NOTE] SPS / PPS が存在しない場合には、デコード情報が変わっていないと判断して何もしない
                if let Ok((sps, pps)) = get_h264_sps_pps(frame) {
                    if sps == self.sps && pps == self.pps {
                        return Ok(());
                    }

                    if self.decoded.is_some() {
                        return Err(crate::Error::new(
                            "cannot reinitialize decoder while decoded frame is pending",
                        ));
                    }
                    *self = Self::new_h264(frame)?;
                }
            }
            VideoFormat::Vp9 | VideoFormat::Av1 => {
                let (new_width, new_height) = get_frame_resolution(
                    frame,
                    if frame.format == VideoFormat::Vp9 {
                        "VP9"
                    } else {
                        "AV1"
                    },
                )?;
                if Some((new_width, new_height)) == self.resolution {
                    return Ok(());
                }

                if self.decoded.is_some() {
                    return Err(crate::Error::new(
                        "cannot reinitialize decoder while decoded frame is pending",
                    ));
                }

                // 解像度が変わったのでデコーダーを再作成する
                if frame.format == VideoFormat::Vp9 {
                    *self = Self::new_vp9(frame)?;
                } else {
                    *self = Self::new_av1(frame)?;
                }
            }
            _ => {}
        }

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

        self.decoded = Some(VideoFrame::new_i420(
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

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.decoded.take()
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
            let Some(SampleEntry::Avc1(Avc1Box {
                avcc_box: AvccBox {
                    sps_list, pps_list, ..
                },
                ..
            })) = &frame.sample_entry
            else {
                return Err(crate::Error::new(
                    "missing sample entry for H.264 first frame",
                ));
            };
            sps = sps_list
                .first()
                .ok_or_else(|| crate::Error::new("missing H.264 SPS in sample entry"))?
                .to_vec();
            pps = pps_list
                .first()
                .ok_or_else(|| crate::Error::new("missing H.264 PPS in sample entry"))?
                .to_vec();
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
    Ok((size.width as u32, size.height as u32))
}

fn get_h265_vps_sps_pps(frame: &VideoFrame) -> crate::Result<(&[u8], &[u8], &[u8])> {
    if !matches!(frame.format, VideoFormat::H265) {
        return Err(crate::Error::new(format!(
            "expected H265 format, got {:?}",
            frame.format
        )));
    }

    let hvcc = match &frame.sample_entry {
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
