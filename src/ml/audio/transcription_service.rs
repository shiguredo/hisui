//! Whisper ワーカープール。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use super::whisper::{WhisperPipeline, WhisperTranscription};

/// 文字起こしリクエスト。
pub struct TranscriptRequest {
    /// 16 kHz mono f32 PCM (最大 30 秒)。
    pub pcm: Vec<f32>,
    /// ISO 639-1 言語コード。None なら自動検出またはモデル既定。
    pub language: Option<String>,
}

/// 文字起こし結果。
pub struct TranscriptResult {
    pub text: String,
    pub language: Option<String>,
    pub no_speech_prob: f32,
    pub avg_logprob: f32,
}

struct Job {
    request: TranscriptRequest,
    reply_tx: oneshot::Sender<crate::Result<TranscriptResult>>,
}

/// Whisper モデルを M 個持つワーカープール。
pub struct TranscriptionService {
    tx: mpsc::Sender<Job>,
}

impl TranscriptionService {
    pub fn new<P: AsRef<Path>>(
        model_dir: P,
        device: candle_core::Device,
        workers: usize,
    ) -> crate::Result<Self> {
        if workers == 0 {
            return Err(crate::Error::new(
                "transcription workers must be greater than zero",
            ));
        }

        let model_dir = model_dir.as_ref().to_path_buf();
        let pipelines = (0..workers)
            .map(|_| WhisperPipeline::load(&model_dir, device.clone()))
            .collect::<crate::Result<Vec<_>>>()?;
        let (tx, rx) = mpsc::channel(workers * 2);
        let shared_rx = Arc::new(Mutex::new(rx));

        for pipeline in pipelines {
            spawn_worker(Arc::clone(&shared_rx), pipeline, model_dir.clone());
        }

        Ok(Self { tx })
    }

    /// キューが満杯なら空くまで待つ。
    pub async fn submit(
        &self,
        request: TranscriptRequest,
    ) -> oneshot::Receiver<crate::Result<TranscriptResult>> {
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

fn spawn_worker(
    shared_rx: Arc<Mutex<mpsc::Receiver<Job>>>,
    mut pipeline: WhisperPipeline,
    model_dir: PathBuf,
) {
    tokio::task::spawn_blocking(move || {
        loop {
            let job = {
                let mut rx = shared_rx
                    .lock()
                    .expect("transcription worker receiver mutex must not be poisoned");
                rx.blocking_recv()
            };
            let Some(job) = job else {
                break;
            };
            let result = transcribe_job(&mut pipeline, job.request).map_err(|e| {
                e.with_context(format!(
                    "transcription worker failed for model {}",
                    model_dir.display()
                ))
            });
            let _ = job.reply_tx.send(result);
        }
    });
}

fn transcribe_job(
    pipeline: &mut WhisperPipeline,
    request: TranscriptRequest,
) -> crate::Result<TranscriptResult> {
    let WhisperTranscription {
        text,
        language,
        no_speech_prob,
        avg_logprob,
    } = pipeline.transcribe_pcm16k(&request.pcm, request.language.as_deref())?;
    Ok(TranscriptResult {
        text,
        language,
        no_speech_prob,
        avg_logprob,
    })
}
