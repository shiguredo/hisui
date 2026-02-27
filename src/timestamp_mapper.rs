use std::num::NonZeroU64;
use std::time::Duration;

/// 周回する生 timestamp を展開し、連続する整数 timestamp として扱う。
#[derive(Debug, Clone)]
struct WrappingTimestampNormalizer {
    mask: u64,
    modulus: u64,
    half_modulus: u64,
    wrap_count: u64,
    wrap_detection_high_water_raw: Option<u64>,
}

impl WrappingTimestampNormalizer {
    fn new(bits: u8) -> crate::Result<Self> {
        // wrap 判定で `half_modulus` を閾値として使うため、
        // `bits=1` だと差分が常に `> half_modulus` を満たせず実用的な判定ができない。
        if !(2..64).contains(&bits) {
            return Err(crate::Error::new("timestamp bits must be in range 2..64"));
        }
        let modulus = 1u64 << bits;
        Ok(Self {
            mask: modulus - 1,
            modulus,
            half_modulus: modulus / 2,
            wrap_count: 0,
            wrap_detection_high_water_raw: None,
        })
    }

    /// 周回のみを補正して timestamp を展開する。
    ///
    /// 小さな逆行入力はそのまま反映する。
    ///
    /// wrap 判定は `high-water mark` 方式で行う。
    /// 一時的な逆行で判定基準が下がると wrap を見逃すため、
    /// 判定基準は同一 epoch 内で最大の生 timestamp を保持する。
    fn normalize(&mut self, raw: u64) -> u64 {
        let raw = raw & self.mask;

        if let Some(high_water_raw) = self.wrap_detection_high_water_raw {
            if raw < high_water_raw && high_water_raw - raw > self.half_modulus {
                self.wrap_count = self.wrap_count.saturating_add(1);
                self.wrap_detection_high_water_raw = Some(raw);
            } else if raw > high_water_raw {
                self.wrap_detection_high_water_raw = Some(raw);
            }
        } else {
            self.wrap_detection_high_water_raw = Some(raw);
        }

        raw.saturating_add(self.wrap_count.saturating_mul(self.modulus))
    }
}

/// 生 timestamp を `Duration` へ変換する補助構造体。
///
/// 内部で次を行う。
/// - bit 幅に応じた周回補正
/// - 初回値を基準とした相対化
/// - `tick_hz` を使った `Duration` 変換
/// - `offset` の加算
///
#[derive(Debug, Clone)]
pub struct TimestampMapper {
    normalizer: WrappingTimestampNormalizer,
    tick_hz: NonZeroU64,
    offset: Duration,
    base: Option<u64>,
}

impl TimestampMapper {
    pub fn new(bits: u8, tick_hz: u64, offset: Duration) -> crate::Result<Self> {
        let tick_hz = NonZeroU64::new(tick_hz)
            .ok_or_else(|| crate::Error::new("tick_hz must be greater than 0"))?;
        Ok(Self {
            normalizer: WrappingTimestampNormalizer::new(bits)?,
            tick_hz,
            offset,
            base: None,
        })
    }

    pub fn map(&mut self, raw: u64) -> Duration {
        let unwrapped = self.normalizer.normalize(raw);
        let base = *self.base.get_or_insert(unwrapped);
        let relative = unwrapped.saturating_sub(base);
        ticks_to_duration(relative, self.tick_hz).saturating_add(self.offset)
    }
}

fn ticks_to_duration(ticks: u64, tick_hz: NonZeroU64) -> Duration {
    Duration::from_micros(ticks.saturating_mul(1_000_000) / tick_hz.get())
}

#[cfg(test)]
mod tests {
    use super::{TimestampMapper, WrappingTimestampNormalizer};
    use std::time::Duration;

    #[test]
    fn normalizer_keeps_sequence_without_wrap() {
        let mut normalizer = WrappingTimestampNormalizer::new(32);
        assert_eq!(normalizer.normalize(100), 100);
        assert_eq!(normalizer.normalize(120), 120);
        assert_eq!(normalizer.normalize(121), 121);
    }

