use std::process::Stdio;
use std::time::{Duration, Instant};

use futures_lite::future::block_on;

use super::{Command, JobGuard};

const SETTLE: Duration = Duration::from_secs(5);

/// Polls `condition` until it holds or `SETTLE` passes.
fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[cfg(unix)]
mod unix {
    use futures_lite::io::{AsyncBufReadExt, BufReader};

    use super::*;

    fn process_exists(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the pid exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// Starts a shell that leaves a sleeping grandchild behind and prints the grandchild's pid.
    fn spawn_shell_with_grandchild() -> (async_process::Child, JobGuard, i32) {
        let mut command = Command::new_in_job("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 30 & echo $!; wait")
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let (mut child, guard) = command.spawn_in_job().expect("spawns");
        let mut line = String::new();
        block_on(BufReader::new(child.stdout.take().expect("piped stdout")).read_line(&mut line))
            .expect("reads the grandchild pid");
        let grandchild = line.trim().parse().expect("a pid");
        assert!(process_exists(child.id() as i32));
        assert!(process_exists(grandchild));
        (child, guard, grandchild)
    }

    #[test]
    fn terminate_ends_the_child_and_its_grandchild() {
        let (mut child, guard, grandchild) = spawn_shell_with_grandchild();
        assert!(guard.is_alive());

        guard.terminate();

        let status = block_on(child.status()).expect("the child is collected");
        assert!(!status.success());
        assert!(
            eventually(|| !process_exists(grandchild)),
            "the grandchild {grandchild} survived"
        );
        assert!(eventually(|| !guard.is_alive()));
    }

    #[test]
    fn request_stop_sends_a_catchable_signal_first() {
        let (mut child, guard, grandchild) = spawn_shell_with_grandchild();

        guard.request_stop();

        // `sleep` and `sh` both die on SIGTERM; nothing needed the kill.
        assert!(eventually(|| !process_exists(grandchild)));
        let status = block_on(child.status()).expect("the child is collected");
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGTERM)
        );
    }

    #[test]
    fn dropping_the_guard_ends_the_tree() {
        let (mut child, guard, grandchild) = spawn_shell_with_grandchild();

        drop(guard);

        block_on(child.status()).expect("the child is collected");
        assert!(
            eventually(|| !process_exists(grandchild)),
            "the grandchild {grandchild} survived the guard being dropped"
        );
    }

    #[test]
    fn a_tree_that_has_ended_is_not_alive_and_signals_are_tolerated() {
        let mut command = Command::new_in_job("/bin/sh");
        command.arg("-c").arg("exit 0");
        let (mut child, guard) = command.spawn_in_job().expect("spawns");
        block_on(child.status()).expect("the child is collected");

        assert!(eventually(|| !guard.is_alive()));
        // Neither call may panic or report anything but the group being gone.
        guard.request_stop();
        guard.terminate();
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    /// Starts `cmd.exe` with a long-running grandchild.
    fn spawn_cmd_with_grandchild() -> (async_process::Child, JobGuard) {
        let mut command = Command::new_in_job("cmd.exe");
        command
            .arg("/C")
            .arg("ping -n 60 127.0.0.1 >NUL")
            .kill_on_drop(true);
        let (child, guard) = command.spawn_in_job().expect("spawns");
        assert!(
            eventually(|| guard.active_processes().is_ok_and(|count| count >= 2)),
            "the job never saw both cmd.exe and ping.exe"
        );
        (child, guard)
    }

    #[test]
    fn terminate_ends_every_process_in_the_job() {
        let (mut child, guard) = spawn_cmd_with_grandchild();
        assert!(guard.is_alive());

        guard.terminate();

        block_on(child.status()).expect("the child is collected");
        assert!(
            eventually(|| guard.active_processes().is_ok_and(|count| count == 0)),
            "a process in the job survived termination"
        );
        assert!(!guard.is_alive());
    }

    #[test]
    fn dropping_the_guard_ends_the_child() {
        let (mut child, guard) = spawn_cmd_with_grandchild();

        drop(guard);

        let mut exited = false;
        assert!(eventually(|| {
            exited = child.try_status().expect("status").is_some();
            exited
        }));
    }
}
