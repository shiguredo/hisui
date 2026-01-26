use std::path::PathBuf;

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, VideoEncoder, VideoEncoderOptions},
    media::MediaStreamId,
    metadata::ContainerFormat,
    processor::RealtimePacer,
    reader::{AudioReader, VideoReader},
    scheduler::Scheduler,
    types::CodecName,
    video::FrameRate,
};

const AUDIO_ENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const VIDEO_ENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(1);
const AUDIO_DECODED_STREAM_ID: MediaStreamId = MediaStreamId::new(2);
const VIDEO_DECODED_STREAM_ID: MediaStreamId = MediaStreamId::new(3);
const AUDIO_REENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(4);
const VIDEO_REENCODED_STREAM_ID: MediaStreamId = MediaStreamId::new(5);
const AUDIO_PACED_STREAM_ID: MediaStreamId = MediaStreamId::new(6);
const VIDEO_PACED_STREAM_ID: MediaStreamId = MediaStreamId::new(7);

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let stream_name: Option<String> = noargs::opt("stream")
        .short('s')
        .doc("ストリーム名（省略時には RTMP_URL 引数にストリーム名が含まれるものとして扱われる）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let input_file_path: PathBuf = noargs::arg("INPUT_FILE")
        .doc("入力ファイル（.mp4 ないし .webm）")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let endpoint_rtmp_url = noargs::arg("RTMP_URL")
        .doc("出力先の RTMP の URL")
        .take(&mut args)
        .then(|a| {
            if let Some(stream) = &stream_name {
                shiguredo_rtmp::RtmpUrl::parse_with_stream_name(a.value(), stream)
            } else {
                shiguredo_rtmp::RtmpUrl::parse(a.value())
            }
        })?;
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let openh264_lib = openh264
        .as_ref()
        .and_then(|path| Openh264Library::load(path).ok());
    let format = ContainerFormat::from_path(&input_file_path).or_fail()?;
    let mut scheduler = Scheduler::new();
    let dummy_source_id = crate::metadata::SourceId::new("rtmp");

    // 音声リーダーを登録
    let reader = AudioReader::new(
        AUDIO_ENCODED_STREAM_ID,
        dummy_source_id.clone(),
        format,
        std::time::Duration::ZERO,
        vec![input_file_path.clone()],
    )
    .or_fail()?;
    scheduler.register(reader).or_fail()?;

    // 映像リーダーを登録
    let reader = VideoReader::new(
        VIDEO_ENCODED_STREAM_ID,
        dummy_source_id.clone(),
        format,
        std::time::Duration::ZERO,
        vec![input_file_path.clone()],
    )
    .or_fail()?;
    scheduler.register(reader).or_fail()?;

    // 音声デコーダーを登録
    let decoder =
        AudioDecoder::new_opus(AUDIO_ENCODED_STREAM_ID, AUDIO_DECODED_STREAM_ID).or_fail()?;
    scheduler.register(decoder).or_fail()?;

    // 映像デコーダーを登録
    let options = VideoDecoderOptions {
        openh264_lib: openh264_lib.clone(),
        decode_params: Default::default(),
        engines: None,
    };
    let decoder = VideoDecoder::new(VIDEO_ENCODED_STREAM_ID, VIDEO_DECODED_STREAM_ID, options);
    scheduler.register(decoder).or_fail()?;

    // 音声エンコーダー（AAC 固定）を登録
    let encoder = AudioEncoder::new(
        CodecName::Aac,
        std::num::NonZeroUsize::new(crate::audio::DEFAULT_BITRATE).or_fail()?,
        AUDIO_DECODED_STREAM_ID,
        AUDIO_REENCODED_STREAM_ID,
    )
    .or_fail()?;
    scheduler.register(encoder).or_fail()?;

    // 映像エンコーダー（H.264 固定）を登録
    let video_options = VideoEncoderOptions {
        codec: CodecName::H264,
        engines: None,
        bitrate: 1000000, // 1 Mbps （実験的コマンドなので固定値で十分）
        width: VideoEncoderOptions::DUMMY_WIDTH,
        height: VideoEncoderOptions::DUMMY_HEIGHT,
        frame_rate: FrameRate::FPS_25,
        encode_params: Default::default(),
    };
    let encoder = VideoEncoder::new(
        &video_options,
        VIDEO_DECODED_STREAM_ID,
        VIDEO_REENCODED_STREAM_ID,
        openh264_lib,
    )
    .or_fail()?;
    scheduler.register(encoder).or_fail()?;

    // リアルタイムペーサーを登録
    let pacer = RealtimePacer::new(
        vec![AUDIO_REENCODED_STREAM_ID, VIDEO_REENCODED_STREAM_ID],
        vec![AUDIO_PACED_STREAM_ID, VIDEO_PACED_STREAM_ID],
    )
    .or_fail()?;
    scheduler.register(pacer).or_fail()?;

    // RTMP パブリッシャーを登録
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .or_fail()?;
    let outbound_endpoint = crate::outbound_endpoint_rtmp::RtmpOutboundEndpoint::start(
        &runtime,
        Some(AUDIO_PACED_STREAM_ID),
        Some(VIDEO_PACED_STREAM_ID),
        endpoint_rtmp_url,
        crate::outbound_endpoint_rtmp::RtmpOutboundEndpointOptions::default(),
    );
    scheduler.register(outbound_endpoint).or_fail()?;

    // スケジューラー実行
    scheduler.run().or_fail()?;
    Ok(())
}
