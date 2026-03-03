use std::net::IpAddr;

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let host: IpAddr = noargs::opt("host")
        .ty("HOST")
        .env("HISUI_OBSWS_HOST")
        .doc("OBS WebSocket の接続先ホスト")
        .default("127.0.0.1")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .ty("PORT")
        .env("HISUI_OBSWS_PORT")
        .doc("OBS WebSocket の接続先ポート")
        .default("4455")
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

    let password_state = if password.is_some() { "set" } else { "unset" };
    println!(
        "obsws subcommand is experimental and not implemented yet (host={host}, port={port}, password={password_state})"
    );
    Ok(())
}
