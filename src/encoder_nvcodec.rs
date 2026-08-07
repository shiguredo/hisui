use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use orfail::OrFail;
use shiguredo_mp4::boxes::SampleEntry;

use crate::{
    encoder::VideoEncoderOptions,
    types::CodecName,
    video::{VideoFormat, VideoFrame},
    video_av1, video_h264, video_h265,
};

/// エンコード結果 (成功したフレーム or エラー) を受け取るためのキュー。
///
/// `shiguredo_nvcodec::Encoder` はコールバックベース API のため、
/// callback スレッドから同期的に取り出し可能な形で結果を蓄積する。
/// エラーは初回のものだけ保持し、次回 `encode()` / `finish()` 呼び出しで
/// 取り出して Err として上位に伝播する
#[derive(Debug, Default)]
struct EncodeOutputQueue {
    ok_frames: VecDeque<EncodedFrameWithMeta>,
    error: Option<orfail::Failure>,
}

/// callback で受け取った圧縮フレームと、user_data として渡した入力側メタデータ
#[derive(Debug)]
struct EncodedFrameWithMeta {
    data: Vec<u8>,
    keyframe: bool,
    input_frame: VideoFrame,
}

/// callback スレッドが参照する遅延確定コンテキスト。
///
/// AV1 の `av1_sequence_header` は `Encoder::new` 後の `get_sequence_params()` でしか得られないが、
/// `Encoder::new` は handler を消費する API のため、handler にはあらかじめ `OnceLock` だけを
/// キャプチャさせておき、`Encoder::new` の直後に `HandlerContext` を確定して `set()` する。
struct HandlerContext {
    /// AV1 のキーフレームに Sequence Header OBU を付与するためのバイト列。
    /// H.264 / H.265 では意味を持たないため `None`
    av1_sequence_header: Option<Vec<u8>>,
}

type HandlerContextSlot = Arc<OnceLock<HandlerContext>>;

/// callback スレッドから呼ばれる Handler をラップした型
type NvcodecHandler = shiguredo_nvcodec::FnEncodeHandler<VideoFrame, shiguredo_nvcodec::Error>;

#[derive(Debug)]
pub struct NvcodecEncoder {
    inner: shiguredo_nvcodec::Encoder<NvcodecHandler>,
    output_queue: Arc<Mutex<EncodeOutputQueue>>,
    encoded_format: VideoFormat,
    /// 最初の出力フレームに sample_entry を載せるために保持する。一度 take() したら以降は None のまま
    sample_entry: Option<SampleEntry>,
    /// `shiguredo_nvcodec::Encoder` 内でまだ drain されていない frame 数の推定値。
    /// `encode()` の呼び出しで +1、`inner.flush()` の完了で 0 にリセットする。
    /// `next_encoded_frame()` の pop は NVENC の外 (`output_queue`) の話なのでここでは触らない。
    in_flight: usize,
}

impl NvcodecEncoder {
    /// `shiguredo_nvcodec::Encoder` の `run_worker` は
    /// `i_to_send - i_got >= n_encoder_buffer` で `"encoder buffer is full"` エラーを返して
    /// frame を捨てる。`n_encoder_buffer = frame_interval_p + 3 = 4` (frame_interval_p = 1 前提) なので、
    /// 送信直前の in-flight を 3 以下に保てば buffer full を回避できる。
    const IN_FLIGHT_LIMIT: usize = 3;

    pub fn new_h264(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        log::debug!("create nvcodec(H264) encoder: {}x{}", width, height);

        let mut config = options.encode_params.nvcodec_h264.clone();
        Self::override_config_size_and_bitrate(&mut config, options);
        log::debug!("nvcodec h264 encoder config: {config:?}");

        Self::build_encoder(config, VideoFormat::H264, |seq_params| {
            let entry =
                video_h264::h264_sample_entry_from_annexb(width, height, &seq_params).or_fail()?;
            Ok((
                entry,
                HandlerContext {
                    av1_sequence_header: None,
                },
            ))
        })
    }

    pub fn new_h265(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        log::debug!("create nvcodec(H265) encoder: {}x{}", width, height);

        let mut config = options.encode_params.nvcodec_h265.clone();
        Self::override_config_size_and_bitrate(&mut config, options);
        log::debug!("nvcodec h265 encoder config: {config:?}");

        let frame_rate = options.frame_rate;
        Self::build_encoder(config, VideoFormat::H265, move |seq_params| {
            let entry = video_h265::h265_sample_entry_from_annexb(
                width,
                height,
                frame_rate,
                &seq_params,
            )
            .or_fail()?;
            Ok((
                entry,
                HandlerContext {
                    av1_sequence_header: None,
                },
            ))
        })
    }

