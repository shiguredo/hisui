use std::borrow::Cow;
use std::collections::VecDeque;

use orfail::OrFail;

use crate::layout_decode_params::LayoutDecodeParams;
use crate::video::{VideoFormat, VideoFrame};
use crate::video_h264::{H264_NALU_TYPE_PPS, H264_NALU_TYPE_SPS, get_h264_sps_pps};
use crate::video_h265::{
    H265_NALU_TYPE_PPS, H265_NALU_TYPE_SPS, H265_NALU_TYPE_VPS, NALU_HEADER_LENGTH,
    get_h265_vps_sps_pps,
};

#[derive(Debug)]
pub struct NvcodecDecoder {
    inner: Option<shiguredo_nvcodec::Decoder>,
    input_queue: VecDeque<VideoFrame>,
    output_queue: VecDeque<VideoFrame>,
    parameter_sets: Option<Vec<u8>>, // VPS/SPS/PPS をキャッシュ

    // ストリーム中の解像度変化に追随するため、以下のフィールドを保持する
    // (upstream shiguredo/nvcodec-rs 2026.2.0 でデコーダー内部での対応が入るため、
    //  crate 更新後はこの層の再初期化は不要になる)
    config: shiguredo_nvcodec::DecoderConfig,
    vps: Vec<u8>,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

impl NvcodecDecoder {
    pub fn new_h264(params: &LayoutDecodeParams) -> orfail::Result<Self> {
        log::debug!("create nvcodec(H264) decoder");
        let config = params.nvcodec_h264.clone();
        Ok(Self {
            inner: Some(shiguredo_nvcodec::Decoder::new_h264(config.clone()).or_fail()?),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
            config,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
        })
    }

    pub fn new_h265(params: &LayoutDecodeParams) -> orfail::Result<Self> {
        log::debug!("create nvcodec(H265) decoder");
        let config = params.nvcodec_h265.clone();
        Ok(Self {
            inner: Some(shiguredo_nvcodec::Decoder::new_h265(config.clone()).or_fail()?),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
            config,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
        })
    }

    pub fn new_av1(params: &LayoutDecodeParams) -> orfail::Result<Self> {
        log::debug!("create nvcodec(AV1) decoder");
        let config = params.nvcodec_av1.clone();
        Ok(Self {
            inner: Some(shiguredo_nvcodec::Decoder::new_av1(config.clone()).or_fail()?),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
            config,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
        })
    }

    pub fn new_vp8(params: &LayoutDecodeParams) -> orfail::Result<Self> {
        log::debug!("create nvcodec(VP8) decoder");
        let config = params.nvcodec_vp8.clone();
        Ok(Self {
            inner: Some(shiguredo_nvcodec::Decoder::new_vp8(config.clone()).or_fail()?),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
            config,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
        })
    }

    pub fn new_vp9(params: &LayoutDecodeParams) -> orfail::Result<Self> {
        log::debug!("create nvcodec(VP9) decoder");
        let config = params.nvcodec_vp9.clone();
        Ok(Self {
            inner: Some(shiguredo_nvcodec::Decoder::new_vp9(config.clone()).or_fail()?),
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            parameter_sets: None,
            config,
            vps: Vec::new(),
            sps: Vec::new(),
            pps: Vec::new(),
        })
    }