    #[test]
    fn normalizer_handles_32bit_wrap() {
        let mut normalizer = WrappingTimestampNormalizer::new(32);
        assert_eq!(
            normalizer.normalize(u32::MAX as u64 - 5),
            u32::MAX as u64 - 5
        );
        assert_eq!(normalizer.normalize(10), (1u64 << 32) + 10);
    }

    #[test]
    fn normalizer_handles_33bit_wrap() {
        let mut normalizer = WrappingTimestampNormalizer::new(33);
        assert_eq!(normalizer.normalize((1u64 << 33) - 2), (1u64 << 33) - 2);
        assert_eq!(normalizer.normalize(3), (1u64 << 33) + 3);
    }

    #[test]
    fn normalizer_keeps_small_backward_input() {
        let mut normalizer = WrappingTimestampNormalizer::new(32);
        assert_eq!(normalizer.normalize(100), 100);
        assert_eq!(normalizer.normalize(90), 90);
        assert_eq!(normalizer.normalize(110), 110);
    }

    #[test]
    fn normalizer_detects_wrap_after_small_backward_step() {
        let mut normalizer = WrappingTimestampNormalizer::new(32);
        let half = 1u64 << 31;
        assert_eq!(normalizer.normalize(half + 1), half + 1);
        assert_eq!(normalizer.normalize(half - 1), half - 1);
        assert_eq!(normalizer.normalize(0), 1u64 << 32);
    }

    #[test]
    fn normalizer_does_not_wrap_when_diff_equals_half_modulus() {
        let mut normalizer = WrappingTimestampNormalizer::new(4);
        assert_eq!(normalizer.normalize(12), 12);
        // 12 -> 4 は差分が 8 (= half_modulus) のため wrap 判定しない。
        assert_eq!(normalizer.normalize(4), 4);
    }

    #[test]
    fn mapper_applies_base_and_offset() {
        let mut mapper =
            TimestampMapper::new(32, 1_000, Duration::from_secs(5)).expect("infallible");
        assert_eq!(mapper.map(100), Duration::from_secs(5));
        assert_eq!(mapper.map(130), Duration::from_millis(5030));
    }

    #[test]
    fn mapper_keeps_progress_across_wrap() {
        let mut mapper = TimestampMapper::new(32, 1_000, Duration::ZERO).expect("infallible");
        let _ = mapper.map(u32::MAX as u64 - 2);
        let mapped = mapper.map(1);
        assert_eq!(mapped, Duration::from_millis(4));
    }

    #[test]
    fn mapper_handles_multiple_wraps() {
        // bits=3 のため modulus は 8、half_modulus は 4。
        let mut mapper = TimestampMapper::new(3, 1, Duration::ZERO).expect("infallible");

        // base は初回値 6。
        assert_eq!(mapper.map(6), Duration::ZERO);
        // 6 -> 1 は差分 5 (> 4) なので 1 回目の wrap。
        assert_eq!(mapper.map(1), Duration::from_secs(3));
        // 1 -> 7 は通常前進。
        assert_eq!(mapper.map(7), Duration::from_secs(9));
        // 7 -> 0 は差分 7 (> 4) なので 2 回目の wrap。
        assert_eq!(mapper.map(0), Duration::from_secs(10));
    }

    #[test]
    fn mapper_rejects_zero_tick_hz() {
        let err = TimestampMapper::new(32, 0, Duration::ZERO).expect_err("must fail");
        assert_eq!(err.reason, "tick_hz must be greater than 0");
    }

    #[test]
    fn mapper_rejects_bits_one() {
        let err = TimestampMapper::new(1, 1_000, Duration::ZERO).expect_err("must fail");
        assert_eq!(err.reason, "timestamp bits must be in range 2..64");
    }
}
