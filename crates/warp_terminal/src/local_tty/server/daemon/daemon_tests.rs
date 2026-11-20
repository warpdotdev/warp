//! Integration tests that run the daemon on a thread of the test process and
//! talk to it through [`DaemonClient`], with `/bin/sh` as the shell.

use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use instant::Instant;

use super::*;
use crate::event::ObservedExitStatus;
use crate::local_tty::server::daemon_client::{AttachedSession, DaemonClient};
use crate::local_tty::server::test_util::sh_options;

const TIMEOUT: Duration = Duration::from_secs(20);
const HOOK_SESSION: u64 = 42;

struct TestDaemon {
    dir: tempfile::TempDir,
    paths: DaemonPaths,
    stop: StopHandle,
    thread: Option<JoinHandle<()>>,
}

impl TestDaemon {
    fn start() -> Self {
        Self::start_with(|_| {})
    }

    fn start_with(configure: impl FnOnce(&mut DaemonConfig)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = DaemonPaths::in_dir(dir.path().join("ts"));
        let mut config = DaemonConfig::new(paths.clone());
        config.idle_timeout = Duration::from_secs(600);
        configure(&mut config);
        let daemon = Daemon::bind(config).unwrap().expect("lock should be free");
        let stop = daemon.stop_handle();
        let thread = std::thread::spawn(move || daemon.run());
        Self {
            dir,
            paths,
            stop,
            thread: Some(thread),
        }
    }

    fn client(&self) -> Arc<DaemonClient> {
        DaemonClient::connect_at(&self.paths.socket)
            .unwrap()
            .expect("daemon should be listening")
    }

    fn marker(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.stop.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn session_id(n: u128) -> PersistentSessionId {
    PersistentSessionId::from(uuid::Uuid::from_u128(n))
}

fn metadata() -> SessionMetadata {
    SessionMetadata {
        hook_session_id: HOOK_SESSION,
        shell_type: "bash".to_owned(),
    }
}

fn hex_hook(json: &str) -> Vec<u8> {
    let mut bytes = b"\x1bP$d".to_vec();
    bytes.extend_from_slice(hex::encode(json).as_bytes());
    bytes.extend_from_slice(b"\x1b\\");
    bytes
}

fn init_shell_hook() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"InitShell","value":{{"session_id":{HOOK_SESSION},"shell":"bash"}}}}"#
    ))
}

fn bootstrapped_hook() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"Bootstrapped","value":{{"session_id":{HOOK_SESSION}}}}}"#
    ))
}

fn command_finished_hook() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"CommandFinished","value":{{"exit_code":0,"session_id":{HOOK_SESSION}}}}}"#
    ))
}

fn precmd_hook() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"Precmd","value":{{"session_id":{HOOK_SESSION}}}}}"#
    ))
}

/// A shell `printf` command that writes `bytes` verbatim.
fn printf(bytes: &[u8]) -> String {
    let escaped: String = bytes.iter().map(|b| format!("\\{b:03o}")).collect();
    format!("printf '{escaped}'")
}

fn stream(fd: OwnedFd) -> UnixStream {
    let stream = UnixStream::from(fd);
    // macOS rejects socket options on a socket whose peer already shut down.
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    stream
}

fn read_until(stream: &mut UnixStream, needle: &[u8]) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf).expect("read should not time out");
        assert!(n > 0, "unexpected EOF; received {received:?}");
        let search_from = received.len().saturating_sub(needle.len());
        received.extend_from_slice(&buf[..n]);
        if contains(&received[search_from..], needle) {
            return received;
        }
    }
}

fn read_to_eof(stream: &mut UnixStream) -> Vec<u8> {
    let mut received = Vec::new();
    stream.read_to_end(&mut received).unwrap();
    received
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for condition");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn write_to_leader(session: &AttachedSession, bytes: &[u8]) {
    let leader = session.leader.as_ref().expect("live session has a leader");
    let mut file = std::fs::File::from(leader.try_clone().unwrap());
    file.write_all(bytes).unwrap();
}

fn process_exists(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}

fn exit_status_of(client: &DaemonClient, id: PersistentSessionId) -> Option<ObservedExitStatus> {
    client
        .list()
        .unwrap()
        .into_iter()
        .find(|session| session.session_id == id)
        .and_then(|session| session.exit_status)
}

