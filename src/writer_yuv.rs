use std::{fs::File, io::Write, path::Path};

use orfail::OrFail;

use crate::{
    media::MediaStreamId,
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::ProcessorStats,
    video::VideoFormat,
};

#[derive(Debug)]
pub struct YuvWriter {
    input_stream_id: MediaStreamId,
    eos: bool,
    file: File,
}

impl YuvWriter {
    pub fn new<P: AsRef<Path>>(input_stream_id: MediaStreamId, path: P) -> orfail::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .or_fail_with(|e| format!("{e}: {}", path.as_ref().display()))?;
        Ok(Self {
            input_stream_id,
            eos: false,
            file,
        })
    }
}

impl MediaProcessor for YuvWriter {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: vec![self.input_stream_id],
            output_stream_ids: Vec::new(),
            stats: ProcessorStats::other("yuv_writer"),
            workload_hint: MediaProcessorWorkloadHint::WRITER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        if let Some(sample) = input.sample {
            let frame = sample.expect_video_frame().or_fail()?;
            matches!(frame.format, VideoFormat::I420).or_fail()?;
            self.file.write_all(&frame.data).or_fail()?;
        } else {
            self.eos = true;
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        if self.eos {
            Ok(MediaProcessorOutput::Finished)
        } else {
            Ok(MediaProcessorOutput::pending(self.input_stream_id))
        }
    }
}
