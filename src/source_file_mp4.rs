use std::path::PathBuf;

use crate::decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions};
use crate::file_reader_mp4::{Mp4FileReader, Mp4FileReaderOptions};
use crate::media::MediaStreamId;
use crate::{
    Error, MediaPipeline, MediaPipelineHandle, Message, ProcessorHandle, ProcessorId, Result,
    TrackId,
};

#[derive(Debug, Clone)]
pub struct Mp4FileSource {
    pub path: PathBuf,
    pub realtime: bool,
    pub loop_playback: bool,
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

impl nojson::DisplayJson for Mp4FileSource {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("path", &self.path)?;
            f.member("realtime", self.realtime)?;
            f.member("loopPlayback", self.loop_playback)?;
            if let Some(id) = &self.audio_track_id {
                f.member("audioTrackId", id)?;
            }
            if let Some(id) = &self.video_track_id {
                f.member("videoTrackId", id)?;
            }
            Ok(())
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Mp4FileSource {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let path: PathBuf = value.to_member("path")?.required()?.try_into()?;
        let realtime: Option<bool> = value.to_member("realtime")?.try_into()?;
        let loop_playback: Option<bool> = value.to_member("loopPlayback")?.try_into()?;
        let audio_track_id: Option<TrackId> = value.to_member("audioTrackId")?.try_into()?;
        let video_track_id: Option<TrackId> = value.to_member("videoTrackId")?.try_into()?;

        // トラック ID のバリデーション
        match (&audio_track_id, &video_track_id) {
            (None, None) => {
                return Err(value.invalid("audioTrackId or videoTrackId is required"));
            }
            (Some(audio), Some(video)) if audio == video => {
                let error_value = value.to_member("audioTrackId")?.required()?;
                return Err(error_value.invalid("audioTrackId and videoTrackId must be different"));
            }
            _ => {}
        }

        // ファイルパスのバリデーション
        if !path.exists() {
            let error_value = value.to_member("path")?.required()?;
            return Err(
                error_value.invalid(format!("input path does not exist: {}", path.display()))
            );
        }

        Ok(Self {
            path,
            realtime: realtime.unwrap_or(true),
            loop_playback: loop_playback.unwrap_or(true),
            audio_track_id,
            video_track_id,
        })
    }
}

impl Mp4FileSource {
    pub async fn run(self, outer_processor: ProcessorHandle) -> Result<()> {
        outer_processor.notify_ready();
        outer_processor.wait_subscribers_ready().await?;

        let inner_pipeline = MediaPipeline::new()?;
        let inner_handle = inner_pipeline.handle();
        let task = tokio::spawn(inner_pipeline.run());

        self.initialize_pipeline(inner_handle, &outer_processor)
            .await?;
        drop(outer_processor);
        task.await?;
        Ok(())
    }

    async fn initialize_pipeline(
        &self,
        inner_handle: MediaPipelineHandle,
        outer_processor: &ProcessorHandle,
    ) -> Result<()> {
        let mut options = Mp4FileReaderOptions {
            realtime: self.realtime,
            loop_playback: self.loop_playback,
            audio_track_id: None,
            video_track_id: None,
        };

        // 音声トラックがあるならデコーダーを起動する＆結果を外側に転送する
        if let Some(id) = self.audio_track_id.clone() {
            let inner_id = TrackId::new(format!("{id}_encoded"));
            let decoder = AudioDecoder::new(MediaStreamId::new(0), MediaStreamId::new(1))
                .map_err(|e| Error::new(e.to_string()))?;

            options.audio_track_id = Some(inner_id.clone());
            start_bridge(id.clone(), &inner_handle, outer_processor).await?;
            inner_handle
                .spawn_processor(ProcessorId::new("audio_decoder"), |handle| {
                    decoder.run(handle, inner_id, id)
                })
                .await?;
        }

        // 映像トラックがあるならデコーダーを起動する＆結果を外側に転送する
        if let Some(id) = self.video_track_id.clone() {
            let inner_id = TrackId::new(format!("{id}_encoded"));
            let decoder = VideoDecoder::new(
                MediaStreamId::new(2),
                MediaStreamId::new(3),
                VideoDecoderOptions::default(),
            );

            options.video_track_id = Some(inner_id.clone());
            start_bridge(id.clone(), &inner_handle, outer_processor).await?;
            inner_handle
                .spawn_processor(ProcessorId::new("video_decoder"), |handle| {
                    decoder.run(handle, inner_id, id)
                })
                .await?;
        }

        // MP4 ファイルリーダーを起動する
        // （最初に起動すると、デコーダーが冒頭を取りこぼす恐れがあるので、最後に起動する）
        let reader = Mp4FileReader::new(&self.path, options)?;
        inner_handle
            .spawn_processor(ProcessorId::new("reader"), |handle| reader.run(handle))
            .await?;

        inner_handle.complete_initial_processor_registration();

        Ok(())
    }
}

async fn start_bridge(
    track_id: TrackId,
    inner_handle: &MediaPipelineHandle,
    outer_processor: &ProcessorHandle,
) -> Result<()> {
    let mut tx = outer_processor.publish_track(track_id.clone()).await?;
    let bridge_processor_id = ProcessorId::new(format!("mp4_source_bridge_{track_id}"));

    inner_handle
        .spawn_processor(bridge_processor_id, async move |inner_processor| {
            inner_processor.notify_ready();
            let mut rx = inner_processor.subscribe_track(track_id);
            loop {
                match rx.recv().await {
                    Message::Media(sample) => {
                        if !tx.send_media(sample) {
                            break;
                        }
                    }
                    Message::Eos => {
                        tx.send_eos();
                        break;
                    }
                    Message::Syn(_) => {}
                }
            }
            Ok(())
        })
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::MediaSample;

    #[tokio::test]
    async fn mp4_file_source_decode_smoke() -> Result<()> {
        let pipeline = MediaPipeline::new()?;
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        {
            let handle = handle; // スコープを抜けたらドロップさせる
            let video_track_id = TrackId::new("mp4_file_source_test_video");
            let subscriber = handle
                .register_processor(ProcessorId::new("test_subscriber"))
                .await?;
            let mut rx = subscriber.subscribe_track(video_track_id.clone());
            subscriber.notify_ready();
            handle.complete_initial_processor_registration();

            let source = Mp4FileSource {
                path: PathBuf::from("testdata/archive-red-320x320-av1.mp4"),
                realtime: false,
                loop_playback: false,
                audio_track_id: None,
                video_track_id: Some(video_track_id.clone()),
            };
            handle
                .spawn_processor(ProcessorId::new("source"), |handle| source.run(handle))
                .await?;

            let mut decoded_count = 0;
            loop {
                match rx.recv().await {
                    crate::Message::Media(MediaSample::Video(_)) => {
                        decoded_count += 1;
                    }
                    crate::Message::Eos => {
                        break;
                    }
                    _ => {}
                }
            }
            assert!(decoded_count > 0, "Should decode at least one video frame");
        }

        pipeline_task.await?;

        Ok(())
    }
}
