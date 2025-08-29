use std::path::PathBuf;
use std::time::Duration;

use crate::media::MediaStreamName;

#[derive(Debug, Clone)]
pub enum PipelineComponent {
    AudioReader {
        input_file: PathBuf,
        output_stream: MediaStreamName,
        start_time: Duration,
    },
    VideoReader {
        input_file: PathBuf,
        output_stream: MediaStreamName,
        start_time: Duration,
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
