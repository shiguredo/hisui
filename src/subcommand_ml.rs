use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

pub fn try_run(args: &mut noargs::RawArgs) -> noargs::Result<bool> {
    let cmd = noargs::cmd("ml")
        .doc("ML 推論デモ (video: 物体検知 / audio: 音声転写)")
        .take(args);

    if !cmd.is_present() {
        return Ok(false);
    }

    #[cfg(not(feature = "candle"))]
    {
        tracing::error!(
            "ml subcommand requires 'candle' feature. \
             Rebuild with: cargo build --features candle,player"
        );
        std::process::exit(1);
    }

    #[cfg(feature = "candle")]
    {
        let audio_cmd = noargs::cmd("audio")
            .doc("マイク入力を Whisper で文字起こしする")
            .take(args);
        if audio_cmd.is_present() {
            return try_run_audio(args);
        }

        #[cfg(not(feature = "player"))]
        {
            tracing::error!(
                "ml video requires 'player' feature. \
                 Use 'hisui ml audio' for speech-to-text, or rebuild with --features candle,player"
            );
            std::process::exit(1);
        }

        #[cfg(feature = "player")]
        {
            return try_run_video(args);
        }
    }

    #[cfg(not(feature = "candle"))]
    Ok(true)
}

#[cfg(all(feature = "candle", feature = "player"))]
fn try_run_video(args: &mut noargs::RawArgs) -> noargs::Result<bool> {
    {
        let list_devices = noargs::flag("list-devices")
            .doc("利用可能なビデオデバイス一覧を表示する")
            .take(args)
            .is_present();

        if list_devices {
            if args.metadata().help_mode {
                return Ok(true);
            }
            print_device_list();
            return Ok(true);
        }

        let model_path: PathBuf = noargs::opt("model-path")
            .ty("PATH")
            .doc(
                "safetensors モデルファイルのパス\n\
                 重みファイルは https://huggingface.co/lmz/candle-yolo-v8 から入手できます",
            )
            .take(args)
            .then(|o| o.value().parse())?;

        let mut device_ids: Vec<String> = Vec::new();
        loop {
            let result: Option<String> = noargs::opt("device-id")
                .ty("ID")
                .doc("ビデオデバイス ID (複数指定可能、省略時はデフォルトデバイス)")
                .take(args)
                .present_and_then(|a| a.value().parse())?;
            match result {
                Some(id) => device_ids.push(id),
                None => break,
            }
        }

        let width: u32 = noargs::opt("width")
            .ty("PX")
            .doc("キャプチャ幅")
            .default("320")
            .take(args)
            .then(|o| o.value().parse())?;

        let height: u32 = noargs::opt("height")
            .ty("PX")
            .doc("キャプチャ高さ")
            .default("240")
            .take(args)
            .then(|o| o.value().parse())?;

        let fps: u32 = noargs::opt("fps")
            .ty("N")
            .doc("フレームレート")
            .default("30")
            .take(args)
            .then(|o| o.value().parse())?;

        let device_str: String = noargs::opt("device")
            .ty("NAME")
            .doc("ML デバイス (auto / cpu / metal / cuda)")
            .default("auto")
            .take(args)
            .then(|o| o.value().parse())?;

        let model_type: String = noargs::opt("model")
            .ty("TYPE")
            .doc("モデル種別 (detect / pose)")
            .default("detect")
            .take(args)
            .then(|o| o.value().parse())?;

        let model_size: usize = noargs::opt("model-size")
            .ty("PX")
            .doc("モデル入力の長辺サイズ (32 の倍数、小さいほど高速)")
            .default("320")
            .take(args)
            .then(|o| o.value().parse())?;

        let confidence: f32 = noargs::opt("confidence")
            .ty("FLOAT")
            .doc("検出の信頼度しきい値")
            .default("0.25")
            .take(args)
            .then(|o| o.value().parse())?;

        let nms: f32 = noargs::opt("nms")
            .ty("FLOAT")
            .doc("NMS の IoU しきい値")
            .default("0.45")
            .take(args)
            .then(|o| o.value().parse())?;

        if args.metadata().help_mode {
            return Ok(true);
        }

        let ml_device = match device_str.to_lowercase().as_str() {
            "cpu" => Some(crate::ml::device::MlDevice::Cpu),
            "metal" => Some(crate::ml::device::MlDevice::Metal(0)),
            "cuda" => Some(crate::ml::device::MlDevice::Cuda(0)),
            "auto" | _ => None,
        };

        run(RunConfig {
            model_path,
            model_type,
            device_ids,
            width,
            height,
            fps,
            model_size,
            device: ml_device,
            confidence,
            nms,
        })
        .map_err(|e| noargs::Error::Other {
            metadata: None,
            error: Box::new(format!("{e:?}")),
        })?;
    }

    Ok(true)
}

