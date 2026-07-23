use std::sync::{Arc, OnceLock};

use crate::{
    encoder::{OutputSink, VideoEncoderOptions},
    sample_entry::SharedSampleEntry,
    types::CodecName,
    video::av1,
    video::h264,
    video::h265,
    video::{RawVideoFrame, VideoFormat, VideoFrame},
};

/// callback スレッドが参照する遅延確定コンテキスト。
/// Encoder::new 後に get_sequence_params() から確定して set() される。
/// av1_sequence_header は AV1 の keyframe に Sequence Header OBU を付与する用途で、
/// H.264 / H.265 では意味を持たないため None にする (「AV1 のときだけ実データを持つ」を型で明示する)。
#[derive(Debug)]
struct HandlerContext {
    sample_entry: SharedSampleEntry,
    av1_sequence_header: Option<Vec<u8>>,
}

/// コールバックにキャプチャさせる HandlerContext の遅延確定スロット。
///
/// `shiguredo_nvcodec::Encoder::new` は handler を消費する API のため、
/// sample_entry を確定するために inner が必要 / inner を作るために handler が必要という循環がある。
/// そこで OnceLock を先に確保して handler にキャプチャさせ、
/// Encoder::new 後に get_sequence_params から HandlerContext を確定して set() する。
/// コールバック (ワーカースレッド) は encode() 経由でしか発火しないため、
/// encode() 前に set() が完了していればデータ競合なく読み取れる。
type HandlerContextSlot = Arc<OnceLock<HandlerContext>>;

#[derive(Debug)]
pub struct NvcodecEncoder {
    inner: shiguredo_nvcodec::Encoder<
        shiguredo_nvcodec::FnEncodeHandler<VideoFrame, shiguredo_nvcodec::Error>,
    >,
    encoded_format: VideoFormat,
    force_keyframe_next: bool,
}

/// shiguredo_nvcodec::Encoder の生成に必要なハンドラを構築する。
fn build_handler(
    sink: OutputSink,
    context_slot: HandlerContextSlot,
    encoded_format: VideoFormat,
) -> shiguredo_nvcodec::FnEncodeHandler<VideoFrame, shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnEncodeHandler::new(move |result| {
        handle_encode_callback(&sink, &context_slot, encoded_format, result);
    })
}

/// コールバックスレッドで発火する処理本体。
///
/// Ok の場合: `EncodedFrame::into_parts` から user_data を取り出してメタデータ (timestamp / size) を復元し、
/// Annex B → MP4 変換 (H.264/H.265) または Sequence Header OBU 付与 (AV1) を経て sink.emit_ok する。
/// Err の場合: メッセージに英語の prefix を付けて sink.emit_err する。
fn handle_encode_callback(
    sink: &OutputSink,
    context_slot: &HandlerContextSlot,
    encoded_format: VideoFormat,
    result: std::result::Result<
        shiguredo_nvcodec::EncodedFrame<VideoFrame>,
        shiguredo_nvcodec::Error,
    >,
) {
    match result {
        Ok(encoded_frame) => {
            let context = context_slot
                .get()
                .expect("BUG: HandlerContext must be set before first encode() call");
            let keyframe = matches!(
                encoded_frame.picture_type(),
                shiguredo_nvcodec::PictureType::I | shiguredo_nvcodec::PictureType::Idr
            );
            let (data, input_frame) = encoded_frame.into_parts();
            let frame_data = match convert_encoded_data(encoded_format, data, keyframe, context) {
                Ok(d) => d,
                Err(e) => {
                    sink.emit_err(e);
                    return;
                }
            };
            sink.emit_ok(VideoFrame {
                data: frame_data,
                format: encoded_format,
                keyframe,
                size: input_frame.size,
                timestamp: input_frame.timestamp,
                sample_entry: Some(context.sample_entry.clone()),
            });
        }
        Err(err) => {
            sink.emit_err(crate::Error::new(format!("nvcodec encode error: {err}")));
        }
    }
}

