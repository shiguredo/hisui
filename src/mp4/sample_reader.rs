//! inspect 用の前方読み専用 reader
//!
//! 通常 MP4 / fMP4 の両方を `Mp4Demuxer` で前方読みし、encoded sample を
//! `TrackPublisher` へ送出する。シーク・デコード・再生制御は持たない。
//! デコードは inspect パイプライン側の別 processor が担当する。

use std::path::{Path, PathBuf};

use shiguredo_mp4::TrackKind;

use super::demuxer::{
    Mp4Demuxer, audio_format_from_entry, calculate_timestamps, is_supported_audio_entry,
    is_supported_video_entry, video_format_from_entry,
};
use super::reader::TrackSender;
use crate::audio::{AudioFormat, Channels, SampleRate};
use crate::sample_entry::SharedSampleEntry;
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

        // 対応コーデックのトラックを選別する (前方読み 1 回目)。
        // 非対応コーデックのトラックは警告してスキップし、対応トラックを採用する。
        // OBSWS 経路の Mp4FileReader の select_audio_track / select_video_track と同じ選別とする。
        let (audio_track_id, video_track_id) = {
            let mut demuxer = Mp4Demuxer::open(&self.path)?;
            select_supported_tracks(&mut demuxer, audio_sender.is_some(), video_sender.is_some())?
        };

        // 選別したトラックのサンプルを送出する (前方読み 2 回目)。
        let mut demuxer = Mp4Demuxer::open(&self.path)?;
        // format 系はサンプルエントリー受信時に上書きされる。それまではダミー初期値を使う。
        let mut audio_format = AudioFormat::Opus;
        let mut audio_channels = Channels::STEREO;
        let mut audio_sample_rate = SampleRate::HZ_48000;
        let mut video_format = VideoFormat::Vp8;
        let mut video_width = 0usize;
        let mut video_height = 0usize;
        // 直近のサンプルエントリーをトラック種別ごとに保持して全フレームに付与する
        // （`VideoFrame.sample_entry` / `AudioFrame.sample_entry` の不変条件・issue 0030）
        let mut last_audio_sample_entry: Option<SharedSampleEntry> = None;
        let mut last_video_sample_entry: Option<SharedSampleEntry> = None;

        while let Some(sample) = demuxer.next_sample()? {
            match sample.track_kind {
                TrackKind::Audio => {
                    let Some(sender) = audio_sender.as_mut() else {
                        continue;
                    };
                    if audio_track_id != Some(sample.track_id) {
                        continue;
                    }
                    // composition_time_offset (B フレーム由来の CTS オフセット) は未対応。
                    // subscribe 対象の track についてのみチェックする (対象外 track は続く continue で
                    // skip されるため、対象外 track の CTS オフセットで pipeline 全体を落とさない)。
                    if sample.composition_time_offset.is_some() {
                        return Err(crate::Error::new(
                            "composition_time_offset is not supported yet".to_owned(),
                        ));
                    }
                    if let Some(entry) = &sample.sample_entry {
                        (audio_format, audio_channels, audio_sample_rate) =
                            audio_format_from_entry(entry)?;
                        last_audio_sample_entry = Some(SharedSampleEntry::new(entry.clone()));
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
                        sample_entry: last_audio_sample_entry.clone(),
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
                    if video_track_id != Some(sample.track_id) {
                        continue;
                    }
                    if sample.composition_time_offset.is_some() {
                        return Err(crate::Error::new(
                            "composition_time_offset is not supported yet".to_owned(),
                        ));
                    }
                    if let Some(entry) = &sample.sample_entry {
                        (video_format, video_width, video_height) = video_format_from_entry(entry)?;
                        last_video_sample_entry = Some(SharedSampleEntry::new(entry.clone()));
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
                        sample_entry: last_video_sample_entry.clone(),
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

/// 前方読みで、対応コーデックを持つトラックを種別ごとに 1 つ選別する
///
/// 各種別について「最初に現れた対応コーデックのトラック」を採用する。非対応コーデックの
/// トラックは警告ログを出してスキップする。対応トラックが見つからないまま、その種別の
/// トラックが存在した場合はエラーにする (OBSWS 経路の Mp4FileReader と同じ挙動)。
fn select_supported_tracks(
    demuxer: &mut Mp4Demuxer,
    want_audio: bool,
    want_video: bool,
) -> Result<(Option<u32>, Option<u32>)> {
    let mut audio_track_id: Option<u32> = None;
    let mut video_track_id: Option<u32> = None;
    let mut has_audio_track = false;
    let mut has_video_track = false;

    while let Some(sample) = demuxer.next_sample()? {
        match sample.track_kind {
            TrackKind::Audio => {
                if !want_audio || audio_track_id.is_some() {
                    continue;
                }
                has_audio_track = true;
                if let Some(entry) = &sample.sample_entry {
                    if is_supported_audio_entry(entry) {
                        audio_track_id = Some(sample.track_id);
                    } else {
                        tracing::warn!(
                            "Unsupported audio codec in track {}: {:?}",
                            sample.track_id,
                            entry
                        );
                    }
                }
            }
            TrackKind::Video => {
                if !want_video || video_track_id.is_some() {
                    continue;
                }
                has_video_track = true;
                if let Some(entry) = &sample.sample_entry {
                    if is_supported_video_entry(entry) {
                        video_track_id = Some(sample.track_id);
                    } else {
                        tracing::warn!(
                            "Unsupported video codec in track {}: {:?}",
                            sample.track_id,
                            entry
                        );
                    }
                }
            }
        }

        // 必要な種別のトラックがすべて確定したら、残りは読まずに打ち切る
        let audio_done = !want_audio || audio_track_id.is_some();
        let video_done = !want_video || video_track_id.is_some();
        if audio_done && video_done {
            break;
        }
    }

    if want_audio && has_audio_track && audio_track_id.is_none() {
        return Err(crate::Error::new(
            "No supported audio track found in the file".to_owned(),
        ));
    }
    if want_video && has_video_track && video_track_id.is_none() {
        return Err(crate::Error::new(
            "No supported video track found in the file".to_owned(),
        ));
    }

    Ok((audio_track_id, video_track_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_picks_supported_audio_and_video_tracks() {
        let mut demuxer =
            Mp4Demuxer::open("testdata/red-320x320-h264-aac.mp4").expect("通常 MP4 を開けること");
        let (audio, video) = select_supported_tracks(&mut demuxer, true, true)
            .expect("対応トラックの選別に成功すること");
        assert!(audio.is_some(), "音声トラックが選別されること");
        assert!(video.is_some(), "映像トラックが選別されること");
    }

    #[test]
    fn select_returns_none_for_absent_track_kind() {
        // 音声のみのファイルでは、映像トラックは None になりエラーにはならない
        let mut demuxer =
            Mp4Demuxer::open("testdata/beep-aac-audio.mp4").expect("音声のみ MP4 を開けること");
        let (audio, video) =
            select_supported_tracks(&mut demuxer, true, true).expect("選別に成功すること");
        assert!(audio.is_some(), "音声トラックが選別されること");
        assert_eq!(video, None, "映像トラックが無い場合は None になること");
    }

    #[test]
    fn select_skips_unwanted_track_kind() {
        // want_video=false のときは、映像トラックがあっても選別しない
        let mut demuxer =
            Mp4Demuxer::open("testdata/red-320x320-h264-aac.mp4").expect("通常 MP4 を開けること");
        let (audio, video) =
            select_supported_tracks(&mut demuxer, true, false).expect("選別に成功すること");
        assert!(audio.is_some(), "音声トラックが選別されること");
        assert_eq!(video, None, "want_video=false では映像を選別しないこと");
    }
}
