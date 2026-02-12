use std::path::PathBuf;

use tokio::task::JoinHandle;

use crate::decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions};
use crate::file_reader_mp4::{Mp4FileReader, Mp4FileReaderOptions};
use crate::media::MediaStreamId;
use crate::{
    Error, MediaPipeline, MediaPipelineHandle, Message, ProcessorHandle, ProcessorId, Result,
    TrackId,
};

#[derive(Debug, Clone)]
pub struct Mp4FileSource {
    pub processor_id: ProcessorId,
    pub path: PathBuf,
    pub realtime: bool,
    pub loop_playback: bool,
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

impl nojson::DisplayJson for Mp4FileSource {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("processorId", &self.processor_id)?;
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
        let processor_id: ProcessorId = value.to_member("processorId")?.required()?.try_into()?;
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
            processor_id,
            path,
            realtime: realtime.unwrap_or(true),
            loop_playback: loop_playback.unwrap_or(true),
            audio_track_id,
            video_track_id,
        })
    }
}

impl Mp4FileSource {
    pub async fn run(self, outer_handle: MediaPipelineHandle) -> Result<()> {
        let base_id = self.processor_id.get();

        // reader / decoder を繋ぐための内部パイプラインを作成する
        let inner_pipeline = MediaPipeline::new();
        let inner_handle = inner_pipeline.handle();
        let inner_task = tokio::spawn(inner_pipeline.run());

        let audio_output = self.audio_track_id.clone();
        let audio_encoded = audio_output
            .as_ref()
            .map(|id| TrackId::new(format!("{}_encoded", id.get())));
        let video_output = self.video_track_id.clone();
        let video_encoded = video_output
            .as_ref()
            .map(|id| TrackId::new(format!("{}_encoded", id.get())));

        if let (Some(input_track_id), Some(output_track_id)) =
            (audio_encoded.clone(), audio_output.clone())
        {
            let decoder = AudioDecoder::new(MediaStreamId::new(0), MediaStreamId::new(1))
                .map_err(|e| Error::new(e.to_string()))?;
            let processor_id = ProcessorId::new(format!("audio_decoder_{base_id}"));
            inner_handle
                .spawn_processor(processor_id, |handle| {
                    let decoder = decoder;
                    async move {
                        if let Err(e) = decoder.run(handle, input_track_id, output_track_id).await {
                            tracing::error!("audio decoder failed: {e}");
                        }
                        Ok(())
                    }
                })
                .await
                .map_err(|e| Error::new(format!("Failed to spawn audio decoder: {e}")))?;
        }

        if let (Some(input_track_id), Some(output_track_id)) =
            (video_encoded.clone(), video_output.clone())
        {
            let decoder = VideoDecoder::new(
                MediaStreamId::new(2),
                MediaStreamId::new(3),
                VideoDecoderOptions::default(),
            );
            let processor_id = ProcessorId::new(format!("video_decoder_{base_id}"));
            inner_handle
                .spawn_processor(processor_id, |handle| {
                    let decoder = decoder;
                    async move {
                        if let Err(e) = decoder.run(handle, input_track_id, output_track_id).await {
                            tracing::error!("video decoder failed: {e}");
                        }
                        Ok(())
                    }
                })
                .await
                .map_err(|e| Error::new(format!("Failed to spawn video decoder: {e}")))?;
        }

        let mut bridge_tasks = Vec::new();
        if let Some(track_id) = self.audio_track_id.clone() {
            let task = start_bridge(
                track_id,
                &inner_handle,
                &outer_handle,
                &base_id,
                self.processor_id.clone(),
            )
            .await?;
            bridge_tasks.push(task);
        }
        if let Some(track_id) = self.video_track_id.clone() {
            let task = start_bridge(
                track_id,
                &inner_handle,
                &outer_handle,
                &base_id,
                self.processor_id.clone(),
            )
            .await?;
            bridge_tasks.push(task);
        }

        let options = Mp4FileReaderOptions {
            realtime: self.realtime,
            loop_playback: self.loop_playback,
            audio_track_id: audio_encoded,
            video_track_id: video_encoded,
        };
        let reader = Mp4FileReader::new(&self.path, options)?;
        let processor_id = ProcessorId::new(format!("mp4_file_reader_{base_id}"));
        inner_handle
            .spawn_processor(processor_id, |handle| {
                let reader = reader;
                async move {
                    if let Err(e) = reader.run(handle).await {
                        tracing::error!("mp4 file reader failed: {e}");
                    }
                    Ok(())
                }
            })
            .await
            .map_err(|e| Error::new(format!("Failed to spawn mp4 file reader: {e}")))?;

        drop(inner_handle);
        drop(outer_handle);

        for task in bridge_tasks {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(Error::new(format!("bridge task failed: {e}"))),
            }
        }

        if let Err(e) = inner_task.await {
            return Err(Error::new(format!("internal pipeline task failed: {e}")));
        }

        Ok(())
    }
}

async fn start_bridge(
    track_id: TrackId,
    inner_handle: &MediaPipelineHandle,
    outer_handle: &MediaPipelineHandle,
    base_id: &str,
    outer_processor_id: ProcessorId,
) -> Result<JoinHandle<Result<()>>> {
    let inner_processor_id = ProcessorId::new(format!("mp4_source_bridge_{base_id}"));

    // [NOTE] inner の方は常に成功するはず
    let inner_processor = inner_handle.register_processor(inner_processor_id).await?;
    let outer_processor = outer_handle.register_processor(outer_processor_id).await?;

    Ok(tokio::spawn(forward_track(
        inner_processor,
        outer_processor,
        track_id,
    )))
}

async fn forward_track(
    inner_processor: ProcessorHandle,
    outer_processor: ProcessorHandle,
    track_id: TrackId,
) -> Result<()> {
    let mut rx = inner_processor.subscribe_track(track_id.clone());
    let mut tx = outer_processor.publish_track(track_id).await?;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::MediaSample;

    #[tokio::test]
    async fn mp4_file_source_decode_smoke() -> Result<()> {
        let pipeline = MediaPipeline::new();
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let video_track_id = TrackId::new("mp4_file_source_test_video");
        let subscriber_handle = handle.clone();
        let subscriber = subscriber_handle
            .register_processor(ProcessorId::new("mp4_file_source_test_subscriber"))
            .await?;
        let mut rx = subscriber.subscribe_track(video_track_id.clone());

        let source = Mp4FileSource {
            processor_id: ProcessorId::new("mp4_source_outer"),
            path: PathBuf::from("testdata/archive-red-320x320-av1.mp4"),
            realtime: false,
            loop_playback: false,
            audio_track_id: None,
            video_track_id: Some(video_track_id.clone()),
        };
        let source_handle = handle.clone();
        let source_task = tokio::spawn(source.run(source_handle));

        drop(handle);

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

        drop(subscriber);
        drop(subscriber_handle);

        let source_result = source_task
            .await
            .map_err(|e| crate::Error::new(format!("source task failed: {e}")))?;
        source_result?;

        pipeline_task
            .await
            .map_err(|e| crate::Error::new(format!("pipeline task failed: {e}")))?;

        assert!(decoded_count > 0, "Should decode at least one video frame");
        Ok(())
    }
}
