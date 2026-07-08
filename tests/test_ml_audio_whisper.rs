//! Whisper 実推論と TranscriptionProcessor の integration テスト。
//!
//! 実音声 fixture の出所:
//! - `testdata/speech-en-16k-mono-s16le.pcm`
//!   - Mozilla Common Voice (CC0) clip ID: `common_voice_en_100540`
//! - `testdata/speech-ja-16k-mono-s16le.pcm`
//!   - Mozilla Common Voice (CC0) clip ID: `common_voice_ja_19486650`
//!
//! 変換手順:
//! - `ffmpeg -i INPUT.mp3 -ac 1 -ar 16000 -f s16le OUTPUT.pcm`

#![cfg(feature = "candle")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use candle_core::Device;
use hisui::audio::{AudioFormat, AudioFrame, Channels, SampleRate};
use hisui::ml::audio::silero_vad::SileroVadModel;
use hisui::ml::audio::transcription_processor::TranscriptionProcessor;
use hisui::ml::audio::transcription_service::TranscriptionService;
use hisui::ml::audio::whisper::WhisperPipeline;
use hisui::{
    MediaPipeline, Message, ProcessorHandle, ProcessorId, ProcessorMetadata, TextFrame, TrackId,
};

const INPUT_TRACK_ID: &str = "transcription_input";
const OUTPUT_TRACK_ID: &str = "transcription_output";
const TARGET_SAMPLE_RATE: usize = 16_000;

/// 各パイプラインタスクの完了待ちの上限。CPU 実推論は 1 チャンクあたり数十秒かかり得るため、
/// 遅い CI ランナーでも誤ってタイムアウトしないよう十分な余裕を持たせる。
const TASK_TIMEOUT: Duration = Duration::from_secs(300);

/// モデル配置ディレクトリを環境変数から解決する。未設定なら `ml-models` を返す。
fn ml_models_dir() -> PathBuf {
    std::env::var("HISUI_ML_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ml-models"))
}

/// Whisper モデルディレクトリを返す。ファイル不在なら skip する。
fn resolve_whisper_model_dir_or_skip(test_name: &str) -> Option<PathBuf> {
    let path = ml_models_dir().join("whisper-tiny");
    if path.join("config.json").is_file()
        && path.join("tokenizer.json").is_file()
        && path.join("model.safetensors").is_file()
    {
        return Some(path);
    }
    if std::env::var("HISUI_CI").as_deref() == Ok("1") {
        panic!(
            "HISUI_CI=1 だが Whisper モデルが見つからない: {} (test={test_name})",
            path.display()
        );
    }
    println!(
        "skip {test_name}: Whisper モデルが見つからない (HISUI_ML_MODELS_DIR={:?}, 解決先={})",
        ml_models_dir(),
        path.display()
    );
    None
}

/// Silero VAD モデルファイルを返す。ファイル不在なら skip する。
fn resolve_silero_model_path_or_skip(test_name: &str) -> Option<PathBuf> {
    let path = ml_models_dir().join("silero-vad/onnx/model.onnx");
    if path.is_file() {
        return Some(path);
    }
    if std::env::var("HISUI_CI").as_deref() == Ok("1") {
        panic!(
            "HISUI_CI=1 だが Silero VAD モデルが見つからない: {} (test={test_name})",
            path.display()
        );
    }
    println!(
        "skip {test_name}: Silero VAD モデルが見つからない (HISUI_ML_MODELS_DIR={:?}, 解決先={})",
        ml_models_dir(),
        path.display()
    );
    None
}

/// raw PCM (s16le mono 16 kHz) を f32 へ変換する。
fn load_pcm16le_mono_f32(path: &str) -> hisui::Result<Vec<f32>> {
    let bytes = std::fs::read(path)?;
    if !bytes.len().is_multiple_of(2) {
        return Err(hisui::Error::new(format!(
            "PCM fixture must have even byte length: {path}"
        )));
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0)
        .collect())
}

/// raw PCM (s16le mono 16 kHz) を I16Be AudioFrame 群へ分割する。
fn load_pcm16le_mono_audio_frames(
    path: &str,
    samples_per_frame: usize,
) -> hisui::Result<Vec<AudioFrame>> {
    let bytes = std::fs::read(path)?;
    if !bytes.len().is_multiple_of(2) {
        return Err(hisui::Error::new(format!(
            "PCM fixture must have even byte length: {path}"
        )));
    }

    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    let sample_rate = SampleRate::from_u32(16_000).expect("16 kHz は有効");

    Ok(samples
        .chunks(samples_per_frame)
        .enumerate()
        .map(|(index, chunk)| AudioFrame {
            data: chunk
                .iter()
                .flat_map(|sample| sample.to_be_bytes())
                .collect(),
            format: AudioFormat::I16Be,
            channels: Channels::MONO,
            sample_rate,
            timestamp: sample_rate.duration_from_samples((index * samples_per_frame) as u64),
            sample_entry: None,
        })
        .collect())
}

