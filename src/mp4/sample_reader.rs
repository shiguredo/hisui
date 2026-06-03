//! inspect 用の前方読み専用 reader
//!
//! 通常 MP4 / fMP4 の両方を `Mp4Demuxer` で前方読みし、encoded sample を
//! `TrackPublisher` へ送出する。シーク・デコード・再生制御は持たない。
//! デコードは inspect パイプライン側の別 processor が担当する。

use std::path::{Path, PathBuf};

use shiguredo_mp4::TrackKind;

use super::demuxer::{
    Mp4Demuxer, audio_format_from_entry, calculate_timestamps, video_format_from_entry,
};
use super::reader::TrackSender;
use crate::audio::{AudioFormat, Channels, SampleRate};
use crate::video::{VideoFormat, VideoFrameSize};
use crate::{AudioFrame, ProcessorHandle, Result, TrackId, VideoFrame};

#[derive(Debug, Clone, Default)]
pub struct Mp4SampleReaderOptions {
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

/// 前方読み専用の軽量 reader
#[derive(Debug)]
pub struct Mp4SampleReader {
    path: PathBuf,
    options: Mp4SampleReaderOptions,
}

impl Mp4SampleReader {
    pub fn new<P: AsRef<Path>>(path: P, options: Mp4SampleReaderOptions) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            options,
        }
    }

    pub async fn run(self, handle: ProcessorHandle) -> Result<()> {
        let mut audio_sender = if let Some(track_id) = self.options.audio_track_id.clone() {
            Some(TrackSender::new(handle.publish_track(track_id).await?))
        } else {
            None
        };
        let mut video_sender = if let Some(track_id) = self.options.video_track_id.clone() {
            Some(TrackSender::new(handle.publish_track(track_id).await?))
        } else {
            None
        };

        handle.notify_ready();

        if audio_sender.is_none() && video_sender.is_none() {
            return Ok(());
        }

        handle.wait_subscribers_ready().await?;

        let mut demuxer = Mp4Demuxer::open(&self.path)?;

        // 最初に現れたトラックを対象として固定する (inspect は単一トラック構成を前提とする)
        let mut audio_track_id: Option<u32> = None;
        let mut video_track_id: Option<u32> = None;
        // format 系はサンプルエントリー受信時に上書きされる。それまではダミー初期値を使う。
        let mut audio_format = AudioFormat::Opus;
        let mut audio_channels = Channels::STEREO;
        let mut audio_sample_rate = SampleRate::HZ_48000;
        let mut video_format = VideoFormat::Vp8;
        let mut video_width = 0usize;
        let mut video_height = 0usize;

        while let Some(sample) = demuxer.next_sample()? {
            // composition_time_offset (B フレーム由来の CTS オフセット) は未対応
            if sample.composition_time_offset.is_some() {
                return Err(crate::Error::new(
                    "composition_time_offset is not supported yet".to_owned(),
                ));
            }

            match sample.track_kind {
                TrackKind::Audio => {
                    let Some(sender) = audio_sender.as_mut() else {
                        continue;
                    };
                    if audio_track_id.is_none() {
                        audio_track_id = Some(sample.track_id);
                    }
                    if audio_track_id != Some(sample.track_id) {
                        continue;
                    }
                    if let Some(entry) = &sample.sample_entry {
                        (audio_format, audio_channels, audio_sample_rate) =
                            audio_format_from_entry(entry)?;
                    }
                    let data = demuxer.read_sample_data(sample.data_offset, sample.data_size)?;
                    let (timestamp, _duration) =
                        calculate_timestamps(sample.timescale, sample.timestamp, sample.duration);
                    let frame = AudioFrame {
                        data,
                        format: audio_format,
                        channels: audio_channels,
                        sample_rate: audio_sample_rate,
                        timestamp,
                        sample_entry: sample.sample_entry,
                    };
                    if !sender.send_audio(frame).await {
                        // パイプライン処理が中断された
                        break;
                    }
                }
                TrackKind::Video => {
                    let Some(sender) = video_sender.as_mut() else {
                        continue;
                    };
                    if video_track_id.is_none() {
                        video_track_id = Some(sample.track_id);
                    }
                    if video_track_id != Some(sample.track_id) {
                        continue;
                    }
                    if let Some(entry) = &sample.sample_entry {
                        (video_format, video_width, video_height) = video_format_from_entry(entry)?;
                    }
                    let data = demuxer.read_sample_data(sample.data_offset, sample.data_size)?;
                    let (timestamp, _duration) =
                        calculate_timestamps(sample.timescale, sample.timestamp, sample.duration);
                    let frame = VideoFrame {
                        data,
                        format: video_format,
                        keyframe: sample.keyframe,
                        size: Some(VideoFrameSize {
                            width: video_width,
                            height: video_height,
                        }),
                        timestamp,
                        sample_entry: sample.sample_entry,
                    };
                    if !sender.send_video(frame).await {
                        // パイプライン処理が中断された
                        break;
                    }
                }
            }
        }

        if let Some(sender) = audio_sender.as_mut() {
            sender.send_eos();
        }
        if let Some(sender) = video_sender.as_mut() {
            sender.send_eos();
        }

        Ok(())
    }
}
