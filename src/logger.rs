use std::time::UNIX_EPOCH;

static LOGGER: Logger = Logger;

#[derive(Debug)]
pub struct Logger;

impl Logger {
    pub fn init(level: log::LevelFilter) -> Result<(), log::SetLoggerError> {
        log::set_logger(&LOGGER).map(|()| log::set_max_level(level))
    }
}

impl log::Log for Logger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        // init() の中で最大ログレベルを指定しているので、ここでは何もしない
        true
    }

    fn log(&self, record: &log::Record) {
        let time = UNIX_EPOCH.elapsed().ok().unwrap_or_default();
        eprintln!(
            "{:.6} [{}] {} - {}",
            time.as_secs_f64(),
            record.level(),
            record.module_path().unwrap_or_default(),
            record.args()
        );
    }

    fn flush(&self) {}
}
