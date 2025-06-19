use orfail::OrFail;
use std::collections::BTreeSet;

// Import the necessary types from the codebase
use crate::decoder::VideoDecoderOptions;
use crate::encoder::{AudioEncoder, VideoEncoder};
use crate::types::{CodecEngines, CodecName, EngineName};

pub fn run(args: noargs::RawArgs) -> noargs::Result<()> {
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    // Get codec engines information
    let mut codec_engines = CodecEngines::default();

    // Update codec engines with available encoders and decoders
    AudioEncoder::update_codec_engines(&mut codec_engines);
    VideoEncoder::update_codec_engines(&mut codec_engines, VideoDecoderOptions::default());
    // Add decoder updates if available in your codebase

    // 利用可能なコーデック情報を収集
    let mut codecs = Vec::new();

    // 音声コーデック
    codecs.push(CodecInfo {
        name: CodecName::Opus,
        encoders: get_engines_for_codec(&codec_engines, CodecName::Opus, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::Opus, false),
    });

    #[cfg(any(target_os = "macos", feature = "fdk-aac"))]
    codecs.push(CodecInfo {
        name: CodecName::Aac,
        encoders: get_engines_for_codec(&codec_engines, CodecName::Aac, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::Aac, false),
    });

    #[cfg(not(any(target_os = "macos", feature = "fdk-aac")))]
    codecs.push(CodecInfo {
        name: CodecName::Aac,
        codec_type: "audio".to_string(),
        encoders: BTreeSet::new(),
        decoders: BTreeSet::new(),
    });

    // 映像コーデック
    codecs.push(CodecInfo {
        name: CodecName::Vp8,
        encoders: get_engines_for_codec(&codec_engines, CodecName::Vp8, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::Vp8, false),
    });

    codecs.push(CodecInfo {
        name: CodecName::Vp9,
        encoders: get_engines_for_codec(&codec_engines, CodecName::Vp9, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::Vp9, false),
    });

    codecs.push(CodecInfo {
        name: CodecName::H264,
        encoders: get_engines_for_codec(&codec_engines, CodecName::H264, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::H264, false),
    });

    #[cfg(target_os = "macos")]
    codecs.push(CodecInfo {
        name: CodecName::H265,
        encoders: get_engines_for_codec(&codec_engines, CodecName::H265, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::H265, false),
    });

    #[cfg(not(target_os = "macos"))]
    codecs.push(CodecInfo {
        name: CodecName::H265,
        encoders: BTreeSet::new(),
        decoders: BTreeSet::new(),
    });

    codecs.push(CodecInfo {
        name: CodecName::Av1,
        encoders: get_engines_for_codec(&codec_engines, CodecName::Av1, true),
        decoders: get_engines_for_codec(&codec_engines, CodecName::Av1, false),
    });

    // JSON形式で出力
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
    encoders: BTreeSet<EngineName>,
    decoders: BTreeSet<EngineName>,
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
            f.member("encoders", &self.encoders)?;
            f.member("decoders", &self.decoders)
        })
    }
}

// Helper function to extract engines for a specific codec
fn get_engines_for_codec(
    codec_engines: &CodecEngines,
    codec: CodecName,
    is_encoder: bool,
) -> BTreeSet<EngineName> {
    // This function would need to be implemented based on how CodecEngines works
    // You'll need to check the CodecEngines implementation to see how to query it
    // This is a placeholder - you'll need to adapt based on the actual API
    BTreeSet::new()
}
