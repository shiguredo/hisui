use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use orfail::OrFail;

use crate::decoder::{AudioDecoder, VideoDecoder};
use crate::json::JsonObject;
use crate::layout::Resolution;
use crate::layout_region::RawRegion;
use crate::media::{MediaStreamName, MediaStreamNameRegistry};
use crate::metadata::{ContainerFormat, SourceId, SourceInfo};
use crate::mixer_audio::AudioMixer;
use crate::mixer_video::{VideoMixer, VideoMixerSpec};
use crate::processor::BoxedMediaProcessor;
use crate::reader::{AudioReader, VideoReader};
use crate::types::TimeOffset;
use crate::video::FrameRate;

const ONE_DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone)]
pub enum PipelineComponent {
    AudioReader {
        input_file: PathBuf,
        output_stream: MediaStreamName,
        start_time: TimeOffset,
    },
    VideoReader {
        input_file: PathBuf,
        output_stream: MediaStreamName,
        start_time: TimeOffset,
    },
    // MEMO: Sora 固有のコンポーネントとして `Archive{Audio,Video}Reader` や `SplitArchive{Audio,Video}Reader` があるとよさそう
    AudioDecoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
    },
    VideoDecoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
    },
    AudioMixer {
        input_stream: Vec<MediaStreamName>,
        output_stream: MediaStreamName,
    },
    VideoMixer {
        input_stream: Vec<MediaStreamName>,
        output_stream: MediaStreamName,
        resolution: Resolution, // TODO: optional にする
        video_layout: BTreeMap<String, RawRegion>,
    },
    AudioEncoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
    },
    VideoEncoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
        // TODO: codec: CodecName
        // TODO: libvpx_vp9_encode_params, etc
    },
    Mp4Writer {
        input_stream: Vec<MediaStreamName>,
        output_file: PathBuf,
    },
    PluginCommand {
        command: PathBuf,
        args: Vec<String>,
        input_stream: Vec<MediaStreamName>,
        output_stream: Vec<MediaStreamName>,
    },
}

