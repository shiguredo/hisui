use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let host: IpAddr = noargs::opt("host")
        .ty("HOST")
        .env("HISUI_OBSWS_HOST")
        .doc("OBS WebSocket のリッスンアドレス")
        .default("127.0.0.1")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .ty("PORT")
        .env("HISUI_OBSWS_PORT")
        .doc("OBS WebSocket のリッスンポート")
        .default("4455")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let password: Option<String> = noargs::opt("password")
        .ty("PASSWORD")
        .env("HISUI_OBSWS_PASSWORD")
        .doc("OBS WebSocket の接続パスワード")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let default_record_dir: Option<PathBuf> = noargs::opt("default-record-dir")
        .ty("PATH")
        .env("HISUI_DEFAULT_RECORD_DIR")
        .doc("obsws の録画先ディレクトリ初期値")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    #[cfg(feature = "fdk-aac")]
    let fdk_aac: Option<PathBuf> = noargs::opt("fdk-aac")
        .ty("PATH")
        .env("HISUI_FDK_AAC_PATH")
        .doc("FDK-AAC の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let canvas_width: crate::types::EvenUsize = noargs::opt("canvas-width")
        .ty("WIDTH")
        .env("HISUI_OBSWS_CANVAS_WIDTH")
        .doc("映像ミキサーのキャンバス幅（偶数のみ）")
        .default("1920")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let canvas_height: crate::types::EvenUsize = noargs::opt("canvas-height")
        .ty("HEIGHT")
        .env("HISUI_OBSWS_CANVAS_HEIGHT")
        .doc("映像ミキサーのキャンバス高さ（偶数のみ）")
        .default("1080")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let frame_rate: crate::video::FrameRate = noargs::opt("frame-rate")
        .ty("FRAME_RATE")
        .env("HISUI_OBSWS_FRAME_RATE")
        .doc("映像のフレームレート")
        .default("30")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let addr = SocketAddr::new(host, port);

    run_internal(
        addr,
        password,
        resolve_default_record_dir(default_record_dir)?,
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
        crate::obsws_server::run_server(
            addr,
            password,
            default_record_dir,
            pipeline_config,
            canvas_width,
            canvas_height,
            frame_rate,
        )
        .await
    })
}

fn resolve_default_record_dir(configured: Option<PathBuf>) -> crate::Result<PathBuf> {
    let record_dir = configured.unwrap_or_else(|| PathBuf::from("recordings"));
    std::path::absolute(record_dir)
        .map_err(|e| crate::Error::new(format!("failed to resolve absolute path: {e}")))
}