/// ひらがな・カタカナ・CJK 統合漢字のいずれかなら true。
///
/// 日本語 fixture の文字起こしが日本語表記になっているかを緩く検証するためのヘルパー。厳密な文字列
/// 一致はモデル・浮動小数点の環境差で脆いため、表記の種類だけを見る。
fn is_japanese_char(c: char) -> bool {
    let code = c as u32;
    (0x3040..=0x309F).contains(&code) // ひらがな
        || (0x30A0..=0x30FF).contains(&code) // カタカナ
        || (0x4E00..=0x9FFF).contains(&code) // CJK 統合漢字
}

#[test]
fn whisper_pipeline_transcribes_english_fixture() {
    let Some(model_dir) =
        resolve_whisper_model_dir_or_skip("whisper_pipeline_transcribes_english_fixture")
    else {
        return;
    };
    let pcm = load_pcm16le_mono_f32("testdata/speech-en-16k-mono-s16le.pcm")
        .expect("英語 PCM fixture を読めること");
    let mut pipeline = WhisperPipeline::load(&model_dir, Device::Cpu).expect("Whisper ロード");

    let result = pipeline
        .transcribe_pcm16k(&pcm, "en")
        .expect("Whisper 推論は成功する想定");

    assert!(!result.text.is_empty(), "文字起こし結果は空でないこと");
    // 厳密な文字列一致はモデル・環境依存で脆いため、英語表記の内容が得られたことだけを緩く検証する。
    let ascii_letters = result
        .text
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .count();
    assert!(
        ascii_letters >= 3,
        "英語 fixture の文字起こしは英字を十分に含むこと: {}",
        result.text
    );
    assert!(
        result.no_speech_prob < 0.5,
        "発話 fixture なので no_speech_prob は 0.5 未満の想定: {}",
        result.no_speech_prob
    );
    assert!(
        result.avg_logprob > -1.5,
        "avg_logprob は極端に低くない想定: {}",
        result.avg_logprob
    );
    assert_eq!(result.language.as_deref(), Some("en"));
}

#[test]
fn whisper_pipeline_transcribes_japanese_fixture() {
    let Some(model_dir) =
        resolve_whisper_model_dir_or_skip("whisper_pipeline_transcribes_japanese_fixture")
    else {
        return;
    };
    let pcm = load_pcm16le_mono_f32("testdata/speech-ja-16k-mono-s16le.pcm")
        .expect("日本語 PCM fixture を読めること");
    let mut pipeline = WhisperPipeline::load(&model_dir, Device::Cpu).expect("Whisper ロード");

    let result = pipeline
        .transcribe_pcm16k(&pcm, "ja")
        .expect("Whisper 推論は成功する想定");

    assert!(!result.text.is_empty(), "文字起こし結果は空でないこと");
    // 厳密な文字列一致はモデル・環境依存で脆いため、日本語表記の内容が得られたことだけを緩く検証する。
    let japanese_chars = result.text.chars().filter(|c| is_japanese_char(*c)).count();
    assert!(
        japanese_chars >= 3,
        "日本語 fixture の文字起こしは日本語文字を十分に含むこと: {}",
        result.text
    );
    assert!(
        result.no_speech_prob < 0.5,
        "発話 fixture なので no_speech_prob は 0.5 未満の想定: {}",
        result.no_speech_prob
    );
    assert!(
        result.avg_logprob > -1.5,
        "avg_logprob は極端に低くない想定: {}",
        result.avg_logprob
    );
    assert_eq!(result.language.as_deref(), Some("ja"));
}

/// 不在ディレクトリでは TranscriptionService::new が Err を返す。
#[tokio::test(flavor = "current_thread")]
async fn transcription_service_returns_err_for_missing_model_dir() {
    let err = match TranscriptionService::new(Path::new("/nonexistent/whisper-model"), Device::Cpu)
    {
        Ok(_) => panic!("不在モデルディレクトリは Err を返す想定"),
        Err(err) => err,
    };
    let message = err.display();
    assert!(
        message.contains("missing") || message.contains("model directory"),
        "エラーメッセージにモデル不在を含むこと: {message}"
    );
}