    pub fn new_av1(options: &VideoEncoderOptions) -> orfail::Result<Self> {
        let width = options.width;
        let height = options.height;
        log::debug!(
            "create nvcodec(AV1) encoder: {}x{}",
            width.get(),
            height.get()
        );

        let mut config = options.encode_params.nvcodec_av1.clone();
        Self::override_config_size_and_bitrate(&mut config, options);
        log::debug!("nvcodec av1 encoder config: {config:?}");

        Self::build_encoder(config, VideoFormat::Av1, move |seq_params| {
            // NVENC SDK 13.0 のドキュメント (https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html#retrieving-sequence-parameters)
            // には以下の記載がある:
            //   "By default, SPS/PPS and Sequence Header OBU data will be attached to every IDR frame and Key frame for H.264/HEVC and AV1 respectively."
            //
            // しかし実際には、AV1 の場合、最初のキーフレームにのみ Sequence Header OBU が付与され、
            // 二番目以降のキーフレームには含まれない。これにより、二番目以降のキーフレームからシークすると、
            // デコーダが解像度やプロファイルなどの情報を取得できず、映像が再生できない問題が発生する。
            //
            // そのため、ここで Sequence Header OBU を get_sequence_params() で取得して保持しておき、
            // キーフレームのエンコード時に Sequence Header が含まれていない場合は明示的に付与する。
            let entry = video_av1::av1_sample_entry(width, height, &seq_params);
            Ok((
                entry,
                HandlerContext {
                    av1_sequence_header: Some(seq_params),
                },
            ))
        })
    }

    /// codec 別 config を受けて、handler 準備 -> Encoder::new -> sample_entry 確定までの共通シーケンスを実行する
    fn build_encoder(
        config: shiguredo_nvcodec::EncoderConfig,
        encoded_format: VideoFormat,
        make_context: impl FnOnce(Vec<u8>) -> orfail::Result<(SampleEntry, HandlerContext)>,
    ) -> orfail::Result<Self> {
        let context_slot: HandlerContextSlot = Arc::new(OnceLock::new());
        let output_queue: Arc<Mutex<EncodeOutputQueue>> = Arc::new(Mutex::new(Default::default()));

        let handler = build_handler(output_queue.clone(), context_slot.clone(), encoded_format);
        let inner = shiguredo_nvcodec::Encoder::new(config, handler).or_fail()?;

        let seq_params = inner.get_sequence_params().or_fail()?;
        let (sample_entry, context) = make_context(seq_params).or_fail()?;
        context_slot
            .set(context)
            .ok()
            .expect("BUG: HandlerContext must not be set before Encoder::new returns");

        Ok(Self {
            inner,
            output_queue,
            encoded_format,
            sample_entry: Some(sample_entry),
            in_flight: 0,
        })
    }

    fn override_config_size_and_bitrate(
        config: &mut shiguredo_nvcodec::EncoderConfig,
        options: &VideoEncoderOptions,
    ) {
        config.width = options.width.get() as u32;
        config.height = options.height.get() as u32;
        config.framerate_num = options.frame_rate.numerator.get() as u32;
        config.framerate_den = options.frame_rate.denumerator.get() as u32;
        config.average_bitrate = Some(options.bitrate as u32);
    }

    pub fn encode(&mut self, frame: &VideoFrame) -> orfail::Result<()> {
        self.take_pending_error()?;

        (frame.format == VideoFormat::I420).or_fail()?;

        // LIMIT に達したら inner.flush() で全 in-flight を drain してから次の送信に進む
        // (batched flush 方式)。shiguredo_nvcodec は送信直前の in-flight が n_encoder_buffer 未満で
        // ないと "encoder buffer is full" で frame を捨てるため、送信前に必ず 3 以下に抑える。
        if self.in_flight >= Self::IN_FLIGHT_LIMIT {
            self.inner.flush().or_fail()?;
            self.in_flight = 0;
        }

        // I420 から NV12 への変換
        let width = frame.width;
        let height = frame.height;
        let (y_plane, u_plane, v_plane) = frame.as_yuv_planes().or_fail()?;

        // NV12 用のバッファを確保
        let y_size = width * height;
        let uv_width = width.div_ceil(2);
        let uv_height = height.div_ceil(2);
        let uv_size = uv_width * uv_height * 2; // U と V が交互に配置されているため
        let total_size = y_size + uv_size;

        let mut nv12_data = vec![0u8; total_size];
        let (nv12_y, nv12_uv) = nv12_data.split_at_mut(y_size);

        // libyuv を使って I420 から NV12 に変換
        let src = shiguredo_libyuv::I420Planes {
            y: y_plane,
            y_stride: width,
            u: u_plane,
            u_stride: uv_width,
            v: v_plane,
            v_stride: uv_width,
        };

        let mut dst = shiguredo_libyuv::Nv12PlanesMut {
            y: nv12_y,
            y_stride: width,
            uv: nv12_uv,
            uv_stride: width,
        };

        let size = shiguredo_libyuv::ImageSize::new(width, height);
        shiguredo_libyuv::i420_to_nv12(&src, &mut dst, size).or_fail()?;

        // callback スレッドで input_frame のメタデータを復元するために軽量な to_stripped() を渡す
        let encode_options = shiguredo_nvcodec::EncodeOptions {
            force_intra: false,
            force_idr: false,
            output_spspps: false,
        };
        self.inner
            .encode(&nv12_data, &encode_options, frame.to_stripped())
            .or_fail()?;
        // encode() は job_tx.send() で fire-and-forget なので、NVENC 内 pending が +1 された分を
        // 追跡する。次回 encode() の冒頭で LIMIT を超えていれば flush で drain する。
        self.in_flight += 1;
        Ok(())
    }

