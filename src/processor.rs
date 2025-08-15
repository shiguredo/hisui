use crate::media::{MediaStreamId, SharedMediaSample};
use crate::stats::ProcessorStats;

pub trait MediaProcessor {
    fn process(&mut self, input: MediaProcessorInput) -> orfail::Result<()>;
    fn poll_output(&mut self) -> orfail::Result<MediaProcessorOutput>;
    fn stats(&self) -> ProcessorStats;
}

#[derive(Debug)]
pub struct MediaProcessorInput {
    pub stream_id: MediaStreamId,
    pub sample: Option<SharedMediaSample>,
}

#[derive(Debug)]
pub enum MediaProcessorOutput {
    Processed {
        stream_id: MediaStreamId,
        sample: SharedMediaSample,
    },
    Pending {
        awaiting_stream_id: MediaStreamId,
    },
    Finished,
}
