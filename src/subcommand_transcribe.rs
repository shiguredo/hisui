//! `hisui -x transcribe` 実験的サブコマンド。
//!
//! MP4 (音声のみの m4a を含む) を入力に取り、Whisper で文字起こしした結果を
//! 標準出力に JSON LINE (1 行 1 セグメント) で流す。 内部は 0062 で実装済みの
//! `TranscriptionService` / `TranscriptionProcessor` / `MediaFrame::Text` を組み合わせる。
//!
//! 実験的機能のため、`--experimental` (`-x`) グローバルフラグが立っている場合のみ有効。

use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use crate::Error;
use crate::decoder::AudioDecoder;
use crate::ml::audio::silero_vad::SileroVadModel;
use crate::ml::audio::transcription_processor::TranscriptionProcessor;
use crate::ml::audio::transcription_service::TranscriptionService;
use crate::ml::device::select_device;
use crate::mp4::sample_reader::{Mp4SampleReader, Mp4SampleReaderOptions};
use crate::text::LanguageCode;

/// MP4 reader が publish する encoded audio track の ID。
const AUDIO_ENCODED_TRACK_ID: &str = "audio_encoded";
/// AudioDecoder が publish する I16Be decoded audio track の ID。
const AUDIO_DECODED_TRACK_ID: &str = "audio_decoded";
/// TranscriptionProcessor が publish する text track の ID。
const TEXT_TRACK_ID: &str = "text";

pub fn try_run(
    args: &mut noargs::RawArgs,
    stats: crate::stats::Stats,
    experimental: bool,
) -> noargs::Result<bool> {
    if !noargs::cmd("transcribe")
        .doc("MP4 音声を Whisper で文字起こしします (実験的機能、--experimental (-x) が必須)")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }
    if !experimental && !args.metadata().help_mode {
        return Err(noargs::Error::other(
            args,
            "実験的機能です。 `--experimental` (`-x`) フラグを付けて起動してください",
        ));
    }
    run(args, stats)?;
    Ok(true)
}

fn run(args: &mut noargs::RawArgs, stats: crate::stats::Stats) -> noargs::Result<()> {
    let model_dir: PathBuf = noargs::opt("model-dir")
        .ty("PATH")
        .doc("Whisper モデルディレクトリ (config.json / tokenizer.json / model.safetensors を含む)")
        .take(args)
        .then(|a| a.value().parse())?;
    let silero_vad_model: PathBuf = noargs::opt("silero-vad-model")
        .ty("PATH")
        .env("HISUI_SILERO_VAD_MODEL_PATH")
        .doc("Silero VAD の ONNX モデルファイル (silero_vad.onnx)")
        .take(args)
        .then(|a| a.value().parse())?;
    let language: String = noargs::opt("language")
        .ty("CODE")
        .doc("Whisper 言語指定 (ISO 639-1、`ja` / `en` 等)")
        .take(args)
        .then(|a| a.value().parse())?;
    let transcribe_threads: Option<NonZeroUsize> = noargs::opt("transcribe-threads")
        .ty("N")
        .env("HISUI_TRANSCRIBE_THREADS")
        .doc(concat!(
            "1 推論あたりの candle rayon スレッド数を上書きします\n",
            "未指定なら既存の RAYON_NUM_THREADS を尊重し、それも無ければ論理コア数 (rayon の既定)"
        ))
        .take(args)
        .present_and_then(|a| a.value().parse())?;
    #[cfg(feature = "fdk-aac")]
    let fdk_aac: Option<PathBuf> = noargs::opt("fdk-aac")
        .ty("PATH")
        .env("HISUI_FDK_AAC_PATH")
        .doc("FDK-AAC の共有ライブラリのパス (AAC in MP4 対応、Linux では指定必須)")
        .take(args)
        .present_and_then(|a| a.value().parse())?;
    let input_file_path: PathBuf = noargs::arg("INPUT_FILE")
        .example("/path/to/speech.mp4")
        .doc("文字起こし対象の MP4 ファイル (.mp4 / .m4a、音声のみの m4a を含む)")
        .take(args)
        .then(|a| a.value().parse())?;

    if args.metadata().help_mode {
        return Ok(());
    }

    // `--transcribe-threads` (または env HISUI_TRANSCRIBE_THREADS) が指定された場合のみ
    // RAYON_NUM_THREADS を上書きする。 未指定なら既存の RAYON_NUM_THREADS を respect する
    // (未設定なら candle / rayon の既定 = 論理コア数)。
    if let Some(n) = transcribe_threads {
        // SAFETY: try_run の実行位置は candle / rayon global pool の初回構築より前。
        // 他スレッドは main.rs で logger init 済みだが env を触らないため、
        // set_var による他スレッドとの race は発生しない。
        unsafe {
            std::env::set_var("RAYON_NUM_THREADS", n.to_string());
        }
    }

    run_internal(
        input_file_path,
        model_dir,
        silero_vad_model,
        language,
        #[cfg(feature = "fdk-aac")]
        fdk_aac,
        stats,
    )
    .map_err(noargs::Error::from)?;
    Ok(())
}

