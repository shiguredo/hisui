use std::path::PathBuf;

use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{AudioDecoder, VideoDecoder},
    encoder::{AudioEncoder, VideoEncoder},
    metadata::ContainerFormat,
    types::{CodecName, EngineName},
};

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let host: String = noargs::opt("host")
        .short('H')
        .doc("RTMP server host")
        .default("127.0.0.1")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: Option<u16> = noargs::opt("port")
        .short('p')
        .doc("RTMP server port (default: 1935, or 443 with --tls)")
        .take(&mut args)
        .present_and_then(|o| o.value().parse())?;
    let app: String = noargs::opt("app")
        .short('a')
        .doc("RTMP application name")
        .default("live")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let stream_name: String = noargs::opt("stream")
        .short('s')
        .doc("RTMP stream name")
        .default("stream")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let tls_flag: bool = noargs::flag("tls")
        .doc("Enable TLS (RTMPS)")
        .take(&mut args)
        .is_present();
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let input_file_path: PathBuf = noargs::arg("INPUT_FILE")
        .doc("入力ファイル（.mp4 ないし .webm）")
        .take(&mut args)
        .then(|a| a.value().parse())?;
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let openh264_lib = openh264
        .as_ref()
        .and_then(|path| Openh264Library::load(path).ok());
    let format = ContainerFormat::from_path(&input_file_path).or_fail()?;

    Ok(())
}
