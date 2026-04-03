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

    // SDL3 は macOS でメインスレッド必須のため、player feature 有効時は常にスレッドモデルを変更する:
    // メインスレッド → player 制御ループ（SDL）、バックグラウンドスレッド → tokio ランタイム
    // cfg(not(feature = "player")) ブロックが後続するため return が必要
    #[expect(clippy::needless_return)]
    #[cfg(feature = "player")]
    {
        let (command_tx, command_rx) =
            std::sync::mpsc::sync_channel::<crate::obsws::player::PlayerCommand>(4);
        let (media_tx, media_rx) =
            std::sync::mpsc::sync_channel::<crate::obsws::player::PlayerMediaMessage>(8);

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
                            #[cfg(feature = "player")]
                            command_tx,
                            #[cfg(feature = "player")]
                            media_tx,
                        ))
                        .await
                })
            })
            .map_err(|e| crate::Error::new(format!("failed to spawn runtime thread: {e}")))?;

        run_player_control_loop(command_rx, media_rx);

        return runtime_thread
            .join()
            .map_err(|_| crate::Error::new("runtime thread panicked"))?;
    }

    #[cfg(not(feature = "player"))]
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
            ))
            .await
    })
}

/// メインスレッドで player の制御ループを実行する。
/// StartOutput で Start コマンドが届いたら SDL ウィンドウを開き、
/// StopOutput やウィンドウ閉じで待機に戻る。Terminate でループ終了。
#[cfg(feature = "player")]
fn run_player_control_loop(
    command_rx: std::sync::mpsc::Receiver<crate::obsws::player::PlayerCommand>,
    media_rx: std::sync::mpsc::Receiver<crate::obsws::player::PlayerMediaMessage>,
) {
    use crate::obsws::player::{PlayerCommand, PlayerMediaMessage};

    // チャネルが閉じたら（= tokio ランタイムが終了したら）ループ終了
    while let Ok(command) = command_rx.recv() {
        match command {
            PlayerCommand::Start {
                canvas_width,
                canvas_height,
            } => {
                if let Err(e) = raw_player::init() {
                    tracing::error!("failed to init raw_player: {e}");
                    continue;
                }
                let player =
                    match raw_player::VideoPlayer::new(canvas_width, canvas_height, "hisui") {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("failed to create raw_player window: {e}");
                            // SAFETY: SDL リソース（player）は直前で close 済みのため安全
                            unsafe { raw_player::quit() };
                            continue;
                        }
                    };
                if let Err(e) = player.play() {
                    tracing::error!("failed to start raw_player playback: {e}");
                    player.close();
                    // SAFETY: SDL リソース（player）は直前で close 済みのため安全
                    unsafe { raw_player::quit() };
                    continue;
                }

                // フレーム受信 + SDL イベントループ
                'frame_loop: loop {
                    // 制御コマンドをノンブロッキングで確認
                    match command_rx.try_recv() {
                        Ok(PlayerCommand::Stop) | Ok(PlayerCommand::Terminate) => break 'frame_loop,
                        Ok(PlayerCommand::Start { .. }) => {} // 既に起動中なので無視
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'frame_loop,
                    }

                    // メディアフレームをノンブロッキングで取得して enqueue
                    while let Ok(msg) = media_rx.try_recv() {
                        match msg {
                            PlayerMediaMessage::Video {
                                y,
                                u,
                                v,
                                width,
                                height,
                                pts_us,
                            } => {
                                if let Err(e) =
                                    player.enqueue_video_i420(&y, &u, &v, width, height, pts_us)
                                {
                                    tracing::warn!("failed to enqueue video frame: {e}");
                                }
                            }
                            PlayerMediaMessage::Audio {
                                data,
                                pts_us,
                                sample_rate,
                                channels,
                            } => {
                                if let Err(e) = player.enqueue_audio(
                                    &data,
                                    pts_us,
                                    sample_rate,
                                    channels,
                                    raw_player::AudioFormat::S16,
                                ) {
                                    tracing::warn!("failed to enqueue audio frame: {e}");
                                }
                            }
                        }
                    }

                    // SDL3 イベント処理とレンダリング
                    match player.poll_events() {
                        Ok(true) => {}
                        Ok(false) => break 'frame_loop, // ウィンドウが閉じられた
                        Err(e) => {
                            tracing::error!("raw_player poll_events error: {e}");
                            break 'frame_loop;
                        }
                    }
                }

                player.close();
                // SAFETY: SDL リソース（player）は直前で close 済みのため安全
                unsafe { raw_player::quit() };

                // メディアチャネルに残っているフレームを破棄する
                while media_rx.try_recv().is_ok() {}
            }
            PlayerCommand::Stop => {
                // 既に停止中なので無視
            }
            PlayerCommand::Terminate => break,
        }
    }
}

fn resolve_default_record_dir(configured: Option<PathBuf>) -> crate::Result<PathBuf> {
    let record_dir = configured.unwrap_or_else(|| PathBuf::from("recordings"));
    std::path::absolute(record_dir)
        .map_err(|e| crate::Error::new(format!("failed to resolve absolute path: {e}")))
}