#[test]
fn detached_output_is_replayed_after_the_preamble_and_live_bytes_follow() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let script = format!(
        "{init}; {boot}; {precmd}; printf 'old-output\\n'; read a; \
         {finished}; {precmd}; printf 'prompt2 '; read b; printf 'detached:%s\\n' \"$b\"; \
         read c; printf 'live:%s\\n' \"$c\"; read d",
        init = printf(&init_shell_hook()),
        boot = printf(&bootstrapped_hook()),
        precmd = printf(&precmd_hook()),
        finished = printf(&command_finished_hook()),
    );
    let id = session_id(1);
    let spawned = client.spawn(id, metadata(), sh_options(&script)).unwrap();
    assert_eq!(spawned.info.replay_len, 0);
    assert!(spawned.leader.is_some());
    let pid = spawned.info.pid;
    let mut data = stream(spawned.data.try_clone().unwrap());
    read_until(&mut data, b"old-output");
    data.write_all(b"one\n").unwrap();
    read_until(&mut data, b"prompt2 ");

    client.detach_all();
    write_to_leader(&spawned, b"two\n");

    let second = daemon.client();
    wait_until(|| !client.is_alive());
    let replay = loop {
        let attached = second.attach(id).unwrap();
        assert_eq!(attached.info.pid, pid);
        assert!(attached.info.has_bootstrap_preamble);
        assert!(attached.info.exit_status.is_none());
        assert!(attached.leader.is_some());
        let mut data = stream(attached.data);
        let mut replay = vec![0u8; attached.info.replay_len as usize];
        data.read_exact(&mut replay).unwrap();
        if contains(&replay, b"detached:two") {
            break (replay, data);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (replay, mut data) = replay;

    let mut preamble = init_shell_hook();
    preamble.extend_from_slice(&bootstrapped_hook());
    assert!(replay.starts_with(&preamble));
    let body = &replay[preamble.len()..];
    let mut prompt_start = command_finished_hook();
    prompt_start.extend_from_slice(&precmd_hook());
    assert!(body.starts_with(&prompt_start));
    assert!(!contains(body, b"old-output"));
    let prompt = find(body, b"prompt2 ").unwrap();
    let detached = find(body, b"detached:two").unwrap();
    assert!(prompt < detached);

    data.write_all(b"three\n").unwrap();
    read_until(&mut data, b"live:three");
}

#[test]
fn kill_terminates_the_process_and_removes_the_session() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let id = session_id(2);
    let mut spawned = client.spawn(id, metadata(), sh_options("read x")).unwrap();
    let pid = spawned.info.pid;
    assert!(process_exists(pid));

    spawned.handle.kill().unwrap();
    assert!(client.list().unwrap().is_empty());
    wait_until(|| !process_exists(pid));
    let mut data = stream(spawned.data);
    assert!(read_to_eof(&mut data).is_empty());
}

#[test]
fn session_exit_while_attached_is_reported_and_drops_the_session() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let id = session_id(3);
    let mut spawned = client
        .spawn(id, metadata(), sh_options("read x; printf 'bye'; exit 3"))
        .unwrap();
    let mut data = stream(spawned.data.try_clone().unwrap());
    data.write_all(b"go\n").unwrap();
    let output = read_to_eof(&mut data);
    assert!(
        contains(&output, b"bye"),
        "output: {:?}",
        String::from_utf8_lossy(&output)
    );

    let mut status = None;
    wait_until(|| {
        status = spawned.handle.process_termination_status().unwrap();
        status.is_some()
    });
    assert_eq!(status, Some(ObservedExitStatus::Code(3)));
    assert!(client.list().unwrap().is_empty());
}

#[test]
fn exited_session_is_kept_until_attached_then_dropped() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let id = session_id(4);
    let spawned = client
        .spawn(
            id,
            metadata(),
            sh_options("read x; printf 'farewell'; exit 5"),
        )
        .unwrap();
    client.detach_all();
    write_to_leader(&spawned, b"go\n");
    wait_until(|| exit_status_of(&client, id).is_some());
    assert_eq!(
        exit_status_of(&client, id),
        Some(ObservedExitStatus::Code(5))
    );

    let second = daemon.client();
    let mut attached = second.attach(id).unwrap();
    assert_eq!(attached.info.exit_status, Some(ObservedExitStatus::Code(5)));
    assert!(attached.leader.is_none());
    assert_eq!(
        attached.handle.process_termination_status().unwrap(),
        Some(ObservedExitStatus::Code(5))
    );
    let mut data = stream(attached.data);
    let replay = read_to_eof(&mut data);
    assert_eq!(replay.len() as u64, attached.info.replay_len);
    assert!(contains(&replay, b"farewell"));
    wait_until(|| second.list().unwrap().is_empty());
}

