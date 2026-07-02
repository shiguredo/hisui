use hisui::{
    MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, TrackId,
    decoder::{VideoDecoder, VideoDecoderOptions},
    sora::recording_mp4_reader::Mp4VideoReader,
    video::VideoFrame,
};
#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
use hisui::{audio::AudioFrame, decoder::AudioDecoder, sora::recording_mp4_reader::Mp4AudioReader};
use shiguredo_openh264::Openh264Library;

const VIDEO_INPUT_TRACK_ID: &str = "decoder_test_video_input";
const VIDEO_OUTPUT_TRACK_ID: &str = "decoder_test_video_output";
#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
const AUDIO_INPUT_TRACK_ID: &str = "decoder_test_audio_input";
#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
const AUDIO_OUTPUT_TRACK_ID: &str = "decoder_test_audio_output";

#[test]
fn h264_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-h264.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-h264.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

#[test]
#[cfg(target_os = "macos")]
fn h265_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-h265.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-h265.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

#[test]
fn vp9_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-vp9.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-vp9.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

#[test]
fn av1_multi_resolutions() -> hisui::Result<()> {
    let reader0 = Mp4VideoReader::new("testdata/archive-blue-640x480-av1.mp4")?;
    let reader1 = Mp4VideoReader::new("testdata/archive-red-320x320-av1.mp4")?;
    multi_resolutions_test(reader0, reader1)?;
    Ok(())
}

/// `AsyncVideoDecoder::next_decoded_frame_async` の非同期取り出し経路を実 VP9 フィクスチャで検証する
///
/// 検証対象パス: `AsyncVideoDecoder::handle_input_sample_sync` → `VideoDecoderInner::decode`
/// → `Initial` → `Libvpx` への遷移 → `LibvpxDecoder::decode` → `sink.emit_ok` → 内部チャンネル
/// → `rx.recv` → `next_decoded_frame_async` の全段を 1 フレームで踏破することを確認する。
/// `VideoDecoder` 経由の同期ラップ経路は別テスト (`_via_wrap_delegation`) が担当する。
#[tokio::test(flavor = "multi_thread")]
async fn async_video_decoder_processes_real_vp9_frame_via_next_decoded_frame_async()
-> hisui::Result<()> {
    use hisui::MediaFrame;
    use hisui::decoder::AsyncVideoDecoder;

    // 既存 `vp9_multi_resolutions` と同じフィクスチャを 1 フレームだけ使う
    let mut reader = Mp4VideoReader::new("testdata/archive-blue-640x480-vp9.mp4")?;
    let first_frame = reader
        .next()
        .expect("少なくとも 1 フレーム含まれているはず")?;

    let options = VideoDecoderOptions::default();
    let stats = hisui::stats::Stats::new();
    let mut decoder = AsyncVideoDecoder::new(options, stats);

    // 非同期経路: `handle_input_sample_sync` 経由で `inner.decode` → `sink.emit_ok` を実行する
    decoder.handle_input_sample_sync(Some(MediaFrame::video(first_frame)))?;
    // EOS で `inner.finish()` 経由を踏ませて未排出フレームをすべて吐かせる
    decoder.handle_input_sample_sync(None)?;

    // 非同期取り出し: `rx.recv` 経由で正常フレームを取得できることを確認する
    match decoder.next_decoded_frame_async().await {
        Some(Ok(frame)) => {
            let size = frame.size().expect("VP9 フィクスチャは size を持つはず");
            assert_eq!(size.width, 640, "フィクスチャ解像度と一致するはず");
            assert_eq!(size.height, 480, "フィクスチャ解像度と一致するはず");
        }
        other => panic!("正常フレーム (Some(Ok(_))) を期待したが {other:?} を受信した"),
    }
    Ok(())
}

