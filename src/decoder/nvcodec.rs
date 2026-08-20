use std::borrow::Cow;

use super::{DecodeConfig, OutputSink};
use crate::video::h264::extract_h264_sps_pps_from_avcc;
use crate::video::h265::{NALU_HEADER_LENGTH, extract_h265_vps_sps_pps_from_avcc};
use crate::video::{VideoFormat, VideoFrame};

#[derive(Debug)]
pub struct NvcodecDecoder {
    inner: shiguredo_nvcodec::Decoder<
        shiguredo_nvcodec::FnDecodeHandler<VideoFrame, shiguredo_nvcodec::Error>,
    >,
    parameter_sets: Option<Vec<u8>>, // VPS/SPS/PPS をキャッシュ
}

/// CUDA ワーカースレッドから呼ばれるコールバックの本体を共有クロージャ化したもの。
fn build_handler(
    sink: OutputSink,
) -> shiguredo_nvcodec::FnDecodeHandler<VideoFrame, shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnDecodeHandler::new(move |result| {
        handle_decode_callback(&sink, result);
    })
}

fn handle_decode_callback(
    sink: &OutputSink,
    result: std::result::Result<
        shiguredo_nvcodec::DecodedFrame<VideoFrame>,
        shiguredo_nvcodec::Error,
    >,
) {
    match result {
        Ok(decoded) => match convert_nv12_to_i420(decoded) {
            Ok(frame) => sink.emit_ok(frame),
            Err(err) => sink.emit_err(err),
        },
        Err(err) => {
            sink.emit_err(crate::Error::new(format!("nvcodec decode error: {err}")));
        }
    }
}

/// NV12 フォーマットのデコード済みフレームを I420 に変換して `VideoFrame` を構築する
///
/// `decoded.user_data()` に入力フレームが含まれており、 変換後のフレームにペアリングする。
fn convert_nv12_to_i420(
    decoded: shiguredo_nvcodec::DecodedFrame<VideoFrame>,
) -> crate::Result<VideoFrame> {
    let width = decoded.width();
    let height = decoded.height();

    let y_size = width * height;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let uv_size = uv_width * uv_height;
    let total_size = y_size + uv_size * 2;

    let mut i420_data = vec![0u8; total_size];
    let (y_plane, rest) = i420_data.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);

    let src = shiguredo_libyuv::Nv12Image {
        y: decoded.y_plane(),
        y_stride: decoded.y_stride(),
        uv: decoded.uv_plane(),
        uv_stride: decoded.uv_stride(),
    };
    let mut dst = shiguredo_libyuv::I420ImageMut {
        y: y_plane,
        y_stride: width,
        u: u_plane,
        u_stride: uv_width,
        v: v_plane,
        v_stride: uv_width,
    };

    let size = shiguredo_libyuv::ImageSize::new(width, height);
    shiguredo_libyuv::nv12_to_i420(&src, &mut dst, size)?;

    // `decoded` の借用終了後、 `into_parts` で入力フレームを取り出す
    let (_nv12_data, input_frame) = decoded.into_parts();

    Ok(VideoFrame::new_i420(
        input_frame,
        width,
        height,
        y_plane,
        u_plane,
        v_plane,
        width,
        uv_width,
        uv_width,
    ))
}

