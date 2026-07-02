use std::path::PathBuf;

use crate::decoder::{AudioDecoder, VideoDecoderOptions};
use crate::mp4::reader::{
    MediaEventContext, MediaInputHandle, Mp4FileReader, Mp4FileReaderOptions,
};
use crate::{ProcessorHandle, Result, TrackId};

#[derive(Debug, Clone)]
pub struct Mp4FileSource {
    pub path: PathBuf,
    pub loop_playback: bool,
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

impl Mp4FileSource {
    /// reader を作成し、メディア再生制御ハンドルを返す。
    /// spawn_processor のクロージャ内で run_reader を呼ぶことで、
    /// ハンドルを外部に返しつつ reader を起動できる。
    pub fn create_reader(
        &self,
        event_ctx: Option<MediaEventContext>,
    ) -> Result<(Mp4FileReader, Option<MediaInputHandle>)> {
        // video_decoder_options を Some で明示することで、 video_track_id が設定されている場合に
        // Mp4FileReader::run 内で video decoder task が spawn される (openh264_lib は run 内で
        // handle.config() から補完される)。
        let video_decoder_options = self
            .video_track_id
            .is_some()
            .then(VideoDecoderOptions::default);
        let options = Mp4FileReaderOptions {
            realtime: true,
            loop_playback: self.loop_playback,
            audio_track_id: self.audio_track_id.clone(),
            video_track_id: self.video_track_id.clone(),
            video_decoder_options,
        };

        let mut reader = Mp4FileReader::new(&self.path, options)?;
        let media_handle = event_ctx.map(|ctx| reader.create_media_handle(ctx));

        Ok((reader, media_handle))
    }

    /// reader にデコーダーを設定して起動する。
    /// video decoder は Mp4FileReaderOptions.video_decoder_options 経由で reader 内部で spawn される。
    pub async fn run_reader(mut reader: Mp4FileReader, processor: ProcessorHandle) -> Result<()> {
        if reader.has_audio_track() {
            let mut decoder_stats = processor.stats();
            decoder_stats.set_default_label("component", "audio_decoder");
            let decoder = AudioDecoder::new(
                #[cfg(feature = "fdk-aac")]
                processor.config().fdk_aac_lib.clone(),
                decoder_stats,
            )?;
            reader.set_audio_decoder(decoder);
        }

        reader.run(processor).await
    }

    #[cfg(test)]
    pub async fn run(self, processor: ProcessorHandle) -> Result<()> {
        let (reader, _media_handle) = self.create_reader(None)?;
        Self::run_reader(reader, processor).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{MediaFrame, MediaPipeline, ProcessorId, ProcessorMetadata, TrackId};
    use shiguredo_openh264::Openh264Library;

    #[tokio::test]
    async fn mp4_file_source_decode_smoke() -> Result<()> {
        let pipeline = MediaPipeline::new(Default::default(), Default::default())?;
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
            let pipeline = MediaPipeline::new(
                crate::MediaPipelineConfig {
                    openh264_lib,
                    #[cfg(feature = "fdk-aac")]
                    fdk_aac_lib: None,
                },
                Default::default(),
            )?;
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