/// `VideoDecoder::poll_output` の同期ラップ経路を実 VP9 フィクスチャで踏破する
///
/// 検証対象パス: `VideoDecoder::handle_input_sample` → `AsyncVideoDecoder::handle_input_sample_sync`
/// → `VideoDecoderInner::decode` → `sink.emit_ok` → 内部チャンネル →
/// `VideoDecoder::poll_output` → `AsyncVideoDecoder::poll_output_sync` の全段を実際に踏む。
/// wrap 側 (`VideoDecoder`) が同期 API をそのまま `AsyncVideoDecoder` に委譲していることを、
/// 上位 API 呼び出しだけで検出できるようにする回帰テスト。
#[test]
fn video_decoder_poll_output_returns_processed_via_wrap_delegation() -> hisui::Result<()> {
    use hisui::MediaFrame;
    use hisui::decoder::DecoderRunOutput;

    let mut reader = Mp4VideoReader::new("testdata/archive-blue-640x480-vp9.mp4")?;
    let first_frame = reader
        .next()
        .expect("少なくとも 1 フレーム含まれているはず")?;

    let options = VideoDecoderOptions::default();
    let stats = hisui::stats::Stats::new();
    let mut decoder = VideoDecoder::new(options, stats);

    // wrap 経路: `VideoDecoder::handle_input_sample` → 内部 `AsyncVideoDecoder` に委譲される
    decoder.handle_input_sample(Some(MediaFrame::video(first_frame)))?;
    // EOS で `inner.finish()` を経由してフラッシュを踏ませ、 非同期な内部デコーダーの
    // コールバック完了を待ち合わせる (`poll_output` が `Pending` を返さないようにするため)。
    decoder.handle_input_sample(None)?;

    // wrap 経路: `VideoDecoder::poll_output` → `AsyncVideoDecoder::poll_output_sync` に委譲
    let output = decoder.poll_output()?;
    assert!(
        matches!(output, DecoderRunOutput::Processed(_)),
        "実 VP9 フィクスチャから Processed を期待した (VideoDecoder::poll_output 経由)"
    );
    Ok(())
}

/// メトリクス二重計上禁止の回帰検出: N フレーム入力 → `total_input` が N 増分、
/// N フレーム出力 → `total_output` が N 増分されることを `VideoDecoder` の wrap 経路で確認する
///
/// `OutputSink` が送信と増分を物理的に強制ペアリングする契約が
/// 「emit_ok 経路でメトリクスの `add(2)` 等の二重計上が混入しても検出されない」状態にならないよう、
/// 量的検証を wrap 経路経由 (`VideoDecoder::handle_input_sample` / `poll_output`) の
/// end-to-end で担保する。
///
/// N=1 では「1 呼び出しで k 倍増分」型 (`add(k)` 直接乗算) の混入は検出できるが、
/// 「k フレーム目だけ倍増する」「(N-1) フレーム目までは正しく (N) フレーム目で +2」等の
/// 累積 off-by-one 系バグは検出範囲外になるため、 複数フレーム (N=3) で assert する。
#[test]
fn video_decoder_metrics_increment_by_input_count_via_wrap_delegation() -> hisui::Result<()> {
    use hisui::MediaFrame;
    use hisui::decoder::DecoderRunOutput;

    const FRAME_COUNT: u64 = 3;

    let mut reader = Mp4VideoReader::new("testdata/archive-blue-640x480-vp9.mp4")?;
    let frames: Vec<_> = (0..FRAME_COUNT)
        .map(|i| {
            reader.next().unwrap_or_else(|| {
                panic!("フレーム {i} が読めるはず (フィクスチャは十分な長さを持つ)")
            })
        })
        .collect::<hisui::Result<Vec<_>>>()?;

    // メトリクスのハンドルを先に取得しておく
    // (`Stats::counter` は同名に対して同一の `Arc` を返す `get_or_insert_entry` なので、
    //  ここで取ったカウンターとデコーダー内部のカウンターは同じ `Arc` を共有する)
    let mut stats = hisui::stats::Stats::new();
    let total_input = stats.counter("total_input_video_frame_count");
    let total_output = stats.counter("total_output_video_frame_count");

    let mut decoder = VideoDecoder::new(VideoDecoderOptions::default(), stats);

    // N フレーム入力 → wrap 経路 (`VideoDecoder::handle_input_sample`) 経由で
    // 内部 `AsyncVideoDecoder::handle_input_sample_sync` → `inner.decode` → `sink.emit_ok` まで実行する
    for frame in frames {
        decoder.handle_input_sample(Some(MediaFrame::video(frame)))?;
    }
    // EOS でフラッシュを踏ませて、 非同期な内部デコーダーのコールバック完了と
    // 未排出フレームの吐き出しを待ち合わせる
    decoder.handle_input_sample(None)?;

    // 全出力を wrap 経路 (`VideoDecoder::poll_output` → `AsyncVideoDecoder::poll_output_sync`) で取り出す
    let mut processed = 0u64;
    loop {
        match decoder.poll_output()? {
            DecoderRunOutput::Processed(_) => processed += 1,
            DecoderRunOutput::Finished => break,
            DecoderRunOutput::Pending => panic!("EOS 後に Pending は想定外 (フラッシュ済み前提)"),
        }
    }
    assert_eq!(
        processed, FRAME_COUNT,
        "入力と同数のフレームが出力されるはず (N={FRAME_COUNT})"
    );

    // 二重計上禁止契約: 入力 N フレーム = `total_input` を N 増分 (add(k) 混入等を検出)
    assert_eq!(
        total_input.get(),
        FRAME_COUNT,
        "N フレーム入力で total_input が N 増分されるはず (二重計上禁止)"
    );
    // 二重計上禁止契約: 出力 N フレーム = `total_output` を N 増分 (`OutputSink::emit_ok` 経由で物理ペアリング)
    assert_eq!(
        total_output.get(),
        FRAME_COUNT,
        "N フレーム出力で total_output が N 増分されるはず (二重計上禁止)"
    );

    Ok(())
}

