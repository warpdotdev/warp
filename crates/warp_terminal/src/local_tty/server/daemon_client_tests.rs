use std::time::Duration;

use instant::Instant;
use parking_lot::Mutex;

use super::*;
use crate::local_tty::server::daemon::{Daemon, DaemonConfig, StopHandle};
use crate::local_tty::server::test_util::sh_options;

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for condition");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct RunningDaemon {
    stop: StopHandle,
    thread: std::thread::JoinHandle<()>,
}

impl RunningDaemon {
    fn start(paths: &DaemonPaths) -> Self {
        let daemon = Daemon::bind(DaemonConfig::new(paths.clone()))
            .unwrap()
            .expect("lock should be free");
        let stop = daemon.stop_handle();
        let thread = std::thread::spawn(move || daemon.run());
        Self { stop, thread }
    }

    fn stop(self) {
        self.stop.stop();
        self.thread.join().unwrap();
    }
}

fn temp_paths() -> (tempfile::TempDir, DaemonPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::in_dir(dir.path().join("ts"));
    (dir, paths)
}

#[test]
fn connect_at_returns_none_without_a_daemon() {
    let (_dir, paths) = temp_paths();
    assert!(DaemonClient::connect_at(&paths.socket).unwrap().is_none());
}

#[test]
fn connect_or_spawn_starts_the_daemon_and_connects() {
    let (_dir, paths) = temp_paths();
    let running = Mutex::new(None);
    let client = DaemonClient::connect_or_spawn_with(&paths.socket, || {
        *running.lock() = Some(RunningDaemon::start(&paths));
        Ok(())
    })
    .unwrap();
    assert!(client.is_alive());
    assert!(client.list().unwrap().is_empty());

    let second = DaemonClient::connect_or_spawn_with(&paths.socket, || {
        panic!("should reuse the running daemon")
    })
    .unwrap();
    assert!(second.is_alive());
    running.lock().take().unwrap().stop();
}

#[test]
fn connect_or_spawn_fails_if_the_spawn_fails() {
    let (_dir, paths) = temp_paths();
    let err = DaemonClient::connect_or_spawn_with(&paths.socket, || anyhow::bail!("cannot start"))
        .err()
        .unwrap();
    assert!(err.to_string().contains("cannot start"));
}

#[test]
fn daemon_exit_marks_attached_sessions_unavailable() {
    let (_dir, paths) = temp_paths();
    let running = RunningDaemon::start(&paths);
    let client = DaemonClient::connect_at(&paths.socket).unwrap().unwrap();
    let mut session = client
        .spawn(
            PersistentSessionId::from(uuid::Uuid::from_u128(1)),
            SessionMetadata {
                hook_session_id: 1,
                shell_type: "bash".to_owned(),
            },
            sh_options("read x"),
        )
        .unwrap();
    assert!(
        session
            .handle
            .process_termination_status()
            .unwrap()
            .is_none()
    );

    running.stop();
    wait_until(|| !client.is_alive());
    assert_eq!(
        session.handle.process_termination_status().unwrap(),
        Some(ObservedExitStatus::Unavailable)
    );
    session.handle.kill().unwrap();
    assert!(client.list().is_err());
}

#[test]
fn connect_at_returns_none_if_the_daemon_closes_during_the_handshake() {
    let (_dir, paths) = temp_paths();
    std::fs::create_dir_all(&paths.dir).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&paths.socket).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
    });
    assert!(DaemonClient::connect_at(&paths.socket).unwrap().is_none());
    server.join().unwrap();
}

#[test]
fn connect_or_spawn_replaces_a_daemon_that_closes_during_the_handshake() {
    let (_dir, paths) = temp_paths();
    std::fs::create_dir_all(&paths.dir).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&paths.socket).unwrap();
    let closer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
        drop(listener);
    });
    let running = Mutex::new(None);
    let client = DaemonClient::connect_or_spawn_with(&paths.socket, || {
        closer.join().unwrap();
        *running.lock() = Some(RunningDaemon::start(&paths));
        Ok(())
    })
    .unwrap();
    assert!(client.list().unwrap().is_empty());
    running.lock().take().unwrap().stop();
}

#[test]
fn oversized_request_fails_without_dropping_the_connection() {
    let (_dir, paths) = temp_paths();
    let running = RunningDaemon::start(&paths);
    let client = DaemonClient::connect_at(&paths.socket).unwrap().unwrap();
    let environment = SpawnEnvironment {
        vars: vec![(
            "EXAMPLE_HUGE".into(),
            "x".repeat(protocol::MAX_CLIENT_FRAME_SIZE).into(),
        )],
    };
    let err = client
        .spawn_with_environment(
            PersistentSessionId::from(uuid::Uuid::from_u128(2)),
            SessionMetadata {
                hook_session_id: 1,
                shell_type: "bash".to_owned(),
            },
            sh_options("true"),
            environment,
        )
        .err()
        .unwrap();
    assert!(err.to_string().contains("too large"), "{err:#}");
    assert!(client.list().unwrap().is_empty());
    running.stop();
}

#[cfg(target_os = "macos")]
mod disclaimed_launcher {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::super::disclaimed_launcher_result;
    use super::super::spawn_macos::DisclaimedSpawn;

    #[test]
    fn a_successful_launcher_is_final() {
        let result = disclaimed_launcher_result(DisclaimedSpawn::Exited(ExitStatus::from_raw(0)));
        assert!(matches!(result, Some(Ok(()))));
    }

    #[test]
    fn a_failed_launcher_is_an_error_without_a_second_spawn() {
        let result =
            disclaimed_launcher_result(DisclaimedSpawn::Exited(ExitStatus::from_raw(1 << 8)));
        assert!(matches!(result, Some(Err(_))));
        let result =
            disclaimed_launcher_result(DisclaimedSpawn::WaitFailed(anyhow::anyhow!("ECHILD")));
        assert!(matches!(result, Some(Err(_))));
    }

    #[test]
    fn nothing_launched_falls_back_to_a_regular_spawn() {
        let result = disclaimed_launcher_result(DisclaimedSpawn::NotLaunched(anyhow::anyhow!(
            "responsibility_spawnattrs_setdisclaim failed"
        )));
        assert!(result.is_none());
    }
}