#[cfg(feature = "candle")]
fn try_run_audio(args: &mut noargs::RawArgs) -> noargs::Result<bool> {
    let list_devices = noargs::flag("list-audio-devices")
        .doc("利用可能なオーディオ入力デバイス一覧を表示する")
        .take(args)
        .is_present();

    if list_devices {
        if args.metadata().help_mode {
            return Ok(true);
        }
        print_audio_device_list();
        return Ok(true);
    }

    let model_dir: PathBuf = noargs::opt("model-dir")
        .ty("PATH")
        .doc(
            "Whisper モデルディレクトリ (config.json, tokenizer.json, model.safetensors)\n\
             例: openai/whisper-tiny を huggingface-cli でダウンロードしたパス",
        )
        .take(args)
        .then(|o| o.value().parse())?;

    let device_id: Option<String> = noargs::opt("device-id")
        .ty("ID")
        .doc("オーディオ入力デバイス ID (省略時はデフォルト)")
        .take(args)
        .present_and_then(|a| a.value().parse())?;

    let chunk_secs: u32 = noargs::opt("chunk-secs")
        .ty("SEC")
        .doc("転写チャンク長（秒）")
        .default("10")
        .take(args)
        .then(|o| o.value().parse())?;

    let vad = noargs::flag("vad")
        .doc("VAD を有効化（Silero ONNX があれば Silero、なければ RMS）")
        .take(args)
        .is_present();

    let vad_model: Option<PathBuf> = noargs::opt("vad-model")
        .ty("PATH")
        .doc("Silero VAD ONNX（例: ml-models/silero-vad/onnx/model.onnx）")
        .take(args)
        .present_and_then(|a| a.value().parse())?;

    let vad_kind: String = noargs::opt("vad-kind")
        .ty("KIND")
        .doc("VAD 種別: auto / silero / energy / off")
        .default("auto")
        .take(args)
        .then(|o| o.value().parse())?;

    let vad_min_speech_ratio: f32 = noargs::opt("vad-min-speech-ratio")
        .ty("RATIO")
        .doc("RMS VAD: 発話フレーム比率の下限")
        .default("0.05")
        .take(args)
        .then(|o| o.value().parse())?;

    let vad_rms_threshold: f32 = noargs::opt("vad-rms-threshold")
        .ty("AMP")
        .doc("RMS VAD: フレーム RMS の下限")
        .default("0.01")
        .take(args)
        .then(|o| o.value().parse())?;

    let vad_probability: f32 = noargs::opt("vad-probability")
        .ty("PROB")
        .doc("Silero VAD: チャンク平均発話確率の下限")
        .default("0.35")
        .take(args)
        .then(|o| o.value().parse())?;

    let vad_trim = noargs::flag("vad-trim")
        .doc("Silero VAD: 発話区間のみを Whisper に渡す")
        .take(args)
        .is_present();

    let language: Option<String> = noargs::opt("language")
        .ty("CODE")
        .doc("Whisper 言語（例: en, ja）。省略時は多言語モデルで自動推定")
        .take(args)
        .present_and_then(|a| a.value().parse())?;

    let task: String = noargs::opt("task")
        .ty("NAME")
        .doc("Whisper タスク: transcribe / translate")
        .default("transcribe")
        .take(args)
        .then(|o| o.value().parse())?;

    let device_str: String = noargs::opt("device")
        .ty("NAME")
        .doc("ML デバイス (auto / cpu / metal / cuda)")
        .default("auto")
        .take(args)
        .then(|o| o.value().parse())?;

    if args.metadata().help_mode {
        return Ok(true);
    }

    let ml_device = match device_str.to_lowercase().as_str() {
        "cpu" => Some(crate::ml::device::MlDevice::Cpu),
        "metal" => Some(crate::ml::device::MlDevice::Metal(0)),
        "cuda" => Some(crate::ml::device::MlDevice::Cuda(0)),
        "auto" | _ => None,
    };

    run_audio(AudioRunConfig {
        model_dir,
        device_id,
        chunk_secs,
        vad,
        vad_model,
        vad_kind,
        vad_min_speech_ratio,
        vad_rms_threshold,
        vad_probability,
        vad_trim,
        language,
        task,
        device: ml_device,
    })
    .map_err(|e| noargs::Error::Other {
        metadata: None,
        error: Box::new(format!("{e:?}")),
    })?;

    Ok(true)
}

