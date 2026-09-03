use std::io;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use futures_lite::future::block_on;
use warp_terminal::model::ansi::Processor;
use warpui::r#async::executor::Background;

use super::{STDOUT_LIMIT, drain_dev_container_pipes};
use crate::terminal::SizeInfo;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::terminal_model::TerminalModel;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::view::dev_container::newline::NewlineNormalizer;

#[test]
fn devcontainer_up_drains_stdout_and_stderr_concurrently() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, threading
def write(fd, payload):
    os.write(fd, payload)
blob = b"x" * (256 * 1024)
threads = [
    threading.Thread(target=write, args=(1, blob)),
    threading.Thread(target=write, args=(2, b"marker-before-exit\n" + blob)),
]
for thread in threads:
    thread.start()
for thread in threads:
    thread.join()
os.write(1, b'\n{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn fake child");

        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let seen_stderr = Arc::new(Mutex::new(Vec::new()));
        let seen_for_callback = seen_stderr.clone();
        let result = drain_dev_container_pipes(stdout, stderr, move |chunk| {
            seen_for_callback.lock().unwrap().extend_from_slice(chunk);
        })
        .await
        .expect("drain");
        let status = child.status().await.expect("wait");
        assert!(status.success());
        let seen = seen_stderr.lock().unwrap().clone();
        assert!(
            seen.windows(b"marker-before-exit".len())
                .any(|window| window == b"marker-before-exit")
        );
        let seen_text = String::from_utf8_lossy(&seen);
        assert!(
            !seen_text.contains(r#""outcome":"success""#),
            "final status JSON must not be displayed, got {seen_text:?}"
        );
        let stdout_text = String::from_utf8_lossy(&result.stdout.bytes);
        assert!(stdout_text.contains(r#""outcome":"success""#));
        assert!(!result.stdout.oversized);
        assert!(!result.stderr_tail.is_empty());
    });
}

#[cfg(unix)]
#[test]
fn pty_stdio_is_tty_and_redraw_reaches_sink_before_exit() {
    use std::time::Duration;

    use futures::future::{self, Either};
    use instant::Instant;

    block_on(async {
        let release_dir = tempfile::tempdir().expect("release dir");
        let release_path = release_dir.path().join("go");
        let mut command = command::r#async::Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time
path = os.environ["DC_RELEASE_PATH"]
os.write(2, f"stdout_tty={int(os.isatty(1))} stderr_tty={int(os.isatty(2))}\n".encode())
os.write(2, b"first\rsecond\n")
os.write(1, b"layer 1MB\rlayer 2MB\n")
os.write(1, b"cursor\x1b[1A\rupdated\n")
while not os.path.exists(path):
    time.sleep(0.01)
os.write(1, b'{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
"#,
            )
            .env("DC_RELEASE_PATH", &release_path)
            .kill_on_drop(true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = seen.clone();
        let drain_fut = super::drain_dev_container_child(command, None, move |chunk| {
            seen_cb.lock().unwrap().extend_from_slice(chunk);
        });
        let wait_fut = async {
            let started = Instant::now();
            loop {
                let seen_bytes = seen.lock().unwrap().clone();
                let output = String::from_utf8_lossy(&seen_bytes);
                if output.contains("stdout_tty=1")
                    && output.contains("stderr_tty=1")
                    && output.contains("second")
                    && output.contains("layer 2MB")
                    && output.contains("updated")
                {
                    std::fs::write(&release_path, b"go").expect("release child");
                    return;
                }
                assert!(
                    started.elapsed().as_secs() < 5,
                    "PTY redraw must stream before process EOF, got {output:?}"
                );
                warpui::r#async::Timer::after(Duration::from_millis(10)).await;
            }
        };
        let work = async { futures::join!(drain_fut, wait_fut) };
        let timeout = async {
            warpui::r#async::Timer::after(Duration::from_secs(5)).await;
        };
        match future::select(Box::pin(work), Box::pin(timeout)).await {
            Either::Right(_) => panic!("timed out waiting for pre-EOF PTY stream"),
            Either::Left(((drain_result, _), _)) => {
                let (drain, success) = drain_result.expect("drain");
                assert!(success);
                let displayed = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
                assert!(
                    displayed.contains("layer 2MB"),
                    "stdout CR progress must be displayed, got {displayed:?}"
                );
                assert!(
                    displayed.contains("updated"),
                    "stdout cursor progress must be displayed, got {displayed:?}"
                );
                assert!(
                    !displayed.contains(r#""outcome":"success""#),
                    "final status JSON must not be displayed, got {displayed:?}"
                );
                let stdout = String::from_utf8_lossy(&drain.stdout.bytes);
                assert!(stdout.contains(r#""outcome":"success""#), "got {stdout:?}");
            }
        }
    });
}

#[test]
fn successful_drain_terminates_process_group_once() {
    let _ = super::take_process_group_terminations();
    block_on(async {
        let mut command = command::r#async::Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg("pass")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain");
        assert!(success);
        assert!(!drain.stdout.oversized);
    });
    assert_eq!(super::take_process_group_terminations(), 1);
}

fn sleep_process_group_command() -> command::r#async::Command {
    let mut command = command::r#async::Command::new_with_process_group("python3");
    command
        .arg("-c")
        .arg("import time; time.sleep(30)")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

#[test]
fn cancel_during_build_terminates_process_group_once() {
    use instant::Instant;

    use crate::terminal::view::dev_container::operation::DevContainerBuildCancel;

    let _ = super::take_process_group_terminations();
    let cancel = DevContainerBuildCancel::new();
    let started = Instant::now();
    block_on(async {
        let drain_fut =
            super::drain_dev_container_child(sleep_process_group_command(), Some(&cancel), |_| {});
        let kill_fut = async {
            loop {
                if cancel.has_armed_kill() {
                    break;
                }
                futures_lite::future::yield_now().await;
            }
            cancel.mark_cancelled();
        };
        let (result, _) = futures::join!(drain_fut, kill_fut);
        assert!(
            started.elapsed().as_secs() < 5,
            "build cancel must return promptly: {:?}",
            started.elapsed()
        );
        assert!(result.is_err() || result.is_ok_and(|(_, success)| !success));
    });
    assert_eq!(super::take_process_group_terminations(), 1);
}

#[test]
fn cancel_during_preflight_terminates_process_group_once() {
    use instant::Instant;

    use crate::terminal::view::dev_container::operation::DevContainerBuildCancel;

    let _ = super::take_process_group_terminations();
    let cancel = DevContainerBuildCancel::new();
    let started = Instant::now();
    block_on(async {
        let run_fut =
            super::run_cancellable_process_group_command(sleep_process_group_command(), &cancel);
        let kill_fut = async {
            loop {
                if cancel.has_armed_kill() {
                    break;
                }
                futures_lite::future::yield_now().await;
            }
            cancel.mark_cancelled();
        };
        let (result, _) = futures::join!(run_fut, kill_fut);
        assert!(
            started.elapsed().as_secs() < 5,
            "preflight cancel must return promptly: {:?}",
            started.elapsed()
        );
        assert!(result.is_err() || result.is_ok_and(|output| !output.status.success()));
    });
    assert_eq!(super::take_process_group_terminations(), 1);
}

#[test]
fn rejected_build_registration_terminates_before_wait() {
    use instant::Instant;

    use crate::terminal::view::dev_container::operation::DevContainerBuildCancel;

    let _ = super::take_process_group_terminations();
    let cancel = DevContainerBuildCancel::new();
    cancel.mark_cancelled();
    let started = Instant::now();
    let result = block_on(super::drain_dev_container_child(
        sleep_process_group_command(),
        Some(&cancel),
        |_| {},
    ));
    assert!(
        started.elapsed().as_secs() < 5,
        "rejected registration must not wait on the child: {:?}",
        started.elapsed()
    );
    assert!(result.is_err());
    assert_eq!(super::take_process_group_terminations(), 1);
}

#[test]
fn drain_marks_stdout_oversized_when_pending_json_prefix_exceeds_one_mib() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(format!(
                "import os; os.write(1, b'{{' * {}); os.write(2, b'')",
                STDOUT_LIMIT + 1
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn oversized child");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let result = drain_dev_container_pipes(stdout, stderr, |_| {})
            .await
            .expect("drain");
        let _ = child.status().await;
        assert!(result.stdout.oversized);
        assert!(result.stdout.bytes.is_empty());
        assert!(result.stdout.bytes.len() <= STDOUT_LIMIT);
    });
}

#[test]
fn drain_keeps_final_json_after_more_than_one_mib_of_complete_records() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(format!(
                r##"
import os
os.write(1, (b"n\n" * {}))
os.write(1, b'{{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}}\n')
os.write(2, b"")
"##,
                (STDOUT_LIMIT / 2) + 32
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn noisy stdout child");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let result = drain_dev_container_pipes(stdout, stderr, |_| {})
            .await
            .expect("drain");
        let _ = child.status().await;
        assert!(!result.stdout.oversized);
        assert!(result.stdout.bytes.len() <= STDOUT_LIMIT);
        let stdout = String::from_utf8_lossy(&result.stdout.bytes);
        assert!(stdout.contains(r#""outcome":"success""#), "got {stdout:?}");
    });
}

#[cfg(unix)]
fn pid_is_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

#[cfg(unix)]
fn wait_for_pid_file(path: &std::path::Path) -> i32 {
    use instant::Instant;

    let started = Instant::now();
    loop {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<i32>()
            && pid > 1
        {
            return pid;
        }
        assert!(
            started.elapsed().as_secs() < 5,
            "descendant pid file was not written"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn reader_error_kills_process_group_descendants() {
    use command::r#async::Command;

    let pid_file = std::env::temp_dir().join(format!("dc-desc-{}", uuid::Uuid::new_v4()));
    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(format!(
                r#"
import os, time
pid = os.fork()
if pid == 0:
    open({pid_file:?}, "w").write(str(os.getpid()))
    time.sleep(30)
    os._exit(0)
time.sleep(30)
"#
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn");
        let process_group_id = child.id();
        let descendant = wait_for_pid_file(&pid_file);
        assert!(pid_is_alive(descendant), "descendant must start alive");
        let result = super::join_drain_and_status(
            super::ProcessGroupKillOnDrop::new(process_group_id),
            async { Err(io::Error::other("reader failed")) },
            async { child.status().await },
        )
        .await;
        assert!(result.is_err(), "reader failure must surface");
        let started = instant::Instant::now();
        while pid_is_alive(descendant) {
            assert!(
                started.elapsed().as_secs() < 5,
                "descendant must not survive a reader error"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    let _ = std::fs::remove_file(pid_file);
}

#[cfg(unix)]
#[test]
fn drain_reaches_failed_without_waiting_for_descendant_holding_pipes() {
    use command::r#async::Command;
    use instant::Instant;

    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time
pid = os.fork()
if pid == 0:
    time.sleep(30)
    os._exit(0)
os.write(2, b"marker-before-exit\n")
os._exit(1)
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let started = Instant::now();
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain after parent exit");
        assert!(
            started.elapsed().as_secs() < 5,
            "descendant holding pipes must not pin drain: {:?}",
            started.elapsed()
        );
        assert!(!success);
        assert!(
            drain
                .stderr_tail
                .windows(b"marker-before-exit".len())
                .any(|window| window == b"marker-before-exit")
        );
    });
}

#[cfg(unix)]
#[test]
fn drain_reaches_success_without_waiting_for_descendant_holding_pipes() {
    use command::r#async::Command;
    use instant::Instant;

    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time
pid = os.fork()
if pid == 0:
    time.sleep(30)
    os._exit(0)
os.write(2, b"Generated translation files for all integrations\n")
os.write(1, b'{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
os._exit(0)
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let started = Instant::now();
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain after successful parent exit");
        assert!(
            started.elapsed().as_secs() < 5,
            "descendant holding pipes must not pin drain after success: {:?}",
            started.elapsed()
        );
        assert!(success);
        assert!(
            drain
                .stderr_tail
                .windows(b"Generated translation files for all integrations".len())
                .any(|window| window == b"Generated translation files for all integrations")
        );
    });
}

#[cfg(unix)]
#[test]
fn drain_returns_and_kills_same_group_holder_of_both_pty_slaves() {
    use command::r#async::Command;
    use instant::Instant;

    let holder_file = std::env::temp_dir().join(format!("dc-pty-holder-{}", uuid::Uuid::new_v4()));
    let container_file =
        std::env::temp_dir().join(format!("dc-pty-container-{}", uuid::Uuid::new_v4()));
    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(format!(
                r#"
import os, time
holder = os.fork()
if holder == 0:
    open({holder_file:?}, "w").write(str(os.getpid()))
    time.sleep(30)
    os._exit(0)
container = os.fork()
if container == 0:
    os.setsid()
    os.close(1)
    os.close(2)
    open({container_file:?}, "w").write(str(os.getpid()))
    time.sleep(30)
    os._exit(0)
while not (os.path.exists({holder_file:?}) and os.path.exists({container_file:?})):
    time.sleep(0.01)
os.write(2, b"Container started\n")
os.write(1, b'{{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}}\n')
os._exit(0)
"#
            ))
            .kill_on_drop(true);
        let started = Instant::now();
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain after CLI exit with same-group dual-slave holder");
        assert!(
            started.elapsed().as_secs() < 5,
            "same-group PTY slave holder must not pin drain: {:?}",
            started.elapsed()
        );
        assert!(success);
        assert!(
            super::super::interpret_up_output_for_test(true, &drain.stdout.bytes, b""),
            "final outcome JSON must still parse for attach, got {:?}",
            String::from_utf8_lossy(&drain.stdout.bytes)
        );
        let holder = wait_for_pid_file(&holder_file);
        let container = wait_for_pid_file(&container_file);
        let holder_gone = Instant::now();
        while pid_is_alive(holder) {
            assert!(
                holder_gone.elapsed().as_secs() < 5,
                "same-group slave holder must be terminated"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            pid_is_alive(container),
            "detached container-like process must survive killing the PTY client"
        );
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(container),
            nix::sys::signal::Signal::SIGKILL,
        );
    });
    let _ = std::fs::remove_file(holder_file);
    let _ = std::fs::remove_file(container_file);
}

#[cfg(unix)]
#[test]
fn drain_returns_when_out_of_group_descendant_holds_pty_slave() {
    use command::r#async::Command;
    use instant::Instant;

    let pid_file = std::env::temp_dir().join(format!("dc-pty-oog-{}", uuid::Uuid::new_v4()));
    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(format!(
                r#"
import os, time
r, w = os.pipe()
pid = os.fork()
if pid == 0:
    os.close(r)
    os.setsid()
    open({pid_file:?}, "w").write(str(os.getpid()))
    os.write(w, b"x")
    os.close(w)
    time.sleep(30)
    os._exit(0)
os.close(w)
os.read(r, 1)
os.close(r)
os.write(2, b"Container started\n")
os.write(1, b'{{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}}\n')
os._exit(0)
"#
            ))
            .kill_on_drop(true);
        let started = Instant::now();
        let (drain, success) = super::drain_dev_container_child(command, None, |_| {})
            .await
            .expect("drain after CLI exit with out-of-group slave holder");
        assert!(
            started.elapsed().as_secs() < 5,
            "out-of-group PTY slave holder must not pin drain: {:?}",
            started.elapsed()
        );
        assert!(success);
        assert!(
            super::super::interpret_up_output_for_test(true, &drain.stdout.bytes, b""),
            "final outcome JSON must still parse for attach, got {:?}",
            String::from_utf8_lossy(&drain.stdout.bytes)
        );
        let descendant = wait_for_pid_file(&pid_file);
        assert!(
            pid_is_alive(descendant),
            "out-of-group descendant must survive CLI exit"
        );
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(descendant),
            nix::sys::signal::Signal::SIGKILL,
        );
    });
    let _ = std::fs::remove_file(pid_file);
}

#[test]
fn drain_stays_pending_while_child_is_silent_but_alive() {
    use command::r#async::Command;
    use futures::future::{self, Either};

    block_on(async {
        let mut command = Command::new_with_process_group("python3");
        command
            .arg("-c")
            .arg(
                r#"
import os, time, sys
sys.stderr.write("Generated translation files for all integrations\n")
sys.stderr.flush()
time.sleep(30)
"#,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let drain = super::drain_dev_container_child(command, None, |_| {});
        let timeout = async {
            warpui::r#async::Timer::after(std::time::Duration::from_millis(400)).await;
        };
        match future::select(Box::pin(drain), Box::pin(timeout)).await {
            Either::Left(_) => {
                panic!("drain completed while the child was still alive")
            }
            Either::Right(_) => {}
        }
    });
}

#[test]
fn devcontainer_text_stream_renders_incrementally() {
    let mut model = TerminalModel::mock(None, None);
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let mut normalizer = NewlineNormalizer::new();
    let mut replies = Vec::new();
    let mut writer = WriteCapture(&mut replies);

    let delayed = b"step-one\n\x1b[31mred";
    let rest = b"-text\x1b[0m\nstep-two\n";
    for chunk in delayed.chunks(3) {
        let normalized = normalizer.push(chunk);
        processor.parse_bytes(&mut model, &normalized, &mut writer);
    }
    let output_so_far = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    assert!(
        output_so_far.contains("step-one"),
        "delayed marker missing before remaining chunks: {output_so_far:?}"
    );

    let normalized_rest = normalizer.push(rest);
    processor.parse_bytes(&mut model, &normalized_rest, &mut writer);
    processor.parse_bytes(&mut model, b"\x1b[6nmore\n", &mut io::sink());
    let output = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    assert!(output.contains("step-one"));
    assert!(output.contains("red-text") || output.contains("red") && output.contains("text"));
    assert!(output.contains("step-two"));
    assert!(output.contains("more"));
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            !line.starts_with(' '),
            "bare LF should left-align after normalization, got {line:?}"
        );
    }
}

#[test]
fn failure_details_with_bare_lf_render_left_aligned() {
    let mut model = TerminalModel::mock(None, None);
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let mut normalizer = NewlineNormalizer::new();
    let message = "Dev container failed to start:\nCommand failed: docker ps -q --filter \
         label=devcontainer.local_folder=/tmp/ws\nCannot connect to the Docker daemon";
    let bytes = normalizer.push(format!("\n{message}\n").as_bytes());
    processor.parse_bytes(&mut model, &bytes, &mut io::sink());
    let output = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    for needle in ["Command failed", "Cannot connect"] {
        let line = output
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from {output:?}"));
        assert_eq!(
            line.trim_start(),
            line,
            "failure details must start at column 0, got {line:?}"
        );
    }
}

fn wide_terminal_model() -> TerminalModel {
    let mut sizes = block_size();
    sizes.size = SizeInfo::new_without_font_metrics(24, 160);
    TerminalModel::new_for_test(
        sizes,
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        None,
    )
}

fn grid_output_from_stderr_chunks(chunks: &[&[u8]]) -> String {
    let mut model = wide_terminal_model();
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let bytes = super::transform_dev_container_stderr(chunks);
    processor.parse_bytes(&mut model, &bytes, &mut io::sink());
    model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None)
}

#[test]
fn raw_cr_progress_overwrites_in_the_grid() {
    let output = grid_output_from_stderr_chunks(&[
        b"[cli] @devcontainers/cli 0.89.0\n",
        b"#15 extracting sha256:abc 1.5MB / 52.40MB\r",
        b"#15 extracting sha256:abc 52.40MB / 52.40MB\n",
        b"#15 DONE 2.1s\n",
    ]);
    assert!(
        output.contains("@devcontainers/cli 0.89.0"),
        "ordinary logs must remain, got {output:?}"
    );
    assert!(
        output.contains("#15 DONE 2.1s"),
        "completed vertex lines must remain, got {output:?}"
    );
    let extracting_lines = output
        .lines()
        .filter(|line| line.contains("extracting sha256:abc"))
        .count();
    assert_eq!(
        extracting_lines, 1,
        "CR snapshots must overwrite in place, got {output:?}"
    );
    assert!(
        !output.contains("1.5MB"),
        "overwritten snapshots must not linger, got {output:?}"
    );
    assert!(output.contains("52.40MB / 52.40MB"));
}

#[test]
fn raw_cursor_up_progress_overwrites_in_the_grid() {
    let output =
        grid_output_from_stderr_chunks(&[b"layer-a 1MB\r\nlayer-b 1MB", b"\x1b[1A\rlayer-a 2MB"]);
    assert!(
        output.contains("layer-a 2MB"),
        "cursor-up must apply the later snapshot, got {output:?}"
    );
    assert!(
        !output.contains("layer-a 1MB"),
        "superseded row must not linger, got {output:?}"
    );
    assert!(output.contains("layer-b 1MB"));
}

#[test]
fn raw_lf_lines_left_align_and_cr_progress_overwrites_across_chunks() {
    let joined = "[+] Building 1.0s (1/3)\n\
#1 [internal] load build definition from Dockerfile\n\
#15 extracting sha256:abc 1.5MB / 52.40MB\r\
#15 extracting sha256:abc 52.40MB / 52.40MB\n\
#15 DONE 2.1s\n";
    let split_at = joined.len() / 2;
    let output = grid_output_from_stderr_chunks(&[
        &joined.as_bytes()[..split_at],
        &joined.as_bytes()[split_at..],
    ]);
    for needle in ["[+] Building", "load build definition", "#15 DONE 2.1s"] {
        let line = output
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from {output:?}"));
        assert_eq!(
            line.trim_start(),
            line,
            "raw LF lines must start at column 0, got {line:?}"
        );
    }
    let extracting_lines = output
        .lines()
        .filter(|line| line.contains("extracting sha256:abc"))
        .count();
    assert_eq!(
        extracting_lines, 1,
        "CR snapshots must overwrite in place, got {output:?}"
    );
    assert!(
        !output.contains("1.5MB"),
        "overwritten snapshots must not linger, got {output:?}"
    );
    assert!(output.contains("52.40MB / 52.40MB"));
}

#[test]
fn drain_preserves_raw_cr_through_the_stream_boundary() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(
                r##"
import os
os.write(2, b"[cli] @devcontainers/cli 0.89.0\n")
os.write(2, b"#15 extracting sha256:abc 1.5MB / 52.40MB\r")
os.write(2, b"#15 extracting sha256:abc 52.40MB / 52.40MB\n")
os.write(2, b"#15 DONE 2.1s\n")
os.write(1, b'{"outcome":"success","containerId":"abc","remoteWorkspaceFolder":"/w"}\n')
"##,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn jsonl child");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let seen_stderr = Arc::new(Mutex::new(Vec::new()));
        let seen_for_callback = seen_stderr.clone();
        let result = drain_dev_container_pipes(stdout, stderr, move |chunk| {
            seen_for_callback.lock().unwrap().extend_from_slice(chunk);
        })
        .await
        .expect("drain");
        let status = child.status().await.expect("wait");
        assert!(status.success());
        assert!(String::from_utf8_lossy(&result.stdout.bytes).contains(r#""outcome":"success""#));
        let decoded = seen_stderr.lock().unwrap().clone();
        let displayed = String::from_utf8_lossy(&decoded);
        assert!(
            !displayed.contains(r#""outcome":"success""#),
            "final status JSON must not be displayed, got {displayed:?}"
        );
        let mut model = wide_terminal_model();
        model.start_commandless_output_block();
        let mut processor = Processor::new();
        processor.parse_bytes(&mut model, &decoded, &mut io::sink());
        let output = model
            .block_list()
            .active_block()
            .output_grid()
            .contents_to_string(false, None);
        let extracting_lines = output
            .lines()
            .filter(|line| line.contains("extracting sha256:abc"))
            .count();
        assert_eq!(
            extracting_lines, 1,
            "stream-boundary CR must overwrite in the grid, got {output:?}"
        );
        assert!(!output.contains("1.5MB"), "got {output:?}");
        assert!(output.contains("52.40MB / 52.40MB"));
        assert!(output.contains("#15 DONE 2.1s"));
    });
}

#[test]
fn stderr_tail_overflow_keeps_decoded_diagnostics_not_json_envelopes() {
    block_on(async {
        let mut command = command::r#async::Command::new("python3");
        command
            .arg("-c")
            .arg(format!(
                "import os; nl=bytes([10]); os.write(2, b'x'*{}+nl+b'Cannot connect to the Docker daemon'+nl); os.write(1, nl)",
                super::STDERR_TAIL_LIMIT + 512
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn overflow child");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let result = drain_dev_container_pipes(stdout, stderr, |_| {})
            .await
            .expect("drain");
        let _ = child.status().await;
        let tail = String::from_utf8_lossy(&result.stderr_tail);
        assert!(
            tail.contains("Cannot connect to the Docker daemon"),
            "decoded diagnostic must survive tail overflow, got {tail:?}"
        );
        assert!(
            !tail.contains("\"type\":"),
            "JSON envelopes must not appear in the failure tail, got {tail:?}"
        );
    });
}

#[test]
fn stdout_cr_progress_reaches_sink_before_outcome_json() {
    let progress = b"layer 1MB\r".to_vec();
    let progress2 = b"layer 2MB\n".to_vec();
    let json =
        b"{\"outcome\":\"success\",\"containerId\":\"abc\",\"remoteWorkspaceFolder\":\"/w\"}\n"
            .to_vec();
    let released = Arc::new(Mutex::new(1usize));
    let stdout = GatedReader {
        chunks: vec![progress, progress2, json],
        next: 0,
        released: released.clone(),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();
    let released_cb = released.clone();
    let saw_progress = Arc::new(Mutex::new(false));
    let saw_progress_cb = saw_progress.clone();

    let result = block_on(async {
        drain_dev_container_pipes(stdout, EofReader, move |chunk| {
            let text = {
                let mut seen = seen_cb.lock().unwrap();
                seen.extend_from_slice(chunk);
                String::from_utf8_lossy(&seen).into_owned()
            };
            let mut saw_progress = saw_progress_cb.lock().unwrap();
            if !*saw_progress && text.contains("layer 1MB") {
                assert!(
                    !text.contains("\"outcome\""),
                    "status JSON must not appear before later chunks, got {text:?}"
                );
                *saw_progress = true;
                *released_cb.lock().unwrap() = usize::MAX;
            }
        })
        .await
        .expect("drain")
    });

    assert!(
        *saw_progress.lock().unwrap(),
        "stdout progress must stream before EOF"
    );
    let displayed = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
    assert!(displayed.contains("layer 2MB"), "got {displayed:?}");
    assert!(
        !displayed.contains(r#""outcome":"success""#),
        "final status JSON must not be displayed, got {displayed:?}"
    );
    let stdout = String::from_utf8_lossy(&result.stdout.bytes);
    assert!(stdout.contains(r#""outcome":"success""#), "got {stdout:?}");
}

#[test]
fn unterminated_outcome_json_at_eof_is_kept_for_attach_and_hidden() {
    let part1 = b"{\"outcome\":\"success\",\"containerId\":\"abc\",".to_vec();
    let part2 = b"\"remoteWorkspaceFolder\":\"/w\"}".to_vec();
    let released = Arc::new(Mutex::new(usize::MAX));
    let stdout = GatedReader {
        chunks: vec![part1, part2],
        next: 0,
        released,
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();

    let result = block_on(async {
        drain_dev_container_pipes(stdout, EofReader, move |chunk| {
            seen_cb.lock().unwrap().extend_from_slice(chunk);
        })
        .await
        .expect("drain")
    });

    let displayed = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
    assert!(
        !displayed.contains(r#""outcome":"success""#),
        "unterminated final status JSON must not reach the sink, got {displayed:?}"
    );
    let stdout = String::from_utf8_lossy(&result.stdout.bytes);
    let parsed: serde_json::Value = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok())
        .unwrap_or_else(|| panic!("expected attach JSON in stdout, got {stdout:?}"));
    assert_eq!(parsed["outcome"], "success");
    assert_eq!(parsed["containerId"], "abc");
    assert_eq!(parsed["remoteWorkspaceFolder"], "/w");
}

#[test]
fn held_outcome_is_released_when_unterminated_stdout_follows_before_eof() {
    let outcome =
        b"preamble\n{\"outcome\":\"success\",\"containerId\":\"abc\",\"remoteWorkspaceFolder\":\"/w\"}\n"
            .to_vec();
    let tail = b"tail-without-nl".to_vec();
    let released = Arc::new(Mutex::new(1usize));
    let stdout = GatedReader {
        chunks: vec![outcome, tail],
        next: 0,
        released: released.clone(),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();
    let released_cb = released.clone();
    let saw_held_before_eof = Arc::new(Mutex::new(false));
    let saw_held_cb = saw_held_before_eof.clone();

    let result = block_on(async {
        drain_dev_container_pipes(stdout, EofReader, move |chunk| {
            let text = {
                let mut seen = seen_cb.lock().unwrap();
                seen.extend_from_slice(chunk);
                String::from_utf8_lossy(&seen).into_owned()
            };
            if text.contains("preamble") && !text.contains("tail-without-nl") {
                *released_cb.lock().unwrap() = usize::MAX;
            }
            if text.contains(r#""outcome":"success""#) && text.contains("tail-without-nl") {
                *saw_held_cb.lock().unwrap() = true;
            }
            if text.contains("tail-without-nl") {
                assert!(
                    text.contains(r#""outcome":"success""#),
                    "held outcome must be visible before EOF, got {text:?}"
                );
            }
        })
        .await
        .expect("drain")
    });

    assert!(
        *saw_held_before_eof.lock().unwrap(),
        "held outcome must be emitted when later unterminated stdout arrives"
    );
    let stdout = String::from_utf8_lossy(&result.stdout.bytes);
    assert!(stdout.contains(r#""outcome":"success""#), "got {stdout:?}");
}

#[test]
fn same_read_outcome_then_unterminated_tail_reaches_sink_before_eof() {
    let payload = b"{\"outcome\":\"success\",\"containerId\":\"abc\",\"remoteWorkspaceFolder\":\"/w\"}\ntail-without-nl"
        .to_vec();
    let released = Arc::new(Mutex::new(1usize));
    let stdout = GatedReader {
        chunks: vec![payload, b"after-eof-guard".to_vec()],
        next: 0,
        released: released.clone(),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();
    let released_cb = released.clone();
    let saw_before_eof = Arc::new(Mutex::new(false));
    let saw_cb = saw_before_eof.clone();

    let result = block_on(async {
        drain_dev_container_pipes(stdout, EofReader, move |chunk| {
            let text = {
                let mut seen = seen_cb.lock().unwrap();
                seen.extend_from_slice(chunk);
                String::from_utf8_lossy(&seen).into_owned()
            };
            if text.contains("tail-without-nl") {
                assert!(
                    text.contains(r#""outcome":"success""#),
                    "same-read held outcome must reach the sink before EOF, got {text:?}"
                );
                *saw_cb.lock().unwrap() = true;
                *released_cb.lock().unwrap() = usize::MAX;
            }
        })
        .await
        .expect("drain")
    });

    assert!(
        *saw_before_eof.lock().unwrap(),
        "same-read tail must be observed before EOF"
    );
    assert!(
        !super::super::interpret_up_output_for_test(true, &result.stdout.bytes, b""),
        "ordinary tail after outcome JSON must not attach, got {:?}",
        String::from_utf8_lossy(&result.stdout.bytes)
    );
}

#[test]
fn drain_renders_first_marker_before_split_json_and_ansi_are_released() {
    let step = b"step-one\n".to_vec();
    let color = b"\x1b[31mred".to_vec();
    let rest = b"-text\x1b[0m\n".to_vec();
    let split_at = color.len() / 2;
    let mut first = step;
    first.extend_from_slice(&color[..split_at]);
    let chunks = vec![first, color[split_at..].to_vec(), rest];
    let released = Arc::new(Mutex::new(1usize));
    let stderr = GatedReader {
        chunks,
        next: 0,
        released: released.clone(),
    };
    let model = Arc::new(Mutex::new({
        let mut model = wide_terminal_model();
        model.start_commandless_output_block();
        model
    }));
    let processor = Arc::new(Mutex::new(Processor::new()));
    let saw_first = Arc::new(Mutex::new(false));
    let model_for_cb = model.clone();
    let processor_for_cb = processor.clone();
    let saw_first_for_cb = saw_first.clone();
    let released_for_cb = released.clone();

    block_on(async {
        drain_dev_container_pipes(EofReader, stderr, move |chunk| {
            processor_for_cb.lock().unwrap().parse_bytes(
                &mut *model_for_cb.lock().unwrap(),
                chunk,
                &mut io::sink(),
            );
            let mut saw_first = saw_first_for_cb.lock().unwrap();
            if !*saw_first {
                let output = model_for_cb
                    .lock()
                    .unwrap()
                    .block_list()
                    .active_block()
                    .output_grid()
                    .contents_to_string(false, None);
                assert!(
                    output.contains("step-one"),
                    "first marker must render before later chunks, got {output:?}"
                );
                assert!(
                    !output.contains("red-text") && !output.contains("-text"),
                    "split ANSI must not be complete before later chunks, got {output:?}"
                );
                *saw_first = true;
                *released_for_cb.lock().unwrap() = usize::MAX;
            }
        })
        .await
        .expect("drain");
    });

    assert!(*saw_first.lock().unwrap(), "first marker callback must run");
    let output = model
        .lock()
        .unwrap()
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    assert!(
        output.contains("red-text") || (output.contains("red") && output.contains("text")),
        "split ANSI must complete after later chunks, got {output:?}"
    );
}

#[test]
fn commandless_output_block_height_grows_with_later_batches() {
    use warpui::units::Lines;

    use crate::terminal::model::block::TranscriptScope;

    let mut model = TerminalModel::mock(None, None);
    model.start_commandless_output_block();
    let mut processor = Processor::new();

    // The mock screen is 7 rows. A later batch must still increase visible
    // height after that viewport is already full; nonzero height alone matches
    // the broken one-line-tall block.
    let first: String = (0..10).map(|i| format!("first-{i}\r\n")).collect();
    processor.parse_bytes(&mut model, first.as_bytes(), &mut io::sink());
    let height_after_first = model.block_list().block_heights().summary().height;
    assert!(
        height_after_first > Lines::zero(),
        "first batch must be visible, got {height_after_first:?}"
    );

    let later: String = (0..20).map(|i| format!("later-{i}\r\n")).collect();
    processor.parse_bytes(&mut model, later.as_bytes(), &mut io::sink());
    let height_after_later = model.block_list().block_heights().summary().height;
    assert!(
        height_after_later > height_after_first,
        "later batch must grow visible height from {height_after_first:?} to more than that, \
         got {height_after_later:?}"
    );
    assert!(
        model
            .block_list()
            .active_block()
            .is_visible(&TranscriptScope::Terminal)
    );
}

#[test]
fn exit_aware_reader_replaces_a_single_waker_per_slot() {
    use std::pin::Pin;
    use std::task::Poll;

    use futures::future::poll_fn;
    use futures_util::AsyncRead;

    let child_exit = std::sync::Arc::new(super::ChildExit::new());
    let mut stdout = super::ExitAwareReader::new(AlwaysPendingReader, child_exit.clone(), 0);
    let mut stderr = super::ExitAwareReader::new(AlwaysPendingReader, child_exit.clone(), 1);
    block_on(async {
        for _ in 0..10_000 {
            poll_fn(|cx| {
                let mut buf = [0_u8; 8];
                assert!(Pin::new(&mut stdout).poll_read(cx, &mut buf).is_pending());
                assert!(Pin::new(&mut stderr).poll_read(cx, &mut buf).is_pending());
                Poll::Ready(())
            })
            .await;
            assert!(
                child_exit.registered_wakers() <= 2,
                "waker slots must stay bounded, got {}",
                child_exit.registered_wakers()
            );
        }
        assert_eq!(child_exit.registered_wakers(), 2);
        child_exit.mark();
        poll_fn(|cx| {
            let mut buf = [0_u8; 8];
            assert!(matches!(
                Pin::new(&mut stdout).poll_read(cx, &mut buf),
                Poll::Ready(Ok(0))
            ));
            assert!(matches!(
                Pin::new(&mut stderr).poll_read(cx, &mut buf),
                Poll::Ready(Ok(0))
            ));
            Poll::Ready(())
        })
        .await;
    });
}

struct AlwaysPendingReader;

impl futures_util::io::AsyncRead for AlwaysPendingReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Pending
    }
}

struct EofReader;

impl futures_util::io::AsyncRead for EofReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Ready(Ok(0))
    }
}

struct GatedReader {
    chunks: Vec<Vec<u8>>,
    next: usize,
    released: Arc<Mutex<usize>>,
}

impl futures_util::io::AsyncRead for GatedReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.next >= this.chunks.len() {
            return std::task::Poll::Ready(Ok(0));
        }
        if this.next >= *this.released.lock().unwrap() {
            cx.waker().wake_by_ref();
            return std::task::Poll::Pending;
        }
        let chunk = &this.chunks[this.next];
        let n = chunk.len().min(buf.len());
        buf[..n].copy_from_slice(&chunk[..n]);
        if n == chunk.len() {
            this.next += 1;
        } else {
            this.chunks[this.next] = chunk[n..].to_vec();
        }
        std::task::Poll::Ready(Ok(n))
    }
}

struct WriteCapture<'a>(&'a mut Vec<u8>);

impl io::Write for WriteCapture<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn pty_size_from_grid_matches_requested_columns_and_rows() {
    assert_eq!(
        super::PtySize::from_grid(64, 20),
        super::PtySize {
            columns: 64,
            rows: 20,
        }
    );
}

#[cfg(unix)]
#[test]
fn stdio_ptys_open_at_requested_winsize() {
    let size = super::PtySize {
        columns: 42,
        rows: 17,
    };
    let mut command = command::r#async::Command::new("true");
    let (stdout, stderr, handle) =
        super::attach_stdio_ptys(&mut command, size).expect("open stdio ptys");
    assert_eq!(handle.reported_sizes().expect("TIOCGWINSZ"), (size, size));
    drop((stdout, stderr, command));
}

#[cfg(unix)]
#[test]
fn stdio_pty_resize_updates_both_winsizes() {
    let initial = super::PtySize {
        columns: 40,
        rows: 12,
    };
    let updated = super::PtySize {
        columns: 64,
        rows: 20,
    };
    let mut command = command::r#async::Command::new("true");
    let (stdout, stderr, handle) =
        super::attach_stdio_ptys(&mut command, initial).expect("open stdio ptys");
    handle.resize(updated).expect("TIOCSWINSZ");
    assert_eq!(
        handle.reported_sizes().expect("TIOCGWINSZ"),
        (updated, updated)
    );
    drop((stdout, stderr, command));
}

#[cfg(unix)]
#[test]
fn drain_installs_resize_handle_and_child_sees_both_pty_winsizes() {
    use std::time::Duration;

    use futures::future::{self, Either};
    use instant::Instant;
    use parking_lot::Mutex as ParkingMutex;

    let initial = super::PtySize {
        columns: 40,
        rows: 12,
    };
    let updated = super::PtySize {
        columns: 64,
        rows: 20,
    };
    let release_dir = tempfile::tempdir().expect("release dir");
    let release_path = release_dir.path().join("go");
    let mut command = command::r#async::Command::new_with_process_group("python3");
    command
        .arg("-c")
        .arg(
            r#"
import fcntl, os, struct, termios, time
def winsize(fd):
    rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\0" * 8))
    return f"{cols}x{rows}"
os.write(2, f"initial stdout={winsize(1)} stderr={winsize(2)}\n".encode())
path = os.environ["DC_RELEASE_PATH"]
while not os.path.exists(path):
    time.sleep(0.01)
os.write(2, f"resized stdout={winsize(1)} stderr={winsize(2)}\n".encode())
"#,
        )
        .env("DC_RELEASE_PATH", &release_path)
        .kill_on_drop(true);
    let resize_slot = Arc::new(ParkingMutex::new(None));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = seen.clone();
    block_on(async {
        let drain_fut = super::drain_dev_container_child_with_size_and_resize(
            command,
            None,
            move |chunk| {
                seen_cb.lock().unwrap().extend_from_slice(chunk);
            },
            initial,
            Some(resize_slot.clone()),
        );
        let wait_fut = async {
            let started = Instant::now();
            loop {
                let output = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
                if output.contains("initial stdout=40x12") && output.contains("stderr=40x12") {
                    let handle = resize_slot
                        .lock()
                        .clone()
                        .expect("resize handle installed at spawn");
                    handle.resize(updated).expect("TIOCSWINSZ both ptys");
                    std::fs::write(&release_path, b"go").expect("release child");
                    return;
                }
                assert!(
                    started.elapsed().as_secs() < 5,
                    "child must report initial winsize, got {output:?}"
                );
                warpui::r#async::Timer::after(Duration::from_millis(10)).await;
            }
        };
        let work = async { futures::join!(drain_fut, wait_fut) };
        let timeout = async {
            warpui::r#async::Timer::after(Duration::from_secs(5)).await;
        };
        match future::select(Box::pin(work), Box::pin(timeout)).await {
            Either::Right(_) => panic!("timed out waiting for dual-PTY winsize"),
            Either::Left(((drain_result, _), _)) => {
                let (_drain, success) = drain_result.expect("drain");
                assert!(success);
                let displayed = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
                assert!(
                    displayed.contains("resized stdout=64x20"),
                    "stdout PTY must observe TIOCSWINSZ, got {displayed:?}"
                );
                assert!(
                    displayed.contains("stderr=64x20"),
                    "stderr PTY must observe TIOCSWINSZ, got {displayed:?}"
                );
            }
        }
    });
}

fn terminal_model_with_cols(cols: usize) -> TerminalModel {
    let mut sizes = block_size();
    sizes.size = SizeInfo::new_without_font_metrics(24, cols);
    TerminalModel::new_for_test(
        sizes,
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        None,
    )
}

#[test]
fn buildkit_duration_stays_on_same_row_when_pty_cols_match_grid() {
    let cols = 80;
    let mut model = terminal_model_with_cols(cols);
    model.start_commandless_output_block();
    let mut processor = Processor::new();
    let prefix = " => [internal] load build definition from Dockerfile";
    let duration = "0.0s";
    let pad = cols - prefix.len() - duration.len();
    let mut line = String::new();
    line.push_str(prefix);
    line.push_str(&" ".repeat(pad));
    line.push_str(duration);
    line.push('\n');
    assert_eq!(line.len() - 1, cols);
    processor.parse_bytes(&mut model, line.as_bytes(), &mut io::sink());
    let output = model
        .block_list()
        .active_block()
        .output_grid()
        .contents_to_string(false, None);
    let row = output
        .lines()
        .find(|line| line.contains("load build definition"))
        .unwrap_or_else(|| panic!("=> row missing from {output:?}"));
    assert!(
        row.contains(duration),
        "duration must stay on the => row when PTY cols match the grid, got {output:?}"
    );
}
