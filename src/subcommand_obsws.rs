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
    )
    .map_err(noargs::Error::from)
}

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

    runtime.block_on(async move {
        // WebRtcP2pSessionManager が spawn_local() で !Send タスクを起動するため、
        // obsws サーバーの実行コンテキストは LocalSet 上で動かす必要がある。
        let local = tokio::task::LocalSet::new();
        local
            .run_until(crate::obsws_server::run_server(
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
            ))
            .await
    })
}

fn resolve_default_record_dir(configured: Option<PathBuf>) -> crate::Result<PathBuf> {
    let record_dir = configured.unwrap_or_else(|| PathBuf::from("recordings"));
    std::path::absolute(record_dir)
        .map_err(|e| crate::Error::new(format!("failed to resolve absolute path: {e}")))
}
