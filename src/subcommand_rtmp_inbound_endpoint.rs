use std::path::PathBuf;

use orfail::OrFail;

use crate::media::MediaStreamId;

const AUDIO_STREAM_ID: MediaStreamId = MediaStreamId::new(0);
const VIDEO_STREAM_ID: MediaStreamId = MediaStreamId::new(1);

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let stream_name: Option<String> = noargs::opt("stream")
        .short('s')
        .doc("ストリーム名（省略時には RTMP_URL 引数にストリーム名が含まれるものとして扱われる）")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let output_file_path: PathBuf = noargs::opt("output-file")
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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .or_fail()?;
    let _guard = runtime.enter();

    let pipeline = crate::MediaPipeline::new();
    let pipeline_handle = pipeline.handle();

    runtime.spawn(async move {
        let stream_name = endpoint_rtmp_url.stream_name.clone();

        // RTMP Inbound Endpoint を起動
        let endpoint = crate::inbound_endpoint_rtmp::RtmpInboundEndpoint::new(
            endpoint_rtmp_url,
            Default::default(),
        );
        pipeline_handle
            .spawn_processor(crate::ProcessorId::new("rtmp_inbound"), |handle| {
                endpoint.run(handle)
            })
            .await
            .or_fail()?;

        // MP4 Writer を起動
        let writer = crate::writer_mp4::Mp4Writer::new(
            output_file_path,
            None,
            Some(AUDIO_STREAM_ID),
            Some(VIDEO_STREAM_ID),
        )
        .or_fail()?;
        pipeline_handle
            .spawn_processor(crate::ProcessorId::new("mp4_writer"), move |handle| {
                writer.run(
                    handle,
                    Some(crate::TrackId::new(format!("{stream_name}_audio"))),
                    Some(crate::TrackId::new(format!("{stream_name}_video"))),
                )
            })
            .await
            .or_fail()?;

        Ok::<(), orfail::Failure>(())
    });

    runtime.block_on(pipeline.run());
    Ok(())
}
