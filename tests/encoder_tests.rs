use std::num::NonZeroUsize;
use std::time::Duration;

use hisui::{
    MediaFrame, MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, TrackId,
    VideoFrame,
    encoder::{
        AsyncVideoEncoder, EncoderRunOutput, VideoEncoder, VideoEncoderOptions,
        default_video_encode_config_for_rpc,
    },
    types::{CodecName, EvenUsize},
    video::{FrameRate, VideoFormat, VideoFrameSize},
};

const VIDEO_INPUT_TRACK_ID: &str = "encoder_test_video_input";
const VIDEO_OUTPUT_TRACK_ID: &str = "encoder_test_video_output";

// integration test 用の VP8 エンコーダーオプション。
// libvpx VP8 は feature gate なし・OPENH264_PATH 等の環境変数も不要なので、
// すべての CI (test-fdk-aac / test-nvidia-video-codec / test-candle / ci 等) で走る。
fn vp8_options() -> VideoEncoderOptions {
    VideoEncoderOptions {
        codec: CodecName::Vp8,
        engines: None,
        bitrate: 100_000,
        width: EvenUsize::truncating_new(64),
        height: EvenUsize::truncating_new(64),
        frame_rate: FrameRate {
            numerator: NonZeroUsize::MIN.saturating_add(29),
            denumerator: NonZeroUsize::MIN,
        },
        encode_params: default_video_encode_config_for_rpc(),
    }
}

// 64x64 の I420 グレーフレームを作る (src/encoder/test_helpers.rs::raw_i420_frame と等価)。
// tests/ 側は crate 外部の integration test として実行されるため pub(crate) helper に
// アクセスできず、 モック禁止規約に沿って同等の入力データを再構築する。
fn i420_video_frame(ts_ms: u64) -> VideoFrame {
    let (width, height) = (64usize, 64usize);
    let y_size = width * height;
    let uv_size = (width / 2) * (height / 2);
    let data: Vec<u8> = std::iter::repeat_n(16u8, y_size)
        .chain(std::iter::repeat_n(128u8, uv_size * 2))
        .collect();
    VideoFrame {
        data,
        format: VideoFormat::I420,
        keyframe: true,
        size: Some(VideoFrameSize { width, height }),
        timestamp: Duration::from_millis(ts_ms),
        sample_entry: None,
    }
}

/// `VideoEncoder` の公開 API (`handle_input_sample` / `poll_output`) の end-to-end 契約
///
/// 実 I420 入力 → 内部の VideoEncoderInner → `OutputSink::emit_ok` → 内部チャンネル
/// → `poll_output::try_recv` が公開 API 呼び出しだけで踏破可能なことを確認する。
/// libvpx VP8 経路は feature gate なしで全 CI で走るため、 e2e 契約を最小コストで固定する。
#[test]
fn video_encoder_poll_output_returns_processed() -> hisui::Result<()> {
    let options = vp8_options();
    let stats = hisui::stats::Stats::new();
    let mut encoder = VideoEncoder::new(&options, None, stats)?;

    // 同期入力: `VideoEncoder::handle_input_sample` で内部 encoder に 1 フレーム流す
    encoder.handle_input_sample(Some(MediaFrame::video(i420_video_frame(0))))?;
    // EOS で inner.finish() 経由でフラッシュを踏ませ、 未出力フレームを吐き出させる。
    // 同期経路 (libvpx VP8) では即時 1:1 出力なのでフラッシュは形式上のもの。
    encoder.handle_input_sample(None)?;

    // 同期取り出し: `VideoEncoder::poll_output` で内部チャンネルから 1 件受信する
    let output = encoder.poll_output()?;
    let EncoderRunOutput::Processed(sample) = output else {
        panic!("実 I420 入力から Processed を期待した (VideoEncoder::poll_output 経由)");
    };
    let frame = sample.expect_video()?;
    let size = frame.size().expect("出力フレームは size を持つはず");
    // 入力解像度と一致していることで、 同期経路 (`handle_input_sample` + `poll_output`) を
    // 通ったフレームが解像度を保ったまま流れていることを確認する。
    assert_eq!(size.width, 64, "入力解像度と一致するはず");
    assert_eq!(size.height, 64, "入力解像度と一致するはず");
    Ok(())
}