#[cfg(feature = "candle")]
struct AudioRunConfig {
    model_dir: PathBuf,
    device_id: Option<String>,
    chunk_secs: u32,
    vad: bool,
    vad_model: Option<PathBuf>,
    vad_kind: String,
    vad_min_speech_ratio: f32,
    vad_rms_threshold: f32,
    vad_probability: f32,
    vad_trim: bool,
    language: Option<String>,
    task: String,
    device: Option<crate::ml::device::MlDevice>,
}

#[cfg(feature = "candle")]
fn resolve_silero_vad_model(explicit: &Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.clone());
    }
    let models_root = std::env::var_os("ML_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ml-models"));
    let default = crate::ml::audio::silero_vad::default_model_path(&models_root);
    default.is_file().then_some(default)
}

#[cfg(feature = "candle")]
fn build_vad_gate(
    config: &AudioRunConfig,
    candle_device: &candle_core::Device,
) -> crate::Result<crate::ml::audio::vad::VadGate> {
    use crate::ml::audio::vad::VadGate;

    let kind = config.vad_kind.to_lowercase();
    if kind == "off" {
        return Ok(VadGate::off());
    }

    let silero_path = resolve_silero_vad_model(&config.vad_model);
    let use_silero = match kind.as_str() {
        "silero" => true,
        "energy" => false,
        "auto" => silero_path.is_some(),
        other => {
            return Err(crate::Error::new(format!(
                "unsupported vad-kind: {other} (use auto, silero, energy, or off)"
            )));
        }
    };

    if kind == "auto" && !config.vad && silero_path.is_none() {
        return Ok(VadGate::off());
    }

    if use_silero {
        let path = silero_path.ok_or_else(|| {
            crate::Error::new(
                "silero VAD requires --vad-model or ml-models/silero-vad/onnx/model.onnx \
                 (run scripts/download_ml_models.sh vad)",
            )
        })?;
        const FRAME_PROBABILITY: f32 = 0.5;
        return VadGate::silero(
            &path,
            candle_device,
            config.vad_probability,
            FRAME_PROBABILITY,
            config.vad_trim,
        );
    }

    if config.vad || kind == "energy" {
        return Ok(VadGate::energy(
            config.vad_min_speech_ratio,
            config.vad_rms_threshold,
        ));
    }

    Err(crate::Error::new(
        "VAD kind silero requires --vad-model or downloaded ml-models/silero-vad/onnx/model.onnx",
    ))
}

