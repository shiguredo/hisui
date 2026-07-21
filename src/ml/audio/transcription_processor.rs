//! MediaPipeline 上の Whisper 文字起こし processor。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::audio::{AudioFormat, AudioFrame, Channels, SampleRate, resample::resample_to_mono};
use crate::media::MediaFrame;
use crate::ml::audio::config::VadConfig;
use crate::ml::audio::silero_vad::SileroVadModel;
use crate::ml::audio::transcription_service::{TranscriptRequest, TranscriptionService};
use crate::ml::audio::vad::{SpeechSegment, VadGate};
use crate::ml::audio::whisper::WhisperTranscript;
use crate::text::{LanguageCode, TextFrame};
use crate::{Message, ProcessorHandle, TrackId};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const MAX_TRANSCRIPT_SAMPLES: usize = 30 * TARGET_SAMPLE_RATE as usize;
const MIN_TRANSCRIPT_SAMPLES: usize = 160;

#[derive(Debug)]
struct QueuedChunk {
    start_sample: u64,
    end_sample: u64,
    pcm: Vec<f32>,
}

#[derive(Debug)]
struct PendingTranscript {
    start: Duration,
    end: Duration,
    result_rx: oneshot::Receiver<crate::Result<WhisperTranscript>>,
}

/// `publish_transcript` の結果。 pipeline が閉じている場合は上位 loop が正常終了する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishOutcome {
    Published,
    PipelineClosed,
}

/// 1 audio track を 1 text track に変換する processor。
#[derive(Debug)]
pub struct TranscriptionProcessor {
    service: Arc<TranscriptionService>,
    silero: Arc<SileroVadModel>,
    language: LanguageCode,
    input_track_id: TrackId,
    output_track_id: TrackId,
}

impl TranscriptionProcessor {
    pub fn new(
        service: Arc<TranscriptionService>,
        silero: Arc<SileroVadModel>,
        language: LanguageCode,
        input_track_id: TrackId,
        output_track_id: TrackId,
    ) -> Self {
        Self {
            service,
            silero,
            language,
            input_track_id,
            output_track_id,
        }
    }

    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let mut input_rx = handle.subscribe_track(self.input_track_id);
        let mut output_tx = handle.publish_track(self.output_track_id).await?;
        handle.notify_ready();
        handle.wait_subscribers_ready().await?;

        let mut state = ProcessorState::new(self.service, self.silero, self.language);

