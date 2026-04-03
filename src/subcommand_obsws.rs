use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

pub fn try_run(args: &mut noargs::RawArgs) -> noargs::Result<bool> {
    if !noargs::cmd("obsws")
        .doc("OBS WebSocket 互換コマンド（実験的）")
        .take(args)
        .is_present()
    {
        return Ok(false);
    }
    run(args)?;
    Ok(true)
}

fn run(args: &mut noargs::RawArgs) -> noargs::Result<()> {
    let host: IpAddr = noargs::opt("host")
        .ty("HOST")
        .env("HISUI_OBSWS_HOST")
        .doc("OBS WebSocket のリッスンアドレス")
        .default("127.0.0.1")
        .take(args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .ty("PORT")
        .env("HISUI_OBSWS_PORT")
        .doc("OBS WebSocket のリッスンポート")
        .default("4455")
        .take(args)
        .then(|o| o.value().parse())?;
    let password: Option<String> = noargs::opt("password")
        .ty("PASSWORD")
        .env("HISUI_OBSWS_PASSWORD")
        .doc("OBS WebSocket の接続パスワード")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    let default_record_dir: Option<PathBuf> = noargs::opt("default-record-dir")
        .ty("PATH")
        .env("HISUI_DEFAULT_RECORD_DIR")
        .doc("obsws の録画先ディレクトリ初期値")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    let ui_remote_url: Option<String> = noargs::opt("ui-remote-url")
        .ty("URL")
        .doc("UI 用リモートサーバーの URL（GET リクエストをリバースプロキシする）")
        .take(args)
        .present_and_then(|o| Ok::<_, std::convert::Infallible>(o.value().to_string()))?;
    let https_cert_path: Option<PathBuf> = noargs::opt("https-cert-path")
        .ty("PATH")
        .doc("HTTPS 用の証明書ファイルパス（PEM 形式）")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    let https_key_path: Option<PathBuf> = noargs::opt("https-key-path")
        .ty("PATH")
        .doc("HTTPS 用の秘密鍵ファイルパス（PEM 形式）")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    #[cfg(feature = "fdk-aac")]
    let fdk_aac: Option<PathBuf> = noargs::opt("fdk-aac")
        .ty("PATH")
        .env("HISUI_FDK_AAC_PATH")
        .doc("FDK-AAC の共有ライブラリのパス")
        .take(args)
        .present_and_then(|o| o.value().parse())?;
    #[cfg(feature = "monitor")]
    let monitor: bool = noargs::flag("monitor")
        .doc("Program 出力をモニターウィンドウに表示する")
        .take(args)
        .is_present();
    let canvas_width: crate::types::EvenUsize = noargs::opt("canvas-width")
        .ty("WIDTH")
        .env("HISUI_OBSWS_CANVAS_WIDTH")
        .doc("映像ミキサーのキャンバス幅（偶数のみ）")
        .default("1920")
        .take(args)
        .then(|o| o.value().parse())?;
    let canvas_height: crate::types::EvenUsize = noargs::opt("canvas-height")
        .ty("HEIGHT")
        .env("HISUI_OBSWS_CANVAS_HEIGHT")
        .doc("映像ミキサーのキャンバス高さ（偶数のみ）")
        .default("1080")
        .take(args)
        .then(|o| o.value().parse())?;
    let frame_rate: crate::video::FrameRate = noargs::opt("frame-rate")
        .ty("FRAME_RATE")
        .env("HISUI_OBSWS_FRAME_RATE")
        .doc("映像のフレームレート")
        .default("30")
        .take(args)
        .then(|o| o.value().parse())?;
    let state_file: Option<PathBuf> = noargs::opt("state-file")
        .ty("PATH")
        .env("HISUI_OBSWS_STATE_FILE")
        .doc("obsws の設定永続化用 state file のパス")
        .take(args)
        .present_and_then(|o| o.value().parse())?;

    if args.metadata().help_mode {
        return Ok(());
    }

    // 片方のみ指定はエラー
    match (&https_cert_path, &https_key_path) {
        (Some(_), None) => {
            return Err(noargs::Error::other(
                args,
                "--https-cert-path requires --https-key-path",
            ));
        }
        (None, Some(_)) => {
            return Err(noargs::Error::other(
                args,
                "--https-key-path requires --https-cert-path",
            ));
        }
        _ => {}
    }

    let addr = SocketAddr::new(host, port);

    run_internal(
        addr,
        password,
        resolve_default_record_dir(default_record_dir)?,
        ui_remote_url,
        https_cert_path,
        https_key_path,
        openh264,
        #[cfg(feature = "fdk-aac")]
        fdk_aac,
        #[cfg(feature = "monitor")]
        monitor,
        canvas_width,
        canvas_height,
        frame_rate,
        state_file,
    )
    .map_err(noargs::Error::from)
}

#[expect(clippy::too_many_arguments)]
fn run_internal(
    addr: SocketAddr,
    password: Option<String>,
    default_record_dir: PathBuf,
    ui_remote_url: Option<String>,
    https_cert_path: Option<PathBuf>,
    https_key_path: Option<PathBuf>,
    openh264: Option<PathBuf>,
    #[cfg(feature = "fdk-aac")] fdk_aac: Option<PathBuf>,
    #[cfg(feature = "monitor")] monitor: bool,
    canvas_width: crate::types::EvenUsize,
    canvas_height: crate::types::EvenUsize,
    frame_rate: crate::video::FrameRate,
    state_file: Option<PathBuf>,
) -> crate::Result<()> {
    let openh264_lib = openh264
        .as_ref()
        .map(shiguredo_openh264::Openh264Library::load)
        .transpose()?;
    #[cfg(feature = "fdk-aac")]
    let fdk_aac_lib = fdk_aac
        .as_ref()
        .map(shiguredo_fdk_aac::FdkAacLibrary::load)
        .transpose()?;
    let pipeline_config = crate::MediaPipelineConfig {
        openh264_lib,
        #[cfg(feature = "fdk-aac")]
        fdk_aac_lib,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(crate::Error::from)?;

    // SDL3 は macOS でメインスレッド必須のため、--monitor 指定時はスレッドモデルを変更する:
    // メインスレッド → SDL3 イベントループ、バックグラウンドスレッド → tokio ランタイム
    #[cfg(feature = "monitor")]
    if monitor {
        raw_player::init()
            .map_err(|e| crate::Error::new(format!("failed to init raw_player: {e}")))?;
        let player = raw_player::VideoPlayer::new(
            canvas_width.get() as i32,
            canvas_height.get() as i32,
            "hisui",
        )
        .map_err(|e| crate::Error::new(format!("failed to create raw_player window: {e}")))?;

        let (frame_tx, frame_rx) =
            std::sync::mpsc::sync_channel::<crate::obsws::monitor::RawPlayerFrame>(2);

        let runtime_thread = std::thread::Builder::new()
            .name("hisui-tokio-main".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    let local = tokio::task::LocalSet::new();
                    local
                        .run_until(crate::obsws::server::run_server(
                            addr,
                            password,
                            default_record_dir,
                            ui_remote_url,
                            https_cert_path,
                            https_key_path,
                            pipeline_config,
                            canvas_width,
                            canvas_height,
                            frame_rate,
                            state_file,
                            #[cfg(feature = "monitor")]
                            Some(frame_tx),
                        ))
                        .await
                })
            })
            .map_err(|e| crate::Error::new(format!("failed to spawn runtime thread: {e}")))?;

        run_monitor_event_loop(player, frame_rx);

        // ウィンドウが閉じた後、サーバーの終了を待つ
        return runtime_thread
            .join()
            .map_err(|_| crate::Error::new("runtime thread panicked"))?;
    }

    runtime.block_on(async move {
        // WebRtcP2pSessionManager が spawn_local() で !Send タスクを起動するため、
        // obsws サーバーの実行コンテキストは LocalSet 上で動かす必要がある。
        let local = tokio::task::LocalSet::new();
        local
            .run_until(crate::obsws::server::run_server(
                addr,
                password,
                default_record_dir,
                ui_remote_url,
                https_cert_path,
                https_key_path,
                pipeline_config,
                canvas_width,
                canvas_height,
                frame_rate,
                state_file,
                #[cfg(feature = "monitor")]
                None,
            ))
            .await
    })
}