/// エンコード出力フレームを VideoFormat に応じて MP4 に載せる形式へ変換する。
///
/// AV1: キーフレームに Sequence Header OBU が欠落している場合のみ先頭に付与する。
/// H.264 / H.265: Annex B 形式から MP4 形式に変換する。
fn convert_encoded_data(
    encoded_format: VideoFormat,
    data: Vec<u8>,
    keyframe: bool,
    context: &HandlerContext,
) -> crate::Result<Vec<u8>> {
    if encoded_format == VideoFormat::Av1 {
        // encoded_format == Av1 の分岐に入るのは new_av1 経路のみで、
        // そこでは make_context が Some(seq_params) を確定して slot に set() する契約。
        let seq_header = context
            .av1_sequence_header
            .as_deref()
            .expect("BUG: AV1 encoder must have av1_sequence_header set");
        Ok(prepend_av1_sequence_header_if_needed(
            data, keyframe, seq_header,
        ))
    } else {
        convert_annexb_to_mp4(&data)
    }
}

/// AV1 のキーフレームで Sequence Header OBU が欠落している場合のみ、先頭に付与して返す。
/// それ以外の場合は data をそのまま返す。
fn prepend_av1_sequence_header_if_needed(
    data: Vec<u8>,
    keyframe: bool,
    seq_header: &[u8],
) -> Vec<u8> {
    if !keyframe || has_sequence_header(&data) {
        return data;
    }
    tracing::debug!(
        "prepending Sequence Header OBU to AV1 keyframe (seq_header: {} bytes, frame: {} bytes)",
        seq_header.len(),
        data.len()
    );
    let mut new_data = Vec::with_capacity(seq_header.len() + data.len());
    new_data.extend_from_slice(seq_header);
    new_data.extend_from_slice(&data);
    new_data
}

impl NvcodecEncoder {
    /// codec 別 config を受けて、 handler 準備 → Encoder::new →
    /// sample_entry 確定までの共通シーケンスを実行する。
    /// make_context は seq_params から HandlerContext を組み立てる責務で、
    /// codec 別の sample_entry 生成と av1_sequence_header 中身の差分を吸収する。
    fn build_encoder(
        config: shiguredo_nvcodec::EncoderConfig,
        sink: OutputSink,
        encoded_format: VideoFormat,
        make_context: impl FnOnce(Vec<u8>) -> crate::Result<HandlerContext>,
    ) -> crate::Result<Self> {
        let context_slot: HandlerContextSlot = Arc::new(OnceLock::new());
        let handler = build_handler(sink, context_slot.clone(), encoded_format);
        let inner = shiguredo_nvcodec::Encoder::new(config, handler)?;

        let seq_params = inner.get_sequence_params()?;
        context_slot
            .set(make_context(seq_params)?)
            .expect("BUG: HandlerContext must not be set before Encoder::new returns");

        Ok(Self {
            inner,
            encoded_format,
            force_keyframe_next: false,
        })
    }

    pub fn new_h264(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        tracing::debug!("create nvcodec(H264) encoder: {}x{}", width, height);

        let mut config = options.encode_params.nvcodec_h264.clone();
        config.codec = match config.codec {
            shiguredo_nvcodec::CodecConfig::H264(config) => {
                shiguredo_nvcodec::CodecConfig::H264(config)
            }
            _ => shiguredo_nvcodec::CodecConfig::H264(shiguredo_nvcodec::H264EncoderConfig {
                profile: None,
                idr_period: None,
            }),
        };
        config.width = width as u32;
        config.height = height as u32;
        config.framerate_num = options.frame_rate.numerator.get() as u32;
        config.framerate_den = options.frame_rate.denumerator.get() as u32;
        config.average_bitrate = Some(options.bitrate as u32);
        tracing::debug!("nvcodec h264 encoder config: {config:?}");

        Self::build_encoder(config, sink, VideoFormat::H264, |seq_params| {
            let sample_entry =
                SharedSampleEntry::new(h264::h264_sample_entry_from_annexb(&seq_params)?);
            Ok(HandlerContext {
                sample_entry,
                av1_sequence_header: None,
            })
        })
    }