        loop {
            // pending が空の間は input のみ待つ。 EOS 到達時は handle_message 内で
            // pending をドレインしてから true を返すので、ここで別途ドレインは不要。
            if state.pending.is_empty() {
                let message = input_rx.recv().await;
                if state.handle_message(message, &mut output_tx).await? {
                    output_tx.send_eos();
                    return Ok(());
                }
                continue;
            }

            let pending_result = &mut state
                .pending
                .front_mut()
                .expect("pending queue must not be empty")
                .result_rx;
            tokio::select! {
                message = input_rx.recv() => {
                    if state.handle_message(message, &mut output_tx).await? {
                        output_tx.send_eos();
                        return Ok(());
                    }
                }
                result = pending_result => {
                    // oneshot::Receiver は Unpin なので &mut Receiver を直接 await して
                    // 結果を受け取る。 RecvError (channel クローズ) は crate::Error に、
                    // 内側の crate::Result はそのまま伝播させる。
                    let pending = state.pending.pop_front().expect("pending queue head exists");
                    let transcript = result
                        .map_err(|e| {
                            crate::Error::new(format!(
                                "transcription result channel closed: {e}"
                            ))
                        })??;
                    let outcome = publish_transcript(
                        pending.start,
                        pending.end,
                        transcript,
                        &mut output_tx,
                    );
                    if matches!(outcome, PublishOutcome::PipelineClosed) {
                        // AsyncVideoDecoder 等と同じく pipeline クローズは
                        // 正常終了扱いとする (Err にしない)。
                        output_tx.send_eos();
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
struct ProcessorState {
    service: Arc<TranscriptionService>,
    vad: VadGate,
    language: LanguageCode,
    input_sample_rate: Option<SampleRate>,
    input_channels: Option<Channels>,
    base_offset: Option<Duration>,
    input_buffer: Vec<f32>,
    retained_pcm16k: Vec<f32>,
    retained_start_sample: u64,
    pending: VecDeque<PendingTranscript>,
}

impl ProcessorState {
    fn new(
        service: Arc<TranscriptionService>,
        silero: Arc<SileroVadModel>,
        language: LanguageCode,
    ) -> Self {
        Self {
            service,
            vad: VadGate::new(silero.new_instance(), VadConfig::default()),
            language,
            input_sample_rate: None,
            input_channels: None,
            base_offset: None,
            input_buffer: Vec::new(),
            retained_pcm16k: Vec::new(),
            retained_start_sample: 0,
            pending: VecDeque::new(),
        }
    }

    async fn handle_message(
        &mut self,
        message: Message,
        output_tx: &mut crate::TrackPublisher,
    ) -> crate::Result<bool> {
        match message {
            Message::Media(frame) => {
                let frame = frame.expect_audio()?;
                self.handle_audio_frame(&frame).await?;
                Ok(false)
            }
            Message::Eos => {
                self.flush_end_of_stream().await?;
                while let Some(pending) = self.pending.pop_front() {
                    if matches!(
                        await_and_publish(pending, output_tx).await?,
                        PublishOutcome::PipelineClosed,
                    ) {
                        // 下流がクローズしたので残りは publish せず捨てる。
                        // 呼び出し元 (run) がこの後 send_eos を呼ぶ。
                        self.pending.clear();
                        break;
                    }
                }
                Ok(true)
            }
            Message::Syn(_) => Ok(false),
        }
    }

    async fn handle_audio_frame(&mut self, frame: &AudioFrame) -> crate::Result<()> {
        ensure_audio_format(frame)?;
        self.base_offset.get_or_insert(frame.timestamp);
        self.remember_audio_layout(frame)?;
        let normalized = frame.samples_i16()?.map(|s| f32::from(s) / 32768.0);
        self.input_buffer.extend(normalized);

        let samples_per_second =
            usize::try_from(
                self.input_sample_rate
                    .expect("sample rate initialized")
                    .get(),
            )? * usize::from(self.input_channels.expect("channels initialized").get());
        while self.input_buffer.len() >= samples_per_second {
            let chunk: Vec<f32> = self.input_buffer.drain(..samples_per_second).collect();
            let resampled = resample_to_mono(
                &chunk,
                self.input_sample_rate.expect("sample rate initialized"),
                SampleRate::from_u32(TARGET_SAMPLE_RATE)?,
                self.input_channels.expect("channels initialized"),
            )?;
            self.handle_resampled_pcm(&resampled).await?;
        }
        Ok(())
    }

    async fn flush_end_of_stream(&mut self) -> crate::Result<()> {
        if !self.input_buffer.is_empty() {
            let resampled = resample_to_mono(
                &self.input_buffer,
                self.input_sample_rate.ok_or_else(|| {
                    crate::Error::new("EOS reached before any audio frame was received")
                })?,
                SampleRate::from_u32(TARGET_SAMPLE_RATE)?,
                self.input_channels.ok_or_else(|| {
                    crate::Error::new("EOS reached before any audio frame was received")
                })?,
            )?;
            self.input_buffer.clear();
            self.handle_resampled_pcm(&resampled).await?;
        }

        for segment in self.vad.flush()? {
            self.queue_segment(segment).await?;
        }
        Ok(())
    }

    async fn handle_resampled_pcm(&mut self, pcm16k: &[f32]) -> crate::Result<()> {
        if pcm16k.is_empty() {
            return Ok(());
        }
        self.retained_pcm16k.extend_from_slice(pcm16k);
        for segment in self.vad.feed(pcm16k)? {
            self.queue_segment(segment).await?;
        }
        self.drop_consumed_pcm();
        Ok(())
    }

    async fn queue_segment(&mut self, segment: SpeechSegment) -> crate::Result<()> {
        // VAD の invariant では segment.start_sample / end_sample は retained 範囲内に
        // 収まるが、万一崩れたときに slice out-of-range panic ではなく Err で拾えるように
        // ガードする。 メッセージには不変条件破りの原因追跡に必要な実値を残す。
        let relative_start = segment
            .start_sample
            .checked_sub(self.retained_start_sample)
            .ok_or_else(|| {
                crate::Error::new(format!(
                    "segment start_sample {} is before retained_start_sample {}",
                    segment.start_sample, self.retained_start_sample,
                ))
            })?;
        let relative_end = segment
            .end_sample
            .checked_sub(self.retained_start_sample)
            .ok_or_else(|| {
                crate::Error::new(format!(
                    "segment end_sample {} is before retained_start_sample {}",
                    segment.end_sample, self.retained_start_sample,
                ))
            })?;
        let relative_start = usize::try_from(relative_start)?;
        let relative_end = usize::try_from(relative_end)?;
        if relative_end > self.retained_pcm16k.len() {
            return Err(crate::Error::new(format!(
                "segment end_sample {} exceeds retained PCM range: retained_start_sample={}, retained_len={}",
                segment.end_sample,
                self.retained_start_sample,
                self.retained_pcm16k.len(),
            )));
        }
        let segment_pcm = &self.retained_pcm16k[relative_start..relative_end];
        for chunk in split_segment_pcm(segment.start_sample, segment_pcm) {
            self.submit_chunk(chunk).await?;
        }
        Ok(())
    }

    async fn submit_chunk(&mut self, chunk: QueuedChunk) -> crate::Result<()> {
        let request = TranscriptRequest {
            pcm: chunk.pcm,
            language: self.language.clone(),
        };
        let start = self.segment_time(chunk.start_sample)?;
        let end = self.segment_time(chunk.end_sample)?;
        let result_rx = self.service.submit(request).await;
        self.pending.push_back(PendingTranscript {
            start,
            end,
            result_rx,
        });
        Ok(())
    }

    fn drop_consumed_pcm(&mut self) {
        let min_required = self.vad.min_required_sample();
        if min_required <= self.retained_start_sample {
            return;
        }
        let drop_len = usize::try_from(min_required - self.retained_start_sample)
            .expect("sample difference must fit into usize");
        if drop_len >= self.retained_pcm16k.len() {
            self.retained_pcm16k.clear();
        } else {
            self.retained_pcm16k.drain(..drop_len);
        }
        self.retained_start_sample = min_required;
    }

    fn remember_audio_layout(&mut self, frame: &AudioFrame) -> crate::Result<()> {
        check_layout_consistency(self.input_sample_rate, self.input_channels, frame)?;
        self.input_sample_rate = Some(frame.sample_rate);
        self.input_channels = Some(frame.channels);
        Ok(())
    }

    fn segment_time(&self, sample: u64) -> crate::Result<Duration> {
        let base = self
            .base_offset
            .ok_or_else(|| crate::Error::new("base offset is not initialized"))?;
        Ok(base + duration_from_16k_samples(sample))
    }
}

/// 受信フレームが I16Be フォーマットかを確認する (副作用なしの純関数)。
///
/// TranscriptionProcessor は upstream 側で Opus / AAC を decode 済みの I16Be 前提で組む
/// ため、それ以外のフォーマットは構成ミスとして Err を返す。
fn ensure_audio_format(frame: &AudioFrame) -> crate::Result<()> {
    if frame.format != AudioFormat::I16Be {
        return Err(crate::Error::new(format!(
            "transcription processor expects I16Be input, got {}",
            frame.format
        )));
    }
    Ok(())
}

/// これまでに保持している sample_rate / channels と、新しく受信した frame のそれとの
/// 矛盾を検出する (副作用なしの純関数)。
///
/// 前回値が `None` (未初期化) なら Ok。前回値が `Some` で frame と異なるなら Err。
/// 実際に代入する責務は `remember_audio_layout` 側 (`&mut self`) に残す。
fn check_layout_consistency(
    prev_sample_rate: Option<SampleRate>,
    prev_channels: Option<Channels>,
    frame: &AudioFrame,
) -> crate::Result<()> {
    match prev_sample_rate {
        Some(sample_rate) if sample_rate != frame.sample_rate => {
            return Err(crate::Error::new(format!(
                "input sample rate changed during transcription: {} -> {}",
                sample_rate.get(),
                frame.sample_rate.get()
            )));
        }
        _ => {}
    }
    match prev_channels {
        Some(channels) if channels != frame.channels => {
            return Err(crate::Error::new(format!(
                "input channel count changed during transcription: {} -> {}",
                channels.get(),
                frame.channels.get()
            )));
        }
        _ => {}
    }
    Ok(())
}

/// 先頭 pending の推論結果を待ち、解決した `WhisperTranscript` を text track に流す。
///
/// self を触らないため `impl ProcessorState` の外に置く。 oneshot の `RecvError` は
/// worker crash / panic のサインなので `crate::Error` に包み直して伝播させる。
async fn await_and_publish(
    pending: PendingTranscript,
    output_tx: &mut crate::TrackPublisher,
) -> crate::Result<PublishOutcome> {
    let transcript = pending
        .result_rx
        .await
        .map_err(|e| crate::Error::new(format!("transcription result channel closed: {e}")))??;
    Ok(publish_transcript(
        pending.start,
        pending.end,
        transcript,
        output_tx,
    ))
}

/// 解決済み `WhisperTranscript` を text track に流す。
///
/// 無音判定 (`is_likely_no_speech`) と空テキストは publish しない (`Published` を返す)。
/// `send_media` が false を返した場合 (下流の receiver がクローズ済み) は
/// `PipelineClosed` を返し、呼び出し元に正常終了処理を任せる。
fn publish_transcript(
    start: Duration,
    end: Duration,
    transcript: WhisperTranscript,
    output_tx: &mut crate::TrackPublisher,
) -> PublishOutcome {
    if !should_publish(&transcript) {
        return PublishOutcome::Published;
    }

    let frame = build_text_frame(start, end, transcript);
    if output_tx.send_media(MediaFrame::new_text(frame)) {
        PublishOutcome::Published
    } else {
        PublishOutcome::PipelineClosed
    }
}

/// `WhisperTranscript` を text track に流すべきか判定する (副作用なしの純関数)。
///
/// 無音判定 (`is_likely_no_speech`) が真、または text が空のときは skip 対象。
fn should_publish(transcript: &WhisperTranscript) -> bool {
    !transcript.is_likely_no_speech() && !transcript.text.is_empty()
}

/// `WhisperTranscript` を track 時刻付きの `TextFrame` に組み立てる (副作用なしの純関数)。
fn build_text_frame(start: Duration, end: Duration, transcript: WhisperTranscript) -> TextFrame {
    TextFrame {
        start,
        end,
        text: transcript.text,
        language: transcript.language,
        no_speech_prob: Some(transcript.no_speech_prob.get() as f32),
        avg_logprob: Some(transcript.avg_logprob.get() as f32),
    }
}

fn split_segment_pcm(start_sample: u64, pcm: &[f32]) -> Vec<QueuedChunk> {
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    while offset < pcm.len() {
        let end = usize::min(offset + MAX_TRANSCRIPT_SAMPLES, pcm.len());
        let len = end - offset;
        if len < MIN_TRANSCRIPT_SAMPLES {
            break;
        }
        chunks.push(QueuedChunk {
            start_sample: start_sample + offset as u64,
            end_sample: start_sample + end as u64,
            pcm: pcm[offset..end].to_vec(),
        });
        offset = end;
    }
    chunks
}

/// 16 kHz サンプル通し番号を `Duration` に丸め誤差ゼロで写像する。
///
/// 1 サンプル = 62_500 ns (16000 は 1_000_000_000 の約数)。
/// `u64::MAX / 62_500 ≈ 9370 年`ぶんまでオーバーフロー無しに扱える。
fn duration_from_16k_samples(samples: u64) -> Duration {
    Duration::from_nanos(samples * 62_500)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PCM 全長が 30 秒超なら 30 秒単位に分割し、末尾 10 ms 未満は破棄する。
    #[test]
    fn split_segment_pcm_splits_by_30_seconds() {
        let pcm = vec![0.0; MAX_TRANSCRIPT_SAMPLES * 2 + 200];
        let chunks = split_segment_pcm(100, &pcm);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].start_sample, 100);
        assert_eq!(chunks[0].end_sample, 100 + MAX_TRANSCRIPT_SAMPLES as u64);
        assert_eq!(chunks[1].start_sample, 100 + MAX_TRANSCRIPT_SAMPLES as u64);
        assert_eq!(
            chunks[1].end_sample,
            100 + (MAX_TRANSCRIPT_SAMPLES * 2) as u64
        );
        assert_eq!(
            chunks[2].start_sample,
            100 + (MAX_TRANSCRIPT_SAMPLES * 2) as u64
        );
        assert_eq!(
            chunks[2].end_sample,
            100 + (MAX_TRANSCRIPT_SAMPLES * 2 + 200) as u64
        );
        assert_eq!(chunks[2].pcm.len(), 200);
    }

    /// 末尾が 10 ms 未満なら捨てる。
    #[test]
    fn split_segment_pcm_drops_short_tail() {
        let pcm = vec![0.0; MAX_TRANSCRIPT_SAMPLES + MIN_TRANSCRIPT_SAMPLES - 1];
        let chunks = split_segment_pcm(0, &pcm);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].pcm.len(), MAX_TRANSCRIPT_SAMPLES);
    }

    /// 16 kHz サンプル番号から track 時刻へ丸め誤差なく写像できる。
    #[test]
    fn duration_from_16k_samples_maps_exactly() {
        assert_eq!(duration_from_16k_samples(16_000), Duration::from_secs(1));
        assert_eq!(duration_from_16k_samples(1), Duration::from_nanos(62_500));
    }

    /// 空 pcm は 0 chunks (while ループの入口で即抜ける)。
    #[test]
    fn split_segment_pcm_returns_empty_for_empty_pcm() {
        let chunks = split_segment_pcm(0, &[]);
        assert!(chunks.is_empty(), "空 pcm は 0 chunks のはず");
    }

    /// MIN 未満は 0 chunks (先頭で break)。
    #[test]
    fn split_segment_pcm_returns_empty_below_min() {
        let pcm = vec![0.0; MIN_TRANSCRIPT_SAMPLES - 1];
        let chunks = split_segment_pcm(0, &pcm);
        assert!(chunks.is_empty(), "MIN 未満は 0 chunks のはず");
    }

    /// MIN ちょうどで 1 chunk (MIN 長)。
    #[test]
    fn split_segment_pcm_yields_one_chunk_at_min() {
        let pcm = vec![0.0; MIN_TRANSCRIPT_SAMPLES];
        let chunks = split_segment_pcm(0, &pcm);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].pcm.len(), MIN_TRANSCRIPT_SAMPLES);
        assert_eq!(chunks[0].start_sample, 0);
        assert_eq!(chunks[0].end_sample, MIN_TRANSCRIPT_SAMPLES as u64);
    }

    /// MAX ちょうどで 1 chunk (MAX 長)。
    #[test]
    fn split_segment_pcm_yields_one_chunk_at_max() {
        let pcm = vec![0.0; MAX_TRANSCRIPT_SAMPLES];
        let chunks = split_segment_pcm(0, &pcm);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].pcm.len(), MAX_TRANSCRIPT_SAMPLES);
    }

    /// MAX + MIN ちょうどで 2 chunks (MAX + MIN の順)。 末尾がちょうど MIN で
    /// 破棄されないケースの境界。
    #[test]
    fn split_segment_pcm_yields_two_chunks_at_max_plus_min() {
        let pcm = vec![0.0; MAX_TRANSCRIPT_SAMPLES + MIN_TRANSCRIPT_SAMPLES];
        let chunks = split_segment_pcm(0, &pcm);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].pcm.len(), MAX_TRANSCRIPT_SAMPLES);
        assert_eq!(chunks[1].pcm.len(), MIN_TRANSCRIPT_SAMPLES);
    }

    /// テスト用に `WhisperTranscript` を組み立てるヘルパ。
    fn make_transcript(text: &str, no_speech: f64, avg_lp: f64) -> WhisperTranscript {
        WhisperTranscript {
            text: text.to_owned(),
            language: Some(LanguageCode::new("en")),
            no_speech_prob: crate::probability::Probability::new(no_speech).expect("有効な確率"),
            avg_logprob: crate::probability::LogProbability::new(avg_lp).expect("有効な対数確率"),
        }
    }

    /// 非無音・非空 text の transcript は publish 対象。
    #[test]
    fn should_publish_returns_true_for_normal_transcript() {
        let transcript = make_transcript("hello", 0.1, -0.3);
        assert!(should_publish(&transcript));
    }

    /// 空テキストは (品質指標が良くても) publish 対象外。
    #[test]
    fn should_publish_returns_false_for_empty_text() {
        let transcript = make_transcript("", 0.1, -0.3);
        assert!(!should_publish(&transcript));
    }

    /// 無音判定 (no_speech_prob > 0.6 かつ avg_logprob < -1.0) は publish 対象外。
    #[test]
    fn should_publish_returns_false_for_likely_no_speech() {
        let transcript = make_transcript("hello", 0.7, -1.5);
        assert!(!should_publish(&transcript));
    }

    /// build_text_frame は start / end / text / language / 品質指標を透過する。
    #[test]
    fn build_text_frame_preserves_all_fields() {
        let transcript = make_transcript("hello", 0.2, -0.4);
        let frame = build_text_frame(
            Duration::from_millis(100),
            Duration::from_millis(500),
            transcript,
        );
        assert_eq!(frame.start, Duration::from_millis(100));
        assert_eq!(frame.end, Duration::from_millis(500));
        assert_eq!(frame.text, "hello");
        assert_eq!(frame.language.as_ref().map(LanguageCode::get), Some("en"));
        assert_eq!(frame.no_speech_prob, Some(0.2_f32));
        assert_eq!(frame.avg_logprob, Some(-0.4_f32));
    }

    /// テスト用に指定した format / sample_rate / channels の AudioFrame を組み立てる。
    /// data は 1 サンプル分の 0x0000 を積んで最低限の valid PCM にする。
    fn make_audio_frame(
        format: AudioFormat,
        sample_rate: SampleRate,
        channels: Channels,
    ) -> AudioFrame {
        let bytes_per_sample = 2 * usize::from(channels.get());
        AudioFrame {
            data: vec![0u8; bytes_per_sample],
            format,
            channels,
            sample_rate,
            timestamp: Duration::ZERO,
            sample_entry: None,
        }
    }

    /// I16Be フォーマットのフレームは ensure_audio_format で Ok。
    #[test]
    fn ensure_audio_format_accepts_i16be() {
        let frame = make_audio_frame(
            AudioFormat::I16Be,
            SampleRate::from_u32(48_000).expect("48 kHz は有効"),
            Channels::STEREO,
        );
        ensure_audio_format(&frame).expect("I16Be は Ok のはず");
    }

    /// Opus フォーマットのフレームは Err を返し、メッセージに I16Be の期待を含む。
    #[test]
    fn ensure_audio_format_rejects_opus() {
        let frame = make_audio_frame(
            AudioFormat::Opus,
            SampleRate::from_u32(48_000).expect("48 kHz は有効"),
            Channels::STEREO,
        );
        let err = ensure_audio_format(&frame).expect_err("Opus は Err のはず");
        let message = err.display().to_string();
        assert!(
            message.contains("I16Be"),
            "エラーメッセージに I16Be の期待を含むこと: {message}"
        );
    }

    /// Aac フォーマットのフレームも Err を返す (I16Be 以外は一律 Err の回帰保護)。
    #[test]
    fn ensure_audio_format_rejects_aac() {
        let frame = make_audio_frame(
            AudioFormat::Aac,
            SampleRate::from_u32(48_000).expect("48 kHz は有効"),
            Channels::STEREO,
        );
        assert!(ensure_audio_format(&frame).is_err(), "Aac は Err のはず");
    }

    /// 前回値が未初期化 (None / None) なら check_layout_consistency は Ok。
    #[test]
    fn check_layout_consistency_accepts_first_frame() {
        let frame = make_audio_frame(
            AudioFormat::I16Be,
            SampleRate::from_u32(48_000).expect("48 kHz は有効"),
            Channels::STEREO,
        );
        check_layout_consistency(None, None, &frame).expect("初回フレームは Ok のはず");
    }

    /// 前回値と frame の sample_rate / channels が同一なら Ok (継続フレーム)。
    #[test]
    fn check_layout_consistency_accepts_matching_layout() {
        let frame = make_audio_frame(
            AudioFormat::I16Be,
            SampleRate::from_u32(48_000).expect("48 kHz は有効"),
            Channels::STEREO,
        );
        check_layout_consistency(Some(frame.sample_rate), Some(frame.channels), &frame)
            .expect("同一レイアウトは Ok のはず");
    }

    /// sample_rate 変化は Err、メッセージに旧→新の値と "sample rate" を含む。
    #[test]
    fn check_layout_consistency_rejects_sample_rate_change() {
        let frame = make_audio_frame(
            AudioFormat::I16Be,
            SampleRate::from_u32(16_000).expect("16 kHz は有効"),
            Channels::STEREO,
        );
        let prev = SampleRate::from_u32(48_000).expect("48 kHz は有効");
        let err = check_layout_consistency(Some(prev), Some(frame.channels), &frame)
            .expect_err("sample rate 変化は Err のはず");
        let message = err.display().to_string();
        assert!(
            message.contains("sample rate")
                && message.contains("48000")
                && message.contains("16000"),
            "エラーメッセージに sample rate と旧→新値 (48000 -> 16000) を含むこと: {message}"
        );
    }

    /// channels 変化は Err、メッセージに旧→新の値と "channel" を含む。
    #[test]
    fn check_layout_consistency_rejects_channels_change() {
        let frame = make_audio_frame(
            AudioFormat::I16Be,
            SampleRate::from_u32(48_000).expect("48 kHz は有効"),
            Channels::MONO,
        );
        let err = check_layout_consistency(Some(frame.sample_rate), Some(Channels::STEREO), &frame)
            .expect_err("channels 変化は Err のはず");
        let message = err.display().to_string();
        assert!(
            message.contains("channel") && message.contains("2") && message.contains("1"),
            "エラーメッセージに channel と旧→新値 (2 -> 1) を含むこと: {message}"
        );
    }
}
