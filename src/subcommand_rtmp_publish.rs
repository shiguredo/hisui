use std::path::PathBuf;

use shiguredo_openh264::Openh264Library;

use crate::{
    decoder::{AudioDecoder, VideoDecoder},
    encoder::{AudioEncoder, VideoEncoder},
    types::{CodecName, EngineName},
};

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパス")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let openh264_lib = openh264
        .as_ref()
        .and_then(|path| Openh264Library::load(path).ok());
    let is_openh264_available = openh264_lib.is_some();

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

    let mut engines = vec![
        EngineInfo {
            repository: Some(shiguredo_opus::BUILD_REPOSITORY),
            build_version: Some(shiguredo_opus::BUILD_VERSION),
            ..EngineInfo::new(EngineName::Opus)
        },
        EngineInfo {
            repository: Some(shiguredo_dav1d::BUILD_REPOSITORY),
            build_version: Some(shiguredo_dav1d::BUILD_VERSION),
            ..EngineInfo::new(EngineName::Dav1d)
        },
        EngineInfo {
            repository: Some(shiguredo_svt_av1::BUILD_REPOSITORY),
            build_version: Some(shiguredo_svt_av1::BUILD_VERSION),
            ..EngineInfo::new(EngineName::SvtAv1)
        },
    ];
    #[cfg(feature = "libvpx")]
    {
        engines.push(EngineInfo {
            repository: Some(shiguredo_libvpx::BUILD_REPOSITORY),
            build_version: Some(shiguredo_libvpx::BUILD_VERSION),
            ..EngineInfo::new(EngineName::Libvpx)
        });
    }
    #[cfg(feature = "fdk-aac")]
    {
        engines.push(EngineInfo {
            ..EngineInfo::new(EngineName::FdkAac)
        });
    }
    #[cfg(target_os = "macos")]
    {
        engines.push(EngineInfo {
            ..EngineInfo::new(EngineName::AudioToolbox)
        });
        engines.push(EngineInfo {
            ..EngineInfo::new(EngineName::VideoToolbox)
        });
    }
    #[cfg(feature = "nvcodec")]
    if shiguredo_nvcodec::is_cuda_library_available() {
        engines.push(EngineInfo {
            build_version: Some(shiguredo_nvcodec::BUILD_VERSION),
            ..EngineInfo::new(EngineName::Nvcodec)
        });
    }
    if let Some(lib) = openh264_lib {
        engines.push(EngineInfo {
            repository: Some(shiguredo_openh264::BUILD_REPOSITORY),
            shared_library_path: Some(lib.path().to_path_buf()),
            build_version: Some(shiguredo_openh264::BUILD_VERSION),
            runtime_version: Some(lib.runtime_version()),
            ..EngineInfo::new(EngineName::Openh264)
        });
    }
    engines.sort_by_key(|x| x.name);

    println!(
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.object(|f| {
                f.member("codecs", &codecs)?;
                f.member("engines", &engines)
            })
        })
    );

    Ok(())
}

#[derive(Debug)]
struct EngineInfo {
    name: EngineName,
    repository: Option<&'static str>,
    shared_library_path: Option<PathBuf>,
    build_version: Option<&'static str>,
    runtime_version: Option<String>,
}

impl EngineInfo {
    fn new(name: EngineName) -> Self {
        Self {
            name,
            repository: None,
            shared_library_path: None,
            build_version: None,
            runtime_version: None,
        }
    }
}

impl nojson::DisplayJson for EngineInfo {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("name", self.name)?;
            if let Some(v) = self.repository {
                f.member("repository", v)?;
            }
            if let Some(v) = &self.shared_library_path {
                f.member("shared_library_path", v)?;
            }
            if let Some(v) = self.build_version {
                f.member("build_version", v)?;
            }
            if let Some(v) = &self.runtime_version {
                f.member("runtime_version", v)?;
            }
            Ok(())
        })
    }
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
                "decoders",
                nojson::json(|f| {
                    f.set_indent_size(0);
                    f.value(&self.decoders)?;
                    f.set_indent_size(2);
                    Ok(())
                }),
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
            Ok(())
        })
    }
}
