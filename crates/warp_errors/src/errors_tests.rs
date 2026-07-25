use std::sync::{Mutex, OnceLock};

use log::{Level, Log, Metadata, Record};

use crate::{LOG_TARGET, ReportErrorLogMode};

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
    report_error!(
        anyhow::anyhow!("once per run"),
        ReportErrorLogMode::OncePerRun
    );
}

fn report_first_callsite_once_per_run_error() {
    report_error!(
        anyhow::anyhow!("separate once per run"),
        ReportErrorLogMode::OncePerRun
    );
}

fn report_second_callsite_once_per_run_error() {
    report_error!(
        anyhow::anyhow!("separate once per run"),
        ReportErrorLogMode::OncePerRun
    );
}

fn report_if_error_once_per_run(result: Result<(), anyhow::Error>) {
    report_if_error!(result, ReportErrorLogMode::OncePerRun);
}

#[test]
fn report_error_log_mode_controls_log_frequency() {
    init_logger();

    for _ in 0..2 {
        report_error!(anyhow::anyhow!("default"));
    }
    assert_eq!(logged_report_count("default"), 2);

    logs().lock().unwrap().clear();
    for _ in 0..2 {
        report_error!(
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

#[test]
fn new_macro_forms_log_as_expected() {
    init_logger();

    // Bare string-literal form wraps the message in an anyhow error and reports it.
    report_error!("a static message");
    assert_eq!(logged_report_count("a static message"), 1);

    // `extra: { .. }` appends fields to the log line (Display default, `?` Debug).
    logs().lock().unwrap().clear();
    let items = vec![1, 2, 3];
    report_error!(
        anyhow::anyhow!("boom"),
        extra: { "count" => 3, "items" => ?items }
    );
    assert_eq!(logged_report_count("boom [count=3, items=[1, 2, 3]]"), 1);

    // Literal message plus extra.
    logs().lock().unwrap().clear();
    report_error!("load failed", extra: { "id" => 7 });
    assert_eq!(logged_report_count("load failed [id=7]"), 1);

    // report_if_error! with extra only reports on Err.
    logs().lock().unwrap().clear();
    let ok: Result<(), anyhow::Error> = Ok(());
    report_if_error!(ok, extra: { "k" => 1 });
    assert_eq!(logs().lock().unwrap().len(), 0);
    let err: Result<(), anyhow::Error> = Err(anyhow::anyhow!("nope"));
    report_if_error!(err, extra: { "k" => 1 });
    assert_eq!(logged_report_count("nope [k=1]"), 1);
}

#[cfg(feature = "reqwest-errors")]
#[test]
fn preserved_reqwest_error_chain_delegates_actionability() {
    use crate::AnyhowErrorExt as _;

    // A connection failure is registered as non-actionable. Wrapping the typed error with
    // anyhow::Error::new preserves the source chain so the classification is retained.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let non_actionable = reqwest::blocking::Client::new()
        .get(format!("http://{address}"))
        .send()
        .expect_err("the closed local port should reject the connection");
    assert!(non_actionable.is_connect(), "{non_actionable:?}");
    assert!(!anyhow::Error::new(non_actionable).is_actionable());

    // An unfiltered client error is actionable and should remain so through the same
    // typed-anyhow boundary.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};

        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let request_bytes = stream.read(&mut request).unwrap();
        assert!(request_bytes > 0);
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });
    let response = reqwest::blocking::get(format!("http://{address}")).unwrap();
    let actionable = response
        .error_for_status()
        .expect_err("a 400 response should produce a reqwest error");
    server.join().unwrap();
    assert_eq!(actionable.status(), Some(http::StatusCode::BAD_REQUEST));
    assert!(anyhow::Error::new(actionable).is_actionable());
}