/// source -> TranscriptionProcessor -> subscriber の in-process pipeline で TextFrame を受信できる。
#[tokio::test(flavor = "current_thread")]
async fn transcription_processor_publishes_text_frames() -> hisui::Result<()> {
    let Some(whisper_model_dir) =
        resolve_whisper_model_dir_or_skip("transcription_processor_publishes_text_frames")
    else {
        return Ok(());
    };
    let Some(silero_model_path) =
        resolve_silero_model_path_or_skip("transcription_processor_publishes_text_frames")
    else {
        return Ok(());
    };

    let silero = SileroVadModel::load(&silero_model_path, Device::Cpu).expect("Silero ロード");
    let service = Arc::new(TranscriptionService::new(&whisper_model_dir, Device::Cpu)?);
    let mut input_frames =
        load_pcm16le_mono_audio_frames("testdata/speech-en-16k-mono-s16le.pcm", 4000)?;
    input_frames.truncate(2 * TARGET_SAMPLE_RATE / 4000);

    let pipeline = MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let mut pipeline_task = tokio::spawn(async move {
        pipeline.run().await;
    });

    let source_handle = register_processor(
        &pipeline_handle,
        ProcessorId::new("transcription_test_source"),
        ProcessorMetadata::new("test_source"),
    )
    .await?;
    let processor_handle = register_processor(
        &pipeline_handle,
        ProcessorId::new("transcription_test_processor"),
        ProcessorMetadata::new("transcription"),
    )
    .await?;
    let sink_handle = register_processor(
        &pipeline_handle,
        ProcessorId::new("transcription_test_sink"),
        ProcessorMetadata::new("test_sink"),
    )
    .await?;

    let source_task = tokio::spawn(run_audio_source(
        source_handle,
        input_frames,
        TrackId::new(INPUT_TRACK_ID),
    ));
    let processor_task = tokio::spawn(
        TranscriptionProcessor::new(
            Arc::clone(&service),
            silero,
            "en".to_owned(),
            TrackId::new(INPUT_TRACK_ID),
            TrackId::new(OUTPUT_TRACK_ID),
        )
        .run(processor_handle),
    );
    drop(service);
    let sink_task = tokio::spawn(collect_text_frames(
        sink_handle,
        TrackId::new(OUTPUT_TRACK_ID),
    ));

    pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| hisui::Error::new("failed to trigger start: pipeline has terminated"))?;

    tokio::time::timeout(TASK_TIMEOUT, source_task)
        .await
        .expect("source_task がタイムアウトした")??;
    tokio::time::timeout(TASK_TIMEOUT, processor_task)
        .await
        .expect("processor_task がタイムアウトした")??;
    let text_frames = tokio::time::timeout(TASK_TIMEOUT, sink_task)
        .await
        .expect("sink_task がタイムアウトした")??;

    assert!(
        !text_frames.is_empty(),
        "少なくとも 1 つの TextFrame が publish されるはず"
    );
    assert!(
        text_frames
            .iter()
            .any(|frame| !frame.text.trim().is_empty()),
        "空でない TextFrame が少なくとも 1 つあるはず: {:?}",
        text_frames
            .iter()
            .map(|frame| frame.text.as_str())
            .collect::<Vec<_>>()
    );

    drop(pipeline_handle);
    match tokio::time::timeout(Duration::from_secs(5), &mut pipeline_task).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(hisui::Error::new(format!("pipeline task failed: {e}"))),
        Err(_) => {
            pipeline_task.abort();
            let _ = pipeline_task.await;
        }
    }
    Ok(())
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

async fn run_audio_source(
    handle: ProcessorHandle,
    frames: Vec<AudioFrame>,
    track_id: TrackId,
) -> hisui::Result<()> {
    let mut tx = handle.publish_track(track_id).await?;
    handle.notify_ready();
    handle.wait_subscribers_ready().await?;
    for frame in frames {
        if !tx.send_audio(frame) {
            break;
        }
    }
    tx.send_eos();
    Ok(())
}

async fn collect_text_frames(
    handle: ProcessorHandle,
    track_id: TrackId,
) -> hisui::Result<Vec<TextFrame>> {
    let mut rx = handle.subscribe_track(track_id);
    handle.notify_ready();
    let mut frames = Vec::new();
    loop {
        match rx.recv().await {
            Message::Media(sample) => {
                let frame = sample.expect_text()?;
                frames.push((*frame).clone());
            }
            Message::Eos => break,
            Message::Syn(_) => {}
        }
    }
    Ok(frames)
}
