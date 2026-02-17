use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

use shiguredo_mp4::{TrackKind, boxes::SampleEntry, demux::Mp4FileDemuxer};

use crate::audio::AudioFormat;
use crate::video::VideoFormat;
use crate::{Ack, AudioData, Error, MessageSender, ProcessorHandle, Result, TrackId, VideoFrame};

const MAX_NOACKED_COUNT: u64 = 100;

#[derive(Debug, Clone, Default)]
pub struct Mp4FileReaderOptions {
    pub realtime: bool,
    pub loop_playback: bool,
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

#[derive(Debug)]
pub struct Mp4FileReader {
    path: PathBuf,
    options: Mp4FileReaderOptions,
    audio_sender: Option<TrackSender>,
    video_sender: Option<TrackSender>,
    base_offset: Duration,
    last_emitted_end: Duration,
    start_instant: tokio::time::Instant,
    emitted_in_loop: bool,
}

impl Mp4FileReader {
    pub fn new<P: AsRef<Path>>(path: P, options: Mp4FileReaderOptions) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            options,
            audio_sender: None,
            video_sender: None,
            base_offset: Duration::ZERO,
            last_emitted_end: Duration::ZERO,
            start_instant: tokio::time::Instant::now(),
            emitted_in_loop: false,
        })
    }

    pub async fn run(mut self, handle: ProcessorHandle) -> Result<()> {
        let loop_enabled = self.resolve_loop_enabled();
        (self.audio_sender, self.video_sender) = self.build_track_senders(handle).await?;

        if self.audio_sender.is_none() && self.video_sender.is_none() {
            return Ok(());
        }

        self.start_instant = tokio::time::Instant::now();

        let should_stop = self.run_loop(loop_enabled).await?;
        if should_stop {
            return Ok(());
        }

        Self::send_eos(&mut self.audio_sender, &mut self.video_sender).await;

        Ok(())
    }

    fn resolve_loop_enabled(&self) -> bool {
        let mut loop_enabled = self.options.loop_playback;
        if loop_enabled && !self.options.realtime {
            tracing::warn!("Loop playback is ignored because realtime is disabled");
            loop_enabled = false;
        }
        loop_enabled
    }

    async fn build_track_senders(
        &mut self,
        handle: ProcessorHandle,
    ) -> Result<(Option<TrackSender>, Option<TrackSender>)> {
        let audio_sender = if let Some(track_id) = self.options.audio_track_id.take() {
            let sender = handle.publish_track(track_id).await?;
            Some(TrackSender::new(sender).await)
        } else {
            None
        };

        let video_sender = if let Some(track_id) = self.options.video_track_id.take() {
            let sender = handle.publish_track(track_id).await?;
            Some(TrackSender::new(sender).await)
        } else {
            None
        };

        Ok((audio_sender, video_sender))
    }

    async fn run_loop(&mut self, loop_enabled: bool) -> Result<bool> {
        loop {
            let mut state = ReaderState::open(
                &self.path,
                self.audio_sender.is_some(),
                self.video_sender.is_some(),
            )?;
            if state.audio_track_id.is_none() && state.video_track_id.is_none() {
                break;
            }

            self.emitted_in_loop = false;
            while let Some(sample) = state.demuxer.next_sample()? {
                let context = SampleContext::from_sample(&sample);
                let should_stop = self.handle_sample(&mut state, context).await?;
                if should_stop {
                    return Ok(true);
                }
            }

            if loop_enabled {
                if !self.emitted_in_loop {
                    tracing::warn!("Loop playback stopped because no samples were read");
                    break;
                }
                self.base_offset = self.last_emitted_end;
                continue;
            }
            break;
        }

        Ok(false)
    }

    async fn handle_sample(
        &mut self,
        state: &mut ReaderState,
        context: SampleContext,
    ) -> Result<bool> {
        match context.track_kind {
            TrackKind::Audio => self.handle_audio_sample(state, context).await,
            TrackKind::Video => self.handle_video_sample(state, context).await,
        }
    }

    async fn handle_audio_sample(
        &mut self,
        state: &mut ReaderState,
        context: SampleContext,
    ) -> Result<bool> {
        if !state.is_audio_enabled(context.track_id) {
            return Ok(false);
        }

        if let Some(entry) = &context.sample_entry {
            state.update_audio_format(entry)?;
        }

        let data = state.read_sample_data(context.data_offset, context.data_size)?;
        let (timestamp, duration) =
            calculate_timestamps(context.timescale, context.timestamp, context.duration);
        let effective_timestamp = self.base_offset + timestamp;

        if self.options.realtime {
            let target = self.start_instant + effective_timestamp;
            tokio::time::sleep_until(target).await;
        }

        let audio_data = AudioData {
            source_id: None,
            data,
            format: state.audio_format,
            stereo: state.audio_stereo,
            sample_rate: state.audio_sample_rate,
            timestamp: effective_timestamp,
            duration,
            sample_entry: context.sample_entry,
        };

        if let Some(sender) = self.audio_sender.as_mut() {
            if !sender.send_audio(audio_data).await {
                return Ok(true);
            }
            self.emitted_in_loop = true;
            let end = effective_timestamp + duration;
            if end > self.last_emitted_end {
                self.last_emitted_end = end;
            }
        }

        Ok(false)
    }

    async fn handle_video_sample(
        &mut self,
        state: &mut ReaderState,
        context: SampleContext,
    ) -> Result<bool> {
        if !state.is_video_enabled(context.track_id) {
            return Ok(false);
        }

        if let Some(entry) = &context.sample_entry {
            state.update_video_format(entry)?;
        }

        let data = state.read_sample_data(context.data_offset, context.data_size)?;
        let (timestamp, duration) =
            calculate_timestamps(context.timescale, context.timestamp, context.duration);
        let effective_timestamp = self.base_offset + timestamp;

        if self.options.realtime {
            let target = self.start_instant + effective_timestamp;
            tokio::time::sleep_until(target).await;
        }

        let video_frame = VideoFrame {
            source_id: None,
            data,
            format: state.video_format,
            keyframe: context.keyframe,
            width: state.video_width,
            height: state.video_height,
            timestamp: effective_timestamp,
            duration,
            sample_entry: context.sample_entry,
        };

        if let Some(sender) = self.video_sender.as_mut() {
            if !sender.send_video(video_frame).await {
                return Ok(true);
            }
            self.emitted_in_loop = true;
            let end = effective_timestamp + duration;
            if end > self.last_emitted_end {
                self.last_emitted_end = end;
            }
        }

        Ok(false)
    }

    async fn send_eos(
        audio_sender: &mut Option<TrackSender>,
        video_sender: &mut Option<TrackSender>,
    ) {
        if let Some(sender) = audio_sender.as_mut() {
            sender.send_eos().await;
        }
        if let Some(sender) = video_sender.as_mut() {
            sender.send_eos().await;
        }
    }
}