fn multi_resolutions_test<I>(reader0: I, reader1: I) -> hisui::Result<()>
where
    I: Iterator<Item = hisui::Result<VideoFrame>>,
{
    let openh264_lib = if let Ok(path) = std::env::var("OPENH264_PATH") {
        Some(Openh264Library::load(path)?)
    } else {
        eprintln!("no available OpenH264 decoder");
        return Ok(());
    };
    let options = VideoDecoderOptions {
        openh264_lib,
        decode_params: Default::default(),
        engines: None,
    };

    // デコードする
    let mut output_frames = Vec::new();
    let mut blue_count = 0;
    let mut red_count = 0;
    let mut input_frames = Vec::new();

    for input_frame in reader0 {
        input_frames.push(input_frame?);
        blue_count += 1;
    }

    // このタイミングで解像度などが切り替わる
    for input_frame in reader1 {
        input_frames.push(input_frame?);
        red_count += 1;
    }
    output_frames.extend(decode_video_frames_with_pipeline(input_frames, options)?);

    // デコード結果を確認する
    for output_frame in output_frames {
        if blue_count > 0 {
            blue_count -= 1;
            let size = output_frame.size().expect("infallible");
            assert_eq!(size.width, 640);
            assert_eq!(size.height, 480);

            // 単色青色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 41));
            u_plane.iter().for_each(|&y| assert_eq!(y, 240));
            v_plane.iter().for_each(|&y| assert_eq!(y, 110));
        } else {
            red_count -= 1;
            let size = output_frame.size().expect("infallible");
            assert_eq!(size.width, 320);
            assert_eq!(size.height, 320);

            // 単色赤色かどうかのチェック
            let (y_plane, u_plane, v_plane) = output_frame
                .as_yuv_planes()
                .ok_or_else(|| hisui::Error::new("value is missing"))?;
            y_plane.iter().for_each(|&y| assert_eq!(y, 81));
            u_plane.iter().for_each(|&u| assert_eq!(u, 90));
            v_plane.iter().for_each(|&v| assert_eq!(v, 240));
        }
    }
    assert_eq!(blue_count, 0);
    assert_eq!(red_count, 0);

    Ok(())
}
#[test]
#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
fn aac_decode() -> hisui::Result<()> {
    let reader = Mp4AudioReader::new("testdata/beep-aac-audio.mp4")?;
    let mut input_samples = Vec::new();
    for input_data in reader {
        input_samples.push(input_data?);
    }
    let decoded_count = decode_audio_count_with_pipeline(input_samples)?;
    assert!(decoded_count > 0, "Should decode at least one audio frame");
    Ok(())
}