    // キーフレーム到来時に VPS / SPS / PPS が切り替わっていたら
    // 内部の shiguredo_nvcodec::Decoder を作り直すことで解像度変化に追随する
    //
    // 現行の shiguredo_nvcodec (=2025.2.1) は pfnSequenceCallback で最初のシーケンス以降を
    // 無視するため、この層で再初期化しないと解像度変化のあるストリームでデコードに失敗する
    fn reinitialize_if_need(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        if !frame.keyframe {
            // 切り替わりが発生するのは必ずキーフレーム
            return Ok(());
        }

        // [DEBUG] 一時的な切り分け用の環境変数
        //   HISUI_NVCODEC_DEBUG_MODE=A (default): 現在の実装 (finish + 再作成)
        //   HISUI_NVCODEC_DEBUG_MODE=B         : finish() を呼ばずに再作成のみ
        //   HISUI_NVCODEC_DEBUG_MODE=C         : 再初期化を行わない (修正前と同等)
        //   HISUI_NVCODEC_DEBUG_MODE=D         : finish + 明示 drop 後に再作成
        //                                        (self.inner = new(...) の semantics だと
        //                                         「create → 旧 drop」の順で GPU リソースが
        //                                         一時的に二重確保されるため、その回避)
        let debug_mode =
            std::env::var("HISUI_NVCODEC_DEBUG_MODE").unwrap_or_else(|_| String::from("A"));

        match frame.format {
            VideoFormat::H265 => {
                // [NOTE] VPS / SPS / PPS が取れない場合は変化なしとみなして何もしない
                if let Ok((vps, sps, pps)) = get_h265_vps_sps_pps(frame) {
                    if vps == self.vps && sps == self.sps && pps == self.pps {
                        return Ok(());
                    }
                    if debug_mode == "C" {
                        eprintln!("[nvcodec-debug] H265 reinit skipped (mode=C)");
                        self.vps = vps.to_vec();
                        self.sps = sps.to_vec();
                        self.pps = pps.to_vec();
                        return Ok(());
                    }
                    if debug_mode != "B" {
                        eprintln!("[nvcodec-debug] H265 finish() BEFORE");
                        let t = std::time::Instant::now();
                        self.inner.as_mut().or_fail()?.finish().or_fail()?;
                        eprintln!("[nvcodec-debug] H265 finish() {}ms", t.elapsed().as_millis());
                        eprintln!("[nvcodec-debug] H265 handle_decoded_frames() BEFORE");
                        let t = std::time::Instant::now();
                        self.handle_decoded_frames().or_fail()?;
                        eprintln!(
                            "[nvcodec-debug] H265 handle_decoded_frames() {}ms",
                            t.elapsed().as_millis()
                        );
                    }
                    self.input_queue.is_empty().or_fail()?;

                    let vps_new = vps.to_vec();
                    let sps_new = sps.to_vec();
                    let pps_new = pps.to_vec();
                    if debug_mode == "D" {
                        // 明示的に旧 Decoder を drop してから新規作成する
                        let t = std::time::Instant::now();
                        drop(self.inner.take());
                        eprintln!(
                            "[nvcodec-debug] H265 drop(prev_inner) {}ms",
                            t.elapsed().as_millis()
                        );
                    }
                    eprintln!("[nvcodec-debug] H265 new_h265() BEFORE (mode={debug_mode})");
                    let t = std::time::Instant::now();
                    self.inner = Some(
                        shiguredo_nvcodec::Decoder::new_h265(self.config.clone()).or_fail()?,
                    );
                    eprintln!(
                        "[nvcodec-debug] H265 new_h265() {}ms (mode={debug_mode})",
                        t.elapsed().as_millis()
                    );
                    self.vps = vps_new;
                    self.sps = sps_new;
                    self.pps = pps_new;
                    self.parameter_sets = None;
                }
            }
            VideoFormat::H264 | VideoFormat::H264AnnexB => {
                if let Ok((sps, pps)) = get_h264_sps_pps(frame) {
                    if sps == self.sps && pps == self.pps {
                        return Ok(());
                    }
                    if debug_mode == "C" {
                        eprintln!("[nvcodec-debug] H264 reinit skipped (mode=C)");
                        self.sps = sps;
                        self.pps = pps;
                        return Ok(());
                    }
                    if debug_mode != "B" {
                        eprintln!("[nvcodec-debug] H264 finish() BEFORE");
                        let t = std::time::Instant::now();
                        self.inner.as_mut().or_fail()?.finish().or_fail()?;
                        eprintln!("[nvcodec-debug] H264 finish() {}ms", t.elapsed().as_millis());
                        eprintln!("[nvcodec-debug] H264 handle_decoded_frames() BEFORE");
                        let t = std::time::Instant::now();
                        self.handle_decoded_frames().or_fail()?;
                        eprintln!(
                            "[nvcodec-debug] H264 handle_decoded_frames() {}ms",
                            t.elapsed().as_millis()
                        );
                    }
                    self.input_queue.is_empty().or_fail()?;

                    if debug_mode == "D" {
                        let t = std::time::Instant::now();
                        drop(self.inner.take());
                        eprintln!(
                            "[nvcodec-debug] H264 drop(prev_inner) {}ms",
                            t.elapsed().as_millis()
                        );
                    }
                    eprintln!("[nvcodec-debug] H264 new_h264() BEFORE (mode={debug_mode})");
                    let t = std::time::Instant::now();
                    self.inner = Some(
                        shiguredo_nvcodec::Decoder::new_h264(self.config.clone()).or_fail()?,
                    );
                    eprintln!(
                        "[nvcodec-debug] H264 new_h264() {}ms (mode={debug_mode})",
                        t.elapsed().as_millis()
                    );
                    self.sps = sps;
                    self.pps = pps;
                    self.parameter_sets = None;
                }
            }
            _ => {
                // VP8 / VP9 / AV1 は今回対応外
            }
        }
        Ok(())
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        matches!(
            frame.format,
            VideoFormat::H264
                | VideoFormat::H264AnnexB
                | VideoFormat::H265
                | VideoFormat::Vp8
                | VideoFormat::Vp9
                | VideoFormat::Av1
        )
        .or_fail()?;

        eprintln!(
            "[nvcodec-debug] decode() enter format={:?} keyframe={} data.len()={}",
            frame.format,
            frame.keyframe,
            frame.data.len()
        );
        self.reinitialize_if_need(frame).or_fail()?;

        // サンプルエントリからパラメータセットを抽出してキャッシュ
        if self.parameter_sets.is_none()
            && let Some(sample_entry) = &frame.sample_entry
        {
            self.parameter_sets =
                Some(extract_parameter_sets_annexb(sample_entry, frame.format).or_fail()?);
        }

        let data = if matches!(
            frame.format,
            VideoFormat::Vp8 | VideoFormat::Vp9 | VideoFormat::Av1
        ) {
            // VP8 / VP9 / AV1 の場合は Annex B 形式は存在しないので、データの変換は不要
            Cow::Borrowed(&frame.data)
        } else if frame.format == VideoFormat::H264AnnexB {
            // すでに Annex B 形式の場合はそのまま使用
            Cow::Borrowed(&frame.data)
        } else {
            // Annex.B 形式に変換する (H264/H265)
            let mut data = &frame.data[..];
            let mut data_annexb = Vec::new();

            // キーフレームで、かつパラメータセットがデータに含まれていない場合は先頭に追加
            if frame.keyframe
                && let Some(parameter_sets) = &self.parameter_sets
                && !contains_parameter_sets(data, frame.format)
            {
                data_annexb.extend_from_slice(parameter_sets);
            }

            while !data.is_empty() {
                (data.len() >= NALU_HEADER_LENGTH).or_fail()?;
                let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                data = &data[NALU_HEADER_LENGTH..];

                (data.len() >= n).or_fail()?;
                data_annexb.extend_from_slice(&[0, 0, 0, 1]);
                data_annexb.extend_from_slice(&data[..n]);

                data = &data[n..];
            }

            Cow::Owned(data_annexb)
        };

        self.inner.as_mut().or_fail()?.decode(&data).or_fail()?;
        self.input_queue.push_back(frame.to_stripped());
        self.handle_decoded_frames().or_fail()?;
        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        self.inner.as_mut().or_fail()?.finish().or_fail()?;
        self.handle_decoded_frames().or_fail()?;
        Ok(())
    }

