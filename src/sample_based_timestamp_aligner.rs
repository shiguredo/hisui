use std::time::Duration;

use crate::audio::SampleRate;

pub const DEFAULT_REBASE_THRESHOLD: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
/// 音声デコード後の timestamp を、入力 timestamp と出力 sample 数の両方を使って安定生成するための補助構造体。
///
/// AAC のようにデコーダー内部でバッファリングが発生する形式では、入力 packet と出力 frame が 1 対 1 に対応しない。
/// そのため入力 timestamp を単純に引き継ぐだけでは、長時間動作時に徐々にずれが見えやすい。
///
/// この構造体は次の方針で timestamp を決める。
/// - 最初の入力 timestamp を基準オフセットとして採用する
/// - 以降は出力 sample 数を積算して連続した timestamp を推定する
/// - 入力 timestamp と推定値の乖離が閾値を超えた場合のみ基準を再設定する
///
/// この設計により、通常時は sample 数基準で誤差蓄積を抑えつつ、入力側のフレーム欠落や飛びが発生した場合にも
/// timestamp を速やかに追従させられる。
pub struct SampleBasedTimestampAligner {
    sample_rate: SampleRate,
    rebase_threshold: Duration,
    base_input_timestamp: Option<Duration>,
    base_output_samples: u64,
}

impl SampleBasedTimestampAligner {
    pub fn new(sample_rate: SampleRate, rebase_threshold: Duration) -> Self {
        Self {
            sample_rate,
            rebase_threshold,
            base_input_timestamp: None,
            base_output_samples: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    pub fn align_input_timestamp(&mut self, input_timestamp: Duration, output_samples: u64) {
        if self.base_input_timestamp.is_none() {
            self.set_alignment_base(input_timestamp, output_samples);
            return;
        }

        let predicted_timestamp = self.estimate_timestamp_from_output_samples(output_samples);
        if predicted_timestamp.abs_diff(input_timestamp) > self.rebase_threshold {
            self.set_alignment_base(input_timestamp, output_samples);
        }
    }

    pub fn estimate_timestamp_from_output_samples(&self, output_samples: u64) -> Duration {
        let Some(base_input_timestamp) = self.base_input_timestamp else {
            return Duration::ZERO;
        };

        let relative_samples = output_samples.saturating_sub(self.base_output_samples);
        base_input_timestamp
            .saturating_add(self.sample_rate.duration_from_samples(relative_samples))
    }

    fn set_alignment_base(&mut self, input_timestamp: Duration, output_samples: u64) {
        self.base_input_timestamp = Some(input_timestamp);
        self.base_output_samples = output_samples;
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_REBASE_THRESHOLD, SampleBasedTimestampAligner};
    use crate::audio::SampleRate;
    use std::time::Duration;

    fn new_aligner() -> SampleBasedTimestampAligner {
        SampleBasedTimestampAligner::new(SampleRate::HZ_48000, DEFAULT_REBASE_THRESHOLD)
    }

    #[test]
    fn aligner_adopts_first_input_timestamp_as_base() {
        let mut aligner = new_aligner();
        let input_timestamp = Duration::from_millis(500);

        aligner.align_input_timestamp(input_timestamp, 0);

        assert_eq!(
            aligner.estimate_timestamp_from_output_samples(0),
            input_timestamp
        );
    }

    #[test]
    fn aligner_advances_timestamp_by_output_samples() {
        let mut aligner = new_aligner();
        aligner.align_input_timestamp(Duration::from_millis(0), 0);

        // 48 kHz なので 4_800 sample は 100 ms。
        assert_eq!(
            aligner.estimate_timestamp_from_output_samples(4_800),
            DEFAULT_REBASE_THRESHOLD
        );
    }

    #[test]
    fn aligner_rebases_when_input_timestamp_drift_is_large() {
        let mut aligner = new_aligner();
        aligner.align_input_timestamp(Duration::from_millis(0), 0);

        // 予測値は 20 ms だが、入力は 250 ms なので 100 ms を超えて乖離している。
        aligner.align_input_timestamp(Duration::from_millis(250), 960);

        assert_eq!(
            aligner.estimate_timestamp_from_output_samples(960),
            Duration::from_millis(250)
        );
        assert_eq!(
            aligner.estimate_timestamp_from_output_samples(1_920),
            Duration::from_millis(270)
        );
    }

    #[test]
    fn aligner_does_not_rebase_on_threshold_boundary() {
        let mut aligner = new_aligner();
        aligner.align_input_timestamp(Duration::from_millis(0), 0);

        // 予測値は 20 ms。入力を 120 ms にして差分をちょうど 100 ms にする。
        aligner.align_input_timestamp(Duration::from_millis(120), 960);

        // 差分が閾値ちょうどの場合はリベースしない。
        assert_eq!(
            aligner.estimate_timestamp_from_output_samples(960),
            Duration::from_millis(20)
        );
    }
}
