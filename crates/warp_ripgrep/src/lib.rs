//! Thin wrapper around ripgrep for searching files.

pub mod search;
#[cfg(not(target_family = "wasm"))]
mod types;

/// On Unix, monitor the parent PID and exit this process if it changes.
///
/// This is used by the ripgrep worker process to ensure we don't keep
/// searching if the main Warp process has exited.
#[cfg(unix)]
pub fn monitor_parent_and_exit_on_change(parent_pid: Option<u32>) {
    use std::thread;
    use std::time::Duration;

    use nix::unistd::Pid;

    let expected_parent = match parent_pid {
        Some(pid) => Pid::from_raw(pid as i32),
        None => return,
    };

    thread::spawn(move || {
        loop {
            // If we've been reparented to a different process, the original
            // parent died so we should exit.
            if Pid::parent() != expected_parent {
                log::info!(
                    "ripgrep helper: detected parent pid change (expected {expected_parent}); \
                     exiting search process."
                );
                std::process::exit(0);
            }

            thread::sleep(Duration::from_secs(1));
        }
    });
}
