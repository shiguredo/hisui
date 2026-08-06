use orfail::OrFail;
use shiguredo_mp4::boxes::{Avc1Box, AvccBox, SampleEntry};

use crate::{
    video::{VideoFormat, VideoFrame},
    video_h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS, H264AnnexBNalUnits, NALU_HEADER_LENGTH},
    video_h265::{H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS},
};

#[derive(Debug)]
pub struct VideoToolboxDecoder {
    inner: shiguredo_video_toolbox::Decoder,
    decoded: Option<VideoFrame>,

    // デコーダーの再初期化が必要かどうかの判定に使うフィールド
    vps: Vec<u8>,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

impl VideoToolboxDecoder {
    pub fn new_h264(frame: &VideoFrame) -> orfail::Result<Self> {
        let (sps, pps) = get_h264_sps_pps(frame).or_fail()?;
        log::debug!("Initialize H.264 decoder: sps={sps:?}, pps={pps:?}");

        let inner =
            shiguredo_video_toolbox::Decoder::new_h264(&sps, &pps, NALU_HEADER_LENGTH).or_fail()?;
        Ok(Self {
            inner,
            decoded: None,
            vps: Vec::new(),
            sps,
            pps,
        })
    }

    pub fn new_h265(frame: &VideoFrame) -> orfail::Result<Self> {
        let (vps, sps, pps) = get_h265_vps_sps_pps(frame).or_fail()?;
        log::debug!("Initialize H.264 decoder: vps={vps:?}, sps={sps:?}, pps={pps:?}");

        let inner = shiguredo_video_toolbox::Decoder::new_h265(vps, sps, pps, NALU_HEADER_LENGTH)
            .or_fail()?;
        Ok(Self {
            inner,
            decoded: None,
            vps: vps.to_vec(),
            sps: sps.to_vec(),
            pps: pps.to_vec(),
        })
    }

    // VPS / SPS / PPS の情報が変わっていたらデコーダーを再初期化する
    //
    // [NOTE] WebM 対応がなくなったら VideoDecoder 側でサンプルエントリーの変更を見てハンドリングできる
    fn reinitialize_if_need(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        if !frame.keyframe {
            // 切り替わりが発生するのは必ずキーフレーム
            return Ok(());
        }

        if frame.format == VideoFormat::H265 {
            // [NOTE] VPS / SPS / PPS が存在しない場合には、デコード情報が変わっていないと判断して何もしない
            if let Ok((vps, sps, pps)) = get_h265_vps_sps_pps(frame) {
                if vps == self.vps && sps == self.sps && pps == self.pps {
                    // 情報は変わっていない
                    return Ok(());
                }

                // 変わっているので再初期化
                self.decoded.is_none().or_fail()?;
                *self = Self::new_h265(frame).or_fail()?;
            }
        } else {
            // [NOTE] VPS / SPS / PPS が存在しない場合には、デコード情報が変わっていないと判断して何もしない
            if let Ok((sps, pps)) = get_h264_sps_pps(frame) {
                if sps == self.sps && pps == self.pps {
                    // 情報は変わっていない
                    return Ok(());
                }

                // 変わっているので再初期化
                self.decoded.is_none().or_fail()?;
                *self = Self::new_h264(frame).or_fail()?;
            }
        }

        Ok(())
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        matches!(
            frame.format,
            VideoFormat::H264 | VideoFormat::H264AnnexB | VideoFormat::H265
        )
        .or_fail()?;

        self.reinitialize_if_need(frame).or_fail()?;

        let decoded = if matches!(frame.format, VideoFormat::H264AnnexB) {
            // AVC 形式に変換する
            let mut data = Vec::new();
            for nalu in H264AnnexBNalUnits::new(&frame.data) {
                let nalu = nalu.or_fail()?;
                data.extend_from_slice(&(nalu.data.len() as u32).to_be_bytes());
                data.extend_from_slice(nalu.data);
            }
            self.inner.decode(&data).or_fail()?
        } else {
            self.inner.decode(&frame.data).or_fail()?
        };
        let Some(decoded) = decoded else {
            return Ok(());
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

fn get_h264_sps_pps(frame: &VideoFrame) -> orfail::Result<(Vec<u8>, Vec<u8>)> {
    matches!(frame.format, VideoFormat::H264 | VideoFormat::H264AnnexB).or_fail()?;

    let mut sps = Vec::new();
    let mut pps = Vec::new();
    match frame.format {
        VideoFormat::H264AnnexB => {
            for nal in H264AnnexBNalUnits::new(&frame.data) {
                let nal = nal.or_fail()?;
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
                return Err(orfail::Failure::new(
                    "missing sample entry for H.264 first frame",
                ));
            };
            sps = sps_list.first().or_fail()?.to_vec();
            pps = pps_list.first().or_fail()?.to_vec();
        }
        _ => unreachable!(),
    }
    (!sps.is_empty()).or_fail()?;
    (!pps.is_empty()).or_fail()?;

    Ok((sps, pps))
}

fn get_h265_vps_sps_pps(frame: &VideoFrame) -> orfail::Result<(&[u8], &[u8], &[u8])> {
    matches!(frame.format, VideoFormat::H265).or_fail()?;

    let Some(SampleEntry::Hev1(b)) = &frame.sample_entry else {
        return Err(orfail::Failure::new("no H.265 sample entry"));
    };

    let mut vps = &[][..];
    let mut sps = &[][..];
    let mut pps = &[][..];
    for arrays in &b.hvcc_box.nalu_arrays {
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
    (!vps.is_empty()).or_fail()?;
    (!sps.is_empty()).or_fail()?;
    (!pps.is_empty()).or_fail()?;

    Ok((vps, sps, pps))
}
