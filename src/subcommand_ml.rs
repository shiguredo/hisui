use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

pub fn try_run(args: &mut noargs::RawArgs) -> noargs::Result<bool> {
    let cmd = noargs::cmd("ml")
        .doc("カメラ入力に ML 物体検知を適用して画面表示する")
        .take(args);

    if !cmd.is_present() {
        return Ok(false);
    }

    #[cfg(not(all(feature = "candle", feature = "player")))]
    {
        tracing::error!(
            "ml subcommand requires both 'candle' and 'player' features. \
             Rebuild with: cargo build --features candle"
        );
        std::process::exit(1);
    }

    #[cfg(all(feature = "candle", feature = "player"))]
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

    let dual = config.device_ids.len() >= 2;
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

        for (i, dev_id) in config.device_ids.iter().enumerate() {
            let cam_track_id = crate::TrackId::new(format!("ml_raw_{i}"));
            let ml_out_track_id = crate::TrackId::new(format!("ml_out_{i}"));

            let camera_source = CameraSource {
                output_video_track_id: cam_track_id.clone(),
                device_id: if dev_id.is_empty() {
                    None
                } else {
                    Some(dev_id.clone())
                },
                width: capture_width as i32,
                height: capture_height as i32,
                fps: config.fps as i32,
                running: running.clone(),
            };
            handle
                .spawn_processor(
                    crate::ProcessorId::new(format!("mlCamera_{i}")),
                    crate::ProcessorMetadata::new("camera_source"),
                    move |h| camera_source.run(h),
                )
                .await?;

            let ml_processor = crate::ml::MlProcessor {
                input_track_id: cam_track_id,
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
                .expect("camera tracks must not be empty")
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
// カメラキャプチャプロセッサ
// ============================================================

#[cfg(all(feature = "candle", feature = "player"))]
struct CameraSource {
    output_video_track_id: crate::TrackId,
    device_id: Option<String>,
    width: i32,
    height: i32,
    fps: i32,
    running: Arc<AtomicBool>,
}

#[cfg(all(feature = "candle", feature = "player"))]
impl CameraSource {
    async fn run(self, handle: crate::ProcessorHandle) -> crate::Result<()> {
        let mut output_tx = handle
            .publish_track(self.output_video_track_id.clone())
            .await?;

        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let config = shiguredo_video_device::VideoCaptureConfig {
            device_id: self.device_id.clone(),
            width: self.width,
            height: self.height,
            fps: self.fps,
            pixel_format: None,
        };

        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<shiguredo_video_device::VideoFrameOwned>();
        let mut capture = shiguredo_video_device::VideoCapture::new(config, move |f| {
            let _ = frame_tx.send(f.to_owned());
        })
        .map_err(|e| crate::Error::new(format!("failed to create video capture session: {e}")))?;

        capture
            .start()
            .map_err(|e| crate::Error::new(format!("failed to start video capture: {e}")))?;

        while self.running.load(Ordering::Relaxed) {
            let captured = match frame_rx.recv().await {
                Some(f) => f,
                None => break,
            };

            let video_frame = captured_to_video_frame(&captured)?;
            if !output_tx.send_video(video_frame) {
                break;
            }
        }

        capture.stop();
        output_tx.send_eos();
        Ok(())
    }
}

/// キャプチャされたフレームを I420 VideoFrame に変換する
#[cfg(all(feature = "candle", feature = "player"))]
fn captured_to_video_frame(
    captured: &shiguredo_video_device::VideoFrameOwned,
) -> crate::Result<crate::VideoFrame> {
    let w = captured.width as usize;
    let h = captured.height as usize;
    if w == 0 || h == 0 {
        return Err(crate::Error::new(format!(
            "invalid frame size: {}x{}",
            captured.width, captured.height
        )));
    }

    let timestamp = std::time::Duration::from_micros(captured.timestamp_us as u64);
    let y_size = w * h;
    let uv_width = w.div_ceil(2);
    let uv_height = h.div_ceil(2);
    let uv_size = uv_width * uv_height;

    // I420: data に Y+U+V が連続で格納
    if captured.data.len() >= y_size + uv_size * 2 {
        let mut data = Vec::with_capacity(y_size + uv_size * 2);
        data.extend_from_slice(&captured.data[..y_size]);
        data.extend_from_slice(&captured.data[y_size..y_size + uv_size]);
        data.extend_from_slice(&captured.data[y_size + uv_size..y_size + uv_size * 2]);
        return Ok(crate::VideoFrame {
            data,
            format: crate::video::VideoFormat::I420,
            keyframe: false,
            size: Some(crate::video::VideoFrameSize::new(w, h)?),
            timestamp,
            sample_entry: None,
        });
    }

    // NV12: data = Y plane, uv_data = インターリーブ UV
    if let Some(uv_plane) = captured.uv_data.as_deref() {
        let raw_y_stride = usize::try_from(captured.stride).unwrap_or(0);
        let raw_uv_stride = usize::try_from(captured.stride_uv).unwrap_or(0);
        let y_stride = if raw_y_stride >= w && raw_y_stride * h <= captured.data.len() {
            raw_y_stride
        } else {
            w
        };
        let uv_stride = if raw_uv_stride >= uv_width && raw_uv_stride * uv_height <= uv_plane.len()
        {
            raw_uv_stride
        } else {
            uv_width
        };

        let mut i420_data = vec![0u8; y_size + uv_size * 2];
        let (y_dst, rest) = i420_data.split_at_mut(y_size);
        let (u_dst, v_dst) = rest.split_at_mut(uv_size);

        let src = shiguredo_libyuv::Nv12Image {
            y: &captured.data,
            y_stride,
            uv: uv_plane,
            uv_stride,
        };
        let mut dst = shiguredo_libyuv::I420ImageMut {
            y: y_dst,
            y_stride: w,
            u: u_dst,
            u_stride: uv_width,
            v: v_dst,
            v_stride: uv_width,
        };
        shiguredo_libyuv::nv12_to_i420(&src, &mut dst, shiguredo_libyuv::ImageSize::new(w, h))
            .map_err(|e| crate::Error::new(format!("NV12 to I420 conversion failed: {e}")))?;

        let mut data = Vec::with_capacity(y_size + uv_size * 2);
        data.extend_from_slice(y_dst);
        data.extend_from_slice(u_dst);
        data.extend_from_slice(v_dst);

        return Ok(crate::VideoFrame {
            data,
            format: crate::video::VideoFormat::I420,
            keyframe: false,
            size: Some(crate::video::VideoFrameSize::new(w, h)?),
            timestamp,
            sample_entry: None,
        });
    }

    Err(crate::Error::new(format!(
        "unexpected camera frame: {} bytes, uv_data={}",
        captured.data.len(),
        captured.uv_data.is_some()
    )))
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
