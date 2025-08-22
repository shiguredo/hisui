use crate::audio::AudioData;
use crate::media::{MediaSample, MediaStreamId};
use crate::stats::ProcessorStats;
use crate::video::VideoFrame;

pub trait MediaProcessor {
    fn spec(&self) -> MediaProcessorSpec;

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()>;
    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput>;

    fn set_error(&self) {
        self.spec().stats.set_error();
    }
}

pub struct BoxedMediaProcessor(Box<dyn 'static + Send + MediaProcessor>);

impl BoxedMediaProcessor {
    pub fn new<P: 'static + Send + MediaProcessor>(processor: P) -> Self {
        Self(Box::new(processor))
    }
}

impl std::fmt::Debug for BoxedMediaProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedMediaProcessor")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct MediaProcessorSpec {
    pub input_stream_ids: Vec<MediaStreamId>,
    pub output_stream_ids: Vec<MediaStreamId>,
    pub stats: ProcessorStats,
}

#[derive(Debug)]
pub struct MediaProcessorInput {
    pub stream_id: MediaStreamId,
    pub sample: Option<MediaSample>, // None なら EOS を表す
}

impl MediaProcessorInput {
    pub fn eos(stream_id: MediaStreamId) -> Self {
        Self {
            stream_id,
            sample: None,
        }
    }

    pub fn audio_data(stream_id: MediaStreamId, data: AudioData) -> Self {
        Self {
            stream_id,
            sample: Some(MediaSample::audio_data(data)),
        }
    }

    pub fn video_frame(stream_id: MediaStreamId, frame: VideoFrame) -> Self {
        Self {
            stream_id,
            sample: Some(MediaSample::video_frame(frame)),
        }
    }
}

#[derive(Debug)]
pub enum MediaProcessorOutput {
    Processed {
        stream_id: MediaStreamId,
        sample: MediaSample,
    },
    Pending {
        awaiting_stream_id: MediaStreamId,
    },
    Finished,
}

impl MediaProcessorOutput {
    pub fn pending(awaiting_stream_id: MediaStreamId) -> Self {
        Self::Pending { awaiting_stream_id }
    }

    pub fn audio_data(stream_id: MediaStreamId, data: AudioData) -> Self {
        Self::Processed {
            stream_id,
            sample: MediaSample::audio_data(data),
        }
    }

    pub fn video_frame(stream_id: MediaStreamId, frame: VideoFrame) -> Self {
        Self::Processed {
            stream_id,
            sample: MediaSample::video_frame(frame),
        }
    }
}
