use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

const BAR_WIDTH: usize = 40;
const DRAW_INTERVAL: Duration = Duration::from_millis(200);
const CLEAR_LINE: &str = "\r\x1b[2K";
const NO_ETA: &str = "?";

#[derive(Debug, Clone, Copy)]
enum ProgressKind {
    Time,
    Frame,
}

#[derive(Debug)]
pub struct ProgressBar {
    total: u64,
    position: u64,
    start: Instant,
    last_draw: Instant,
    kind: ProgressKind,
    enabled: bool,
    use_color: bool,
    finished: bool,
}

impl ProgressBar {
    fn new(total: u64, kind: ProgressKind) -> Self {
        let stderr = io::stderr();
        let enabled = stderr.is_terminal();
        let use_color = enabled;
        let now = Instant::now();
        let last_draw = now.checked_sub(DRAW_INTERVAL).unwrap_or(now);
        Self {
            total,
            position: 0,
            start: now,
            last_draw,
            kind,
            enabled,
            use_color,
            finished: false,
        }
    }

    pub fn inc(&mut self, delta: u64) {
        let next = self.position.saturating_add(delta);
        self.set_position(next);
    }

    pub fn set_position(&mut self, position: u64) {
        if self.finished {
            return;
        }
        self.position = position;
        self.draw(false);
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        if self.total > 0 {
            self.position = self.total;
        }
        self.draw(true);
        if self.enabled {
            let mut stderr = io::stderr();
            let _ = writeln!(stderr);
        }
        self.finished = true;
    }

    fn draw(&mut self, force: bool) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        if !force && now.duration_since(self.last_draw) < DRAW_INTERVAL {
            return;
        }
        self.last_draw = now;
        let line = self.render_line(now.duration_since(self.start));
        let mut stderr = io::stderr();
        let _ = write!(stderr, "{}{}", CLEAR_LINE, line);
        let _ = stderr.flush();
    }

    fn render_line(&self, elapsed: Duration) -> String {
        let elapsed_text = format_duration(elapsed);
        let eta_text = format_eta(self.total, self.position, elapsed);
        let bar_text = render_bar(self.total, self.position, BAR_WIDTH, self.use_color);
        let percent = calc_percent(self.total, self.position);
        match self.kind {
            ProgressKind::Time => format!(
                "[{elapsed_text} (ETA: {eta_text})] [{bar_text}] complete {percent}% of {len}s total output duration",
                len = self.total
            ),
            ProgressKind::Frame => format!(
                "[{elapsed_text} (ETA: {eta_text})] [{bar_text}] complete {percent}% of {len} total frames",
                len = self.total
            ),
        }
    }
}

pub fn create_time_progress_bar(total_duration: Duration) -> ProgressBar {
    ProgressBar::new(total_duration.as_secs(), ProgressKind::Time)
}

pub fn create_frame_progress_bar(total_frames: u64) -> ProgressBar {
    ProgressBar::new(total_frames, ProgressKind::Frame)
}

#[derive(Clone, Copy)]
enum AnsiColor {
    Cyan,
    Blue,
}

impl AnsiColor {
    fn code(self) -> &'static str {
        match self {
            AnsiColor::Cyan => "\x1b[36m",
            AnsiColor::Blue => "\x1b[34m",
        }
    }
}

fn colorize(text: &str, color: AnsiColor, use_color: bool) -> String {
    if !use_color || text.is_empty() {
        return text.to_string();
    }
    format!("{}{}{}", color.code(), text, "\x1b[0m")
}

fn calc_percent(total: u64, position: u64) -> u64 {
    if total == 0 {
        return 100;
    }
    let clamped = position.min(total);
    ((clamped as u128) * 100 / total as u128) as u64
}

fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn format_eta(total: u64, position: u64, elapsed: Duration) -> String {
    if total == 0 || position == 0 {
        return NO_ETA.to_string();
    }
    let elapsed_secs = elapsed.as_secs_f64();
    if elapsed_secs <= 0.0 {
        return NO_ETA.to_string();
    }
    let rate = position as f64 / elapsed_secs;
    if rate <= 0.0 || !rate.is_finite() {
        return NO_ETA.to_string();
    }
    let remaining = total.saturating_sub(position) as f64 / rate;
    if !remaining.is_finite() || remaining < 0.0 {
        return NO_ETA.to_string();
    }
    let remaining_secs = remaining.floor().max(0.0) as u64;
    format_eta_text(remaining_secs)
}

fn render_bar(total: u64, position: u64, width: usize, use_color: bool) -> String {
    if width == 0 {
        return String::new();
    }
    let clamped = position.min(total);
    let filled = if total == 0 {
        width
    } else {
        (clamped.saturating_mul(width as u64) / total) as usize
    };
    let (filled_len, head_len, empty_len) = if filled >= width {
        (width, 0, 0)
    } else if filled == 0 {
        (0, 1, width - 1)
    } else {
        (filled - 1, 1, width - filled)
    };
    let mut filled_text = String::new();
    if filled_len > 0 {
        filled_text.push_str(&"#".repeat(filled_len));
    }
    if head_len > 0 {
        filled_text.push('>');
    }
    let empty_text = "-".repeat(empty_len);
    if use_color {
        format!(
            "{}{}",
            colorize(&filled_text, AnsiColor::Cyan, true),
            colorize(&empty_text, AnsiColor::Blue, true)
        )
    } else {
        format!("{filled_text}{empty_text}")
    }
}

fn format_eta_text(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_handles_basic_values() {
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_duration(Duration::from_secs(59)), "00:00:59");
        assert_eq!(format_duration(Duration::from_secs(60)), "00:01:00");
        assert_eq!(format_duration(Duration::from_secs(3661)), "01:01:01");
    }

    #[test]
    fn render_bar_renders_expected_segments() {
        assert_eq!(render_bar(100, 0, 10, false), ">---------");
        assert_eq!(render_bar(100, 50, 10, false), "####>-----");
        assert_eq!(render_bar(100, 100, 10, false), "##########");
    }

    #[test]
    fn format_eta_returns_placeholder_when_unavailable() {
        assert_eq!(
            format_eta(100, 0, Duration::from_secs(0)),
            NO_ETA.to_string()
        );
    }

    #[test]
    fn format_eta_text_formats_human_readable() {
        assert_eq!(format_eta_text(0), "0s");
        assert_eq!(format_eta_text(59), "59s");
        assert_eq!(format_eta_text(60), "1m 0s");
        assert_eq!(format_eta_text(3661), "1h 1m 1s");
    }

    #[test]
    fn calc_percent_handles_zero_total() {
        assert_eq!(calc_percent(0, 0), 100);
        assert_eq!(calc_percent(0, 42), 100);
    }

    #[test]
    fn calc_percent_clamps_position() {
        assert_eq!(calc_percent(100, 150), 100);
        assert_eq!(calc_percent(100, 99), 99);
    }

    #[test]
    fn calc_percent_avoids_overflow() {
        let total = u64::MAX;
        assert_eq!(calc_percent(total, total), 100);
    }
}
