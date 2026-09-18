use std::collections::HashMap;
use std::fs;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::executor::block_on;
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use signal_hook::iterator::SignalsInfo;
use signal_hook::iterator::exfiltrator::WithOrigin;
use signal_hook::low_level::siginfo::{Cause, Sent};
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

use super::InterruptSignal;

/// Set on the re-executed child so it takes the [`signal_lifecycle_child`] path instead
/// of running the test body again. Its value selects which lifecycle to exercise.
const SIGNAL_CHILD_ENV: &str = "WARP_AGENT_DRIVER_SIGNAL_CHILD";
/// Printed by the child once its interrupt watch is armed, so the parent knows when the
/// first signal can be delivered.
const READY_MARKER: &str = "warp-signal-test: ready";
/// Printed by the child once it starts (and, for `int-hang`, deliberately stalls) its
/// shutdown work, so the parent knows when the second signal is meaningful.
const SHUTDOWN_MARKER: &str = "warp-signal-test: shutdown_started";
/// How long an `int-hang` child stalls before giving up. Only reached when the parent
/// fails to deliver the second signal, in which case exiting keeps the test from
/// leaving an orphaned process behind.
const STUCK_SHUTDOWN_BAILOUT: Duration = Duration::from_secs(60);

fn signal_lifecycle_child() -> ! {
    let kind = std::env::var(SIGNAL_CHILD_ENV).expect("child kind");
    let expected = match kind.as_str() {
        "term" => InterruptSignal::Term,
        "int" | "int-hang" => InterruptSignal::Int,
        other => panic!("unknown child kind {other}"),
    };

    let background = warpui::r#async::executor::Background::default();
    let (signal_rx, _watch) =
        block_on(super::watch_interrupt_signals(&background)).expect("signal watch");
    println!("{READY_MARKER}");

    let signal: InterruptSignal = block_on(signal_rx).expect("interrupt signal");
    assert_eq!(signal, expected);

    if kind == "int-hang" {
        println!("{SHUTDOWN_MARKER}");
        std::thread::sleep(STUCK_SHUTDOWN_BAILOUT);
        std::process::exit(1);
    }
    super::emulate_default_and_exit(signal);
}

/// Re-executes this test binary as a child running only `test_name`, delivers
/// `first_sig` (and `second_sig`, once the child reports it is stuck in shutdown), and
/// asserts the child died from `expected_sig`.
///
/// Signal dispositions are process-wide, so the behavior under test is only observable
/// in a dedicated process; `test_name` must therefore be the fully-qualified name of the
/// calling test.
fn spawn_signal_lifecycle_child(
    kind: &str,
    test_name: &str,
    first_sig: Signal,
    second_sig: Option<Signal>,
    expected_sig: Signal,
    expect_stuck_shutdown: bool,
) {
    use std::os::unix::process::ExitStatusExt as _;

    if std::env::var_os(SIGNAL_CHILD_ENV).is_some() {
        signal_lifecycle_child();
    }

    let dir = TempDir::new().unwrap();
    let stdout_path = dir.path().join("stdout");
    let stderr_path = dir.path().join("stderr");
    let mut cmd = command::blocking::Command::new(std::env::current_exe().unwrap());
    cmd.arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .env(SIGNAL_CHILD_ENV, kind)
        .env("RUST_TEST_THREADS", "1")
        .stdout(fs::File::create(&stdout_path).unwrap())
        .stderr(fs::File::create(&stderr_path).unwrap());
    for (key, _) in std::env::vars() {
        if key.starts_with("NEXTEST") {
            cmd.env_remove(&key);
        }
    }
    let mut child = cmd.spawn().unwrap();

    let wait_for_marker = |child: &mut std::process::Child, marker: &str| {
        for _ in 0..1_000 {
            if fs::read_to_string(&stdout_path)
                .unwrap_or_default()
                .contains(marker)
            {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "signal child exited before printing {marker:?}: status={status:?} \
                     stdout={} stderr={}",
                    fs::read_to_string(&stdout_path).unwrap_or_default(),
                    fs::read_to_string(&stderr_path).unwrap_or_default()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "signal child never printed {marker:?}; stdout={} stderr={}",
            fs::read_to_string(&stdout_path).unwrap_or_default(),
            fs::read_to_string(&stderr_path).unwrap_or_default()
        );
    };

    wait_for_marker(&mut child, READY_MARKER);

    let child_pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
    nix::sys::signal::kill(child_pid, first_sig).unwrap();

    if let Some(second_sig) = second_sig {
        wait_for_marker(&mut child, SHUTDOWN_MARKER);
        nix::sys::signal::kill(child_pid, second_sig).unwrap();
    }

    let status = child.wait().unwrap();
    let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
    assert_eq!(
        status.signal(),
        Some(expected_sig as i32),
        "status={status:?} stdout={stdout} stderr={}",
        fs::read_to_string(&stderr_path).unwrap_or_default()
    );
    assert_eq!(
        stdout.contains(SHUTDOWN_MARKER),
        expect_stuck_shutdown,
        "stdout={stdout}"
    );
}

#[test]
fn sigterm_subprocess_exits_signaled() {
    spawn_signal_lifecycle_child(
        "term",
        "ai::agent_sdk::driver::termination::tests::sigterm_subprocess_exits_signaled",
        Signal::SIGTERM,
        None,
        Signal::SIGTERM,
        false,
    );
}

#[test]
fn sigint_subprocess_exits_signaled() {
    spawn_signal_lifecycle_child(
        "int",
        "ai::agent_sdk::driver::termination::tests::sigint_subprocess_exits_signaled",
        Signal::SIGINT,
        None,
        Signal::SIGINT,
        false,
    );
}

#[test]
fn second_sigint_kills_during_stuck_shutdown() {
    spawn_signal_lifecycle_child(
        "int-hang",
        "ai::agent_sdk::driver::termination::tests::second_sigint_kills_during_stuck_shutdown",
        Signal::SIGINT,
        Some(Signal::SIGINT),
        Signal::SIGINT,
        true,
    );
}

#[derive(Default)]
struct CapturedTrace {
    field_names: Vec<String>,
    values: HashMap<String, String>,
}

struct SignalTraceCapture(Arc<Mutex<Option<CapturedTrace>>>);

impl<S> tracing_subscriber::Layer<S> for SignalTraceCapture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut captured = CapturedTrace {
            field_names: event
                .metadata()
                .fields()
                .iter()
                .map(|field| field.name().to_owned())
                .collect(),
            ..Default::default()
        };
        event.record(&mut TraceValueVisitor(&mut captured.values));
        *self.0.lock().unwrap() = Some(captured);
    }
}

