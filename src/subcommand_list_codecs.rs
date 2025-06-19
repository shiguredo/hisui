use crate::{
    decoder::{AudioDecoder, VideoDecoder},
    encoder::{AudioEncoder, VideoEncoder},
    types::{CodecName, EngineName},
};

pub fn run(args: noargs::RawArgs) -> noargs::Result<()> {
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // TODO:
    let is_openh264_available = false;

    let mut codecs = Vec::new();

    for name in [CodecName::Opus, CodecName::Aac] {
        codecs.push(CodecInfo {
            name,
            decoders: AudioDecoder::get_engines(name),
            encoders: AudioEncoder::get_engines(name),
        });
    }

    for name in [
        CodecName::Vp8,
        CodecName::Vp9,
        CodecName::H264,
        CodecName::H265,
        CodecName::Av1,
    ] {
        codecs.push(CodecInfo {
            name,
            decoders: VideoDecoder::get_engines(name, is_openh264_available),
            encoders: VideoEncoder::get_engines(name, is_openh264_available),
        });
    }

    println!(
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(&codecs)
        })
    );

    Ok(())
}

#[derive(Debug)]
struct CodecInfo {
    name: CodecName,
    decoders: Vec<EngineName>,
    encoders: Vec<EngineName>,
}

impl nojson::DisplayJson for CodecInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("name", self.name)?;
            f.member(
                "type",
                if matches!(self.name, CodecName::Opus | CodecName::Aac) {
                    "audio"
                } else {
                    "video"
                },
            )?;
            f.member(
                "encoders",
                nojson::json(|f| {
                    f.set_indent_size(0);
                    f.value(&self.encoders)?;
                    f.set_indent_size(2);
                    Ok(())
                }),
            )?;
            f.member(
                "decoders",
                nojson::json(|f| {
                    f.set_indent_size(0);
                    f.value(&self.decoders)?;
                    f.set_indent_size(2);
                    Ok(())
                }),
            )?;
            Ok(())
        })
    }
}
