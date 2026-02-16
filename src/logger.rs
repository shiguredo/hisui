use std::time::UNIX_EPOCH;

use shiguredo_webrtc::log::Severity;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format;
use tracing_subscriber::registry::LookupSpan;

struct Formatter;

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

        writeln!(
            writer,
            "{:.6} [{}] {} - {}",
            time.as_secs_f64(),
            level,
            module_path,
            message
        )
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
    tracing_subscriber::fmt()
        .with_max_level(level)
        .event_format(Formatter)
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
        Severity::Raw(_) => "raw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_webrtc_log_severity_accepts_aliases() {
        assert_eq!(
            parse_webrtc_log_severity("verbose"),
            Some(Severity::Verbose)
        );
        assert_eq!(parse_webrtc_log_severity("DEBUG"), Some(Severity::Verbose));
        assert_eq!(parse_webrtc_log_severity("1"), Some(Severity::Info));
        assert_eq!(parse_webrtc_log_severity("warn"), Some(Severity::Warning));
        assert_eq!(parse_webrtc_log_severity("error"), Some(Severity::Error));
        assert_eq!(parse_webrtc_log_severity("off"), Some(Severity::None));
    }

    #[test]
    fn parse_webrtc_log_severity_rejects_unknown_value() {
        assert_eq!(parse_webrtc_log_severity("loud"), None);
    }
}