    pub fn new_h265(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        tracing::debug!("create nvcodec(H265) encoder: {}x{}", width, height);

        let mut config = options.encode_params.nvcodec_h265.clone();
        config.codec = match config.codec {
            shiguredo_nvcodec::CodecConfig::Hevc(config) => {
                shiguredo_nvcodec::CodecConfig::Hevc(config)
            }
            _ => shiguredo_nvcodec::CodecConfig::Hevc(shiguredo_nvcodec::HevcEncoderConfig {
                profile: None,
                idr_period: None,
            }),
        };
        config.width = width as u32;
        config.height = height as u32;
        config.framerate_num = options.frame_rate.numerator.get() as u32;
        config.framerate_den = options.frame_rate.denumerator.get() as u32;
        config.average_bitrate = Some(options.bitrate as u32);
        tracing::debug!("nvcodec h265 encoder config: {config:?}");

        Self::build_encoder(config, sink, VideoFormat::H265, |seq_params| {
            let sample_entry = SharedSampleEntry::new(h265::h265_sample_entry_from_annexb(
                &seq_params,
                options.frame_rate,
            )?);
            Ok(HandlerContext {
                sample_entry,
                av1_sequence_header: None,
            })
        })
    }

    pub fn new_av1(options: &VideoEncoderOptions, sink: OutputSink) -> crate::Result<Self> {
        let width = options.width;
        let height = options.height;
        tracing::debug!(
            "create nvcodec(AV1) encoder: {}x{}",
            width.get(),
            height.get()
        );

        let mut config = options.encode_params.nvcodec_av1.clone();
        config.codec = match config.codec {
            shiguredo_nvcodec::CodecConfig::Av1(config) => {
                shiguredo_nvcodec::CodecConfig::Av1(config)
            }
            _ => shiguredo_nvcodec::CodecConfig::Av1(shiguredo_nvcodec::Av1EncoderConfig {
                profile: None,
                idr_period: None,
            }),
        };
        config.width = width.get() as u32;
        config.height = height.get() as u32;
        config.framerate_num = options.frame_rate.numerator.get() as u32;
        config.framerate_den = options.frame_rate.denumerator.get() as u32;
        config.average_bitrate = Some(options.bitrate as u32);
        tracing::debug!("nvcodec av1 encoder config: {config:?}");

        Self::build_encoder(config, sink, VideoFormat::Av1, |seq_params| {
            // NVENC SDK 13.0 のドキュメント (https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html#retrieving-sequence-parameters)
            // には以下の記載がある:
            //   "By default, SPS/PPS and Sequence Header OBU data will be attached to every IDR frame and Key frame for H.264/HEVC and AV1 respectively."
            //
            // しかし実際には、AV1 の場合、最初のキーフレームにのみ Sequence Header OBU が付与され、
            // 二番目以降のキーフレームには含まれない。これにより、二番目以降のキーフレームからシークすると、
            // デコーダが解像度やプロファイルなどの情報を取得できず、映像が再生できない問題が発生する。
            //
            // そのため、ここで Sequence Header OBU を get_sequence_params() で取得して保持しておき、
            // キーフレームのエンコード時に Sequence Header が含まれていない場合は、
            // hisui 側で明示的に付与するワークアラウンドを実装している。
            let sample_entry =
                SharedSampleEntry::new(av1::av1_sample_entry(width, height, &seq_params));
            Ok(HandlerContext {
                sample_entry,
                av1_sequence_header: Some(seq_params),
            })
        })
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let video_frame = frame.as_video_frame();

        // I420 から NV12 への変換
        let size = frame.size();
        let width = size.width;
        let height = size.height;
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;

        // NV12 用のバッファを確保
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height * 2; // U と V が交互に配置されているため
        let total_size = y_size + uv_size;

        let mut nv12_data = vec![0u8; total_size];
        let (nv12_y, nv12_uv) = nv12_data.split_at_mut(y_size);

        // libyuv を使って I420 から NV12 に変換
        let src = shiguredo_libyuv::I420Image {
            y: y_plane,
            y_stride: width,
            u: u_plane,
            u_stride: uv_width,
            v: v_plane,
            v_stride: uv_width,
        };

        let mut dst = shiguredo_libyuv::Nv12ImageMut {
            y: nv12_y,
            y_stride: width,
            uv: nv12_uv,
            uv_stride: width,
        };

        let size = shiguredo_libyuv::ImageSize::new(width, height);
        shiguredo_libyuv::i420_to_nv12(&src, &mut dst, size)?;

        // 入力フレームの軽量 clone を user_data として渡す。
        // コールバック (build_handler) で EncodedFrame::into_parts から取り出す。
        let encode_options = shiguredo_nvcodec::EncodeOptions {
            force_intra: self.force_keyframe_next,
            force_idr: self.force_keyframe_next,
            output_spspps: false,
        };
        self.force_keyframe_next = false;
        self.inner
            .encode(&nv12_data, &encode_options, video_frame.to_stripped())?;
        Ok(())
    }

