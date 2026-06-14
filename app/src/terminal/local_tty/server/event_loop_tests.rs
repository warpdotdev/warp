use std::io::{BufRead as _, BufReader};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::*;

struct ProcessGroupGuard(u32);

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        let _ = Children::signal_process_group(self.0, nix::sys::signal::SIGKILL);
    }
}

fn spawn_shell_process_group(script: &str) -> (Child, u32, ProcessGroupGuard) {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script).stdout(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn().expect("shell process group should spawn");
    let group_guard = ProcessGroupGuard(child.id());
    let mut descendant_pid = String::new();
    BufReader::new(
        child
            .stdout
            .take()
            .expect("shell process should expose stdout"),
    )
    .read_line(&mut descendant_pid)
    .expect("shell should report descendant pid");

    (
        child,
        descendant_pid
            .trim()
            .parse()
            .expect("reported descendant pid should be numeric"),
        group_guard,
    )
}

fn assert_process_exits(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("process {pid} did not exit before the deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn terminate_child_signals_the_entire_pty_process_group() {
    let (child, descendant_pid, _group_guard) =
        spawn_shell_process_group("sleep 30 & printf '%s\n' \"$!\"; wait");
    let leader_pid = child.id();
    let mut children = Children::new();
    children.insert(child);

    children
        .terminate_child(leader_pid)
        .expect("PTY process group should terminate");

    assert_eq!(children.0.len(), 0);
    assert_process_exits(descendant_pid);
}

#[test]
fn terminate_child_escalates_when_a_descendant_ignores_sighup_after_the_shell_exits() {
    let (child, descendant_pid, _group_guard) = spawn_shell_process_group(
        "/bin/sh -c 'trap \"\" HUP; printf \"%s\\n\" \"$$\"; sleep 30' & wait",
    );
    let leader_pid = child.id();
    let mut children = Children::new();
    children.insert(child);
    let started_at = Instant::now();

    children
        .terminate_child(leader_pid)
        .expect("PTY process group should terminate after escalation");

    assert!(started_at.elapsed() >= CHILD_TERMINATION_GRACE_PERIOD);
    assert_eq!(children.0.len(), 0);
    assert_process_exits(descendant_pid);
}

#[test]
fn terminate_all_escalates_when_a_pty_process_group_ignores_sighup() {
    let (child, descendant_pid, _group_guard) =
        spawn_shell_process_group("trap '' HUP; sleep 30 & printf '%s\n' \"$!\"; wait");
    let mut children = Children::new();
    children.insert(child);
    let started_at = Instant::now();

    children.terminate_all();

    assert!(started_at.elapsed() >= CHILD_TERMINATION_GRACE_PERIOD);
    assert_eq!(children.0.len(), 0);
    assert_process_exits(descendant_pid);
}