#[derive(Debug, Clone)]
struct SampleContext {
    track_kind: TrackKind,
    track_id: u32,
    timescale: u32,
    timestamp: u64,
    duration: u64,
    data_offset: u64,
    data_size: usize,
    keyframe: bool,
    sample_entry: Option<SampleEntry>,
}

impl SampleContext {
    fn from_sample(sample: &shiguredo_mp4::demux::Sample<'_>) -> Self {
        Self {
            track_kind: sample.track.kind,
            track_id: sample.track.track_id,
            timescale: sample.track.timescale.get(),
            timestamp: sample.timestamp,
            duration: sample.duration as u64,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            keyframe: sample.keyframe,
            sample_entry: sample.sample_entry.cloned(),
        }
    }
}

#[derive(Debug)]
struct TrackSender {
    sender: MessageSender,
    ack: Option<Ack>,
    noacked_sent: u64,
}

impl TrackSender {
    async fn new(mut sender: MessageSender) -> Self {
        let ack = Some(sender.send_syn().await);
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
            self.ack = Some(self.sender.send_syn().await);
            self.noacked_sent = 0;
        }
    }

    async fn send_audio(&mut self, data: AudioData) -> bool {
        self.prepare_send().await;
        let ok = self.sender.send_audio(data).await;
        if ok {
            self.noacked_sent += 1;
        }
        ok
    }

    async fn send_video(&mut self, frame: VideoFrame) -> bool {
        self.prepare_send().await;
        let ok = self.sender.send_video(frame).await;
        if ok {
            self.noacked_sent += 1;
        }
        ok
    }

    async fn send_eos(&mut self) {
        let _ = self.sender.send_eos().await;
    }
}