#[cfg(feature = "candle")]
fn run_audio(config: AudioRunConfig) -> crate::Result<()> {
    use crate::ml::audio::decode::Task;
    use crate::ml::audio::vad::VadGate;
    use crate::ml::audio::{AudioMlProcessor, WhisperPipeline};

    crate::ml::audio::whisper::validate_model_dir(&config.model_dir)?;

    let ml_device = config.device.clone();
    let candle_device = ml_device
        .unwrap_or_else(crate::ml::device::MlDevice::auto)
        .to_candle_device()
        .map_err(|e| crate::Error::new(format!("failed to create ML device: {e}")))?;

    let whisper_task = match config.task.to_lowercase().as_str() {
        "transcribe" => Task::Transcribe,
        "translate" => Task::Translate,
        other => {
            return Err(crate::Error::new(format!(
                "unsupported whisper task: {other} (use transcribe or translate)"
            )));
        }
    };

    tracing::info!("Loading Whisper from {}", config.model_dir.display());
    let whisper = WhisperPipeline::load(
        &config.model_dir,
        candle_device.clone(),
        config.language.clone(),
        whisper_task,
    )?;
    tracing::info!("Whisper model loaded");

    let vad = build_vad_gate(&config, &candle_device)?;
    match &vad {
        VadGate::Off => tracing::info!("VAD disabled"),
        VadGate::Energy { .. } => tracing::info!("VAD: energy (RMS)"),
        VadGate::Silero { .. } => {
            tracing::info!("VAD: silero (trim={})", config.vad_trim);
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    let running_ctrlc = running.clone();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .map_err(|e| crate::Error::new(format!("failed to create tokio runtime: {e}")))?;

    rt.block_on(async move {
        let pipeline = crate::MediaPipeline::new()
            .map_err(|e| e.with_context("failed to create media pipeline"))?;
        let handle = pipeline.handle();
        let _pipeline_task = tokio::spawn(pipeline.run());

        let raw_track_id = crate::TrackId::new("ml_audio_raw");
        let audio_source = crate::obsws::source::audio_device::AudioDeviceSource {
            output_audio_track_id: raw_track_id.clone(),
            device_id: config.device_id,
            sample_rate: None,
            channels: None,
            running: Some(running.clone()),
        };
        handle
            .spawn_processor(
                crate::ProcessorId::new("mlAudioDevice"),
                crate::ProcessorMetadata::new("audio_device_source"),
                move |h| audio_source.run(h),
            )
            .await?;

        let ml_processor = AudioMlProcessor {
            input_track_id: raw_track_id,
            whisper,
            chunk_secs: config.chunk_secs,
            vad,
            running: running.clone(),
        };
        handle
            .spawn_processor(
                crate::ProcessorId::new("mlAudioProcessor"),
                crate::ProcessorMetadata::new("audio_ml_processor"),
                move |h| ml_processor.run(h),
            )
            .await?;

        handle.trigger_start().await?;
        tracing::info!(
            "audio ml pipeline started (chunk={}s, Ctrl+C to stop)",
            config.chunk_secs
        );

        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                tracing::info!("Ctrl+C received, stopping audio ml pipeline");
                running_ctrlc.store(false, Ordering::Relaxed);
            }
        });

        while running.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok::<(), crate::Error>(())
    })?;

    Ok(())
}

#[cfg(feature = "candle")]
fn print_audio_device_list() {
    match shiguredo_audio_device::AudioDeviceList::enumerate_input() {
        Ok(list) => {
            let devices = list.devices();
            if devices.is_empty() {
                println!("no audio input devices found");
                return;
            }
            for d in devices {
                let name = d.name().unwrap_or_else(|_| "Unknown".to_owned());
                let id = d.unique_id().unwrap_or_else(|_| "unknown".to_owned());
                println!("  {id}  {name}");
            }
        }
        Err(e) => {
            eprintln!("failed to enumerate audio devices: {e}");
        }
    }
}

