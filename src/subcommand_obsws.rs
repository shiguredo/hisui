use std::net::IpAddr;
use std::path::PathBuf;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let ws_host: IpAddr = noargs::opt("host")
        .ty("HOST")
        .env("HISUI_OBSWS_HOST")
        .doc("OBS WebSocket のリッスンアドレス")
        .default("127.0.0.1")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let ws_port: u16 = noargs::opt("port")
        .ty("PORT")
        .env("HISUI_OBSWS_PORT")
        .doc("OBS WebSocket のリッスンポート")
        .default("4455")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let http_listen_address: IpAddr = noargs::opt("http-listen-address")
        .ty("ADDRESS")
        .env("HISUI_OBSWS_HTTP_LISTEN_ADDRESS")
        .doc("obsws 用 HTTP サーバーのリッスンアドレス")
        .default("127.0.0.1")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let http_port: u16 = noargs::opt("http-port")
        .ty("PORT")
        .env("HISUI_OBSWS_HTTP_PORT")
        .doc("obsws 用 HTTP サーバーのリッスンポート")
        .default("4456")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let password: Option<String> = noargs::opt("password")
        .ty("PASSWORD")
        .env("HISUI_OBSWS_PASSWORD")
        .doc("OBS WebSocket の接続パスワード")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    run_internal(
        ws_host,
        ws_port,
        http_listen_address,
        http_port,
        password,
        openh264,
    )
    .map_err(noargs::Error::from)
}

fn run_internal(
    ws_host: IpAddr,
    ws_port: u16,
    http_host: IpAddr,
    http_port: u16,
    password: Option<String>,
    openh264: Option<PathBuf>,
) -> crate::Result<()> {
    let openh264_lib = openh264
        .as_ref()
        .map(shiguredo_openh264::Openh264Library::load)
        .transpose()?;
    let pipeline_config = crate::MediaPipelineConfig { openh264_lib };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(crate::Error::from)?;

    runtime.block_on(async move {
        crate::obsws_server::run_server(
            ws_host,
            ws_port,
            http_host,
            http_port,
            password,
            pipeline_config,
        )
        .await
    })
}
