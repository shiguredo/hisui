use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};

use orfail::OrFail;
use shiguredo_mp4::{TrackKind, boxes::SampleEntry, demux::Mp4FileDemuxer};

use crate::{
    audio::{AudioData, AudioFormat},
    metadata::SourceId,
    stats::{Mp4AudioReaderStats, Mp4VideoReaderStats, VideoResolution},
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct Mp4VideoReader {
    file: File,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
    format: VideoFormat,
    width: usize,
    height: usize,
    stats: Mp4VideoReaderStats,
}

impl Mp4VideoReader {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4VideoReaderStats,
    ) -> orfail::Result<Self> {
        let mut file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut demuxer = Mp4FileDemuxer::new();
        initialize_mp4_demuxer(&mut file, &mut demuxer, &path).or_fail()?;

        Ok(Self {
            file,
            demuxer,
            source_id,
            stats,

            // 後で更新されるので適当な初期値を設定しておく
            format: VideoFormat::Vp8,
            width: 0,
            height: 0,
        })
    }

    pub fn stats(&self) -> &Mp4VideoReaderStats {
        &self.stats
    }

    fn next_sample(&mut self) -> orfail::Result<Option<VideoFrame>> {
        let sample = 'next_sample: loop {
            match self
                .demuxer
                .next_sample()
                .or_fail_with(|e| format!("Read sample error: {e}"))?
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
                    return Err(orfail::Failure::new(format!(
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
            .or_fail_with(|e| format!("Seek error: {e}"))?;
        self.file
            .read_exact(&mut data)
            .or_fail_with(|e| format!("Read error: {e}"))?;

        // タイムスタンプを計算する
        let timescale = sample.track.timescale.get();
        let timestamp = Duration::from_secs(sample.timestamp) / timescale;
        let duration = Duration::from_secs(sample.duration as u64) / timescale;

        // 統計値を更新する
        self.stats.total_sample_count.add(1);
        self.stats.total_track_duration.set(timestamp + duration);
        if self.stats.codec.get().is_none()
            && let Some(name) = self.format.codec_name()
        {
            self.stats.codec.set(name);
        }
        self.stats.resolutions.insert(VideoResolution {
            width: self.width,
            height: self.height,
        });

        Ok(Some(VideoFrame {
            source_id: Some(self.source_id.clone()),
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
    type Item = orfail::Result<VideoFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_sample().or_fail().transpose()
    }
}

#[derive(Debug)]
pub struct Mp4AudioReader {
    file: File,
    demuxer: Mp4FileDemuxer,
    source_id: SourceId,
    format: AudioFormat,
    stereo: bool,
    sample_rate: u16,
    stats: Mp4AudioReaderStats,
}

impl Mp4AudioReader {
    pub fn new<P: AsRef<Path>>(
        source_id: SourceId,
        path: P,
        stats: Mp4AudioReaderStats,
    ) -> orfail::Result<Self> {
        let mut file = File::open(&path)
            .or_fail_with(|e| format!("Cannot open file {}: {e}", path.as_ref().display()))?;
        let mut demuxer = Mp4FileDemuxer::new();
        initialize_mp4_demuxer(&mut file, &mut demuxer, &path).or_fail()?;

        Ok(Self {
            file,
            demuxer,
            source_id,
            stats,
            // 後で更新されるので適当な初期値を設定しておく
            format: AudioFormat::Opus,
            stereo: false,
            sample_rate: 0,
        })
    }

    pub fn stats(&self) -> &Mp4AudioReaderStats {
        &self.stats
    }

    fn next_sample(&mut self) -> orfail::Result<Option<AudioData>> {
        let sample = 'next_sample: loop {
            match self
                .demuxer
                .next_sample()
                .or_fail_with(|e| format!("Read sample error: {e}"))?
            {
                None => return Ok(None),
                Some(sample) if sample.track.kind != TrackKind::Audio => {}
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
                    return Err(orfail::Failure::new(format!(
                        "unsupported sample entry: {entry:?}"
                    )));
                }
            };

            self.format = format;
            self.stereo = metadata.channelcount != 1;
            self.sample_rate = metadata.samplerate.integer;
        }

        // サンプルデータを読み込む
        let mut data = vec![0; sample.data_size];
        self.file
            .seek(SeekFrom::Start(sample.data_offset))
            .or_fail_with(|e| format!("Seek error: {e}"))?;
        self.file
            .read_exact(&mut data)
            .or_fail_with(|e| format!("Read error: {e}"))?;

        // タイムスタンプを計算する
        let timescale = sample.track.timescale.get();
        let timestamp = Duration::from_secs(sample.timestamp) / timescale;
        let duration = Duration::from_secs(sample.duration as u64) / timescale;

        // 統計値を更新する
        self.stats.total_sample_count.add(1);
        self.stats.total_track_duration.set(timestamp + duration);

        Ok(Some(AudioData {
            source_id: Some(self.source_id.clone()),
            data,
            format: self.format,
            sample_entry,
            stereo: self.stereo,
            sample_rate: self.sample_rate,
            timestamp,
            duration,
        }))
    }
}

impl Iterator for Mp4AudioReader {
    type Item = orfail::Result<AudioData>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_sample().or_fail().transpose()
    }
}

/// MP4 ファイルからトラック情報を初期化する
///
/// NOTE: fMP4 には未対応なので、この関数完了後、demuxer はファイル読み込みを要求しない
fn initialize_mp4_demuxer<R: Read + Seek, P: AsRef<Path>>(
    file: &mut R,
    demuxer: &mut Mp4FileDemuxer,
    path: P,
) -> orfail::Result<()> {
    // 念のために（壊れたファイルが渡された時のため）、バッファサイズの上限を 100 MBに設定しておく。
    // 正常なファイルの場合には、これは moov ボックスのサイズ上限となるが、
    // 典型的には、100 MB あれば、MP4 ファイル自体としては数百 GB 程度のものを扱えるため、実用上の問題はない想定。
    const MAX_BUF_SIZE: usize = 100 * 1024 * 1024;

    while let Some(required) = demuxer.required_input() {
        let size = required.size.or_fail_with(|()| {
            format!(
                "MP4 file contains unexpected variable size box {}",
                path.as_ref().display()
            )
        })?;
        if size > MAX_BUF_SIZE {
            return Err(orfail::Failure::new(format!(
                "MP4 file contains box larger than maximum allowed size ({size} > {MAX_BUF_SIZE}): {}",
                path.as_ref().display()
            )));
        }

        let mut buf = vec![0; size];
        file.seek(SeekFrom::Start(required.position))
            .or_fail_with(|e| format!("Seek error {}: {e}", path.as_ref().display()))?;
        file.read_exact(&mut buf)
            .or_fail_with(|e| format!("Read error {}: {e}", path.as_ref().display()))?;
        let input = required.to_input(&buf);
        demuxer.handle_input(input);
    }
    Ok(())
}
