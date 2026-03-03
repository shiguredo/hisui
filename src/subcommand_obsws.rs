use std::net::IpAddr;

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

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    crate::obsws_server::run_internal(ws_host, ws_port, http_listen_address, http_port, password)
        .map_err(noargs::Error::from)
}