fn decode_video_frames_with_pipeline(
    input_frames: Vec<VideoFrame>,
    options: VideoDecoderOptions,
) -> hisui::Result<Vec<VideoFrame>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let pipeline = MediaPipeline::new(Default::default(), Default::default())?;
        let pipeline_handle = pipeline.handle();
        let mut pipeline_task = tokio::spawn(async move {
            pipeline.run().await;
        });

        let source_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_video_source"),
            ProcessorMetadata::new("decoder_test_video_source"),
        )
        .await?;
        let source_task = tokio::spawn(async move {
            run_video_source(
                source_handle,
                input_frames,
                TrackId::new(VIDEO_INPUT_TRACK_ID),
            )
            .await
        });

        let decoder_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_video_decoder"),
            ProcessorMetadata::new("video_decoder"),
        )
        .await?;
        let decoder_task = tokio::spawn(async move {
            let decoder = VideoDecoder::new(options, decoder_handle.stats());
            decoder
                .run(
                    decoder_handle,
                    TrackId::new(VIDEO_INPUT_TRACK_ID),
                    TrackId::new(VIDEO_OUTPUT_TRACK_ID),
                )
                .await
        });

        let sink_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_video_sink"),
            ProcessorMetadata::new("decoder_test_video_sink"),
        )
        .await?;
        let sink_task = tokio::spawn(async move {
            collect_video_frames(sink_handle, TrackId::new(VIDEO_OUTPUT_TRACK_ID)).await
        });

        pipeline_handle
            .trigger_start()
            .await
            .map_err(|_| hisui::Error::new("failed to trigger start: pipeline has terminated"))?;

        let output_frames = await_video_pipeline_tasks(
            source_task,
            decoder_task,
            sink_task,
            pipeline_handle,
            &mut pipeline_task,
        )
        .await?;
        Ok(output_frames)
    })
}

#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
fn decode_audio_count_with_pipeline(input_samples: Vec<AudioFrame>) -> hisui::Result<usize> {
    // FDK-AAC ライブラリを環境変数から読み込む（macOS の場合は不要）
    #[cfg(feature = "fdk-aac")]
    let fdk_aac_lib = if let Ok(path) = std::env::var("HISUI_FDK_AAC_PATH") {
        Some(shiguredo_fdk_aac::FdkAacLibrary::load(path)?)
    } else {
        None
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let pipeline = MediaPipeline::new(Default::default(), Default::default())?;
        let pipeline_handle = pipeline.handle();
        let mut pipeline_task = tokio::spawn(async move {
            pipeline.run().await;
        });

        let source_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_audio_source"),
            ProcessorMetadata::new("decoder_test_audio_source"),
        )
        .await?;
        let source_task = tokio::spawn(async move {
            run_audio_source(
                source_handle,
                input_samples,
                TrackId::new(AUDIO_INPUT_TRACK_ID),
            )
            .await
        });

        let decoder_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_audio_decoder"),
            ProcessorMetadata::new("audio_decoder"),
        )
        .await?;
        let decoder_task = tokio::spawn(async move {
            let decoder = AudioDecoder::new(
                #[cfg(feature = "fdk-aac")]
                fdk_aac_lib,
                decoder_handle.stats(),
            )?;
            decoder
                .run(
                    decoder_handle,
                    TrackId::new(AUDIO_INPUT_TRACK_ID),
                    TrackId::new(AUDIO_OUTPUT_TRACK_ID),
                )
                .await
        });

        let sink_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("decoder_test_audio_sink"),
            ProcessorMetadata::new("decoder_test_audio_sink"),
        )
        .await?;
        let sink_task = tokio::spawn(async move {
            collect_audio_count(sink_handle, TrackId::new(AUDIO_OUTPUT_TRACK_ID)).await
        });

        pipeline_handle
            .trigger_start()
            .await
            .map_err(|_| hisui::Error::new("failed to trigger start: pipeline has terminated"))?;

        let output_count = await_audio_pipeline_tasks(
            source_task,
            decoder_task,
            sink_task,
            pipeline_handle,
            &mut pipeline_task,
        )
        .await?;
        Ok(output_count)
    })
}

async fn register_processor(
    pipeline_handle: &hisui::MediaPipelineHandle,
    processor_id: ProcessorId,
    metadata: ProcessorMetadata,
) -> hisui::Result<ProcessorHandle> {
    pipeline_handle
        .register_processor(processor_id.clone(), metadata)
        .await
        .map_err(|e| match e {
            hisui::RegisterProcessorError::PipelineTerminated => {
                hisui::Error::new("failed to register processor: pipeline has terminated")
            }
            hisui::RegisterProcessorError::DuplicateProcessorId => hisui::Error::new(format!(
                "processor ID already exists: {}",
                processor_id.get()
            )),
        })
}

