use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crate::ml::audio::{buffer::AudioChunkBuffer, vad::VadGate, whisper::WhisperPipeline};
use crate::{MediaFrame, Result};

/// 入力オーディオトラックに Whisper 転写を適用し、結果をログ出力する
pub struct AudioMlProcessor {
    pub input_track_id: crate::TrackId,
    pub whisper: WhisperPipeline,
    pub chunk_secs: u32,
    pub vad: VadGate,
    pub running: Arc<AtomicBool>,
}

impl AudioMlProcessor {
    pub async fn run(mut self, handle: crate::ProcessorHandle) -> Result<()> {
        let input_track_id = self.input_track_id;
        let chunk_secs = self.chunk_secs;
        let mut vad = self.vad;

        let mut input_rx = handle.subscribe_track(input_track_id);
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let mut buffer: Option<AudioChunkBuffer> = None;
        let mut chunk_index: u64 = 0;

        while self.running.load(Ordering::Relaxed) {
            match input_rx.recv().await {
                crate::Message::Media(MediaFrame::Audio(frame)) => {
                    if buffer.is_none() {
                        let sample_rate = frame.sample_rate.get();
                        buffer = Some(AudioChunkBuffer::new(sample_rate, chunk_secs)?);
                        tracing::info!(
                            "audio ml: capture {} Hz, chunk {}s",
                            sample_rate,
                            chunk_secs
                        );
                    }

                    let Some(buf) = buffer.as_mut() else {
                        continue;
                    };
                    if let Some(pcm) = buf.push_frame(&frame)? {
                        chunk_index += 1;
                        Self::transcribe_chunk(&mut self.whisper, &mut vad, chunk_index, &pcm)?;
                    }
                }
                crate::Message::Eos => {
                    if let Some(mut buf) = buffer {
                        if let Some(pcm) = buf.flush() {
                            chunk_index += 1;
                            Self::transcribe_chunk(&mut self.whisper, &mut vad, chunk_index, &pcm)?;
                        }
                    }
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn transcribe_chunk(
        whisper: &mut WhisperPipeline,
        vad: &mut VadGate,
        chunk_index: u64,
        pcm: &[f32],
    ) -> Result<()> {
        let Some(pcm_for_whisper) = vad.pcm_for_whisper(pcm)? else {
            tracing::debug!(chunk = chunk_index, "vad: skip chunk");
            return Ok(());
        };
        match whisper.transcribe_pcm16k(&pcm_for_whisper) {
            Ok(text) if !text.is_empty() => {
                tracing::info!(chunk = chunk_index, "transcript: {text}");
            }
            Ok(_) => {
                tracing::debug!(chunk = chunk_index, "no speech in chunk");
            }
            Err(e) => {
                tracing::error!(chunk = chunk_index, "transcribe error: {e:?}");
            }
        }
        Ok(())
    }
}
