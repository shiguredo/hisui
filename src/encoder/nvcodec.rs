use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::{
    encoder::{OutputSink, VideoEncoderOptions},
    sample_entry::SharedSampleEntry,
    types::CodecName,
    video::av1,
    video::h264,
    video::h265,
    video::{RawVideoFrame, VideoFormat, VideoFrame},
};

/// 本スレッドと callback スレッドで共有される入力フレームキュー
type SharedInputQueue = Arc<Mutex<VecDeque<VideoFrame>>>;

#[derive(Debug)]
pub struct NvcodecEncoder {
    inner: shiguredo_nvcodec::Encoder<
        shiguredo_nvcodec::FnEncodeHandler<(), shiguredo_nvcodec::Error>,
    >,
    // 本スレッドで encode() 直後に push_back し、callback スレッドで pop_front する。
    // Mutex ホールドスコープは push_back / pop_front のみに限定する。
    input_queue: SharedInputQueue,
    // 全出力フレームに載せるサンプルエントリー。Arc 共有なので毎フレームの clone は安価。
    sample_entry: SharedSampleEntry,
    encoded_format: VideoFormat,
    av1_sequence_header: Arc<Vec<u8>>,
    force_keyframe_next: bool,
}

/// shiguredo_nvcodec::Encoder の生成に必要なハンドラを構築する。
///
/// callback スレッドで input_queue から pop → Annex B → MP4 変換 (H.264/H.265) または
/// Sequence Header OBU 付与 (AV1) → sink.emit_ok までを一貫して実施する。
fn build_handler(
    sink: OutputSink,
    input_queue: SharedInputQueue,
    sample_entry: SharedSampleEntry,
    encoded_format: VideoFormat,
    av1_sequence_header: Arc<Vec<u8>>,
) -> shiguredo_nvcodec::FnEncodeHandler<(), shiguredo_nvcodec::Error> {
    shiguredo_nvcodec::FnEncodeHandler::new(move |result| match result {
        Ok(encoded_frame) => {
            let input_frame = {
                let mut queue = input_queue
                    .lock()
                    .expect("nvcodec input queue lock poisoned");
                queue.pop_front()
            };
            let Some(input_frame) = input_frame else {
                sink.emit_err(crate::Error::new(
                    "encoded frame produced without input frame",
                ));
                return;
            };

            // キーフレーム判定
            let keyframe = matches!(
                encoded_frame.picture_type(),
                shiguredo_nvcodec::PictureType::I | shiguredo_nvcodec::PictureType::Idr
            );

            // AV1 の場合は変換不要だが、キーフレームに Sequence Header が含まれていない場合は付与
            // H.264/H.265 の場合は Annex B から MP4 形式に変換
            let frame_data = if encoded_format == VideoFormat::Av1 {
                let (mut data, _) = encoded_frame.into_parts();

                // AV1 のキーフレームで Sequence Header OBU が含まれていない場合は先頭に付与
                if keyframe && !has_sequence_header(&data) {
                    tracing::debug!(
                        "prepending Sequence Header OBU to AV1 keyframe (seq_header: {} bytes, frame: {} bytes)",
                        av1_sequence_header.len(),
                        data.len()
                    );
                    let mut new_data = Vec::new();
                    new_data.extend_from_slice(&av1_sequence_header);
                    new_data.extend_from_slice(&data);
                    data = new_data;
                }
                data
            } else {
                match convert_annexb_to_mp4(encoded_frame.data()) {
                    Ok(data) => data,
                    Err(e) => {
                        sink.emit_err(e);
                        return;
                    }
                }
            };

            sink.emit_ok(VideoFrame {
                data: frame_data,
                format: encoded_format,
                keyframe,
                size: input_frame.size,
                timestamp: input_frame.timestamp,
                sample_entry: Some(sample_entry.clone()),
            });
        }
        Err(err) => {
            sink.emit_err(crate::Error::new(format!("nvcodec encode error: {err}")));
        }
    })
}

impl NvcodecEncoder {
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

        let input_queue: SharedInputQueue = Arc::new(Mutex::new(VecDeque::new()));
        // sequence params の取得のために一旦 handler なしで inner を作れないため、
        // handler を先に構築するが、 sequence params は inner から取得する必要がある。
        // shiguredo_nvcodec::Encoder::new は handler を消費するため、後段で必要な sample_entry
        // 生成には inner.get_sequence_params() が必要。 したがってまず build_handler を呼び、
        // その後 inner を new して sequence params → sample entry を作る流れは循環する。
        // 実装上は「inner を作った後で sample_entry を生成できないと handler にキャプチャできない」
        // ため、二段階に分ける: 先に sample_entry 相当を保持する Arc を確保して handler に渡し、
        // inner 生成後に実体を書き込む方式で解決する。
        //
        // ただし現状の SharedSampleEntry (Arc 内包) は new 時に確定値を必要とするため、
        // ここでは inner を一度 handler なしで作れないので、
        // 「sequence params 取得 → sample_entry 生成 → 別 handler で本番用 inner を作り直す」の
        // 二重コンストラクトはコスト高。
        //
        // 実装単純化のため、 handler を生成した後の sample_entry は Arc に包み、
        // handler の capture では Arc<Mutex<Option<SharedSampleEntry>>> のような
        // 遅延確定にすることも可能だが、 現状 shiguredo_nvcodec の API は
        // handler を new に渡す前提のため、 workaround として「sequence params を取得する
        // ためだけの一時 handler」を作って捨てる形にする。 これは encode の前なので副作用なし。
        let (tmp_handler, _tmp_input_queue) = build_tmp_handler_for_seq_params_probe();
        let tmp_inner = shiguredo_nvcodec::Encoder::new(config.clone(), tmp_handler)?;
        let seq_params = tmp_inner.get_sequence_params()?;
        drop(tmp_inner);
        let sample_entry =
            SharedSampleEntry::new(h264::h264_sample_entry_from_annexb(&seq_params)?);