#[test]
fn prune_kills_detached_sessions_that_are_not_kept() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let kept = client
        .spawn(session_id(5), metadata(), sh_options("read x"))
        .unwrap();
    let pruned = client
        .spawn(session_id(6), metadata(), sh_options("read x"))
        .unwrap();
    client.detach_all();

    let second = daemon.client();
    let attached = second
        .spawn(session_id(7), metadata(), sh_options("read x"))
        .unwrap();
    second.prune(vec![session_id(5)]).unwrap();

    let mut remaining: Vec<_> = second
        .list()
        .unwrap()
        .into_iter()
        .map(|session| (session.session_id, session.attached))
        .collect();
    remaining.sort_by_key(|(id, _)| id.0);
    assert_eq!(
        remaining,
        vec![(session_id(5), false), (session_id(7), true)]
    );
    wait_until(|| !process_exists(pruned.info.pid));
    assert!(process_exists(kept.info.pid));
    assert!(process_exists(attached.info.pid));

    second.prune(vec![]).unwrap();
    wait_until(|| !process_exists(kept.info.pid));
    second.kill_all_attached();
    wait_until(|| !process_exists(attached.info.pid));
    assert!(second.list().unwrap().is_empty());
}

#[test]
fn spawning_an_existing_session_replaces_it() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let first = client
        .spawn(session_id(8), metadata(), sh_options("read x"))
        .unwrap();
    let old_pid = first.info.pid;
    let second = client
        .spawn(session_id(8), metadata(), sh_options("read x"))
        .unwrap();
    assert_ne!(second.info.pid, old_pid);
    assert_eq!(second.info.replay_len, 0);
    wait_until(|| !process_exists(old_pid));
    assert!(process_exists(second.info.pid));
    let mut old_data = stream(first.data);
    assert!(read_to_eof(&mut old_data).is_empty());

    let sessions = client.list().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].pid, second.info.pid);
    assert!(sessions[0].attached);
    assert!(client.attach(session_id(9)).is_err());
}

#[test]
fn spawning_over_a_detached_session_replaces_it() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let first = client
        .spawn(session_id(15), metadata(), sh_options("read x"))
        .unwrap();
    client.detach_all();

    let second_client = daemon.client();
    let second = second_client
        .spawn(session_id(15), metadata(), sh_options("read x"))
        .unwrap();
    wait_until(|| !process_exists(first.info.pid));
    let sessions = second_client.list().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].pid, second.info.pid);
}

#[test]
fn new_client_evicts_the_previous_one() {
    let daemon = TestDaemon::start();
    let first = daemon.client();
    let mut spawned = first
        .spawn(session_id(10), metadata(), sh_options("read x"))
        .unwrap();
    let pid = spawned.info.pid;
    let _second = daemon.client();
    wait_until(|| !first.is_alive());
    assert_eq!(
        spawned.handle.process_termination_status().unwrap(),
        Some(ObservedExitStatus::Unavailable)
    );
    let mut data = stream(spawned.data.try_clone().unwrap());
    read_to_eof(&mut data);
    spawned.handle.kill().unwrap();
    assert!(process_exists(pid));
}

#[test]
fn detach_all_makes_kill_a_no_op() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let mut spawned = client
        .spawn(session_id(11), metadata(), sh_options("read x"))
        .unwrap();
    client.detach_all();
    spawned.handle.kill().unwrap();
    assert!(process_exists(spawned.info.pid));
    let sessions = client.list().unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].attached);
}

#[test]
fn idle_daemon_exits_and_removes_its_socket() {
    let mut daemon = TestDaemon::start_with(|config| {
        config.idle_timeout = Duration::from_millis(200);
    });
    let thread = daemon.thread.take().unwrap();
    wait_until(|| thread.is_finished());
    thread.join().unwrap();
    assert!(!daemon.paths.socket.exists());
}

