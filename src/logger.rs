use std::time::UNIX_EPOCH;

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
}
