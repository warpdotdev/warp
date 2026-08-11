#[cfg(unix)]
mod unix {
    use std::collections::HashMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::os::unix::process::ExitStatusExt as _;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use async_io::Timer;
    use futures_util::future::{AbortHandle, Abortable, Aborted};
    use instant::Instant;
    use nix::sys::signal::kill;
    use nix::sys::stat::Mode;
    use nix::unistd::{Pid, mkfifo};

    use super::super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);

    fn executor() -> LocalCommandExecutor {
        LocalCommandExecutor::new(Some("/bin/bash".into()), ShellType::Bash)
    }

    fn descendant_command() -> &'static str {
        "/bin/sh -c 'printf %s \"$$\" > \"$READY_FILE\"; \
         IFS= read -r _ < \"$RELEASE_FIFO\"; \
         printf descendant-ran > \"$SIDE_EFFECT_FILE\"' \
         </dev/null >/dev/null 2>&1 & wait"
    }

    fn detached_descendant_command() -> &'static str {
        "/bin/sh -c 'printf %s \"$$\" > \"$READY_FILE\"; \
         IFS= read -r _ < \"$RELEASE_FIFO\"; \
         printf descendant-ran > \"$SIDE_EFFECT_FILE\"' \
         </dev/null >/dev/null 2>&1 &"
    }

    fn command_environment(temp_dir: &Path) -> HashMap<String, String> {
        HashMap::from([
            (
                "READY_FILE".into(),
                temp_dir.join("ready").to_string_lossy().into_owned(),
            ),
            (
                "RELEASE_FIFO".into(),
                temp_dir.join("release-fifo").to_string_lossy().into_owned(),
            ),
            (
                "SIDE_EFFECT_FILE".into(),
                temp_dir.join("side-effect").to_string_lossy().into_owned(),
            ),
        ])
    }

    fn create_release_fifo(temp_dir: &Path) -> File {
        let path = temp_dir.join("release-fifo");
        mkfifo(&path, Mode::S_IRUSR | Mode::S_IWUSR).expect("create release FIFO");
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .expect("open release FIFO")
    }

    async fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + TIMEOUT;
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            Timer::after(Duration::from_millis(10)).await;
        }
    }
    async fn wait_for_process_exit(pid: Pid) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match kill(pid, None) {
                Err(nix::errno::Errno::ESRCH) => return,
                Ok(()) | Err(nix::errno::Errno::EPERM) => {}
                Err(error) => panic!("failed to inspect descendant {pid}: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for descendant {pid} to exit"
            );
            Timer::after(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_descendant_pid(path: &Path) -> Pid {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(contents) = fs::read_to_string(path)
                && let Ok(pid) = contents.parse()
            {
                return Pid::from_raw(pid);
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for descendant PID in {}",
                path.display()
            );
            Timer::after(Duration::from_millis(10)).await;
        }
    }

    #[test]
    fn dropping_command_future_kills_descendant_process() {
        futures_lite::future::block_on(async {
            let temp_dir = tempfile::tempdir().expect("create temp dir");
            let mut release_fifo = create_release_fifo(temp_dir.path());
            let ready_file = temp_dir.path().join("ready");
            let side_effect_file = temp_dir.path().join("side-effect");
            let executor = executor();
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let command = executor.execute_local_command(
                descendant_command(),
                None,
                Some(command_environment(temp_dir.path())),
                ExecuteCommandOptions::default(),
            );
            let command = Abortable::new(command, abort_registration);

            let task_executor = async_executor::LocalExecutor::new();
            task_executor
                .run(async {
                    let command_task = task_executor.spawn(command);
                    let descendant_pid = wait_for_descendant_pid(&ready_file).await;

                    abort_handle.abort();
                    assert!(matches!(command_task.await, Err(Aborted)));

                    writeln!(release_fifo, "continue").expect("release descendant");
                    wait_for_process_exit(descendant_pid).await;
                    assert!(
                        !side_effect_file.exists(),
                        "canceled descendant unexpectedly created {}",
                        side_effect_file.display()
                    );
                })
                .await;
        });
    }

    #[test]
    fn cancel_active_commands_kills_descendant_process() {
        futures_lite::future::block_on(async {
            let temp_dir = tempfile::tempdir().expect("create temp dir");
            let mut release_fifo = create_release_fifo(temp_dir.path());
            let ready_file = temp_dir.path().join("ready");
            let side_effect_file = temp_dir.path().join("side-effect");
            let executor = executor();
            let command = executor.execute_local_command(
                descendant_command(),
                None,
                Some(command_environment(temp_dir.path())),
                ExecuteCommandOptions::default(),
            );

            let task_executor = async_executor::LocalExecutor::new();
            task_executor
                .run(async {
                    let command_task = task_executor.spawn(command);
                    let descendant_pid = wait_for_descendant_pid(&ready_file).await;

                    executor.cancel_active_commands();
                    let _ = command_task.await;

                    writeln!(release_fifo, "continue").expect("release descendant");
                    wait_for_process_exit(descendant_pid).await;
                    assert!(
                        !side_effect_file.exists(),
                        "canceled descendant unexpectedly created {}",
                        side_effect_file.display()
                    );
                })
                .await;
        });
    }

    #[test]
    fn completed_command_is_not_canceled_later() {
        futures_lite::future::block_on(async {
            let temp_dir = tempfile::tempdir().expect("create temp dir");
            let mut release_fifo = create_release_fifo(temp_dir.path());
            let ready_file = temp_dir.path().join("ready");
            let side_effect_file = temp_dir.path().join("side-effect");
            let executor = executor();

            executor
                .execute_local_command(
                    detached_descendant_command(),
                    None,
                    Some(command_environment(temp_dir.path())),
                    ExecuteCommandOptions::default(),
                )
                .await
                .expect("complete wrapper command");
            wait_for_file(&ready_file).await;

            executor.cancel_active_commands();
            writeln!(release_fifo, "continue").expect("release descendant");
            wait_for_file(&side_effect_file).await;
            assert_eq!(
                fs::read_to_string(side_effect_file).expect("read side effect"),
                "descendant-ran"
            );
        });
    }

    fn spawn_owned_group_leader(sleep_seconds: u32) -> std::process::Child {
        let mut cmd = command::blocking::Command::new("/bin/sleep");
        cmd.arg(sleep_seconds.to_string());
        // SAFETY: `setpgid` is async-signal-safe (see signal-safety(7)), and
        // only makes the child the leader of a new process group with its
        // own pid, mirroring `Command::new_with_process_group`.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.spawn().expect("spawn /bin/sleep")
    }

    #[test]
    fn terminate_process_group_refuses_dangerous_pids() {
        // If this bound were missing, `terminate_process_group(0)` would
        // attempt `kill(-0, SIGKILL)` (this process's own process group,
        // i.e. the test binary itself) and `terminate_process_group(1)`
        // would attempt `kill(-1, SIGKILL)`, a broadcast to every process
        // this test binary is allowed to signal. Reaching the assertion
        // below is itself the proof the bound held.
        terminate_process_group(0);
        terminate_process_group(1);
        assert_ne!(std::process::id(), 0);
    }

    #[test]
    fn terminate_process_group_kills_group_leader() {
        let mut child = spawn_owned_group_leader(5);

        terminate_process_group(child.id());

        let status = child.wait().expect("wait for sleep");
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn terminate_process_group_kills_descendant_after_leader_exits() {
        // Reproduces the scenario a plain `getpgid(leader_pid) == leader_pid`
        // check gets wrong: a generator shell backgrounds a child, then the
        // shell (the registered pid) exits and is reaped, while the
        // backgrounded descendant is still alive in the same process group.
        // `kill(-pgid, SIGKILL)` must still reach that descendant.
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let pid_file = temp_dir.path().join("descendant-pid");

        let mut leader_cmd = command::blocking::Command::new("/bin/sh");
        leader_cmd.arg("-c").arg(format!(
            "sleep 30 </dev/null >/dev/null 2>&1 & echo $! > {}; exit 0",
            pid_file.display()
        ));
        // SAFETY: see `spawn_owned_group_leader`.
        unsafe {
            leader_cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut leader = leader_cmd.spawn().expect("spawn leader shell");
        let leader_pid = leader.id();

        // Reap the leader now, so it is gone by the time we cancel: this is
        // the window the old `getpgid(leader_pid)` check misread as "not
        // ours".
        let status = leader.wait().expect("wait for leader shell");
        assert!(status.success());

        let deadline = Instant::now() + TIMEOUT;
        let descendant_pid = loop {
            if let Ok(contents) = fs::read_to_string(&pid_file)
                && let Ok(pid) = contents.trim().parse::<i32>()
            {
                break Pid::from_raw(pid);
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for descendant pid"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(
            kill(descendant_pid, None).is_ok(),
            "descendant should still be alive after the leader exits"
        );

        terminate_process_group(leader_pid);

        let deadline = Instant::now() + TIMEOUT;
        loop {
            match kill(descendant_pid, None) {
                Err(nix::errno::Errno::ESRCH) => break,
                Ok(()) | Err(nix::errno::Errno::EPERM) => {}
                Err(error) => panic!("failed to inspect descendant {descendant_pid}: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for descendant {descendant_pid} to exit"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
