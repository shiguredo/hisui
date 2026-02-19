use std::path::{Path, PathBuf};

use crate::{
    Ack, AudioData, Error, MessageSender, ProcessorHandle, Result, TrackId, VideoFrame,
    metadata::SourceId,
    reader_webm::{WebmAudioReader, WebmVideoReader},
};

const MAX_NOACKED_COUNT: u64 = 100;

#[derive(Debug, Clone, Default)]
pub struct WebmFileReaderOptions {
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

#[derive(Debug)]
pub struct WebmFileReader {
    path: PathBuf,
    options: WebmFileReaderOptions,
    audio_sender: Option<TrackSender>,
    video_sender: Option<TrackSender>,
}

impl WebmFileReader {
    pub fn new<P: AsRef<Path>>(path: P, options: WebmFileReaderOptions) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            options,
            audio_sender: None,
            video_sender: None,
        }
    }

    pub async fn run(mut self, handle: ProcessorHandle) -> Result<()> {
        (self.audio_sender, self.video_sender) = self.build_track_senders(&handle).await?;
        handle.notify_ready();

        if self.audio_sender.is_none() && self.video_sender.is_none() {
            return Ok(());
        }
        handle.wait_subscribers_ready().await?;

        let should_stop = self.run_once().await?;
        if should_stop {
            return Ok(());
        }

        Self::send_eos(&mut self.audio_sender, &mut self.video_sender).await;
        Ok(())
    }

    async fn build_track_senders(
        &mut self,
        handle: &ProcessorHandle,
    ) -> Result<(Option<TrackSender>, Option<TrackSender>)> {
        let audio_sender = if let Some(track_id) = self.options.audio_track_id.take() {
            let sender = handle.publish_track(track_id).await?;
            Some(TrackSender::new(sender))
        } else {
            None
        };

        let video_sender = if let Some(track_id) = self.options.video_track_id.take() {
            let sender = handle.publish_track(track_id).await?;
            Some(TrackSender::new(sender))
        } else {
            None
        };

        Ok((audio_sender, video_sender))
    }

    async fn run_once(&mut self) -> Result<bool> {
        let mut state = ReaderState::open(
            &self.path,
            self.audio_sender.is_some(),
            self.video_sender.is_some(),
        )?;

        if !state.has_enabled_readers() {
            return Ok(false);
        }

        while let Some(sample) = state.next_sample()? {
            let should_stop = self.handle_sample(sample).await?;
            if should_stop {
                return Ok(true);
            }
        }

        Ok(false)
    }

    async fn handle_sample(&mut self, sample: PendingSample) -> Result<bool> {
        match sample {
            PendingSample::Audio(data) => {
                if let Some(sender) = self.audio_sender.as_mut()
                    && !sender.send_audio(data).await
                {
                    return Ok(true);
                }
            }
            PendingSample::Video(frame) => {
                if let Some(sender) = self.video_sender.as_mut()
                    && !sender.send_video(frame).await
                {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    async fn send_eos(
        audio_sender: &mut Option<TrackSender>,
        video_sender: &mut Option<TrackSender>,
    ) {
        if let Some(sender) = audio_sender.as_mut() {
            sender.send_eos();
        }
        if let Some(sender) = video_sender.as_mut() {
            sender.send_eos();
        }
    }
}

#[derive(Debug)]
struct ReaderState {
    audio_reader: Option<WebmAudioReader>,
    video_reader: Option<WebmVideoReader>,
    next_audio: Option<AudioData>,
    next_video: Option<VideoFrame>,
}

impl ReaderState {
    fn open<P: AsRef<Path>>(path: P, enable_audio: bool, enable_video: bool) -> Result<Self> {
        let source_id = SourceId::new("webm_file_reader");

        let audio_reader = if enable_audio {
            Some(
                WebmAudioReader::new(source_id.clone(), path.as_ref())
                    .map_err(|e| Error::new(e.to_string()))?,
            )
        } else {
            None
        };

        let video_reader = if enable_video {
            Some(
                WebmVideoReader::new(source_id, path.as_ref())
                    .map_err(|e| Error::new(e.to_string()))?,
            )
        } else {
            None
        };

        let mut state = Self {
            audio_reader,
            video_reader,
            next_audio: None,
            next_video: None,
        };
        state.fill_next_audio()?;
        state.fill_next_video()?;
        Ok(state)
    }

    fn has_enabled_readers(&self) -> bool {
        self.audio_reader.is_some() || self.video_reader.is_some()
    }

    fn fill_next_audio(&mut self) -> Result<()> {
        if self.next_audio.is_some() {
            return Ok(());
        }
        if let Some(reader) = self.audio_reader.as_mut() {
            self.next_audio = reader
                .next()
                .transpose()
                .map_err(|e| Error::new(e.to_string()))?;
        }
        Ok(())
    }

    fn fill_next_video(&mut self) -> Result<()> {
        if self.next_video.is_some() {
            return Ok(());
        }
        if let Some(reader) = self.video_reader.as_mut() {
            self.next_video = reader
                .next()
                .transpose()
                .map_err(|e| Error::new(e.to_string()))?;
        }
        Ok(())
    }

    fn next_sample(&mut self) -> Result<Option<PendingSample>> {
        self.fill_next_audio()?;
        self.fill_next_video()?;

        let next_kind = match (&self.next_audio, &self.next_video) {
            (None, None) => return Ok(None),
            (Some(_), None) => NextKind::Audio,
            (None, Some(_)) => NextKind::Video,
            (Some(audio), Some(video)) => {
                if audio.timestamp <= video.timestamp {
                    NextKind::Audio
                } else {
                    NextKind::Video
                }
            }
        };

        let sample = match next_kind {
            NextKind::Audio => {
                let sample = self
                    .next_audio
                    .take()
                    .ok_or_else(|| Error::new("audio sample is missing unexpectedly"))?;
                self.fill_next_audio()?;
                PendingSample::Audio(sample)
            }
            NextKind::Video => {
                let sample = self
                    .next_video
                    .take()
                    .ok_or_else(|| Error::new("video sample is missing unexpectedly"))?;
                self.fill_next_video()?;
                PendingSample::Video(sample)
            }
        };

        Ok(Some(sample))
    }
}

#[derive(Debug, Clone, Copy)]
enum NextKind {
    Audio,
    Video,
}

#[derive(Debug)]
enum PendingSample {
    Audio(AudioData),
    Video(VideoFrame),
}

#[derive(Debug)]
struct TrackSender {
    sender: MessageSender,
    ack: Option<Ack>,
    noacked_sent: u64,
}

impl TrackSender {
    fn new(mut sender: MessageSender) -> Self {
        let ack = Some(sender.send_syn());
        Self {
            sender,
            ack,
            noacked_sent: 0,
        }
    }

    async fn prepare_send(&mut self) {
        if self.noacked_sent > MAX_NOACKED_COUNT {
            if let Some(ack) = self.ack.take() {
                ack.await;
            }
            self.ack = Some(self.sender.send_syn());
            self.noacked_sent = 0;
        }
    }

    async fn send_audio(&mut self, data: AudioData) -> bool {
        self.prepare_send().await;
        let ok = self.sender.send_audio(data);
        if ok {
            self.noacked_sent += 1;
        }
        ok
    }

    async fn send_video(&mut self, frame: VideoFrame) -> bool {
        self.prepare_send().await;
        let ok = self.sender.send_video(frame);
        if ok {
            self.noacked_sent += 1;
        }
        ok
    }

    fn send_eos(&mut self) {
        let _ = self.sender.send_eos();
    }
}
