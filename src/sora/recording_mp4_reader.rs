use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

use shiguredo_mp4::{TrackKind, boxes::SampleEntry, demux::Mp4FileDemuxer};

use crate::{
    audio::{AudioFormat, AudioFrame, Channels, SampleRate},
    sample_entry::SharedSampleEntry,
    types::CodecName,
    video::{VideoFormat, VideoFrame, VideoFrameSize},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VideoResolution {
    pub width: usize,
    pub height: usize,
}

#[derive(Debug)]
pub struct Mp4VideoReader {
    file: File,
    demuxer: Mp4FileDemuxer,
    format: VideoFormat,
    width: usize,
    height: usize,
    /// 直近のサンプルエントリーを保持して全フレームに付与する
    /// （`VideoFrame.sample_entry` の不変条件・issue 0030）
    last_sample_entry: Option<SharedSampleEntry>,

    pub current_input_file: Option<PathBuf>,
    pub codec: Option<CodecName>,
    pub resolutions: BTreeSet<VideoResolution>,
    pub total_sample_count: u64,
    pub total_track_duration: Duration,
    pub track_duration_offset: Duration,
}

impl Mp4VideoReader {
    pub fn new<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let mut file = File::open(&path).map_err(|e| {
            crate::Error::new(format!("Cannot open file {}: {e}", path.as_ref().display()))
        })?;
        let mut demuxer = Mp4FileDemuxer::new();
        initialize_mp4_demuxer(&mut file, &mut demuxer, &path)?;

        Ok(Self {
            file,
            demuxer,

            // 後で更新されるので適当な初期値を設定しておく
            format: VideoFormat::Vp8,
            width: 0,
            height: 0,
            last_sample_entry: None,
            current_input_file: Some(path.as_ref().to_path_buf()),
            codec: None,
            resolutions: BTreeSet::new(),
            total_sample_count: 0,
            total_track_duration: Duration::ZERO,
            track_duration_offset: Duration::ZERO,
        })
    }

    pub fn stats(&self) -> &Self {
        self
    }

    pub fn stats_mut(&mut self) -> &mut Self {
        self
    }

    pub fn inherit_stats_from(&mut self, prev: &Self) {
        self.codec = prev.codec;
        self.resolutions = prev.resolutions.clone();
        self.total_sample_count = prev.total_sample_count;
        self.total_track_duration = prev.total_track_duration;
        self.track_duration_offset = prev.track_duration_offset;
    }

    fn next_sample(&mut self) -> crate::Result<Option<VideoFrame>> {
        let sample = 'next_sample: loop {
            match self
                .demuxer
                .next_sample()
                .map_err(|e| crate::Error::new(format!("Read sample error: {e}")))?
            {
                None => return Ok(None),
                Some(sample) if sample.track.kind != TrackKind::Video => {}
                Some(sample) => break 'next_sample sample,
            }
        };

        // composition_time_offset は未対応
        if sample.composition_time_offset.is_some() {
            return Err(crate::Error::new(
                "composition_time_offset is not supported yet".to_owned(),
            ));
        }

        if let Some(sample_entry) = sample.sample_entry.cloned() {
            // 新しいサンプルエントリーが来たのでハンドリングする
            let (metadata, format) = match &sample_entry {
                SampleEntry::Avc1(b) => (&b.visual, VideoFormat::H264),
                SampleEntry::Hev1(b) => (&b.visual, VideoFormat::H265),
                SampleEntry::Hvc1(b) => (&b.visual, VideoFormat::H265),
                SampleEntry::Vp08(b) => (&b.visual, VideoFormat::Vp8),
                SampleEntry::Vp09(b) => (&b.visual, VideoFormat::Vp9),
                SampleEntry::Av01(b) => (&b.visual, VideoFormat::Av1),
                entry => {
                    return Err(crate::Error::new(format!(
                        "unsupported sample entry: {entry:?}"
                    )));
                }
            };

            self.format = format;
            self.width = metadata.width as usize;
            self.height = metadata.height as usize;
            // 直近のサンプルエントリーを保持して全フレームに付与する（issue 0030）
            self.last_sample_entry = Some(SharedSampleEntry::new(sample_entry));
        }

        // サンプルデータを読み込む
        let mut data = vec![0; sample.data_size];
        self.file
            .seek(SeekFrom::Start(sample.data_offset))
            .map_err(|e| crate::Error::new(format!("Seek error: {e}")))?;
        self.file
            .read_exact(&mut data)
            .map_err(|e| crate::Error::new(format!("Read error: {e}")))?;

        // タイムスタンプを計算する
        let timescale = sample.track.timescale.get();
        let timestamp = Duration::from_secs(sample.timestamp) / timescale;
        let duration = Duration::from_secs(sample.duration as u64) / timescale;

        // 統計値を更新する
        self.total_sample_count += 1;
        self.total_track_duration = timestamp + duration;
        if self.codec.is_none()
            && let Some(name) = self.format.codec_name()
        {
            self.codec = Some(name);
        }
        self.resolutions.insert(VideoResolution {
            width: self.width,
            height: self.height,
        });

        Ok(Some(VideoFrame {
            sample_entry: self.last_sample_entry.clone(),
            data,
            format: self.format,
            keyframe: sample.keyframe,
            size: Some(VideoFrameSize {
                width: self.width,
                height: self.height,
            }),
            timestamp,
        }))
    }
}

