//! Whisper 推論を単一の blocking worker で受け付けるサービス。

use std::path::{Path, PathBuf};

use candle_transformers::models::whisper::{LOGPROB_THRESHOLD, NO_SPEECH_THRESHOLD};
use tokio::sync::{mpsc, oneshot};

use super::whisper::{WhisperPipeline, WhisperTranscription};
use crate::text::LanguageCode;

/// 文字起こしリクエスト。
#[derive(Debug)]
pub struct TranscriptRequest {
    /// 16 kHz mono f32 PCM (最大 30 秒)。
    pub pcm: Vec<f32>,
    /// ISO 639-1 言語コード (多言語モデルで必須)。
    pub language: LanguageCode,
}

/// 文字起こし結果。
#[derive(Debug)]
pub struct TranscriptResult {
    pub text: String,
    pub language: Option<LanguageCode>,
    pub no_speech_prob: f32,
    pub avg_logprob: f32,
}

impl TranscriptResult {
    /// 詳細は `crate::ml::audio::whisper::decode::WhisperDecodedChunk::is_likely_no_speech` を参照。
    pub fn is_likely_no_speech(&self) -> bool {
        f64::from(self.no_speech_prob) > NO_SPEECH_THRESHOLD
            && f64::from(self.avg_logprob) < LOGPROB_THRESHOLD
    }
}

#[derive(Debug)]
struct Job {
    request: TranscriptRequest,
    reply_tx: oneshot::Sender<crate::Result<TranscriptResult>>,
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

fn spawn_worker(mut rx: mpsc::Receiver<Job>, mut pipeline: WhisperPipeline, model_dir: PathBuf) {
    tokio::task::spawn_blocking(move || {
        while let Some(job) = rx.blocking_recv() {
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
    } = pipeline.transcribe_pcm16k(&request.pcm, &request.language)?;
    Ok(TranscriptResult {
        text,
        language,
        no_speech_prob,
        avg_logprob,
    })
}
