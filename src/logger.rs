use std::io::IsTerminal;
use std::time::UNIX_EPOCH;

use shiguredo_webrtc::log::Severity;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format;
use tracing_subscriber::registry::LookupSpan;

struct Formatter {
    ansi: bool,
}

impl<S, N> FormatEvent<S, N> for Formatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let time = UNIX_EPOCH.elapsed().ok().unwrap_or_default();

        // メタデータからレベルとモジュールパスを取得する
        let metadata = event.metadata();
        let level = metadata.level();
        let module_path = metadata.module_path().unwrap_or_default();

        // メッセージフィールドを取得する
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);

        // 行全体に severity に応じた色を適用する
        if self.ansi {
            write!(writer, "{}", level_color(level))?;
        }

        write_iso8601(&mut writer, time)?;
        write!(writer, " [{}] {} - {}", level, module_path, message)?;

        if self.ansi {
            writeln!(writer, "\x1b[0m")
        } else {
            writeln!(writer)
        }
    }
}

/// エポックからの日数を年月日に変換する (Howard Hinnant のアルゴリズム)
/// http://howardhinnant.github.io/date_algorithms.html#civil_from_days
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64 + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Duration を ISO 8601 UTC 形式 (YYYY-MM-DDTHH:MM:SS.ffffffZ) で書き出す
fn write_iso8601(writer: &mut format::Writer<'_>, dur: std::time::Duration) -> std::fmt::Result {
    let total_secs = dur.as_secs();
    let usec = dur.subsec_micros();
    let days = (total_secs / 86400) as i64;
    let day_secs = total_secs % 86400;
    let (y, mo, d) = civil_from_days(days);
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    write!(
        writer,
        "{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{usec:06}Z"
    )
}

/// severity に対応する ANSI カラーコードを返す
fn level_color(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::ERROR => "\x1b[1;31m",
        tracing::Level::WARN => "\x1b[1;33m",
        tracing::Level::INFO => "\x1b[0;96m",
        tracing::Level::DEBUG => "\x1b[0;97m",
        tracing::Level::TRACE => "\x1b[0;36m",
    }
}

/// イベントから message フィールドを取り出すビジター
struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            use std::fmt::Write;
            let _ = write!(self.0, "{:?}", value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        }
    }
}

pub fn init(level: tracing::level_filters::LevelFilter) {
    // .init() 内部で tracing_log::LogTracer が自動的に初期化されるため、
    // サブクレートの log 出力も tracing に橋渡しされる
    //
    // なお NO_COLOR 環境変数が設定されている場合は色付けを無効にする
    // https://no-color.org/
    let ansi = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    tracing_subscriber::fmt()
        .with_max_level(level)
        .event_format(Formatter { ansi })
        .with_writer(std::io::stderr)
        .init();

    init_webrtc_log_from_env();
}

fn init_webrtc_log_from_env() {
    let Ok(raw) = std::env::var("HISUI_WEBRTC_LOG") else {
        return;
    };

    let Some(severity) = parse_webrtc_log_severity(&raw) else {
        tracing::warn!(
            "invalid HISUI_WEBRTC_LOG value: {raw} (expected: verbose|info|warning|error|none)"
        );
        return;
    };

    shiguredo_webrtc::log::log_to_debug(severity);
    if severity != Severity::None {
        shiguredo_webrtc::log::enable_timestamps();
        shiguredo_webrtc::log::enable_threads();
    }
    tracing::info!(
        "WebRTC native log enabled: {}",
        webrtc_log_severity_name(severity)
    );
}

fn parse_webrtc_log_severity(value: &str) -> Option<Severity> {
    match value.trim().to_ascii_lowercase().as_str() {
        "verbose" => Some(Severity::Verbose),
        "info" => Some(Severity::Info),
        "warning" => Some(Severity::Warning),
        "error" => Some(Severity::Error),
        "none" => Some(Severity::None),
        _ => None,
    }
}

fn webrtc_log_severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Verbose => "verbose",
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
        Severity::None => "none",
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_webrtc_log_severity_accepts_levels() {
        assert_eq!(
            parse_webrtc_log_severity("verbose"),
            Some(Severity::Verbose)
        );
        assert_eq!(parse_webrtc_log_severity("info"), Some(Severity::Info));
        assert_eq!(
            parse_webrtc_log_severity("warning"),
            Some(Severity::Warning)
        );
        assert_eq!(parse_webrtc_log_severity("error"), Some(Severity::Error));
        assert_eq!(parse_webrtc_log_severity("none"), Some(Severity::None));
    }

    #[test]
    fn parse_webrtc_log_severity_rejects_unknown_value() {
        assert_eq!(parse_webrtc_log_severity("loud"), None);
    }

    #[test]
    fn civil_from_days_unix_epoch() {
        // 1970-01-01
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_known_dates() {
        // 2024-04-03
        assert_eq!(civil_from_days(19816), (2024, 4, 3));
        // 2000-01-01
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
        // 2026-04-03
        assert_eq!(civil_from_days(20546), (2026, 4, 3));
    }
}