impl Iterator for Mp4VideoReader {
    type Item = crate::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_sample().transpose()
    }
}

#[derive(Debug)]
pub struct Mp4AudioReader {
    file: File,
    demuxer: Mp4FileDemuxer,
    audio_track_id: Option<u32>,
    format: AudioFormat,
    channels: Channels,
    sample_rate: SampleRate,
    /// 直近のサンプルエントリーを保持して全フレームに付与する
    /// （`AudioFrame.sample_entry` の不変条件・issue 0030）
    last_sample_entry: Option<SharedSampleEntry>,

    pub current_input_file: Option<PathBuf>,
    pub codec: Option<CodecName>,
    pub total_sample_count: u64,
    pub total_track_duration: Duration,
    pub track_duration_offset: Duration,
}

impl Mp4AudioReader {
    pub fn new<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let mut file = File::open(&path).map_err(|e| {
            crate::Error::new(format!("Cannot open file {}: {e}", path.as_ref().display()))
        })?;
        let mut demuxer = Mp4FileDemuxer::new();
        initialize_mp4_demuxer(&mut file, &mut demuxer, &path)?;

        // 利用可能な音声トラックがあるかをチェックする
        //
        // チェックのためにサンプルエントリーを取得するためには、
        // demuxer のサンプル読み込みが必要なので、clone して別インスタンスで行っている
        let audio_track_id = check_audio_track(demuxer.clone())?;

