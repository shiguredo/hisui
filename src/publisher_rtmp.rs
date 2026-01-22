use crate::{
    media::{MediaSample, MediaStreamId},
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::ProcessorStats,
};

#[derive(Debug)]
pub struct RtmpPublisher {
    input_audio_stream_id: Option<MediaStreamId>,
    input_video_stream_id: Option<MediaStreamId>,
}

impl MediaProcessor for RtmpPublisher {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self
                .input_audio_stream_id
                .into_iter()
                .chain(self.input_video_stream_id)
                .collect(),
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("rtmp-publisher"), // TODO: 専用の構造体を用意する
            workload_hint: MediaProcessorWorkloadHint::WRITER, // TODO: 非同期 I/O 用の値を追加する
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        /* match input.sample {
            Some(MediaSample::Audio(sample))
                if Some(input.stream_id) == self.input_audio_stream_id =>
            {
                self.input_audio_queue.push_back(sample);
            }
            None if Some(input.stream_id) == self.input_audio_stream_id => {
                self.input_audio_stream_id = None;
            }
            Some(MediaSample::Video(sample))
                if Some(input.stream_id) == self.input_video_stream_id =>
            {
                self.input_video_queue.push_back(sample);
            }
            None if Some(input.stream_id) == self.input_video_stream_id => {
                self.input_video_stream_id = None;
            }
            _ => return Err(orfail::Failure::new("BUG: unexpected input stream")),
        }*/
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        /*loop {
            if let Some(id) = self.input_video_stream_id
                && self.input_video_queue.is_empty()
            {
                return Ok(MediaProcessorOutput::pending(id));
            } else if let Some(id) = self.input_audio_stream_id
                && self.input_audio_queue.is_empty()
            {
                return Ok(MediaProcessorOutput::pending(id));
            }

            let audio_timestamp = self.input_audio_queue.front().map(|x| x.timestamp);
            let video_timestamp = self.input_video_queue.front().map(|x| x.timestamp);

            let in_progress = self
                .handle_next_audio_and_video(audio_timestamp, video_timestamp)
                .or_fail()?;

            if !in_progress {
                return Ok(MediaProcessorOutput::Finished);
            }
        }*/
        todo!()
    }
}
