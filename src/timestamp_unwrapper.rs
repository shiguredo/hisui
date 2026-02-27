use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TimestampUnwrapper {
    mask: u64,
    modulus: u64,
    half_modulus: u64,
    wrap_count: u64,
    last_raw: Option<u64>,
    last_unwrapped: u64,
}

impl TimestampUnwrapper {
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
            last_unwrapped: 0,
        }
    }

    pub fn unwrap(&mut self, raw: u64) -> u64 {
        let raw = raw & self.mask;

        if let Some(last_raw) = self.last_raw
            && raw < last_raw
            && last_raw - raw > self.half_modulus
        {
            self.wrap_count = self.wrap_count.saturating_add(1);
        }

        let candidate = raw.saturating_add(self.wrap_count.saturating_mul(self.modulus));
        let unwrapped = if self.last_raw.is_none() {
            candidate
        } else {
            candidate.max(self.last_unwrapped)
        };

        self.last_raw = Some(raw);
        self.last_unwrapped = unwrapped;
        unwrapped
    }
}

#[derive(Debug, Clone)]
pub struct TimestampMapper {
    unwrapper: TimestampUnwrapper,
    tick_hz: u64,
    offset: Duration,
    base: Option<u64>,
}

impl TimestampMapper {
    pub fn new(bits: u8, tick_hz: u64, offset: Duration) -> Self {
        assert!(tick_hz > 0, "tick_hz must be greater than 0");
        Self {
            unwrapper: TimestampUnwrapper::new(bits),
            tick_hz,
            offset,
            base: None,
        }
    }

    pub fn map(&mut self, raw: u64) -> Duration {
        let unwrapped = self.unwrapper.unwrap(raw);
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
    use super::{TimestampMapper, TimestampUnwrapper};
    use std::time::Duration;

    #[test]
    fn unwrap_keeps_monotonic_without_wrap() {
        let mut unwrapper = TimestampUnwrapper::new(32);
        assert_eq!(unwrapper.unwrap(100), 100);
        assert_eq!(unwrapper.unwrap(120), 120);
        assert_eq!(unwrapper.unwrap(121), 121);
    }

    #[test]
    fn unwrap_handles_32bit_wrap() {
        let mut unwrapper = TimestampUnwrapper::new(32);
        assert_eq!(unwrapper.unwrap(u32::MAX as u64 - 5), u32::MAX as u64 - 5);
        assert_eq!(unwrapper.unwrap(10), (1u64 << 32) + 10);
    }

    #[test]
    fn unwrap_handles_33bit_wrap() {
        let mut unwrapper = TimestampUnwrapper::new(33);
        assert_eq!(unwrapper.unwrap((1u64 << 33) - 2), (1u64 << 33) - 2);
        assert_eq!(unwrapper.unwrap(3), (1u64 << 33) + 3);
    }

    #[test]
    fn unwrap_clamps_out_of_order_input() {
        let mut unwrapper = TimestampUnwrapper::new(32);
        assert_eq!(unwrapper.unwrap(100), 100);
        assert_eq!(unwrapper.unwrap(90), 100);
    }

    #[test]
    fn mapper_uses_first_value_as_base_and_applies_offset() {
        let mut mapper = TimestampMapper::new(32, 1_000, Duration::from_secs(5));
        assert_eq!(mapper.map(100), Duration::from_secs(5));
        assert_eq!(mapper.map(130), Duration::from_millis(5030));
    }

    #[test]
    fn mapper_keeps_monotonic_across_wrap() {
        let mut mapper = TimestampMapper::new(32, 1_000, Duration::ZERO);
        let _ = mapper.map(u32::MAX as u64 - 2);
        let mapped = mapper.map(1);
        assert_eq!(mapped, Duration::from_millis(4));
    }
}