    fn handle_decoded_frames(&mut self) -> orfail::Result<()> {
        while let Some(nv12_frame) = self.inner.as_mut().or_fail()?.next_frame().or_fail()? {
            let input_frame = self.input_queue.pop_front().or_fail()?;

            // NV12 から I420 への変換
            let width = nv12_frame.width();
            let height = nv12_frame.height();

            // I420 用のバッファを確保
            let y_size = width * height;
            let uv_width = width.div_ceil(2);
            let uv_height = height.div_ceil(2);
            let uv_size = uv_width * uv_height;
            let total_size = y_size + uv_size * 2;

            let mut i420_data = vec![0u8; total_size];
            let (y_plane, rest) = i420_data.split_at_mut(y_size);
            let (u_plane, v_plane) = rest.split_at_mut(uv_size);

            // libyuv を使って NV12 から I420 に変換
            let src = shiguredo_libyuv::Nv12Planes {
                y: nv12_frame.y_plane(),
                y_stride: nv12_frame.y_stride(),
                uv: nv12_frame.uv_plane(),
                uv_stride: nv12_frame.uv_stride(),
            };

            let mut dst = shiguredo_libyuv::I420PlanesMut {
                y: y_plane,
                y_stride: width,
                u: u_plane,
                u_stride: uv_width,
                v: v_plane,
                v_stride: uv_width,
            };

            let size = shiguredo_libyuv::ImageSize::new(width, height);
            shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size).or_fail()?;

            // I420 VideoFrame を作成
            self.output_queue.push_back(VideoFrame::new_i420(
                input_frame,
                width,
                height,
                y_plane,
                u_plane,
                v_plane,
                width,
                uv_width,
                uv_width,
            ));
        }
        Ok(())
    }

