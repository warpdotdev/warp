use std::sync::{Mutex, OnceLock};

use log::{Level, Log, Metadata, Record};

use crate::errors::{ReportErrorLogMode, LOG_TARGET};

#[derive(Clone, Debug, Eq, PartialEq)]
struct LogEntry {
    target: String,
    level: Level,
    message: String,
}

struct TestLogger;

static LOGGER: TestLogger = TestLogger;
static LOGS: OnceLock<Mutex<Vec<LogEntry>>> = OnceLock::new();

impl Log for TestLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        logs().lock().unwrap().push(LogEntry {
            target: record.target().to_owned(),
            level: record.level(),
            message: record.args().to_string(),
        });
    }

    fn flush(&self) {}
}

fn logs() -> &'static Mutex<Vec<LogEntry>> {
    LOGS.get_or_init(|| Mutex::new(Vec::new()))
}

fn init_logger() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Trace);
    logs().lock().unwrap().clear();
}

fn logged_report_count(message: &str) -> usize {
    logs()
        .lock()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry.target == LOG_TARGET && entry.level == Level::Error && entry.message == message
        })
        .count()
}

fn report_once_per_run_error() {
    crate::report_error!(
        anyhow::anyhow!("once per run"),
        ReportErrorLogMode::OncePerRun
    );
}

fn report_first_callsite_once_per_run_error() {
    crate::report_error!(
        anyhow::anyhow!("separate once per run"),
        ReportErrorLogMode::OncePerRun
    );
}

fn report_second_callsite_once_per_run_error() {
    crate::report_error!(
        anyhow::anyhow!("separate once per run"),
        ReportErrorLogMode::OncePerRun
    );
}

fn report_if_error_once_per_run(result: Result<(), anyhow::Error>) {
    crate::report_if_error!(result, ReportErrorLogMode::OncePerRun);
}

#[test]
fn report_error_log_mode_controls_log_frequency() {
    init_logger();

    for _ in 0..2 {
        crate::report_error!(anyhow::anyhow!("default"));
    }
    assert_eq!(logged_report_count("default"), 2);

    logs().lock().unwrap().clear();
    for _ in 0..2 {
        crate::report_error!(
            anyhow::anyhow!("explicit every time"),
            ReportErrorLogMode::EveryTime
        );
    }
    assert_eq!(logged_report_count("explicit every time"), 2);

    logs().lock().unwrap().clear();
    report_once_per_run_error();
    report_once_per_run_error();
    assert_eq!(logged_report_count("once per run"), 1);

    logs().lock().unwrap().clear();
    for _ in 0..2 {
        report_first_callsite_once_per_run_error();
        report_second_callsite_once_per_run_error();
    }
    assert_eq!(logged_report_count("separate once per run"), 2);

    logs().lock().unwrap().clear();
    for _ in 0..2 {
        report_if_error_once_per_run(Err(anyhow::anyhow!("result once per run")));
    }
    assert_eq!(logged_report_count("result once per run"), 1);
}
