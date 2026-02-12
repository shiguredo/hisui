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
    path: PathBuf,
    realtime: bool,
    loop_playback: bool,
    audio_track_id: Option<TrackId>,
    video_track_id: Option<TrackId>,
}

impl Mp4FileSource {
    pub fn new<P: Into<PathBuf>>(
        path: P,
        realtime: bool,
        loop_playback: bool,
        audio_track_id: Option<TrackId>,
        video_track_id: Option<TrackId>,
    ) -> Self {
        Self {
            path: path.into(),
            realtime,
            loop_playback,
            audio_track_id,
            video_track_id,
        }
    }

    pub async fn run(self, outer_handle: MediaPipelineHandle) -> Result<()> {
        self.validate()?;
        let base_id = self.base_id();

        let inner_pipeline = MediaPipeline::new();
        let inner_handle = inner_pipeline.handle();
        let inner_task = tokio::spawn(async move {
            inner_pipeline.run().await;
        });

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

        let mut bridge_tasks = Vec::new();
        if let Some(track_id) = self.audio_track_id.clone() {
            let task =
                start_bridge("audio", track_id, &inner_handle, &outer_handle, &base_id).await?;
            bridge_tasks.push(task);
        }
        if let Some(track_id) = self.video_track_id.clone() {
            let task =
                start_bridge("video", track_id, &inner_handle, &outer_handle, &base_id).await?;
            bridge_tasks.push(task);
        }

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

    fn validate(&self) -> Result<()> {
        if self.audio_track_id.is_none() && self.video_track_id.is_none() {
            return Err(Error::new("audio_track_id or video_track_id is required"));
        }
        if let (Some(audio_id), Some(video_id)) =
            (self.audio_track_id.as_ref(), self.video_track_id.as_ref())
            && audio_id == video_id
        {
            return Err(Error::new(
                "audio_track_id and video_track_id must be different",
            ));
        }
        if !self.path.exists() {
            return Err(Error::new("input path does not exist"));
        }
        Ok(())
    }

    fn base_id(&self) -> String {
        self.audio_track_id
            .as_ref()
            .or(self.video_track_id.as_ref())
            .map(|id| id.get().to_string())
            .unwrap_or_else(|| "mp4_file_source".to_string())
    }
}

async fn start_bridge(
    kind: &str,
    track_id: TrackId,
    inner_handle: &MediaPipelineHandle,
    outer_handle: &MediaPipelineHandle,
    base_id: &str,
) -> Result<JoinHandle<Result<()>>> {
    let inner_processor_id = ProcessorId::new(format!("mp4_source_inner_{kind}_{base_id}"));
    let outer_processor_id = ProcessorId::new(format!("mp4_source_outer_{kind}_{base_id}"));

    let inner_processor = inner_handle
        .register_processor(inner_processor_id)
        .await
        .map_err(|e| Error::new(format!("Failed to register internal processor: {e}")))?;
    let outer_processor = outer_handle
        .register_processor(outer_processor_id)
        .await
        .map_err(|e| Error::new(format!("Failed to register external processor: {e}")))?;

    Ok(tokio::spawn(async move {
        forward_track(inner_processor, outer_processor, track_id).await
    }))
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