#[derive(Debug)]
struct ReaderState {
    path: PathBuf,
    file: File,
    demuxer: Mp4FileDemuxer,
    audio_track_id: Option<u32>,
    video_track_id: Option<u32>,
    audio_format: AudioFormat,
    audio_stereo: bool,
    audio_sample_rate: u16,
    video_format: VideoFormat,
    video_width: usize,
    video_height: usize,
}

impl ReaderState {
    fn open(path: &Path, enable_audio: bool, enable_video: bool) -> Result<Self> {
        let mut file = File::open(path)
            .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
        let mut demuxer = Mp4FileDemuxer::new();
        initialize_mp4_demuxer(&mut file, &mut demuxer, path)?;

        let audio_track_id = if enable_audio {
            select_audio_track(demuxer.clone())?
        } else {
            None
        };
        let video_track_id = if enable_video {
            select_video_track(demuxer.clone())?
        } else {
            None
        };

        Ok(Self {
            path: path.to_path_buf(),
            file,
            demuxer,
            audio_track_id,
            video_track_id,
            audio_format: AudioFormat::Opus,
            audio_stereo: false,
            audio_sample_rate: 0,
            video_format: VideoFormat::Vp8,
            video_width: 0,
            video_height: 0,
        })
    }

    fn is_audio_enabled(&self, track_id: u32) -> bool {
        self.audio_track_id == Some(track_id)
    }

    fn is_video_enabled(&self, track_id: u32) -> bool {
        self.video_track_id == Some(track_id)
    }

    fn update_audio_format(&mut self, sample_entry: &SampleEntry) -> Result<()> {
        let (metadata, format) = match sample_entry {
            SampleEntry::Opus(b) => (&b.audio, AudioFormat::Opus),
            SampleEntry::Mp4a(b) => (&b.audio, AudioFormat::Aac),
            entry => {
                return Err(Error::new(format!("unsupported sample entry: {entry:?}")));
            }
        };

        self.audio_format = format;
        self.audio_stereo = metadata.channelcount != 1;
        self.audio_sample_rate = metadata.samplerate.integer;

        Ok(())
    }

    fn update_video_format(&mut self, sample_entry: &SampleEntry) -> Result<()> {
        let (metadata, format) = match sample_entry {
            SampleEntry::Avc1(b) => (&b.visual, VideoFormat::H264),
            SampleEntry::Hev1(b) => (&b.visual, VideoFormat::H265),
            SampleEntry::Hvc1(b) => (&b.visual, VideoFormat::H265),
            SampleEntry::Vp08(b) => (&b.visual, VideoFormat::Vp8),
            SampleEntry::Vp09(b) => (&b.visual, VideoFormat::Vp9),
            SampleEntry::Av01(b) => (&b.visual, VideoFormat::Av1),
            entry => {
                return Err(Error::new(format!("unsupported sample entry: {entry:?}")));
            }
        };

        self.video_format = format;
        self.video_width = metadata.width as usize;
        self.video_height = metadata.height as usize;

        Ok(())
    }

    fn read_sample_data(&mut self, data_offset: u64, data_size: usize) -> Result<Vec<u8>> {
        let mut data = vec![0; data_size];
        self.file
            .seek(SeekFrom::Start(data_offset))
            .map_err(|e| Error::new(format!("Seek error {}: {e}", self.path.display())))?;
        self.file
            .read_exact(&mut data)
            .map_err(|e| Error::new(format!("Read error {}: {e}", self.path.display())))?;
        Ok(data)
    }
}

fn calculate_timestamps(timescale: u32, timestamp: u64, duration: u64) -> (Duration, Duration) {
    let timestamp = Duration::from_secs(timestamp) / timescale;
    let duration = Duration::from_secs(duration) / timescale;
    (timestamp, duration)
}