impl NvcodecDecoder {
    pub fn new_h264(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H264) decoder");
        let mut config = params.nvcodec_h264.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::H264;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_h265(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(H265) decoder");
        let mut config = params.nvcodec_h265.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Hevc;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_av1(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(AV1) decoder");
        let mut config = params.nvcodec_av1.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Av1;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_vp8(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP8) decoder");
        let mut config = params.nvcodec_vp8.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp8;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn new_vp9(params: &DecodeConfig, sink: OutputSink) -> crate::Result<Self> {
        tracing::debug!("create nvcodec(VP9) decoder");
        let mut config = params.nvcodec_vp9.clone();
        config.codec = shiguredo_nvcodec::DecoderCodec::Vp9;
        Ok(Self {
            inner: shiguredo_nvcodec::Decoder::new(config, build_handler(sink))?,
            parameter_sets: None,
        })
    }

    pub fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(
            frame.format,
            VideoFormat::H264
                | VideoFormat::H264AnnexB
                | VideoFormat::H265
                | VideoFormat::Vp8
                | VideoFormat::Vp9
                | VideoFormat::Av1
        ) {
            return Err(crate::Error::new(format!(
                "unsupported input format for NVDEC: {:?}",
                frame.format
            )));
        }

        // サンプルエントリーからパラメータセットを抽出してキャッシュを更新する
        //
        // develop の MP4 reader (`src/mp4/sync_reader.rs`) は全フレームに sample_entry を
        // 付与する。解像度変化などで sample_entry が更新された場合に、新しいパラメータセット
        // (H.264 は SPS / PPS、H.265 は VPS / SPS / PPS) を prepend できるよう毎フレーム抽出する。
        if let Some(sample_entry) = &frame.sample_entry {
            self.parameter_sets = Some(extract_parameter_sets_annexb(
                sample_entry.get(),
                frame.format,
            )?);
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
                && !contains_parameter_sets(data, frame.format)?
            {
                data_annexb.extend_from_slice(parameter_sets);
            }

            while !data.is_empty() {
                if data.len() < NALU_HEADER_LENGTH {
                    return Err(crate::Error::new(format!(
                        "invalid AVC/HEVC payload: NALU length header is truncated (remaining={})",
                        data.len()
                    )));
                }
                let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                data = &data[NALU_HEADER_LENGTH..];

                if data.len() < n {
                    return Err(crate::Error::new(format!(
                        "invalid AVC/HEVC payload: NALU data is truncated (required={n}, remaining={})",
                        data.len()
                    )));
                }
                data_annexb.extend_from_slice(&[0, 0, 0, 1]);
                data_annexb.extend_from_slice(&data[..n]);

                data = &data[n..];
            }

            Cow::Owned(data_annexb)
        };

        // 入力フレームを軽量化した VideoFrame を `UserData` として渡す。
        // shiguredo_nvcodec が `pending_user_data` を FIFO 管理し、 Err 時の clear まで自動で行う。
        self.inner.decode(&data, frame.to_stripped())?;
        Ok(())
    }

    /// 進行中のフレームのデコード完了を待ち合わせる。
    ///
    /// `shiguredo_nvcodec::Decoder::flush()` の戻り時点でコールバックはすべて同期的に呼び切られている
    /// 前提であり、 残フレームおよびエラーのシンクへの emit はそのコールバック内で完了するため、
    /// ここでは追加処理不要。 コールバック内で発生した `Err` は `sink.emit_err()` 経由で内部
    /// チャンネルに積まれ、 `finish()` の戻り値からは検出できないため、 利用側は `finish()` の直後に
    /// `poll_output` の `try_recv` ループで残物を全て吸い出すこと。
    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.flush()?;
        Ok(())
    }
}