/// メトリクス二重計上禁止の回帰検出: N フレーム入力 → `total_input_video_frame_count` が
/// N 増分、 N フレーム出力 → `total_output_video_frame_count` が N 増分されることを確認する
///
/// `OutputSink` が送信とカウンター増分を物理的に強制ペアリングする契約が
/// 「emit_ok 経路で `add(2)` 等の二重計上が混入しても検出されない」状態にならないよう、
/// 量的検証を `VideoEncoder::handle_input_sample` / `poll_output` の end-to-end で担保する。
///
/// N=1 では「1 呼び出しで k 倍増分」型 (`add(k)` 直接乗算) の混入は検出できるが、
/// 「k フレーム目だけ倍増する」「(N-1) フレーム目までは正しく (N) フレーム目で +2」等の
/// 累積 off-by-one 系バグは検出範囲外になるため、 複数フレーム (N=3) で assert する。
#[test]
fn video_encoder_metrics_increment_by_input_count() -> hisui::Result<()> {
    const FRAME_COUNT: u64 = 3;

    // メトリクスのハンドルを先に取得しておく
    // (Stats::counter は同名に対して同一の Arc を返す挙動を利用して、
    //  ここで取ったカウンターとエンコーダー内部のカウンターが同じ Arc を共有する)
    let mut stats = hisui::stats::Stats::new();
    let total_input = stats.counter("total_input_video_frame_count");
    let total_output = stats.counter("total_output_video_frame_count");

    let options = vp8_options();
    let mut encoder = VideoEncoder::new(&options, None, stats)?;

    // N フレーム入力 → handle_input_sample → 内部 encoder → sink.emit_ok まで実行する
    for i in 0..FRAME_COUNT {
        encoder.handle_input_sample(Some(MediaFrame::video(i420_video_frame(i * 33))))?;
    }
    encoder.handle_input_sample(None)?;

    // 全出力を `VideoEncoder::poll_output` で取り出す
    let mut processed = 0u64;
    loop {
        match encoder.poll_output()? {
            EncoderRunOutput::Processed(_) => processed += 1,
            EncoderRunOutput::Finished => break,
            EncoderRunOutput::Pending => panic!("EOS 後に Pending は想定外 (フラッシュ済み前提)"),
        }
    }
    assert_eq!(
        processed, FRAME_COUNT,
        "libvpx VP8 は 1:1 出力なので入力と同数のフレームが出力されるはず (N={FRAME_COUNT})"
    );

    // 二重計上禁止契約: 入力 N フレーム = total_input を N 増分 (add(k) 混入等を検出)
    assert_eq!(
        total_input.get(),
        FRAME_COUNT,
        "N フレーム入力で total_input_video_frame_count が N 増分されるはず (二重計上禁止)"
    );
    // 二重計上禁止契約: 出力 N フレーム = total_output を N 増分 (OutputSink::emit_ok 経由で物理ペアリング)
    assert_eq!(
        total_output.get(),
        FRAME_COUNT,
        "N フレーム出力で total_output_video_frame_count が N 増分されるはず (二重計上禁止)"
    );

    Ok(())
}

/// keyframe metric の量的検証: N フレーム入力のうち、
/// libvpx VP8 が keyframe として出力するフレーム数だけ
/// `total_output_video_keyframe_count` が inc される。
///
/// libvpx VP8 は force_keyframe 指定なしでも最初のフレームは強制的に keyframe になる契約で、
/// N=3 の短時間入力では以降のフレームは非 keyframe (P frame) となる。
/// この非対称性を利用して「常に両カウンタを inc する」バグ (R-2 の核心) を e2e で担保する。
///
/// R-2 は `OutputSink` 単体の unit test で、 本テストは encoder 全体の integration test。
/// unit / integration の両側で対称に固めることで、 中間層の変更で片方だけ緑になる状況を防ぐ。
#[test]
fn video_encoder_keyframe_metric_increments_only_for_keyframes() -> hisui::Result<()> {
    const FRAME_COUNT: u64 = 3;

    let mut stats = hisui::stats::Stats::new();
    let total_output = stats.counter("total_output_video_frame_count");
    let total_keyframe = stats.counter("total_output_video_keyframe_count");

    let options = vp8_options();
    let mut encoder = VideoEncoder::new(&options, None, stats)?;

    for i in 0..FRAME_COUNT {
        encoder.handle_input_sample(Some(MediaFrame::video(i420_video_frame(i * 33))))?;
    }
    encoder.handle_input_sample(None)?;

    // 全出力を drain (テストは keyframe metric の量的挙動に集中する)
    loop {
        match encoder.poll_output()? {
            EncoderRunOutput::Processed(_) => {}
            EncoderRunOutput::Finished => break,
            EncoderRunOutput::Pending => panic!("EOS 後に Pending は想定外"),
        }
    }

    assert_eq!(
        total_output.get(),
        FRAME_COUNT,
        "libvpx VP8 は 1:1 出力なので N={FRAME_COUNT} フレーム出力されるはず"
    );
    // libvpx VP8 は最初の 1 フレームのみ keyframe (I frame)、それ以降は P frame の想定。
    // 「常に両カウンタを inc する」バグが混入すれば total_keyframe = FRAME_COUNT になり検出できる。
    assert_eq!(
        total_keyframe.get(),
        1,
        "libvpx VP8 は initial frame のみ keyframe になる契約 (N={FRAME_COUNT} 中 1 件のみ)"
    );

    Ok(())
}