impl PipelineComponent {
    pub fn create_processor(
        &self,
        registry: &mut MediaStreamNameRegistry,
    ) -> orfail::Result<BoxedMediaProcessor> {
        match self {
            Self::AudioReader {
                input_file,
                output_stream,
                start_time,
            } => {
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;
                let source_id = output_stream.to_source_id();
                let format = ContainerFormat::from_path(input_file).or_fail()?;
                let reader = AudioReader::new(
                    output_stream_id,
                    source_id,
                    format,
                    start_time.get(),
                    vec![input_file.clone()],
                )
                .or_fail()?;
                Ok(BoxedMediaProcessor::new(reader))
            }
            Self::VideoReader {
                input_file,
                output_stream,
                start_time,
            } => {
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;
                let source_id = output_stream.to_source_id();
                let format = ContainerFormat::from_path(input_file).or_fail()?;
                let reader = VideoReader::new(
                    output_stream_id,
                    source_id,
                    format,
                    start_time.get(),
                    vec![input_file.clone()],
                )
                .or_fail()?;
                Ok(BoxedMediaProcessor::new(reader))
            }
            Self::AudioDecoder {
                input_stream,
                output_stream,
            } => {
                let input_stream_id = registry.get_id(input_stream).or_fail()?;
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;
                let processor =
                    AudioDecoder::new_opus(input_stream_id, output_stream_id).or_fail()?;
                Ok(BoxedMediaProcessor::new(processor))
            }
            Self::VideoDecoder {
                input_stream,
                output_stream,
            } => {
                // TODO: openh264 を指定できるようにする
                let input_stream_id = registry.get_id(input_stream).or_fail()?;
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;
                let options = Default::default();
                let processor = VideoDecoder::new(input_stream_id, output_stream_id, options);
                Ok(BoxedMediaProcessor::new(processor))
            }
            Self::AudioMixer {
                input_stream,
                output_stream,
            } => {
                let input_stream_ids = input_stream
                    .iter()
                    .map(|name| registry.get_id(name).or_fail())
                    .collect::<orfail::Result<_>>()?;
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;
                let trim_spans = Default::default();
                let processor = AudioMixer::new(trim_spans, input_stream_ids, output_stream_id);
                Ok(BoxedMediaProcessor::new(processor))
            }
            Self::VideoMixer {
                input_stream,
                output_stream,
                resolution,
                video_layout,
            } => {
                let input_stream_ids: Vec<_> = input_stream
                    .iter()
                    .map(|name| registry.get_id(name).or_fail())
                    .collect::<orfail::Result<_>>()?;
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;
                let resolution = *resolution;
                let mut dummy_sources = BTreeMap::new();

                let resolver = |_base: &Path,
                                sources: &[PathBuf],
                                sources_excluded: &[PathBuf]|
                 -> orfail::Result<Vec<(SourceInfo, PathBuf)>> {
                    // TODO: いろいろとちゃんとする
                    sources_excluded.is_empty().or_fail_with(|()| {
                        "not supported yet: non empty 'sources_excluded'".to_owned()
                    })?;

                    fn source_info(id: &str) -> SourceInfo {
                        // ID 以外のメタデータはトリム周り以外には影響しないので、ダミー値でいい
                        SourceInfo {
                            id: SourceId::new(id),
                            format: ContainerFormat::Mp4,
                            audio: true,
                            video: true,
                            start_timestamp: Duration::ZERO,
                            stop_timestamp: ONE_DAY,
                        }
                    }

                    let mut resolved = Vec::new();
                    for source in sources {
                        let s = source.display().to_string();
                        if s == "*" {
                            resolved.extend(
                                input_stream
                                    .iter()
                                    .map(|name| (source_info(name.get()), source.clone())),
                            );
                        } else if s.contains('*') {
                            return Err(orfail::Failure::new(format!("not supported yet: {s:?}")));
                        } else {
                            (input_stream.iter().any(|name| name.get() == s))
                                .or_fail_with(|()| format!("unknown source ID: {s:?}"))?;
                            resolved.push((source_info(&s), source.clone()));
                        }
                    }

                    Ok(resolved)
                };

                let spec = VideoMixerSpec {
                    trim_spans: Default::default(),
                    resolution,
                    frame_rate: FrameRate::FPS_25,
                    // TODO: z-pos を考慮する
                    regions: video_layout
                        .iter()
                        .map(|(_name, raw_region)| {
                            Ok(raw_region.clone().into_region(
                                &Path::new("/dummy/"),
                                &mut dummy_sources,
                                Some(resolution),
                                resolver,
                            )?)
                        })
                        .collect::<orfail::Result<_>>()
                        .or_fail()?,
                };
                let processor = VideoMixer::new(spec, input_stream_ids, output_stream_id);
                Ok(BoxedMediaProcessor::new(processor))
            }
            Self::AudioEncoder {
                input_stream,
                output_stream,
            } => {
                let input_stream_id = registry.get_id(input_stream).or_fail()?;
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;

                // TODO: 変更可能にする
                let codec = crate::types::CodecName::Opus;
                let bitrate =
                    std::num::NonZeroUsize::new(crate::audio::DEFAULT_BITRATE).or_fail()?;

                let processor = crate::encoder::AudioEncoder::new(
                    codec,
                    bitrate,
                    input_stream_id,
                    output_stream_id,
                )
                .or_fail()?;

                Ok(BoxedMediaProcessor::new(processor))
            }
            Self::VideoEncoder {
                input_stream,
                output_stream,
            } => {
                let input_stream_id = registry.get_id(input_stream).or_fail()?;
                let output_stream_id = registry.register_name(output_stream.clone()).or_fail()?;

                // TODO: ちゃんとする
                let options = crate::encoder::VideoEncoderOptions {
                    codec: crate::types::CodecName::Vp8,
                    bitrate: 1_000_000, // 1 Mbps
                    width: crate::types::EvenUsize::new(1280).or_fail()?,
                    height: crate::types::EvenUsize::new(720).or_fail()?,
                    frame_rate: crate::video::FrameRate::FPS_25,
                    encode_params: Default::default(),
                };

                let processor = crate::encoder::VideoEncoder::new(
                    &options,
                    input_stream_id,
                    output_stream_id,
                    None,
                )
                .or_fail()?;

                Ok(BoxedMediaProcessor::new(processor))
            }
            Self::Mp4Writer {
                input_stream,
                output_file,
            } => {
                let input_stream_ids: Vec<_> = input_stream
                    .iter()
                    .map(|name| registry.get_id(name).or_fail())
                    .collect::<orfail::Result<_>>()?;

                // TODO: ちゃんとする
                let input_audio_stream_id = input_stream_ids.get(0).copied();
                let input_video_stream_id = input_stream_ids.get(1).copied();
                let options = crate::writer_mp4::Mp4WriterOptions {
                    resolution: Resolution::new(1280, 720).or_fail()?,
                    duration: ONE_DAY,
                    frame_rate: FrameRate::FPS_25,
                };

                let processor = crate::writer_mp4::Mp4Writer::new(
                    output_file,
                    &options,
                    input_audio_stream_id,
                    input_video_stream_id,
                )
                .or_fail()?;

                Ok(BoxedMediaProcessor::new(processor))
            }
            _ => todo!(),
        }
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for PipelineComponent {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let obj = JsonObject::new(value)?;
        let component_type: String = obj.get_required("type")?;

        match component_type.as_str() {
            "audio_reader" => Ok(Self::AudioReader {
                input_file: obj.get_required("input_file")?,
                output_stream: obj.get_required("output_stream")?,
                start_time: obj.get("start_time")?.unwrap_or_default(),
            }),
            "video_reader" => Ok(Self::VideoReader {
                input_file: obj.get_required("input_file")?,
                output_stream: obj.get_required("output_stream")?,
                start_time: obj.get("start_time")?.unwrap_or_default(),
            }),
            "audio_decoder" => Ok(Self::AudioDecoder {
                input_stream: obj.get_required("input_stream")?,
                output_stream: obj.get_required("output_stream")?,
            }),
            "video_decoder" => Ok(Self::VideoDecoder {
                input_stream: obj.get_required("input_stream")?,
                output_stream: obj.get_required("output_stream")?,
            }),
            "audio_mixer" => Ok(Self::AudioMixer {
                input_stream: obj.get_required("input_stream")?,
                output_stream: obj.get_required("output_stream")?,
            }),
            "video_mixer" => Ok(Self::VideoMixer {
                input_stream: obj.get_required("input_stream")?,
                output_stream: obj.get_required("output_stream")?,
                resolution: obj.get_required("resolution")?,
                video_layout: obj.get_required("video_layout")?,
            }),
            "audio_encoder" => Ok(Self::AudioEncoder {
                input_stream: obj.get_required("input_stream")?,
                output_stream: obj.get_required("output_stream")?,
            }),
            "video_encoder" => Ok(Self::VideoEncoder {
                input_stream: obj.get_required("input_stream")?,
                output_stream: obj.get_required("output_stream")?,
            }),
            "mp4_writer" => Ok(Self::Mp4Writer {
                input_stream: obj.get_required("input_stream")?,
                output_file: obj.get_required("output_file")?,
            }),
            "plugin_command" => Ok(Self::PluginCommand {
                command: obj.get_required("command")?,
                args: obj.get("args")?.unwrap_or_default(),
                input_stream: obj.get("input_stream")?.unwrap_or_default(),
                output_stream: obj.get("output_stream")?.unwrap_or_default(),
            }),
            unknown => Err(value.invalid(format!("unknown pipeline component type: {unknown:?}"))),
        }
    }
}
