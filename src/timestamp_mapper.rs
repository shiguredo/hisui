use std::time::Duration;

/// 周回する生 timestamp を展開し、連続する整数 timestamp として扱う。
#[derive(Debug, Clone)]
pub struct WrappingTimestampNormalizer {
    mask: u64,
    modulus: u64,
    half_modulus: u64,
    wrap_count: u64,
    last_raw: Option<u64>,
}

impl WrappingTimestampNormalizer {
    pub fn new(bits: u8) -> Self {
        assert!(
            (1..64).contains(&bits),
            "timestamp bits must be in range 1..64"
        );
        let modulus = 1u64 << bits;
        Self {
            mask: modulus - 1,
            modulus,
            half_modulus: modulus / 2,
            wrap_count: 0,
            last_raw: None,
        }
    }

    /// 周回のみを補正して timestamp を展開する。
    ///
    /// 小さな逆行入力はそのまま反映する。
    pub fn normalize(&mut self, raw: u64) -> u64 {
        let raw = raw & self.mask;

        if let Some(last_raw) = self.last_raw
            && raw < last_raw
            && last_raw - raw > self.half_modulus
        {
            self.wrap_count = self.wrap_count.saturating_add(1);
        }

        let unwrapped = raw.saturating_add(self.wrap_count.saturating_mul(self.modulus));

        self.last_raw = Some(raw);
        unwrapped
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
    tick_hz: u64,
    offset: Duration,
    base: Option<u64>,
}

impl TimestampMapper {
    pub fn new(bits: u8, tick_hz: u64, offset: Duration) -> Self {
        assert!(tick_hz > 0, "tick_hz must be greater than 0");
        Self {
            normalizer: WrappingTimestampNormalizer::new(bits),
            tick_hz,
            offset,
            base: None,
        }
    }

    pub fn map(&mut self, raw: u64) -> Duration {
        let unwrapped = self.normalizer.normalize(raw);
        let base = *self.base.get_or_insert(unwrapped);
        let relative = unwrapped.saturating_sub(base);
        ticks_to_duration(relative, self.tick_hz).saturating_add(self.offset)
    }
}

fn ticks_to_duration(ticks: u64, tick_hz: u64) -> Duration {
    Duration::from_micros(ticks.saturating_mul(1_000_000) / tick_hz)
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
    }

    #[test]
    fn mapper_applies_base_and_offset() {
        let mut mapper = TimestampMapper::new(32, 1_000, Duration::from_secs(5));
        assert_eq!(mapper.map(100), Duration::from_secs(5));
        assert_eq!(mapper.map(130), Duration::from_millis(5030));
    }

    #[test]
    fn mapper_keeps_progress_across_wrap() {
        let mut mapper = TimestampMapper::new(32, 1_000, Duration::ZERO);
        let _ = mapper.map(u32::MAX as u64 - 2);
        let mapped = mapper.map(1);
        assert_eq!(mapped, Duration::from_millis(4));
    }
}