/// `AsyncVideoEncoder::run` (processor 経路) の end-to-end 契約
///
/// source → `AsyncVideoEncoder::run` → sink の 3 processor async pipeline を組み、
/// 実 I420 入力を VP8 に圧縮した出力が sink に届くことを確認する。
/// 使用側 (`recording_subcommand_compose.rs:577` 等) と同じ `spawn_processor` 経路の
/// 挙動を最低 1 経路担保する。
#[test]
fn video_encoder_run_processes_i420_via_async_pipeline() -> hisui::Result<()> {
    const FRAME_COUNT: u64 = 3;
    let input_frames: Vec<VideoFrame> =
        (0..FRAME_COUNT).map(|i| i420_video_frame(i * 33)).collect();
    let output_frames = encode_video_frames_with_async_pipeline(input_frames, vp8_options())?;
    assert_eq!(
        output_frames.len() as u64,
        FRAME_COUNT,
        "libvpx VP8 は 1:1 出力なので入力と同数のフレームが出力されるはず (N={FRAME_COUNT})"
    );
    for frame in &output_frames {
        assert_eq!(
            frame.format,
            VideoFormat::Vp8,
            "出力フレームは VP8 形式であるべき"
        );
    }
    assert!(
        output_frames[0].keyframe,
        "最初の出力フレームは keyframe (I frame) であるべき"
    );
    Ok(())
}

fn encode_video_frames_with_async_pipeline(
    input_frames: Vec<VideoFrame>,
    options: VideoEncoderOptions,
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
            ProcessorId::new("encoder_test_video_source"),
            ProcessorMetadata::new("encoder_test_video_source"),
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

        let encoder_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("encoder_test_video_encoder"),
            ProcessorMetadata::new("video_encoder"),
        )
        .await?;
        let encoder_task = tokio::spawn(async move {
            let encoder = AsyncVideoEncoder::new(&options, None, encoder_handle.stats())?;
            encoder
                .run(
                    encoder_handle,
                    TrackId::new(VIDEO_INPUT_TRACK_ID),
                    TrackId::new(VIDEO_OUTPUT_TRACK_ID),
                )
                .await
        });

        let sink_handle = register_processor(
            &pipeline_handle,
            ProcessorId::new("encoder_test_video_sink"),
            ProcessorMetadata::new("encoder_test_video_sink"),
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
            encoder_task,
            sink_task,
            pipeline_handle,
            &mut pipeline_task,
        )
        .await?;
        Ok(output_frames)
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

async fn await_video_pipeline_tasks(
    source_task: tokio::task::JoinHandle<hisui::Result<()>>,
    encoder_task: tokio::task::JoinHandle<hisui::Result<()>>,
    sink_task: tokio::task::JoinHandle<hisui::Result<Vec<VideoFrame>>>,
    pipeline_handle: hisui::MediaPipelineHandle,
    pipeline_task: &mut tokio::task::JoinHandle<()>,
) -> hisui::Result<Vec<VideoFrame>> {
    match source_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("source task join failed: {e}"))),
    }
    match encoder_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(hisui::Error::new(format!("encoder task join failed: {e}"))),
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