    pub fn request_keyframe(&mut self) {
        self.force_keyframe_next = true;
    }

    pub fn finish(&mut self) -> crate::Result<()> {
        // flush で in-flight 完了を待ち合わせる
        self.inner.flush()?;
        Ok(())
    }

    pub fn codec(&self) -> CodecName {
        self.encoded_format.codec_name().expect("infallible")
    }
}

/// AV1 ペイロードの先頭に Sequence Header OBU が含まれているかチェック
fn has_sequence_header(data: &[u8]) -> bool {
    // 最低限 OBU Header の 1 バイトが必要
    if data.is_empty() {
        return false;
    }

    // 先頭の OBU Header を解析
    // obu_header のビット構成（LSB 基準）:
    //   - bit 7: obu_forbidden_bit (常に0)
    //   - bit 6-3: obu_type
    //   - bit 2: obu_extension_flag
    //   - bit 1: obu_has_size_field
    //   - bit 0: obu_reserved_1bit
    let obu_header = data[0];
    let obu_has_extension = (obu_header & 0b0000_0100) != 0;

    // OBU Extension が存在する場合は 2 バイト目も必要
    if obu_has_extension && data.len() < 2 {
        return false;
    }

    let obu_type = (obu_header >> 3) & 0x0F;

    // 先頭が Sequence Header (type=1) なら true
    obu_type == 1
}

/// Annex B 形式から MP4 形式への変換
///
/// Annex B 形式: スタートコード (0x00000001 or 0x000001) + NALU データ
/// MP4 形式: サイズ (4バイト) + NALU データ
fn convert_annexb_to_mp4(annexb_data: &[u8]) -> crate::Result<Vec<u8>> {
    let mut mp4_data = Vec::new();
    let mut pos = 0;

    while pos < annexb_data.len() {
        // スタートコードを探す (0x00000001 or 0x000001)
        let start_code_len =
            if pos + 4 <= annexb_data.len() && annexb_data[pos..pos + 4] == [0, 0, 0, 1] {
                4
            } else if pos + 3 <= annexb_data.len() && annexb_data[pos..pos + 3] == [0, 0, 1] {
                3
            } else if pos == 0 {
                return Err(crate::Error::new("No start code found at beginning"));
            } else {
                break;
            };

        pos += start_code_len;

        // 次のスタートコードまたはデータ終端を探す
        let nalu_start = pos;
        let mut nalu_end = annexb_data.len();

        for i in (pos + 3)..annexb_data.len() {
            if i + 4 <= annexb_data.len() && annexb_data[i..i + 4] == [0, 0, 0, 1] {
                nalu_end = i;
                break;
            }
            if i + 3 <= annexb_data.len() && annexb_data[i..i + 3] == [0, 0, 1] {
                nalu_end = i;
                break;
            }
        }

        let nalu_size = nalu_end - nalu_start;

        // MP4 形式: 4 バイトのサイズ + NALU データ
        mp4_data.extend_from_slice(&(nalu_size as u32).to_be_bytes());
        mp4_data.extend_from_slice(&annexb_data[nalu_start..nalu_end]);

        pos = nalu_end;
    }

    Ok(mp4_data)
}
