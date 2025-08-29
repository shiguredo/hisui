use std::path::PathBuf;

use crate::media::MediaStreamName;

#[derive(Debug, Clone)]
pub enum PipelineComponent {
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