#[cfg(feature = "monitor")]
fn run_monitor_event_loop(
    player: raw_player::VideoPlayer,
    frame_rx: std::sync::mpsc::Receiver<crate::obsws::monitor::RawPlayerFrame>,
) {
    if let Err(e) = player.play() {
        tracing::error!("failed to start raw_player playback: {e}");
        return;
    }
    loop {
        // ノンブロッキングでフレームを取得して enqueue
        while let Ok(frame) = frame_rx.try_recv() {
            if let Err(e) = player.enqueue_video_i420(
                &frame.y,
                &frame.u,
                &frame.v,
                frame.width,
                frame.height,
                frame.pts_us,
            ) {
                tracing::warn!("failed to enqueue video frame: {e}");
            }
        }
        // SDL3 イベント処理とレンダリング
        match player.poll_events() {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                tracing::error!("raw_player poll_events error: {e}");
                break;
            }
        }
    }
    player.close();
    raw_player::quit();
}

fn resolve_default_record_dir(configured: Option<PathBuf>) -> crate::Result<PathBuf> {
    let record_dir = configured.unwrap_or_else(|| PathBuf::from("recordings"));
    std::path::absolute(record_dir)
        .map_err(|e| crate::Error::new(format!("failed to resolve absolute path: {e}")))
}