#[test]
fn daemon_with_a_session_does_not_exit_when_idle() {
    let daemon = TestDaemon::start_with(|config| {
        config.idle_timeout = Duration::from_millis(200);
    });
    let client = daemon.client();
    let spawned = client
        .spawn(session_id(12), metadata(), sh_options("read x"))
        .unwrap();
    client.detach_all();
    drop(spawned);
    drop(client);
    wait_until(|| {
        daemon
            .stop
            .stats
            .unattended
            .lock()
            .is_some_and(|(since, checked)| checked - since >= Duration::from_millis(400))
    });
    assert!(!daemon.thread.as_ref().unwrap().is_finished());
    assert_eq!(daemon.client().list().unwrap().len(), 1);
}

#[test]
fn second_daemon_does_not_start_while_the_lock_is_held() {
    let daemon = TestDaemon::start();
    let second = Daemon::bind(DaemonConfig::new(daemon.paths.clone())).unwrap();
    assert!(second.is_none());
    assert!(daemon.client().is_alive());
}

#[test]
fn socket_and_directory_are_private() {
    let daemon = TestDaemon::start();
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&daemon.paths.dir), 0o700);
    assert_eq!(mode(&daemon.paths.socket), 0o600);
    assert_eq!(mode(&daemon.paths.lock), 0o600);
}

#[test]
fn hello_with_another_protocol_version_is_rejected() {
    let daemon = TestDaemon::start();
    let stream = UnixStream::connect(&daemon.paths.socket).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    protocol::send_frame(
        stream.as_raw_fd(),
        &ClientMessage::Hello {
            protocol_version: DAEMON_PROTOCOL_VERSION + 1,
            client_pid: 1,
            app_version: None,
        },
        &[],
    )
    .unwrap();
    let (reply, _): (DaemonMessage, _) = protocol::receive_frame(stream.as_raw_fd())
        .unwrap()
        .unwrap();
    assert!(matches!(
        reply,
        DaemonMessage::HelloReject {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            ..
        }
    ));
    assert!(
        protocol::receive_frame::<DaemonMessage>(stream.as_raw_fd())
            .unwrap()
            .is_none()
    );
}

#[test]
fn request_before_hello_closes_the_connection() {
    let daemon = TestDaemon::start();
    let stream = UnixStream::connect(&daemon.paths.socket).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    protocol::send_frame(
        stream.as_raw_fd(),
        &ClientMessage::Request {
            request_id: 1,
            request: DaemonRequest::List,
        },
        &[],
    )
    .unwrap();
    assert!(
        protocol::receive_frame::<DaemonMessage>(stream.as_raw_fd())
            .unwrap()
            .is_none()
    );
}

#[test]
fn detached_session_keeps_running_through_heavy_output() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let done = daemon.marker("done");
    let script = format!(
        "read x; head -c 52428800 /dev/zero; printf 'tail-marker'; : > '{}'; read y",
        done.display()
    );
    let spawned = client
        .spawn(session_id(13), metadata(), sh_options(&script))
        .unwrap();
    client.detach_all();
    write_to_leader(&spawned, b"go\n");
    wait_until(|| done.exists());
    let sessions = client.list().unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].exit_status.is_none());

    let second = daemon.client();
    let mut replay = Vec::new();
    wait_until(|| {
        let attached = second.attach(session_id(13)).unwrap();
        let mut data = stream(attached.data);
        replay = vec![0u8; attached.info.replay_len as usize];
        data.read_exact(&mut replay).unwrap();
        contains(&replay, b"tail-marker")
    });
    assert!(replay.len() <= RING_CAPACITY + super::ring::TRUNCATION_MARKER.len());
}

#[test]
fn backpressure_pauses_reading_without_losing_bytes() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let script = "read x; head -c 12582912 /dev/zero | tr '\\0' 'a'; printf 'END'; read y";
    let spawned = client
        .spawn(session_id(14), metadata(), sh_options(script))
        .unwrap();
    let mut data = stream(spawned.data.try_clone().unwrap());
    data.write_all(b"go\n").unwrap();
    wait_until(|| daemon.stop.stats.pauses.load(Ordering::SeqCst) > 0);
    let received = read_until(&mut data, b"END");
    let a_count = received.iter().filter(|b| **b == b'a').count();
    assert_eq!(a_count, 12 * 1024 * 1024);
}

