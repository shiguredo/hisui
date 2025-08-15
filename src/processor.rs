use crate::media::{MediaSample, MediaStreamId};
use crate::stats::ProcessorStats;

pub trait MediaProcessor {
    fn spec(&self) -> MediaProcessorSpec;
    fn process(&mut self, input: MediaProcessorInput) -> orfail::Result<()>;
    fn poll_output(&mut self) -> orfail::Result<MediaProcessorOutput>;
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
    pub sample: Option<MediaSample>,
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