        let av1_sequence_header = Arc::new(Vec::new());
        let handler = build_handler(
            sink,
            input_queue.clone(),
            sample_entry.clone(),
            VideoFormat::H264,
            av1_sequence_header.clone(),
        );
        let inner = shiguredo_nvcodec::Encoder::new(config, handler)?;

        Ok(Self {
            inner,
            input_queue,
            sample_entry,
            encoded_format: VideoFormat::H264,
            av1_sequence_header,
            force_keyframe_next: false,
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

        let input_queue: SharedInputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let (tmp_handler, _tmp_input_queue) = build_tmp_handler_for_seq_params_probe();
        let tmp_inner = shiguredo_nvcodec::Encoder::new(config.clone(), tmp_handler)?;
        let seq_params = tmp_inner.get_sequence_params()?;
        drop(tmp_inner);
        let sample_entry = SharedSampleEntry::new(h265::h265_sample_entry_from_annexb(
            &seq_params,
            options.frame_rate,
        )?);

        let av1_sequence_header = Arc::new(Vec::new());
        let handler = build_handler(
            sink,
            input_queue.clone(),
            sample_entry.clone(),
            VideoFormat::H265,
            av1_sequence_header.clone(),
        );
        let inner = shiguredo_nvcodec::Encoder::new(config, handler)?;

        Ok(Self {
            inner,
            input_queue,
            sample_entry,
            encoded_format: VideoFormat::H265,
            av1_sequence_header,
            force_keyframe_next: false,
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

        let input_queue: SharedInputQueue = Arc::new(Mutex::new(VecDeque::new()));
        let (tmp_handler, _tmp_input_queue) = build_tmp_handler_for_seq_params_probe();
        let tmp_inner = shiguredo_nvcodec::Encoder::new(config.clone(), tmp_handler)?;

        // NVENC SDK 13.0 のドキュメント (https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html#retrieving-sequence-parameters)
        // には以下の記載がある:
        //   "By default, SPS/PPS and Sequence Header OBU data will be attached to every IDR frame and Key frame for H.264/HEVC and AV1 respectively."
        //
        // しかし実際には、AV1の場合、最初のキーフレームにのみ Sequence Header OBU が付与され、
        // 二番目以降のキーフレームには含まれない。これにより、二番目以降のキーフレームからシークすると、
        // デコーダが解像度やプロファイルなどの情報を取得できず、映像が再生できない問題が発生する。
        //
        // そのため、ここで Sequence Header OBU を get_sequence_params() で取得して保持しておき、
        // キーフレームのエンコード時に Sequence Header が含まれていない場合は、
        // hisui 側で明示的に付与するワークアラウンドを実装している。
        let seq_params = tmp_inner.get_sequence_params()?;
        drop(tmp_inner);

        let sample_entry =
            SharedSampleEntry::new(av1::av1_sample_entry(width, height, &seq_params));

        let av1_sequence_header = Arc::new(seq_params);
        let handler = build_handler(
            sink,
            input_queue.clone(),
            sample_entry.clone(),
            VideoFormat::Av1,
            av1_sequence_header.clone(),
        );
        let inner = shiguredo_nvcodec::Encoder::new(config, handler)?;

        Ok(Self {
            inner,
            input_queue,
            sample_entry,
            encoded_format: VideoFormat::Av1,
            av1_sequence_header,
            force_keyframe_next: false,
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

        // 順序保証: callback で pop する前に必ず push_back する。
        // flush() は callback 完了までブロックするため、push が先行することが担保される。
        {
            let mut queue = self
                .input_queue
                .lock()
                .expect("nvcodec input queue lock poisoned");
            queue.push_back(video_frame.to_stripped());
        }

        // エンコード実行
        let encode_options = shiguredo_nvcodec::EncodeOptions {
            force_intra: self.force_keyframe_next,
            force_idr: self.force_keyframe_next,
            output_spspps: false,
        };
        self.force_keyframe_next = false;
        self.inner.encode(&nv12_data, &encode_options, ())?;
        // shiguredo_nvcodec のエンコーダーは内部の worker スレッドで非同期にエンコードし、
        // encode() は即時 return する。上位パイプラインは同期 pull 型で、上位側でペース制御
        // しないと内部キューが溢れて encode() が "encoder buffer is full" で失敗するため、
        // 投入直後に flush() で 1 フレーム分の完了を待って同期動作させる。
        self.inner.flush()?;
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

/// sample entry を作るためだけに sequence params を取り出す用途の一時 handler。
/// callback は空 (Ok も Err も無視) で、 encoder を drop するまで呼ばれない前提。
fn build_tmp_handler_for_seq_params_probe() -> (
    shiguredo_nvcodec::FnEncodeHandler<(), shiguredo_nvcodec::Error>,
    (),
) {
    let handler = shiguredo_nvcodec::FnEncodeHandler::new(move |_result| {
        // 一時的な probe 用 handler なので何もしない
    });
    (handler, ())
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