fn is_shell_identifier(key: &std::ffi::OsStr) -> bool {
    let bytes = key.as_encoded_bytes();
    bytes
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

#[test]
fn shell_starts_from_the_client_environment() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let home = daemon.marker("home");
    std::fs::create_dir(&home).unwrap();
    let dropped = std::env::vars_os()
        .map(|(key, _)| key)
        .find(|key| {
            is_shell_identifier(key)
                && !["PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR"]
                    .iter()
                    .any(|name| key == name)
                && !key.to_string_lossy().starts_with("TERM")
                && !key.to_string_lossy().starts_with("WARP_")
        })
        .expect("the test process should have an ordinary environment variable");
    let mut vars: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| *key != dropped && key != "HOME")
        .collect();
    vars.push(("HOME".into(), home.clone().into_os_string()));
    vars.push(("EXAMPLE_CLIENT_VAR".into(), "from-client".into()));
    let script = format!(
        "printf 'client=%s;dropped=%s;cwd=%s;\\n' \"$EXAMPLE_CLIENT_VAR\" \"${{{dropped}-unset}}\" \"$(pwd -P)\"; read x",
        dropped = dropped.to_string_lossy(),
    );
    let spawned = client
        .spawn_with_environment(
            session_id(20),
            metadata(),
            sh_options(&script),
            SpawnEnvironment { vars },
        )
        .unwrap();
    let mut data = stream(spawned.data.try_clone().unwrap());
    let output = String::from_utf8_lossy(&read_until(&mut data, b";\r\n")).into_owned();
    let expected = format!(
        "client=from-client;dropped=unset;cwd={};",
        home.canonicalize().unwrap().display()
    );
    assert!(output.contains(&expected), "output: {output:?}");
    assert!(std::env::var_os(&dropped).is_some());
    assert!(std::env::var_os("EXAMPLE_CLIENT_VAR").is_none());
}

#[test]
fn connection_without_hello_is_closed_after_the_timeout() {
    let daemon = TestDaemon::start_with(|config| {
        config.hello_timeout = Duration::from_millis(100);
    });
    let mut stream = UnixStream::connect(&daemon.paths.socket).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    assert!(read_to_eof(&mut stream).is_empty());
    assert!(daemon.client().is_alive());
}

#[test]
fn connections_above_the_limit_are_refused() {
    let daemon = TestDaemon::start_with(|config| {
        config.max_connections = 2;
    });
    let first = UnixStream::connect(&daemon.paths.socket).unwrap();
    let second = UnixStream::connect(&daemon.paths.socket).unwrap();
    let mut third = UnixStream::connect(&daemon.paths.socket).unwrap();
    third.set_read_timeout(Some(TIMEOUT)).unwrap();
    assert!(read_to_eof(&mut third).is_empty());

    drop((first, second));
    wait_until(|| {
        DaemonClient::connect_at(&daemon.paths.socket)
            .unwrap()
            .is_some()
    });
}

#[test]
fn oversized_client_frame_closes_the_connection() {
    let daemon = TestDaemon::start();
    let mut stream = UnixStream::connect(&daemon.paths.socket).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    stream
        .write_all(&(protocol::MAX_CLIENT_FRAME_SIZE + 1).to_be_bytes())
        .unwrap();
    assert!(read_to_eof(&mut stream).is_empty());
}

#[test]
fn kill_does_not_wait_for_a_process_that_ignores_sigterm() {
    let daemon = TestDaemon::start();
    let client = daemon.client();
    let mut spawned = client
        .spawn(
            session_id(21),
            metadata(),
            sh_options("trap '' TERM; read x"),
        )
        .unwrap();
    let pid = spawned.info.pid;
    spawned.handle.kill().unwrap();
    wait_until(|| !process_exists(pid));
    assert!(client.list().unwrap().is_empty());
}

#[test]
fn private_dir_is_created_with_mode_0700() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::in_dir(dir.path().join("nested").join("ts"));
    assert!(!paths.validate_private_dir().unwrap());
    paths.ensure_private_dir().unwrap();
    let mode = std::fs::metadata(&paths.dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert!(paths.validate_private_dir().unwrap());
}

#[test]
fn private_dir_with_loose_permissions_fails_validation_and_is_tightened() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DaemonPaths::in_dir(dir.path().join("ts"));
    std::fs::create_dir(&paths.dir).unwrap();
    std::fs::set_permissions(&paths.dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(paths.validate_private_dir().is_err());
    paths.ensure_private_dir().unwrap();
    assert!(paths.validate_private_dir().unwrap());
}

