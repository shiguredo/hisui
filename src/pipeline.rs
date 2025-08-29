use std::path::PathBuf;

use orfail::OrFail;

use crate::json::JsonObject;
use crate::media::{MediaStreamName, MediaStreamNameRegistry};
use crate::metadata::ContainerFormat;
use crate::processor::BoxedMediaProcessor;
use crate::reader::AudioReader;
use crate::types::TimeOffset;

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
    AudioDecoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
    },
    // MEMO: Sora 固有のコンポーネントとして `Archive{Audio,Video}Reader` や `SplitArchive{Audio,Video}Reader` があるとよさそう
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
        // video_layout: ()
    },
    AudioEncoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
    },
    VideoEncoder {
        input_stream: MediaStreamName,
        output_stream: MediaStreamName,
        // codec: String,
        // libvpx_vp9_encode_params: ()
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