struct TraceValueVisitor<'a>(&'a mut HashMap<String, String>);

impl Visit for TraceValueVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

#[test]
fn emits_signal_trace_schema() {
    let captured = Arc::new(Mutex::new(None));
    let subscriber = tracing_subscriber::registry().with(SignalTraceCapture(Arc::clone(&captured)));

    tracing::subscriber::with_default(subscriber, || {
        super::emit_signal_trace(InterruptSignal::Int, None, Cause::Sent(Sent::User));
    });

    let captured = captured.lock().unwrap().take().unwrap();
    assert_eq!(
        captured.field_names,
        [
            "message",
            "tags.cloud_agent",
            "signal",
            "signal.cause",
            "signal.sender.pid",
            "signal.sender.uid",
            "signal.sender.username",
            "signal.sender.command_line",
        ]
    );
    assert_eq!(captured.values.get("signal").unwrap(), "SIGINT");
    assert_eq!(captured.values.get("signal.cause").unwrap(), "Sent(User)");
}
#[test]
fn redacts_secrets_from_signal_sender_command_line() {
    const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

    let mut signals = SignalsInfo::<WithOrigin>::new([libc::SIGWINCH]).unwrap();
    let mut command = command::blocking::Command::new("sh");
    command
        .arg("-c")
        .arg("kill -WINCH \"$1\"; while :; do sleep 1; done")
        .arg(SECRET)
        .arg(nix::unistd::getpid().as_raw().to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().unwrap();
    let sender = signals.forever().next().unwrap().process.unwrap();
    let captured = Arc::new(Mutex::new(None));
    let subscriber = tracing_subscriber::registry().with(SignalTraceCapture(Arc::clone(&captured)));

    tracing::subscriber::with_default(subscriber, || {
        super::emit_signal_trace(InterruptSignal::Term, Some(sender), Cause::Sent(Sent::User));
    });
    child.kill().unwrap();
    child.wait().unwrap();

    let captured = captured.lock().unwrap().take().unwrap();
    let command_line = captured.values.get("signal.sender.command_line").unwrap();
    assert!(!command_line.contains(SECRET));
    assert!(command_line.contains(&"*".repeat(SECRET.len())));
}

#[test]
fn formats_process_arguments_as_a_shell_command_line() {
    let arguments = ["agent", "--prompt", "hello world"].map(Into::into);

    assert_eq!(
        super::format_command_line(&arguments),
        Some("agent --prompt 'hello world'".to_owned())
    );
}

#[test]
fn omits_command_line_when_process_arguments_are_empty() {
    assert_eq!(super::format_command_line(&[]), None);
}

#[test]
fn omits_command_line_for_invalid_sender_pid() {
    assert_eq!(super::resolve_command_line(0), None);
}