#[cfg(all(feature = "candle", feature = "player"))]
fn print_device_list() {
    match shiguredo_video_device::VideoDeviceList::enumerate() {
        Ok(list) => {
            let devices = list.devices();
            if devices.is_empty() {
                println!("no video devices found");
                return;
            }
            for d in devices {
                let name = d.name().unwrap_or_else(|_| "Unknown".to_owned());
                let id = d.unique_id().unwrap_or_else(|_| "unknown".to_owned());
                println!("  {id}  {name}");
            }
        }
        Err(e) => {
            eprintln!("failed to enumerate video devices: {e}");
        }
    }
}

#[cfg(all(feature = "candle", feature = "player"))]
struct RunConfig {
    model_path: PathBuf,
    model_type: String,
    device_ids: Vec<String>,
    width: u32,
    height: u32,
    fps: u32,
    model_size: usize,
    device: Option<crate::ml::device::MlDevice>,
    confidence: f32,
    nms: f32,
}

#[cfg(all(feature = "candle", feature = "player"))]
fn run(config: RunConfig) -> crate::Result<()> {
    use crate::mixer::video;
    use crate::ml::yolo;
    use std::num::NonZeroUsize;

    tracing::info!("Loading model from {}", config.model_path.display());

    let ml_device = config
        .device
        .unwrap_or_else(crate::ml::device::MlDevice::auto)
        .to_candle_device()
        .map_err(|e| crate::Error::new(format!("failed to create ML device: {e}")))?;

    let is_pose = config.model_type == "pose";

    let vb = unsafe {
        candle_nn::VarBuilder::from_mmaped_safetensors(
            &[&config.model_path],
            candle_core::DType::F32,
            &ml_device,
        )
        .map_err(|e| crate::Error::new(format!("failed to load model file: {e}")))?
    };

    let model = Arc::new(if is_pose {
        crate::ml::MlModel::Pose(
            yolo::YoloV8Pose::load(vb, yolo::Multiples::s(), 1, (17, 3))
                .map_err(|e| crate::Error::new(format!("failed to load pose model: {e}")))?,
        )
    } else {
        crate::ml::MlModel::Detect(
            yolo::YoloV8::load(vb, yolo::Multiples::s(), 80)
                .map_err(|e| crate::Error::new(format!("failed to load detect model: {e}")))?,
        )
    });
    tracing::info!(
        "Model loaded successfully ({})",
        if is_pose { "pose" } else { "detect" }
    );

    let running = Arc::new(AtomicBool::new(true));
    let latest_frame = Arc::new(std::sync::Mutex::new(None::<ProcessedFrame>));
    let frame_ready = Arc::new(AtomicBool::new(false));

    let latest_frame_for_player = latest_frame.clone();
    let frame_ready_for_player = frame_ready.clone();
    let running_for_player = running.clone();

    // 省略時はデフォルトデバイス 1 台
    let device_ids = if config.device_ids.is_empty() {
        vec![String::new()]
    } else {
        config.device_ids
    };
    let dual = device_ids.len() >= 2;
    let capture_width = config.width;
    let capture_height = config.height;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .map_err(|e| crate::Error::new(format!("failed to create tokio runtime: {e}")))?;

    let result: crate::Result<()> = rt.block_on(async move {
        let pipeline = crate::MediaPipeline::new()
            .map_err(|e| e.with_context("failed to create media pipeline"))?;
        let handle = pipeline.handle();

        let _pipeline_task = tokio::spawn(pipeline.run());

        let mut ml_out_track_ids = Vec::new();

        for (i, dev_id) in device_ids.iter().enumerate() {
            let raw_track_id = crate::TrackId::new(format!("ml_raw_{i}"));
            let ml_out_track_id = crate::TrackId::new(format!("ml_out_{i}"));

            let video_device_source = crate::obsws::source::video_device::VideoDeviceSource {
                output_video_track_id: raw_track_id.clone(),
                device_id: if dev_id.is_empty() {
                    None
                } else {
                    Some(dev_id.clone())
                },
                pixel_format: None,
                width: Some(capture_width as i32),
                height: Some(capture_height as i32),
                fps: Some(config.fps as i32),
                running: Some(running.clone()),
            };
            handle
                .spawn_processor(
                    crate::ProcessorId::new(format!("mlVideoDevice_{i}")),
                    crate::ProcessorMetadata::new("video_device_source"),
                    move |h| video_device_source.run(h),
                )
                .await?;

            let ml_processor = crate::ml::MlProcessor {
                input_track_id: raw_track_id,
                output_track_id: ml_out_track_id.clone(),
                model: model.clone(),
                model_size: config.model_size,
                is_pose,
                confidence: config.confidence,
                nms: config.nms,
                device: ml_device.clone(),
                running: running.clone(),
            };
            handle
                .spawn_processor(
                    crate::ProcessorId::new(format!("mlProcessor_{i}")),
                    crate::ProcessorMetadata::new("ml_processor"),
                    move |h| ml_processor.run(h),
                )
                .await?;

            ml_out_track_ids.push(ml_out_track_id);
        }

        let display_track_id = if dual {
            let mixer_track_id = crate::TrackId::new("ml_composite");
            let canvas_w = capture_width as usize * 2;
            let canvas_h = capture_height as usize;
            let mixer = video::VideoRealtimeMixer {
                canvas_width: crate::types::EvenUsize::new(canvas_w)
                    .expect("canvas width must be even"),
                canvas_height: crate::types::EvenUsize::new(canvas_h)
                    .expect("canvas height must be even"),
                frame_rate: crate::video::FrameRate {
                    numerator: NonZeroUsize::new(config.fps as usize)
                        .expect("fps must be non-zero"),
                    denumerator: NonZeroUsize::MIN,
                },
                input_tracks: vec![
                    video::InputTrack {
                        track_id: ml_out_track_ids[0].clone(),
                        x: 0,
                        y: 0,
                        z: 0,
                        width: Some(
                            crate::types::EvenUsize::new(capture_width as usize)
                                .expect("width must be even"),
                        ),
                        height: Some(
                            crate::types::EvenUsize::new(capture_height as usize)
                                .expect("height must be even"),
                        ),
                        scale_x: None,
                        scale_y: None,
                        crop_top: 0,
                        crop_bottom: 0,
                        crop_left: 0,
                        crop_right: 0,
                    },
                    video::InputTrack {
                        track_id: ml_out_track_ids[1].clone(),
                        x: capture_width as isize,
                        y: 0,
                        z: 0,
                        width: Some(
                            crate::types::EvenUsize::new(capture_width as usize)
                                .expect("width must be even"),
                        ),
                        height: Some(
                            crate::types::EvenUsize::new(capture_height as usize)
                                .expect("height must be even"),
                        ),
                        scale_x: None,
                        scale_y: None,
                        crop_top: 0,
                        crop_bottom: 0,
                        crop_left: 0,
                        crop_right: 0,
                    },
                ],
                output_track_id: mixer_track_id.clone(),
            };
            video::create_processor(&handle, mixer, None).await?;
            mixer_track_id
        } else {
            ml_out_track_ids
                .into_iter()
                .next()
                .expect("video device tracks must not be empty")
        };

        let display_sink = DisplaySink {
            input_track_id: display_track_id,
            latest_frame: latest_frame.clone(),
            frame_ready: frame_ready.clone(),
            running: running.clone(),
        };
        handle
            .spawn_processor(
                crate::ProcessorId::new("mlDisplaySink"),
                crate::ProcessorMetadata::new("display_sink"),
                move |h| display_sink.run(h),
            )
            .await?;

        handle.trigger_start().await?;

        tracing::info!(
            "pipeline started ({}x{} @ {}fps) model={} dual={dual}",
            capture_width,
            capture_height,
            config.fps,
            if is_pose { "pose" } else { "detect" }
        );

        Ok(())
    });
    result?;

    let canvas_w = if dual {
        capture_width * 2
    } else {
        capture_width
    };
    run_player_loop(
        latest_frame_for_player,
        frame_ready_for_player,
        canvas_w,
        capture_height,
        &running_for_player,
    );

    drop(rt);
    Ok(())
}