async fn run_video_source(
    handle: ProcessorHandle,
    frames: Vec<VideoFrame>,
    track_id: TrackId,
) -> hisui::Result<()> {
    let mut tx = handle.publish_track(track_id).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;
    for frame in frames {
        if !tx.send_video(frame) {
            break;
        }
    }
    tx.send_eos();
    Ok(())
}

#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
async fn run_audio_source(
    handle: ProcessorHandle,
    samples: Vec<AudioFrame>,
    track_id: TrackId,
) -> hisui::Result<()> {
    let mut tx = handle.publish_track(track_id).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;
    for sample in samples {
        if !tx.send_audio(sample) {
            break;
        }
    }
    tx.send_eos();
    Ok(())
}

async fn collect_video_frames(
    handle: ProcessorHandle,
    track_id: TrackId,
) -> hisui::Result<Vec<VideoFrame>> {
    let mut rx = handle.subscribe_track(track_id);
    handle.notify_ready();
    let mut frames = Vec::new();
    loop {
        match rx.recv().await {
            Message::Media(sample) => {
                let frame = sample.expect_video()?;
                frames.push((*frame).clone());
            }
            Message::Eos => break,
            Message::Syn(_) => {}
        }
    }
    Ok(frames)
}

#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
async fn collect_audio_count(handle: ProcessorHandle, track_id: TrackId) -> hisui::Result<usize> {
    let mut rx = handle.subscribe_track(track_id);
    handle.notify_ready();
    let mut count = 0usize;
    loop {
        match rx.recv().await {
            Message::Media(sample) => {
                let _data = sample.expect_audio()?;
                count += 1;
            }
            Message::Eos => break,
            Message::Syn(_) => {}
        }
    }
    Ok(count)
}

async fn await_video_pipeline_tasks(
    source_task: tokio::task::JoinHandle<hisui::Result<()>>,
    decoder_task: tokio::task::JoinHandle<hisui::Result<()>>,
    sink_task: tokio::task::JoinHandle<hisui::Result<Vec<VideoFrame>>>,
    pipeline_handle: hisui::MediaPipelineHandle,
    pipeline_task: &mut tokio::task::JoinHandle<()>,
) -> hisui::Result<Vec<VideoFrame>> {
    match source_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("source task join failed: {e}"))),
    }
    match decoder_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("decoder task join failed: {e}"))),
    }
    let output_frames = match sink_task.await {
        Ok(Ok(frames)) => frames,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("sink task join failed: {e}"))),
    };

    drop(pipeline_handle);
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut *pipeline_task).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(hisui::Error::new(format!(
                "media pipeline task failed: {e}"
            )));
        }
        Err(_) => {
            pipeline_task.abort();
            let _ = pipeline_task.await;
        }
    }

    Ok(output_frames)
}

#[cfg(any(target_os = "macos", feature = "fdk-aac"))]
async fn await_audio_pipeline_tasks(
    source_task: tokio::task::JoinHandle<hisui::Result<()>>,
    decoder_task: tokio::task::JoinHandle<hisui::Result<()>>,
    sink_task: tokio::task::JoinHandle<hisui::Result<usize>>,
    pipeline_handle: hisui::MediaPipelineHandle,
    pipeline_task: &mut tokio::task::JoinHandle<()>,
) -> hisui::Result<usize> {
    match source_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("source task join failed: {e}"))),
    }
    match decoder_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("decoder task join failed: {e}"))),
    }
    let output_count = match sink_task.await {
        Ok(Ok(count)) => count,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("sink task join failed: {e}"))),
    };

    drop(pipeline_handle);
    match tokio::time::timeout(std::time::Duration::from_secs(5), &mut *pipeline_task).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(hisui::Error::new(format!(
                "media pipeline task failed: {e}"
            )));
        }
        Err(_) => {
            pipeline_task.abort();
            let _ = pipeline_task.await;
        }
    }

    Ok(output_count)
}
