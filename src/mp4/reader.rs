use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

use shiguredo_mp4::{TrackKind, boxes::SampleEntry, demux::Mp4FileDemuxer};

use crate::audio::{AudioFormat, Channels, SampleRate};
use crate::video::{VideoFormat, VideoFrameSize};
use crate::{Ack, AudioFrame, Error, MessageSender, ProcessorHandle, Result, TrackId, VideoFrame};

const MAX_NOACKED_COUNT: u64 = 100;

#[derive(Debug, Clone, Default)]
pub struct Mp4FileReaderOptions {
    // true の場合は実時間再生を行う。
    // 出力 timestamp は実時刻ベースで単調増加するように補正する。
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
    audio_decoder: Option<crate::decoder::AudioDecoder>,
    video_decoder: Option<crate::decoder::VideoDecoder>,
    base_offset: Duration,
    last_emitted_end: Duration,
    start_instant: tokio::time::Instant,
    last_realtime_timestamp: Option<Duration>,
    emitted_in_loop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp4FileTrackAvailability {
    pub has_audio: bool,
    pub has_video: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp4FileVideoDimensions {
    pub width: usize,
    pub height: usize,
}

impl Mp4FileReader {
    pub fn new<P: AsRef<Path>>(path: P, options: Mp4FileReaderOptions) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            options,
            audio_sender: None,
            video_sender: None,
            audio_decoder: None,
            video_decoder: None,
            base_offset: Duration::ZERO,
            last_emitted_end: Duration::ZERO,
            start_instant: tokio::time::Instant::now(),
            last_realtime_timestamp: None,
            emitted_in_loop: false,
        })
    }

    /// デコーダーを設定する。設定された場合、encoded frame を decode してから送信する。
    pub fn set_audio_decoder(&mut self, decoder: crate::decoder::AudioDecoder) {
        self.audio_decoder = Some(decoder);
    }

    /// デコーダーを設定する。設定された場合、encoded frame を decode してから送信する。
    pub fn set_video_decoder(&mut self, decoder: crate::decoder::VideoDecoder) {
        self.video_decoder = Some(decoder);
    }

    pub async fn run(mut self, handle: ProcessorHandle) -> Result<()> {
        let loop_enabled = self.resolve_loop_enabled();
        (self.audio_sender, self.video_sender) = self.build_track_senders(&handle).await?;
        handle.notify_ready();

        if self.audio_sender.is_none() && self.video_sender.is_none() {
            return Ok(());
        }
        handle.wait_subscribers_ready().await?;

        self.start_instant = tokio::time::Instant::now();
        self.last_realtime_timestamp = None;

        let should_stop = self.run_loop(loop_enabled).await?;
        if should_stop {
            return Ok(());
        }

        self.flush_and_send_eos()?;

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
        // composition_time_offset は未対応
        if context.composition_time_offset.is_some() {
            return Err(Error::new(
                "composition_time_offset is not supported yet".to_owned(),
            ));
        }

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
        let output_timestamp = self.output_timestamp(effective_timestamp);

        let audio_data = AudioFrame {
            data,
            format: state.audio_format,
            channels: state.audio_channels,
            sample_rate: state.audio_sample_rate,
            timestamp: output_timestamp,
            sample_entry: context.sample_entry,
        };

        if let Some(sender) = self.audio_sender.as_mut() {
            if let Some(decoder) = self.audio_decoder.as_mut() {
                // デコーダーが設定されている場合、decode してから送信する
                decoder.handle_input_sample(Some(crate::MediaFrame::Audio(
                    std::sync::Arc::new(audio_data),
                )))?;
                if crate::decoder::drain_audio_decoder_output(decoder, &mut sender.sender)?
                    != crate::decoder::DrainResult::Pending
                {
                    return Ok(true);
                }
            } else if !sender.send_audio(audio_data).await {
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
        let output_timestamp = self.output_timestamp(effective_timestamp);

        let video_frame = VideoFrame {
            data,
            format: state.video_format,
            keyframe: context.keyframe,
            size: Some(VideoFrameSize {
                width: state.video_width,
                height: state.video_height,
            }),
            timestamp: output_timestamp,
            sample_entry: context.sample_entry,
        };

        if let Some(sender) = self.video_sender.as_mut() {
            if let Some(decoder) = self.video_decoder.as_mut() {
                // デコーダーが設定されている場合、decode してから送信する
                decoder.handle_input_sample(Some(crate::MediaFrame::Video(
                    std::sync::Arc::new(video_frame),
                )))?;
                if crate::decoder::drain_video_decoder_output(decoder, &mut sender.sender)?
                    != crate::decoder::DrainResult::Pending
                {
                    return Ok(true);
                }
            } else if !sender.send_video(video_frame).await {
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

    fn output_timestamp(&mut self, effective_timestamp: Duration) -> Duration {
        if !self.options.realtime {
            return effective_timestamp;
        }

        let mut timestamp = self.start_instant.elapsed().max(effective_timestamp);
        if let Some(last) = self.last_realtime_timestamp {
            let min_next = last.saturating_add(Duration::from_micros(1));
            if timestamp < min_next {
                timestamp = min_next;
            }
        }
        self.last_realtime_timestamp = Some(timestamp);
        timestamp
    }

    fn flush_and_send_eos(&mut self) -> Result<()> {
        // デコーダーの残りのフレームを flush する。
        // EOS flush 中に pipeline が閉じるのは正常な停止シーケンスなので、DrainResult は無視する。
        if let Some(decoder) = self.audio_decoder.as_mut()
            && let Some(sender) = self.audio_sender.as_mut()
        {
            decoder.handle_input_sample(None)?;
            let _ = crate::decoder::drain_audio_decoder_output(decoder, &mut sender.sender)?;
        }
        if let Some(decoder) = self.video_decoder.as_mut()
            && let Some(sender) = self.video_sender.as_mut()
        {
            decoder.handle_input_sample(None)?;
            let _ = crate::decoder::drain_video_decoder_output(decoder, &mut sender.sender)?;
        }
        if let Some(sender) = self.audio_sender.as_mut() {
            sender.send_eos();
        }
        if let Some(sender) = self.video_sender.as_mut() {
            sender.send_eos();
        }
        Ok(())
    }
}

pub fn probe_mp4_track_availability<P: AsRef<Path>>(path: P) -> Result<Mp4FileTrackAvailability> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
    let mut demuxer = Mp4FileDemuxer::new();
    initialize_mp4_demuxer(&mut file, &mut demuxer, path)?;

    let has_audio = select_audio_track(demuxer.clone())?.is_some();
    let has_video = select_video_track(demuxer)?.is_some();

    Ok(Mp4FileTrackAvailability {
        has_audio,
        has_video,
    })
}

pub fn probe_mp4_video_dimensions<P: AsRef<Path>>(
    path: P,
) -> Result<Option<Mp4FileVideoDimensions>> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
    let mut demuxer = Mp4FileDemuxer::new();
    initialize_mp4_demuxer(&mut file, &mut demuxer, path)?;

    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Video {
            continue;
        }
        let Some(sample_entry) = sample.sample_entry else {
            continue;
        };
        let metadata = match sample_entry {
            SampleEntry::Avc1(b) => Some(&b.visual),
            SampleEntry::Hev1(b) => Some(&b.visual),
            SampleEntry::Hvc1(b) => Some(&b.visual),
            SampleEntry::Vp08(b) => Some(&b.visual),
            SampleEntry::Vp09(b) => Some(&b.visual),
            SampleEntry::Av01(b) => Some(&b.visual),
            _ => None,
        };
        if let Some(metadata) = metadata {
            return Ok(Some(Mp4FileVideoDimensions {
                width: metadata.width as usize,
                height: metadata.height as usize,
            }));
        }
    }

    Ok(None)
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
    composition_time_offset: Option<i64>,
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
            composition_time_offset: sample.composition_time_offset,
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

    async fn send_audio(&mut self, data: AudioFrame) -> bool {
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

#[derive(Debug)]
struct ReaderState {
    path: PathBuf,
    file: File,
    demuxer: Mp4FileDemuxer,
    audio_track_id: Option<u32>,
    video_track_id: Option<u32>,
    audio_format: AudioFormat,
    audio_channels: Channels,
    audio_sample_rate: SampleRate,
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
            // ダミー初期値。実際の値はサンプルエントリー受信時に上書きされる。
            audio_format: AudioFormat::Opus,
            audio_channels: Channels::STEREO,
            audio_sample_rate: SampleRate::HZ_48000,
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
        self.audio_channels = Channels::from_u16(metadata.channelcount)?;
        self.audio_sample_rate = SampleRate::from_u16(metadata.samplerate.integer)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_mp4_track_availability_detects_audio_only_file() -> Result<()> {
        let availability = probe_mp4_track_availability("testdata/beep-aac-audio.mp4")?;
        assert_eq!(
            availability,
            Mp4FileTrackAvailability {
                has_audio: true,
                has_video: false,
            }
        );
        Ok(())
    }

    #[test]
    fn probe_mp4_track_availability_detects_video_only_file() -> Result<()> {
        let availability = probe_mp4_track_availability("testdata/archive-red-320x320-h264.mp4")?;
        assert_eq!(
            availability,
            Mp4FileTrackAvailability {
                has_audio: false,
                has_video: true,
            }
        );
        Ok(())
    }

    #[test]
    fn probe_mp4_track_availability_detects_av_file() -> Result<()> {
        let availability = probe_mp4_track_availability("testdata/red-320x320-h264-aac.mp4")?;
        assert_eq!(
            availability,
            Mp4FileTrackAvailability {
                has_audio: true,
                has_video: true,
            }
        );
        Ok(())
    }
}