#[test]
fn symlinked_private_dir_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = DaemonPaths::in_dir(dir.path().join("ts"));
    std::os::unix::fs::symlink(&target, &paths.dir).unwrap();
    assert!(paths.validate_private_dir().is_err());
    assert!(paths.ensure_private_dir().is_err());
}

#[test]
fn fallback_dir_prefers_the_runtime_dir() {
    use super::listener::{fallback_dir, state_dir_key};
    use std::path::{Path, PathBuf};
    let state_dir = Path::new("/home/alice/.local/state/warp-terminal");
    let key = state_dir_key(state_dir);
    assert_eq!(
        fallback_dir(
            Some("/run/user/501".into()),
            Some("/var/tmp".into()),
            501,
            state_dir
        ),
        PathBuf::from(format!("/run/user/501/warp-ts-{key}"))
    );
    assert_eq!(
        fallback_dir(
            Some("relative".into()),
            Some("/var/tmp".into()),
            501,
            state_dir
        ),
        PathBuf::from(format!("/var/tmp/warp-ts-501-{key}"))
    );
    assert_eq!(
        fallback_dir(None, None, 501, state_dir),
        PathBuf::from(format!("/tmp/warp-ts-501-{key}"))
    );
}

#[test]
fn state_dir_key_is_stable_short_and_distinct_per_state_dir() {
    use super::listener::state_dir_key;
    use std::path::Path;
    let stable = Path::new("/home/alice/.local/state/warp-terminal");
    let preview = Path::new("/home/alice/.local/state/warp-terminal-preview");
    let other_home = Path::new("/home/bob/.local/state/warp-terminal");
    assert_eq!(state_dir_key(Path::new("")), "9ce484222325");
    assert_eq!(state_dir_key(Path::new("a")), "dc4c8601ec8c");
    assert_eq!(state_dir_key(stable), state_dir_key(stable));
    assert_eq!(state_dir_key(stable).len(), 12);
    assert!(state_dir_key(stable).chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(state_dir_key(stable), state_dir_key(preview));
    assert_ne!(state_dir_key(stable), state_dir_key(other_home));
}

#[test]
fn short_state_dirs_keep_their_own_socket_directory() {
    use std::path::{Path, PathBuf};
    let paths = DaemonPaths::for_state_dir(
        Path::new("/home/alice/.local/state/warp-terminal"),
        None,
        None,
        501,
    );
    assert_eq!(
        paths.dir,
        PathBuf::from("/home/alice/.local/state/warp-terminal/ts")
    );
}

#[test]
fn long_state_dirs_fall_back_to_distinct_directories() {
    use std::path::Path;
    let long_home = format!("/home/{}", "a".repeat(90));
    let first = Path::new(&long_home).join("state").join("warp-terminal");
    let second = Path::new(&long_home)
        .join("state")
        .join("warp-terminal-preview");
    let first_paths = DaemonPaths::for_state_dir(&first, None, None, 501);
    let second_paths = DaemonPaths::for_state_dir(&second, None, None, 501);
    assert!(first_paths.dir.starts_with("/tmp"));
    assert!(second_paths.dir.starts_with("/tmp"));
    assert_ne!(first_paths.dir, second_paths.dir);
    assert_ne!(first_paths.socket, second_paths.socket);
    assert!(first_paths.socket.as_os_str().len() <= 100);
}

#[test]
fn peer_uid_mismatch_is_rejected() {
    let (ours, _theirs) = UnixStream::pair().unwrap();
    let uid = nix::unistd::geteuid().as_raw();
    check_peer_uid(ours.as_raw_fd(), uid).unwrap();
    assert!(check_peer_uid(ours.as_raw_fd(), uid.wrapping_add(1)).is_err());
}

#[test]
fn log_is_rotated_once_it_exceeds_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.log");
    std::fs::write(&path, b"0123456789").unwrap();
    let target = OpenOptions::new().append(true).open(&path).unwrap();

    assert!(!rotate_log(&path, 10, &[target.as_raw_fd()]).unwrap());
    std::fs::write(&path, b"0123456789AB").unwrap();
    assert!(rotate_log(&path, 10, &[target.as_raw_fd()]).unwrap());

    (&target).write_all(b"fresh").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"fresh");
    let mut old = path.clone().into_os_string();
    old.push(".old");
    assert_eq!(std::fs::read(old).unwrap(), b"0123456789AB");
}
