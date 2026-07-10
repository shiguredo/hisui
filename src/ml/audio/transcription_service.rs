//! Whisper 推論を単一の blocking worker で受け付けるサービス。

use std::path::{Path, PathBuf};

use tokio::sync::{mpsc, oneshot};

use super::whisper::{WhisperPipeline, WhisperTranscript};
use crate::text::LanguageCode;

/// 文字起こしリクエスト。
#[derive(Debug)]
pub struct TranscriptRequest {
    /// 16 kHz mono f32 PCM (最大 30 秒)。
    pub pcm: Vec<f32>,
    /// ISO 639-1 言語コード (多言語モデルで必須)。
    pub language: LanguageCode,
}

#[derive(Debug)]
struct Job {
    request: TranscriptRequest,
    reply_tx: oneshot::Sender<crate::Result<WhisperTranscript>>,
}

/// Whisper 推論を単一の blocking worker で処理するサービス。
///
/// candle CPU 推論は既定でホスト物理コア数まで並列化するため、hisui 側で worker を複数持つと
/// per-decode の並列度がコア競合で相殺される。実効スループットは「1 worker + `RAYON_NUM_THREADS`
/// を絞らない」で頭打ちになるので、pool 化は行わない (将来並列化が必要になれば復活させる)。
#[derive(Debug)]
pub struct TranscriptionService {
    tx: mpsc::Sender<Job>,
}

impl TranscriptionService {
    pub fn new<P: AsRef<Path>>(model_dir: P, device: candle_core::Device) -> crate::Result<Self> {
        let model_dir = model_dir.as_ref().to_path_buf();
        let pipeline = WhisperPipeline::load(&model_dir, device)?;
        let (tx, rx) = mpsc::channel(2);
        spawn_worker(rx, pipeline, model_dir);
        Ok(Self { tx })
    }

    /// キューが満杯なら空くまで待つ。
    pub async fn submit(
        &self,
        request: TranscriptRequest,
    ) -> oneshot::Receiver<crate::Result<WhisperTranscript>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let job = Job { request, reply_tx };
        if let Err(err) = self.tx.send(job).await {
            let _ = err.0.reply_tx.send(Err(crate::Error::new(
                "transcription worker queue is closed",
            )));
        }
        reply_rx
    }
}

fn spawn_worker(mut rx: mpsc::Receiver<Job>, mut pipeline: WhisperPipeline, model_dir: PathBuf) {
    tokio::task::spawn_blocking(move || {
        while let Some(job) = rx.blocking_recv() {
            let result = pipeline
                .transcribe_pcm16k(&job.request.pcm, &job.request.language)
                .map_err(|e| {
                    e.with_context(format!(
                        "transcription worker failed for model {}",
                        model_dir.display()
                    ))
                });
            let _ = job.reply_tx.send(result);
        }
    });
}