/// MP4 ファイルからトラック情報を初期化する
///
/// NOTE: fMP4 には未対応なので、この関数完了後、demuxer はファイル読み込みを要求しない
fn initialize_mp4_demuxer<R: Read + Seek, P: AsRef<Path>>(
    file: &mut R,
    demuxer: &mut Mp4FileDemuxer,
    path: P,
) -> Result<()> {
    // 念のために（壊れたファイルが渡された時のため）、バッファサイズの上限を 100 MB に設定しておく。
    // 正常なファイルの場合には、これは moov ボックスのサイズ上限となるが、
    // 典型的には、100 MB あれば、MP4 ファイル自体としては数百 GB 程度のものを扱えるため、実用上の問題はない想定。
    const MAX_BUF_SIZE: usize = 100 * 1024 * 1024;

    while let Some(required) = demuxer.required_input() {
        let size = required.size.ok_or_else(|| {
            Error::new(format!(
                "MP4 file contains unexpected variable size box {}",
                path.as_ref().display()
            ))
        })?;
        if size > MAX_BUF_SIZE {
            return Err(Error::new(format!(
                "MP4 file contains box larger than maximum allowed size ({size} > {MAX_BUF_SIZE}): {}",
                path.as_ref().display()
            )));
        }

        let mut buf = vec![0; size];
        file.seek(SeekFrom::Start(required.position))
            .map_err(|e| Error::new(format!("Seek error {}: {e}", path.as_ref().display())))?;
        file.read_exact(&mut buf)
            .map_err(|e| Error::new(format!("Read error {}: {e}", path.as_ref().display())))?;
        let input = required.to_input(&buf);
        demuxer.handle_input(input);
    }
    Ok(())
}

/// 音声トラックをチェックして、サポートされているコーデックを持つトラック ID を取得する
fn select_audio_track(mut demuxer: Mp4FileDemuxer) -> Result<Option<u32>> {
    let mut has_audio_track = false;
    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Audio {
            continue;
        }
        has_audio_track = true;

        if let Some(sample_entry) = sample.sample_entry {
            let is_supported = match &sample_entry {
                SampleEntry::Opus(_) => true,
                SampleEntry::Mp4a(mp4a) => is_aac_codec(&mp4a.esds_box),
                _ => false,
            };

            if is_supported {
                return Ok(Some(sample.track.track_id));
            } else {
                tracing::warn!(
                    "Unsupported audio codec in track {}: {:?}",
                    sample.track.track_id,
                    sample_entry
                );
            }
        }
    }

    if has_audio_track {
        tracing::warn!("No supported audio track found in the file");
    } else {
        tracing::warn!("No audio track found in the file");
    }

    Ok(None)
}

/// 映像トラックをチェックして、サポートされているコーデックを持つトラック ID を取得する
fn select_video_track(mut demuxer: Mp4FileDemuxer) -> Result<Option<u32>> {
    let mut has_video_track = false;
    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Video {
            continue;
        }
        has_video_track = true;

        if let Some(sample_entry) = sample.sample_entry {
            let is_supported = matches!(
                sample_entry,
                SampleEntry::Avc1(_)
                    | SampleEntry::Hev1(_)
                    | SampleEntry::Hvc1(_)
                    | SampleEntry::Vp08(_)
                    | SampleEntry::Vp09(_)
                    | SampleEntry::Av01(_)
            );

            if is_supported {
                return Ok(Some(sample.track.track_id));
            } else {
                tracing::warn!(
                    "Unsupported video codec in track {}: {:?}",
                    sample.track.track_id,
                    sample_entry
                );
            }
        }
    }

    if has_video_track {
        tracing::warn!("No supported video track found in the file");
    } else {
        tracing::warn!("No video track found in the file");
    }

    Ok(None)
}

/// AAC コーデックであることを確認する
fn is_aac_codec(esds_box: &shiguredo_mp4::boxes::EsdsBox) -> bool {
    // DecoderConfigDescriptor の object_type_indication が AAC を示しているかチェック
    // AAC LC は 0x40 (64)
    // AAC Main Profile は 0x41 (65)
    // AAC SSR は 0x42 (66)
    // AAC LTP は 0x43 (67)
    matches!(
        esds_box.es.dec_config_descr.object_type_indication,
        0x40..=0x43
    )
}