// ============================================================
// 表示用シンクプロセッサ
// ============================================================

#[cfg(all(feature = "candle", feature = "player"))]
struct DisplaySink {
    input_track_id: crate::TrackId,
    latest_frame: Arc<std::sync::Mutex<Option<ProcessedFrame>>>,
    frame_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

#[cfg(all(feature = "candle", feature = "player"))]
impl DisplaySink {
    async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        use crate::MediaFrame;

        let mut input_rx = handle.subscribe_track(self.input_track_id);
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        while self.running.load(Ordering::Relaxed) {
            match input_rx.recv().await {
                crate::Message::Media(MediaFrame::Video(frame)) => {
                    let raw = crate::video::RawVideoFrame::from_i420_video_frame(frame.clone())?;
                    let size = raw.size();
                    let (y_plane, u_plane, v_plane) = raw.as_i420_planes().map_err(|e| {
                        crate::Error::new(format!("failed to extract I420 planes: {}", e.display()))
                    })?;

                    {
                        let mut guard = self.latest_frame.lock().unwrap();
                        *guard = Some(ProcessedFrame {
                            y: y_plane.to_vec(),
                            u: u_plane.to_vec(),
                            v: v_plane.to_vec(),
                            width: size.width,
                            height: size.height,
                            timestamp_us: frame.timestamp.as_micros() as i64,
                        });
                    }
                    self.frame_ready.store(true, Ordering::Release);
                }
                crate::Message::Eos => break,
                _ => {}
            }
        }