        Ok(Self {
            file,
            demuxer,
            audio_track_id,
            // ダミー初期値。実際の値はサンプルエントリー受信時に上書きされる。
            format: AudioFormat::Opus,
            channels: Channels::STEREO,
            sample_rate: SampleRate::HZ_48000,
            last_sample_entry: None,
            current_input_file: Some(path.as_ref().to_path_buf()),
            codec: None,
            total_sample_count: 0,
            total_track_duration: Duration::ZERO,
            track_duration_offset: Duration::ZERO,
        })
    }

    pub fn stats(&self) -> &Self {
        self
    }

    pub fn stats_mut(&mut self) -> &mut Self {
        self
    }

    pub fn inherit_stats_from(&mut self, prev: &Self) {
        self.codec = prev.codec;
        self.total_sample_count = prev.total_sample_count;
        self.total_track_duration = prev.total_track_duration;
        self.track_duration_offset = prev.track_duration_offset;
    }

    fn next_sample(&mut self) -> crate::Result<Option<AudioFrame>> {
        let sample = 'next_sample: loop {
            match self
                .demuxer
                .next_sample()
                .map_err(|e| crate::Error::new(format!("Read sample error: {e}")))?
            {
                None => return Ok(None),
                Some(sample) if Some(sample.track.track_id) != self.audio_track_id => {}
                Some(sample) => break 'next_sample sample,
            }
        };

        // composition_time_offset は未対応
        if sample.composition_time_offset.is_some() {
            return Err(crate::Error::new(
                "composition_time_offset is not supported yet".to_owned(),
            ));
        }

        if let Some(sample_entry) = sample.sample_entry.cloned() {
            // 新しいサンプルエントリーが来たのでハンドリングする
            let (metadata, format) = match &sample_entry {
                SampleEntry::Opus(b) => (&b.audio, AudioFormat::Opus),
                SampleEntry::Mp4a(b) => (&b.audio, AudioFormat::Aac),
                entry => {
                    return Err(crate::Error::new(format!(
                        "unsupported sample entry: {entry:?}"
                    )));
                }
            };

            self.format = format;
            self.channels = Channels::from_u16(metadata.channelcount)?;
            self.sample_rate = SampleRate::from_u16(metadata.samplerate.integer)?;
            // 直近のサンプルエントリーを保持して全フレームに付与する（issue 0030）
            self.last_sample_entry = Some(SharedSampleEntry::new(sample_entry));
        }

        // サンプルデータを読み込む
        let mut data = vec![0; sample.data_size];
        self.file
            .seek(SeekFrom::Start(sample.data_offset))
            .map_err(|e| crate::Error::new(format!("Seek error: {e}")))?;
        self.file
            .read_exact(&mut data)
            .map_err(|e| crate::Error::new(format!("Read error: {e}")))?;

        // タイムスタンプを計算する
        let timescale = sample.track.timescale.get();
        let timestamp = Duration::from_secs(sample.timestamp) / timescale;
        let duration = Duration::from_secs(sample.duration as u64) / timescale;

        // 統計値を更新する
        self.total_sample_count += 1;
        self.total_track_duration = timestamp + duration;
        if self.codec.is_none()
            && let Some(name) = self.format.codec_name()
        {
            self.codec = Some(name);
        }

        Ok(Some(AudioFrame {
            data,
            format: self.format,
            sample_entry: self.last_sample_entry.clone(),
            channels: self.channels,
            sample_rate: self.sample_rate,
            timestamp,
        }))
    }
}

impl Iterator for Mp4AudioReader {
    type Item = crate::Result<AudioFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_sample().transpose()
    }
}

/// MP4 ファイルからトラック情報を初期化する
///
/// NOTE: fMP4 には未対応なので、この関数完了後、demuxer はファイル読み込みを要求しない
fn initialize_mp4_demuxer<R: Read + Seek, P: AsRef<Path>>(
    file: &mut R,
    demuxer: &mut Mp4FileDemuxer,
    path: P,
) -> crate::Result<()> {
    // 念のために（壊れたファイルが渡された時のため）、バッファサイズの上限を 100 MBに設定しておく。
    // 正常なファイルの場合には、これは moov ボックスのサイズ上限となるが、
    // 典型的には、100 MB あれば、MP4 ファイル自体としては数百 GB 程度のものを扱えるため、実用上の問題はない想定。
    const MAX_READ_SIZE: usize = 100 * 1024 * 1024;

    while let Some(required) = demuxer.required_input() {
        let size = required.size.ok_or_else(|| {
            crate::Error::new(format!(
                "MP4 file contains unexpected variable size box {}",
                path.as_ref().display()
            ))
        })?;
        if size > MAX_READ_SIZE {
            return Err(crate::Error::new(format!(
                "MP4 file contains box larger than maximum allowed size ({size} > {MAX_READ_SIZE}): {}",
                path.as_ref().display()
            )));
        }

        let mut buf = vec![0; size];
        file.seek(SeekFrom::Start(required.position)).map_err(|e| {
            crate::Error::new(format!("Seek error {}: {e}", path.as_ref().display()))
        })?;
        file.read_exact(&mut buf).map_err(|e| {
            crate::Error::new(format!("Read error {}: {e}", path.as_ref().display()))
        })?;
        let input = required.to_input(&buf);
        demuxer.handle_input(input);
    }
    Ok(())
}

