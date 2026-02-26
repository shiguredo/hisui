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
    types::CodecName,
    video::{VideoFormat, VideoFrame},
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

        let sample_entry = sample.sample_entry.cloned();
        if let Some(sample_entry) = &sample_entry {
            // 新しいサンプルエントリーが来たのでハンドリングする
            let (metadata, format) = match sample_entry {
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
            sample_entry,
            data,
            format: self.format,
            keyframe: sample.keyframe,
            width: self.width,
            height: self.height,
            timestamp,
            duration,
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

        let sample_entry = sample.sample_entry.cloned();
        if let Some(sample_entry) = &sample_entry {
            // 新しいサンプルエントリーが来たのでハンドリングする
            let (metadata, format) = match sample_entry {
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
            sample_entry,
            channels: self.channels,
            sample_rate: self.sample_rate,
            timestamp,
            duration,
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
    const MAX_BUF_SIZE: usize = 100 * 1024 * 1024;

    while let Some(required) = demuxer.required_input() {
        let size = required.size.ok_or_else(|| {
            crate::Error::new(format!(
                "MP4 file contains unexpected variable size box {}",
                path.as_ref().display()
            ))
        })?;
        if size > MAX_BUF_SIZE {
            return Err(crate::Error::new(format!(
                "MP4 file contains box larger than maximum allowed size ({size} > {MAX_BUF_SIZE}): {}",
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
