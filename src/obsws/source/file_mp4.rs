use std::path::PathBuf;

use crate::decoder::{AudioDecoder, VideoDecoder, VideoDecoderOptions};
use crate::file_reader_mp4::{Mp4FileReader, Mp4FileReaderOptions};
use crate::{ProcessorHandle, ProcessorId, ProcessorMetadata, Result, TrackId};

#[derive(Debug, Clone)]
pub struct Mp4FileSource {
    pub path: PathBuf,
    pub loop_playback: bool,
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

impl Mp4FileSource {
    pub async fn run(self, processor: ProcessorHandle) -> Result<()> {
        let pipeline_handle = processor.pipeline_handle();
        let openh264_lib = processor.config().openh264_lib.clone();
        let processor_id = processor.processor_id().clone();

        let mut options = Mp4FileReaderOptions {
            realtime: true,
            loop_playback: self.loop_playback,
            audio_track_id: None,
            video_track_id: None,
        };

        // 音声デコーダーを外側パイプラインに登録
        if let Some(id) = self.audio_track_id.clone() {
            let encoded_id = TrackId::new(format!("{id}_encoded"));
            options.audio_track_id = Some(encoded_id.clone());
            pipeline_handle
                .spawn_processor(
                    ProcessorId::new(format!("{processor_id}_audio_decoder")),
                    ProcessorMetadata::new("audio_decoder"),
                    |handle| async move {
                        let decoder = AudioDecoder::new(
                            #[cfg(feature = "fdk-aac")]
                            handle.config().fdk_aac_lib.clone(),
                            handle.stats(),
                        )?;
                        decoder.run(handle, encoded_id, id).await
                    },
                )
                .await?;
        }

        // 映像デコーダーを外側パイプラインに登録
        if let Some(id) = self.video_track_id.clone() {
            let encoded_id = TrackId::new(format!("{id}_encoded"));
            options.video_track_id = Some(encoded_id.clone());
            pipeline_handle
                .spawn_processor(
                    ProcessorId::new(format!("{processor_id}_video_decoder")),
                    ProcessorMetadata::new("video_decoder"),
                    |handle| async move {
                        let decoder = VideoDecoder::new(
                            VideoDecoderOptions {
                                openh264_lib,
                                ..Default::default()
                            },
                            handle.stats(),
                        );
                        decoder.run(handle, encoded_id, id).await
                    },
                )
                .await?;
        }

        // source プロセッサ自身がリーダーとして動作する
        let reader = Mp4FileReader::new(&self.path, options)?;
        reader.run(processor).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{MediaFrame, MediaPipeline, ProcessorId, ProcessorMetadata, TrackId};
    use shiguredo_openh264::Openh264Library;

    #[tokio::test]
    async fn mp4_file_source_decode_smoke() -> Result<()> {
        let pipeline = MediaPipeline::new()?;
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        {
            let handle = handle; // スコープを抜けたらドロップさせる
            let video_track_id = TrackId::new("mp4_file_source_test_video");
            let subscriber = handle
                .register_processor(
                    ProcessorId::new("test_subscriber"),
                    ProcessorMetadata::new("test_subscriber"),
                )
                .await?;
            let mut rx = subscriber.subscribe_track(video_track_id.clone());
            subscriber.notify_ready();
            assert!(
                handle
                    .trigger_start()
                    .await
                    .expect("trigger_start must succeed")
            );

            let source = Mp4FileSource {
                path: PathBuf::from("testdata/archive-red-320x320-av1.mp4"),
                loop_playback: false,
                audio_track_id: None,
                video_track_id: Some(video_track_id.clone()),
            };
            handle
                .spawn_processor(
                    ProcessorId::new("source"),
                    ProcessorMetadata::new("mp4_file_source"),
                    |handle| source.run(handle),
                )
                .await?;

            let mut decoded_count = 0;
            loop {
                match rx.recv().await {
                    crate::Message::Media(MediaFrame::Video(_)) => {
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

    #[test]
    fn mp4_file_source_h264_decode_smoke() -> Result<()> {
        let openh264_lib = if let Ok(path) = std::env::var("OPENH264_PATH") {
            Some(Openh264Library::load(path)?)
        } else {
            eprintln!("no available OpenH264 decoder");
            return Ok(());
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let pipeline = MediaPipeline::new_with_config(crate::MediaPipelineConfig {
                openh264_lib,
                #[cfg(feature = "fdk-aac")]
                fdk_aac_lib: None,
            })?;
            let handle = pipeline.handle();
            let pipeline_task = tokio::spawn(pipeline.run());
            {
                let handle = handle;
                let video_track_id = TrackId::new("mp4_file_source_test_h264_video");
                let subscriber = handle
                    .register_processor(
                        ProcessorId::new("test_h264_subscriber"),
                        ProcessorMetadata::new("test_h264_subscriber"),
                    )
                    .await?;
                let mut rx = subscriber.subscribe_track(video_track_id.clone());
                subscriber.notify_ready();
                assert!(
                    handle
                        .trigger_start()
                        .await
                        .expect("trigger_start must succeed")
                );

                let source = Mp4FileSource {
                    path: PathBuf::from("testdata/archive-red-320x320-h264.mp4"),
                    loop_playback: false,
                    audio_track_id: None,
                    video_track_id: Some(video_track_id.clone()),
                };
                handle
                    .spawn_processor(
                        ProcessorId::new("h264_source"),
                        ProcessorMetadata::new("mp4_file_source"),
                        |handle| source.run(handle),
                    )
                    .await?;

                let mut decoded_count = 0;
                loop {
                    match rx.recv().await {
                        crate::Message::Media(MediaFrame::Video(_)) => {
                            decoded_count += 1;
                        }
                        crate::Message::Eos => {
                            break;
                        }
                        _ => {}
                    }
                }
                assert!(
                    decoded_count > 0,
                    "Should decode at least one H264 video frame"
                );
            }

            pipeline_task.abort();
            Ok(())
        })
    }
}