/// 音声トラックをチェックして、サポートされているコーデックを持つトラック ID を取得する
fn check_audio_track(mut demuxer: Mp4FileDemuxer) -> crate::Result<Option<u32>> {
    let mut has_audio_track = false;
    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Audio {
            continue;
        }
        has_audio_track = true;

        if let Some(sample_entry) = sample.sample_entry {
            // hisui がサポートしているコーデックかどうかをチェック
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
        // 音声トラックがあるのにサポートしているコーデックがない場合はエラーにする
        Err(crate::Error::new(
            "No supported audio track found in the file".to_owned(),
        ))
    } else {
        // そもそも音声トラックがない場合には空扱いをする
        Ok(None)
    }
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

    // issue 0030 の不変条件「エンコード済み圧縮フォーマットの VideoFrame は常に sample_entry を持つ」を検証する。
    // sora 録画形式の H.264 ファイルを Mp4VideoReader で読んだとき、全ての映像フレーム（初回・後続を問わず）
    // に sample_entry が載っており、かつ後続フレームの sample_entry が初回フレームと等価
    // （SharedSampleEntry::changed_since が false）であることを確認する。
    // 等価性まで見るのは、reader が dummy SampleEntry を毎回新規生成しても素通りしないようにするため。
    #[test]
    fn mp4_video_reader_emits_sample_entry_on_every_frame() -> crate::Result<()> {
        let reader = Mp4VideoReader::new("testdata/archive-red-320x320-h264.mp4")?;
        let mut frame_count = 0;
        let mut first_sample_entry: Option<SharedSampleEntry> = None;
        for frame in reader {
            let frame = frame?;
            let sample_entry = frame.sample_entry.unwrap_or_else(|| {
                panic!("映像フレーム #{frame_count} に sample_entry が載っていないこと")
            });
            if let Some(ref first) = first_sample_entry {
                assert!(
                    !sample_entry.changed_since(Some(first)),
                    "後続の映像フレームが初回と等価な sample_entry を持つこと (frame #{frame_count})"
                );
            } else {
                first_sample_entry = Some(sample_entry);
            }
            frame_count += 1;
        }
        assert!(frame_count > 1, "複数フレームを読めていること");
        Ok(())
    }

    // issue 0030 の不変条件「エンコード済み圧縮フォーマットの AudioFrame は常に sample_entry を持つ」を検証する。
    // 通常 MP4 の AAC ファイルを Mp4AudioReader で読んだとき、全ての音声フレームに sample_entry が載っており、
    // かつ後続フレームの sample_entry が初回フレームと等価であることを確認する。
    #[test]
    fn mp4_audio_reader_emits_sample_entry_on_every_frame() -> crate::Result<()> {
        let reader = Mp4AudioReader::new("testdata/beep-aac-audio.mp4")?;
        let mut frame_count = 0;
        let mut first_sample_entry: Option<SharedSampleEntry> = None;
        for frame in reader {
            let frame = frame?;
            let sample_entry = frame.sample_entry.unwrap_or_else(|| {
                panic!("音声フレーム #{frame_count} に sample_entry が載っていないこと")
            });
            if let Some(ref first) = first_sample_entry {
                assert!(
                    !sample_entry.changed_since(Some(first)),
                    "後続の音声フレームが初回と等価な sample_entry を持つこと (frame #{frame_count})"
                );
            } else {
                first_sample_entry = Some(sample_entry);
            }
            frame_count += 1;
        }
        assert!(frame_count > 1, "複数フレームを読めていること");
        Ok(())
    }
}