/// サンプルエントリーからパラメータセットを Annex.B 形式で抽出
fn extract_parameter_sets_annexb(
    sample_entry: &shiguredo_mp4::boxes::SampleEntry,
    format: VideoFormat,
) -> crate::Result<Vec<u8>> {
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
        (SampleEntry::Hvc1(entry), VideoFormat::H265) => {
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

/// データ内にパラメータセットの完全な組 (H.264 は SPS / PPS、H.265 は VPS / SPS / PPS) が
/// 含まれているかチェック
///
/// length prefix ベースで全 NALU を走査する共通ロジック
/// (`extract_h264_sps_pps_from_avcc` / `extract_h265_vps_sps_pps_from_avcc`) を再利用する。
/// `[SEI][SPS][PPS][IDR]` や `[AUD][SPS][PPS][IDR]` のように SPS / PPS が先頭以外に現れる
/// keyframe でも正しく検出できる。
///
/// NVDEC が keyframe をデコードするにはパラメータセットの完全な組が必要なため、一部だけ
/// 揃っている場合は false を返し、呼び出し側で sample_entry 由来の完全な組を補完させる。
///
/// フレームデータが壊れている場合 (長さプレフィックスがデータ末尾を超える) は `Err` を返す。
fn contains_parameter_sets(data: &[u8], format: VideoFormat) -> crate::Result<bool> {
    match format {
        VideoFormat::H264 => {
            let p = extract_h264_sps_pps_from_avcc(data)?;
            // NVDEC が H.264 をデコードするには SPS と PPS の両方が揃っている必要があるため、
            // 完全な組が揃ったときだけ true を返す (一部のみでは sample_entry から補完する)。
            Ok(p.sps.is_some() && p.pps.is_some())
        }
        VideoFormat::H265 => {
            let p = extract_h265_vps_sps_pps_from_avcc(data)?;
            // NVDEC が H.265 をデコードするには VPS / SPS / PPS が全て揃っている必要があるため、
            // 完全な組が揃ったときだけ true を返す。
            Ok(p.vps.is_some() && p.sps.is_some() && p.pps.is_some())
        }
        _ => {
            // AV1 等はパラメータセットの概念が異なるため常に false
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // PPS NAL を Annex-B 形式 (先頭 4 バイト start code + NAL バイト列) で表現したフィクスチャ
    const PPS_ANNEXB: &[u8] = &[0, 0, 0, 1, 0x68, 0xce, 0x06, 0xe2];

    #[test]
    fn extract_parameter_sets_annexb_h264_concats_sps_pps() -> crate::Result<()> {
        // H.264 の avcC (SPS / PPS) が Annex-B 形式に変換されること
        let annexb: Vec<u8> = [
            &crate::video::h264::tests::SPS_320X240_ANNEXB[..],
            PPS_ANNEXB,
        ]
        .concat();
        let entry = crate::video::h264::h264_sample_entry_from_annexb(&annexb)?;
        let out = extract_parameter_sets_annexb(&entry, VideoFormat::H264)?;
        assert_eq!(out, annexb, "SPS / PPS が Annex-B 形式で抽出されること");
        Ok(())
    }

    #[test]
    fn extract_parameter_sets_annexb_h265_concats_vps_sps_pps() -> crate::Result<()> {
        // H.265 の hvcc (VPS / SPS / PPS) が Annex-B 形式に変換されること
        let vps = vec![0x40, 0x01, 0xaa];
        let sps = crate::video::h265::tests::HEVC_SPS_640X480.to_vec();
        let pps = vec![0x44, 0x01, 0xcc];
        let (entry, _frame_size) = crate::video::h265::h265_sample_entry_from_vps_sps_pps_lists(
            vec![vps.clone()],
            vec![sps.clone()],
            vec![pps.clone()],
            crate::video::FrameRate::FPS_30,
        )?;
        let out = extract_parameter_sets_annexb(&entry, VideoFormat::H265)?;
        let mut expected = Vec::new();
        for nalu in [&vps[..], &sps[..], &pps[..]] {
            expected.extend_from_slice(&[0, 0, 0, 1]);
            expected.extend_from_slice(nalu);
        }
        assert_eq!(
            out, expected,
            "VPS / SPS / PPS が Annex-B 形式で抽出されること"
        );
        Ok(())
    }

    #[test]
    fn extract_parameter_sets_annexb_returns_empty_for_non_h264_h265() -> crate::Result<()> {
        // VP8 / VP9 / AV1 はパラメータセットを持たないため空を返すこと
        let annexb: Vec<u8> = [
            &crate::video::h264::tests::SPS_320X240_ANNEXB[..],
            PPS_ANNEXB,
        ]
        .concat();
        let entry = crate::video::h264::h264_sample_entry_from_annexb(&annexb)?;
        for format in [VideoFormat::Vp8, VideoFormat::Vp9, VideoFormat::Av1] {
            let out = extract_parameter_sets_annexb(&entry, format)?;
            assert!(out.is_empty(), "{format:?} では空を返すこと");
        }
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h264_returns_true_for_full_sps_pps() -> crate::Result<()> {
        // SPS (0x67) と PPS (0x68) の完全な組が揃っている場合は true を返すこと
        assert!(contains_parameter_sets(
            &[0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x68],
            VideoFormat::H264
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h264_returns_true_for_sei_aud_prefix_full() -> crate::Result<()> {
        // SEI (0x06) や AUD (0x09) が先頭、後続に SPS / PPS の完全な組がある場合は true を返すこと
        assert!(contains_parameter_sets(
            &[0, 0, 0, 1, 0x06, 0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x68],
            VideoFormat::H264
        )?);
        assert!(contains_parameter_sets(
            &[0, 0, 0, 1, 0x09, 0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x68],
            VideoFormat::H264
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h264_returns_false_for_partial_sps() -> crate::Result<()> {
        // SPS のみ (PPS 欠落) の場合は false を返し、sample_entry から PPS を補完させること。
        // 旧実装は SPS だけでも true を返していたが、NVDEC には SPS と PPS の両方が必要。
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x67],
            VideoFormat::H264
        )?);
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x68],
            VideoFormat::H264
        )?);
        // SEI 先行で後続に SPS のみ (PPS 欠落) の場合も false
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x06, 0, 0, 0, 1, 0x67],
            VideoFormat::H264
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h264_returns_false_for_idr_sei() -> crate::Result<()> {
        // 先頭 NAL が IDR (0x65) / SEI (0x06) のみで SPS / PPS を含まない場合は false を返すこと
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x65],
            VideoFormat::H264
        )?);
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x06],
            VideoFormat::H264
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h265_returns_true_for_full_vps_sps_pps() -> crate::Result<()> {
        // VPS (0x40) / SPS (0x42) / PPS (0x44) の完全な組が揃っている場合は true を返すこと
        assert!(contains_parameter_sets(
            &[0, 0, 0, 1, 0x40, 0, 0, 0, 1, 0x42, 0, 0, 0, 1, 0x44],
            VideoFormat::H265
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h265_returns_true_for_sei_aud_prefix_full() -> crate::Result<()> {
        // SEI (NAL type 39 = 0x4e) や AUD (NAL type 35 = 0x46) が先頭、後続に VPS / SPS / PPS の
        // 完全な組がある場合は true を返すこと。
        assert!(contains_parameter_sets(
            &[
                0, 0, 0, 1, 0x4e, 0, 0, 0, 1, 0x40, 0, 0, 0, 1, 0x42, 0, 0, 0, 1, 0x44
            ],
            VideoFormat::H265
        )?);
        assert!(contains_parameter_sets(
            &[
                0, 0, 0, 1, 0x46, 0, 0, 0, 1, 0x40, 0, 0, 0, 1, 0x42, 0, 0, 0, 1, 0x44
            ],
            VideoFormat::H265
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h265_returns_false_for_partial_parameter_sets() -> crate::Result<()>
    {
        // VPS / SPS / PPS の一部だけ (完全な組ではない) 場合は false を返し、
        // sample_entry から補完させること。
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x40],
            VideoFormat::H265
        )?);
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x42],
            VideoFormat::H265
        )?);
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x44],
            VideoFormat::H265
        )?);
        // SEI 先行で後続に SPS のみ (VPS / PPS 欠落) の場合も false
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x4e, 0, 0, 0, 1, 0x42],
            VideoFormat::H265
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_h265_returns_false_for_idr() -> crate::Result<()> {
        // 先頭 NAL が IDR (0x26) のみでパラメータセットを含まない場合は false を返すこと
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0x26],
            VideoFormat::H265
        )?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_empty_returns_false() -> crate::Result<()> {
        // 空バッファではループが回らずパラメータセットなしとして false を返すこと
        assert!(!contains_parameter_sets(&[], VideoFormat::H264)?);
        assert!(!contains_parameter_sets(&[], VideoFormat::H265)?);
        Ok(())
    }

    #[test]
    fn contains_parameter_sets_truncated_returns_err() {
        // NALU 長プレフィックスが宣言する長さに対してデータが不足する壊れたバッファは
        // Err を返すこと
        assert!(contains_parameter_sets(&[0, 0, 0, 1], VideoFormat::H264).is_err());
        assert!(contains_parameter_sets(&[0, 0, 0, 1], VideoFormat::H265).is_err());
        // 長さ 1 の NALU を宣言したがデータが 2 バイトある場合も末尾が壊れている (H.264)
        assert!(contains_parameter_sets(&[0, 0, 0, 1, 0x67, 0x42], VideoFormat::H264).is_err());
        // H.265 も同様に末尾の余分バイトで Err になる
        assert!(contains_parameter_sets(&[0, 0, 0, 1, 0x42, 0x01], VideoFormat::H265).is_err());
    }

    #[test]
    fn contains_parameter_sets_av1_returns_false() -> crate::Result<()> {
        // AV1 はパラメータセットの概念が異なるため常に false を返すこと
        assert!(!contains_parameter_sets(
            &[0, 0, 0, 1, 0xaa, 0xbb],
            VideoFormat::Av1
        )?);
        Ok(())
    }
}