    pub fn next_decoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }
}

/// サンプルエントリからパラメータセットを Annex.B 形式で抽出
fn extract_parameter_sets_annexb(
    sample_entry: &shiguredo_mp4::boxes::SampleEntry,
    format: VideoFormat,
) -> orfail::Result<Vec<u8>> {
    use shiguredo_mp4::boxes::SampleEntry;

    match (sample_entry, format) {
        (SampleEntry::Hev1(entry), VideoFormat::H265) => {
            let mut annexb_data = Vec::new();
            for array in &entry.hvcc_box.nalu_arrays {
                for nalu in &array.nalus {
                    annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                    annexb_data.extend_from_slice(nalu);
                }
            }
            Ok(annexb_data)
        }
        (SampleEntry::Avc1(entry), VideoFormat::H264) => {
            let mut annexb_data = Vec::new();
            // SPS
            for sps in &entry.avcc_box.sps_list {
                annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                annexb_data.extend_from_slice(sps);
            }
            // PPS
            for pps in &entry.avcc_box.pps_list {
                annexb_data.extend_from_slice(&[0, 0, 0, 1]);
                annexb_data.extend_from_slice(pps);
            }
            Ok(annexb_data)
        }
        _ => {
            // VP8 / VP9 / AV1はパラメータセットを個別に送る必要がないため空のVecを返す
            Ok(Vec::new())
        }
    }
}

/// データの先頭にパラメータセットが含まれているかチェック
fn contains_parameter_sets(data: &[u8], format: VideoFormat) -> bool {
    if data.len() < NALU_HEADER_LENGTH + 1 {
        return false;
    }

    match format {
        VideoFormat::H265 => {
            // H.265 の NAL unit type は 2バイト目の上位6ビット
            let nal_unit_type = (data[NALU_HEADER_LENGTH] >> 1) & 0x3F;
            matches!(
                nal_unit_type,
                H265_NALU_TYPE_PPS | H265_NALU_TYPE_SPS | H265_NALU_TYPE_VPS
            )
        }
        VideoFormat::H264 => {
            // H.264 の NAL unit type は下位5ビット
            let nal_unit_type = data[NALU_HEADER_LENGTH] & 0x1F;
            matches!(nal_unit_type, H264_NALU_TYPE_SPS | H264_NALU_TYPE_PPS)
        }
        VideoFormat::Av1 => {
            // AV1はパラメータセットの概念が異なるため常にfalse
            false
        }
        _ => false,
    }
}