fn run_internal(
    input_file_path: PathBuf,
    model_dir: PathBuf,
    silero_vad_model: PathBuf,
    language: String,
    #[cfg(feature = "fdk-aac")] fdk_aac: Option<PathBuf>,
    stats: crate::stats::Stats,
) -> crate::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| Error::new(e.to_string()))?;

    // `TranscriptionService::new` は内部で `spawn_blocking` を使うので tokio runtime 内で呼ぶ。
    // device は 1 回だけ選び (GPU 初期化のログを二重に出さない) 両者で共有する。
    let (service, silero) = runtime.block_on(async {
        let device = select_device();
        let service = Arc::new(TranscriptionService::new(&model_dir, device.clone())?);
        let silero = SileroVadModel::load(&silero_vad_model, device)?;
        crate::Result::Ok((service, silero))
    })?;

    let pipeline = crate::MediaPipeline::new(Default::default(), stats)?;
    let pipeline_handle = pipeline.handle();
    let language_code = LanguageCode::new(language);

    runtime.spawn(async move {
        if let Err(e) = setup_pipeline(
            pipeline_handle,
            input_file_path,
            #[cfg(feature = "fdk-aac")]
            fdk_aac,
            service,
            silero,
            language_code,
        )
        .await
        {
            tracing::error!("pipeline setup failed: {e:?}");
        }
    });

    let processor_failed = runtime.block_on(pipeline.run());
    if processor_failed {
        return Err(Error::new(
            "transcribe failed: one or more processors terminated abnormally",
        ));
    }
    Ok(())
}

async fn setup_pipeline(
    pipeline_handle: crate::MediaPipelineHandle,
    input_file_path: PathBuf,
    #[cfg(feature = "fdk-aac")] fdk_aac: Option<PathBuf>,
    service: Arc<TranscriptionService>,
    silero: Arc<SileroVadModel>,
    language: LanguageCode,
) -> crate::Result<()> {
    // MP4 reader (音声トラックのみ subscribe、video は不要)
    let reader = Mp4SampleReader::new(
        input_file_path,
        Mp4SampleReaderOptions {
            audio_track_id: Some(crate::TrackId::new(AUDIO_ENCODED_TRACK_ID)),
            video_track_id: None,
        },
    );
    pipeline_handle
        .spawn_processor(
            crate::ProcessorId::new("mp4_file_reader"),
            crate::ProcessorMetadata::new("mp4_file_reader"),
            |handle| reader.run(handle),
        )
        .await?;

    // 音声デコーダ (Opus / AAC → I16Be)
    #[cfg(feature = "fdk-aac")]
    let fdk_aac_lib = fdk_aac
        .map(shiguredo_fdk_aac::FdkAacLibrary::load)
        .transpose()?;

    let audio_decoder = AudioDecoder::new(
        #[cfg(feature = "fdk-aac")]
        fdk_aac_lib,
        crate::stats::Stats::new(),
    )?;
    pipeline_handle
        .spawn_processor(
            crate::ProcessorId::new("audio_decoder"),
            crate::ProcessorMetadata::new("audio_decoder"),
            |handle| {
                audio_decoder.run(
                    handle,
                    crate::TrackId::new(AUDIO_ENCODED_TRACK_ID),
                    crate::TrackId::new(AUDIO_DECODED_TRACK_ID),
                )
            },
        )
        .await?;

    // Whisper 文字起こし processor (0062 で実装済み)
    let transcription = TranscriptionProcessor::new(
        service,
        silero,
        language,
        crate::TrackId::new(AUDIO_DECODED_TRACK_ID),
        crate::TrackId::new(TEXT_TRACK_ID),
    );
    pipeline_handle
        .spawn_processor(
            crate::ProcessorId::new("transcription"),
            crate::ProcessorMetadata::new("transcription"),
            |handle| transcription.run(handle),
        )
        .await?;

    // text sink processor (JSON LINE を stdout に書き出す)
    pipeline_handle
        .spawn_processor(
            crate::ProcessorId::new("text_stdout_sink"),
            crate::ProcessorMetadata::new("text_stdout_sink"),
            text_stdout_sink,
        )
        .await?;

    pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;

    Ok(())
}

/// TextFrame を JSON LINE として stdout に流す text sink processor。
///
/// `StdoutLock` は `!Send` なので tokio task の await 越しに保持できない。
/// 1 メッセージ処理内で lock 取得 → writeln + flush → drop する。
/// stdout の pipe reader が閉じた (BrokenPipe) 場合は Err で早期終了し、
/// MediaPipeline に processor_failed を伝えて全体を停止させる。
async fn text_stdout_sink(handle: crate::ProcessorHandle) -> crate::Result<()> {
    let mut rx = handle.subscribe_track(crate::TrackId::new(TEXT_TRACK_ID));
    handle.notify_ready();

    loop {
        match rx.recv().await {
            crate::Message::Media(sample) => {
                let frame = sample.expect_text()?;
                let json = nojson::json(|f| {
                    f.set_indent_size(0);
                    f.value(&*frame)
                });
                // StdoutLock は !Send。 1 メッセージ処理内で lock 取得 → writeln → flush → drop。
                let mut stdout = std::io::stdout().lock();
                let write_result = writeln!(stdout, "{json}").and_then(|_| stdout.flush());
                if let Err(e) = write_result {
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        tracing::warn!("text_stdout_sink: stdout pipe closed, terminating");
                        return Err(crate::Error::new("text stdout pipe closed"));
                    }
                    return Err(crate::Error::new(format!("text_stdout_sink write: {e}")));
                }
            }
            crate::Message::Eos => break,
            crate::Message::Syn(_) => {}
        }
    }

    Ok(())
}