    pub fn finish(&mut self) -> orfail::Result<()> {
        // encode() は LIMIT 到達時のみ flush する batched flush 方式なので、
        // finish() 時点では in_flight が 0〜LIMIT の範囲を取りうる。
        // 残 in-flight を drain して EOS 前の callback を発火させる。
        self.inner.flush().or_fail()?;
        self.in_flight = 0;
        self.take_pending_error()?;
        Ok(())
    }

    /// callback スレッドで発生したエラーがあれば取り出して返す
    fn take_pending_error(&self) -> orfail::Result<()> {
        let error = self
            .output_queue
            .lock()
            .expect("output queue is poisoned")
            .error
            .take();
        if let Some(err) = error {
            return Err(err);
        }
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        let encoded = self
            .output_queue
            .lock()
            .expect("output queue is poisoned")
            .ok_frames
            .pop_front()?;
        Some(VideoFrame {
            source_id: encoded.input_frame.source_id.clone(),
            data: encoded.data,
            format: self.encoded_format,
            keyframe: encoded.keyframe,
            width: encoded.input_frame.width,
            height: encoded.input_frame.height,
            timestamp: encoded.input_frame.timestamp,
            duration: encoded.input_frame.duration,
            sample_entry: self.sample_entry.take(),
        })
    }

    pub fn codec(&self) -> CodecName {
        self.encoded_format.codec_name().expect("infallible")
    }
}

/// shiguredo_nvcodec::Encoder が消費する handler を構築する
fn build_handler(
    output_queue: Arc<Mutex<EncodeOutputQueue>>,
    context_slot: HandlerContextSlot,
    encoded_format: VideoFormat,
) -> NvcodecHandler {
    shiguredo_nvcodec::FnEncodeHandler::new(move |result| {
        handle_encode_callback(&output_queue, &context_slot, encoded_format, result);
    })
}

/// callback スレッドから呼ばれるコールバック本体
fn handle_encode_callback(
    output_queue: &Mutex<EncodeOutputQueue>,
    context_slot: &OnceLock<HandlerContext>,
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
            let frame_data =
                match convert_encoded_data(encoded_format, data, keyframe, context) {
                    Ok(d) => d,
                    Err(e) => {
                        output_queue
                            .lock()
                            .expect("output queue is poisoned")
                            .error
                            .get_or_insert(e);
                        return;
                    }
                };
            output_queue
                .lock()
                .expect("output queue is poisoned")
                .ok_frames
                .push_back(EncodedFrameWithMeta {
                    data: frame_data,
                    keyframe,
                    input_frame,
                });
        }
        Err(err) => {
            output_queue
                .lock()
                .expect("output queue is poisoned")
                .error
                .get_or_insert_with(|| {
                    orfail::Failure::new(format!("nvcodec encode error: {err}"))
                });
        }
    }
}

/// エンコード出力フレームを VideoFormat に応じて MP4 に載せる形式へ変換する
fn convert_encoded_data(
    encoded_format: VideoFormat,
    data: Vec<u8>,
    keyframe: bool,
    context: &HandlerContext,
) -> orfail::Result<Vec<u8>> {
    if encoded_format == VideoFormat::Av1 {
        // encoded_format == Av1 の分岐に入るのは new_av1 経路のみ。そこでは
        // make_context が Some(seq_params) を確定して slot に set() する契約
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

/// AV1 のキーフレームで Sequence Header OBU が欠落している場合のみ、先頭に付与して返す
fn prepend_av1_sequence_header_if_needed(
    data: Vec<u8>,
    keyframe: bool,
    seq_header: &[u8],
) -> Vec<u8> {
    if !keyframe || has_sequence_header(&data) {
        return data;
    }
    log::debug!(
        "prepending Sequence Header OBU to AV1 keyframe (seq_header: {} bytes, frame: {} bytes)",
        seq_header.len(),
        data.len()
    );
    let mut new_data = Vec::with_capacity(seq_header.len() + data.len());
    new_data.extend_from_slice(seq_header);
    new_data.extend_from_slice(&data);
    new_data
}

/// AV1 ペイロードの先頭に Sequence Header OBU が含まれているかチェック
fn has_sequence_header(data: &[u8]) -> bool {
    // 最低限 OBU Header の 1 バイトが必要
    if data.is_empty() {
        return false;
    }

    // 先頭の OBU Header を解析
    // obu_header のビット構成:
    //   - bit 0: obu_forbidden_bit (常に0)
    //   - bit 1-4: obu_type
    //   - bit 5: obu_extension_flag
    //   - bit 6: obu_has_size_field
    //   - bit 7: obu_reserved_1bit
    let obu_header = data[0];
    let obu_has_extension = (obu_header & 0b0010_0000) != 0;

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
fn convert_annexb_to_mp4(annexb_data: &[u8]) -> orfail::Result<Vec<u8>> {
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
                return Err(orfail::Failure::new("No start code found at beginning"));
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
