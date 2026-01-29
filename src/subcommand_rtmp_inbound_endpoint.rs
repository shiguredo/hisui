use std::path::PathBuf;

use orfail::OrFail;

use crate::{media::MediaStreamId, scheduler::Scheduler};

const AUDIO_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const VIDEO_STREAM_ID: MediaStreamId = MediaStreamId::new(1);

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let stream_name: Option<String> = noargs::opt("stream")
        .short('s')
        .doc("ストリーム名（省略時には RTMP_URL 引数にストリーム名が含まれるものとして扱われる）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let _openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let _output_file_path: PathBuf = noargs::opt("output-file")
        .short('o')
        .doc("出力ファイル（.mp4 ないし .webm）")
        .default("output.mp4")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let endpoint_rtmp_url = noargs::arg("RTMP_URL")
        .doc("配信を受け付ける RTMP の URL")
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

    let mut scheduler = Scheduler::new();
    let _dummy_source_id = crate::metadata::SourceId::new("rtmp");

    // RTMP サーバーを登録
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .or_fail()?;
    let inbound_endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint::start(
        &runtime,
        Some(AUDIO_STREAM_ID),
        Some(VIDEO_STREAM_ID),
        endpoint_rtmp_url,
        crate::inbound_endpoint_rtmp::RtmpInboundEndpointOptions::default(),
    );
    scheduler.register(inbound_endpoint).or_fail()?;

    // スケジューラー実行
    scheduler.run().or_fail()?;
    Ok(())
}