        Ok(())
    }
}

// ============================================================
// データ型
// ============================================================

#[cfg(all(feature = "candle", feature = "player"))]
struct ProcessedFrame {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    width: usize,
    height: usize,
    timestamp_us: i64,
}

// ============================================================
// プレイヤー表示ループ
// ============================================================

#[cfg(all(feature = "candle", feature = "player"))]
fn run_player_loop(
    latest_frame: Arc<std::sync::Mutex<Option<ProcessedFrame>>>,
    frame_ready: Arc<AtomicBool>,
    width: u32,
    height: u32,
    running: &AtomicBool,
) {
    raw_player::init().expect("failed to init raw_player");

    let player =
        raw_player::VideoPlayer::new(width as i32, height as i32, "hisui - ML object detection")
            .expect("failed to create player window");

    if let Err(e) = player.play() {
        tracing::error!("failed to start player: {e}");
        player.close();
        // SAFETY: SDL リソース (player) は直前で close 済みのため安全
        unsafe { raw_player::quit() };
        return;
    }

    let mut frame_count: u64 = 0;
    let mut enqueue_failures: u64 = 0;

    loop {
        let mut received = false;
        if frame_ready.load(Ordering::Acquire) {
            if let Ok(mut guard) = latest_frame.try_lock() {
                if let Some(frame) = guard.take() {
                    received = true;
                    if let Err(e) = player.enqueue_video_i420(
                        &frame.y,
                        &frame.u,
                        &frame.v,
                        frame.width as i32,
                        frame.height as i32,
                        frame.timestamp_us,
                    ) {
                        enqueue_failures += 1;
                        if enqueue_failures <= 3 {
                            tracing::warn!("enqueue fail #{enqueue_failures}: {e}");
                        }
                    } else {
                        frame_count += 1;
                    }
                }
                frame_ready.store(false, Ordering::Release);
            }
        }

        match player.poll_events() {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!("window closed (enqueued={frame_count}, failed={enqueue_failures})");
                break;
            }
            Err(e) => {
                tracing::error!("poll_events error: {e}");
                break;
            }
        }

        if !received {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    running.store(false, Ordering::Relaxed);
    player.close();
    // SAFETY: SDL リソース (player) は直前で close 済みのため安全
    unsafe { raw_player::quit() };
}
