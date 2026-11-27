//! End-to-end tests that drive the real app binary as the terminal daemon.
//!
//! They are skipped unless `WARP_DAEMON_E2E_BIN` points at a built binary:
//!
//! ```sh
//! cargo build --bin warp-oss --features gui
//! WARP_DAEMON_E2E_BIN=$PWD/target/debug/warp-oss \
//!   cargo test -p warp_terminal daemon_e2e -- --test-threads=1 --nocapture
//! ```
//!
//! Each test runs the daemon with `HOME` and `TMPDIR` pointing at a throwaway
//! directory, so a real Warp installation is never touched.

use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use command::blocking::Command;
use instant::Instant;

use super::*;
use crate::local_tty::server::test_util::sh_options;
use crate::local_tty::server::{RING_CAPACITY, protocol};

const BIN_ENV: &str = "WARP_DAEMON_E2E_BIN";
const ROLE_ENV: &str = "WARP_DAEMON_E2E_ROLE";
const SOCKET_ENV: &str = "WARP_DAEMON_E2E_SOCKET";
const READY_ENV: &str = "WARP_DAEMON_E2E_READY";
const COUNTER_SCRIPT: &str = "i=0; while true; do echo $i; i=$((i+1)); sleep 0.2; done";
const TIMEOUT: Duration = Duration::from_secs(30);

static SERIAL: Mutex<()> = Mutex::new(());

fn binary() -> Option<PathBuf> {
    std::env::var_os(BIN_ENV).map(PathBuf::from)
}

macro_rules! e2e {
    ($env:ident) => {
        let Some(bin) = binary() else {
            eprintln!("skipping: {} is not set", BIN_ENV);
            return;
        };
        let _serial = SERIAL.lock().unwrap_or_else(|err| err.into_inner());
        let mut $env = E2eEnv::new(bin);
    };
}

struct E2eEnv {
    bin: PathBuf,
    home: PathBuf,
    daemon_pids: Vec<u32>,
    session_pids: Vec<u32>,
}

impl E2eEnv {
    fn new(bin: PathBuf) -> Self {
        let home = PathBuf::from("/tmp").join(format!(
            "wde2e-{}-{}",
            std::process::id(),
            rand::random::<u16>()
        ));
        std::fs::create_dir_all(home.join("tmp")).unwrap();
        Self {
            bin,
            home,
            daemon_pids: Vec::new(),
            session_pids: Vec::new(),
        }
    }

    fn paths(&self) -> DaemonPaths {
        let state = if cfg!(target_os = "macos") {
            self.home
                .join("Library/Application Support/dev.warp.WarpOss")
        } else {
            self.home.join(".local/state/warp-oss")
        };
        DaemonPaths::in_dir(state.join("ts"))
    }

    fn command(&self, daemon_args: bool) -> Command {
        let mut command = Command::new(&self.bin);
        command
            .env_clear()
            .env("HOME", &self.home)
            .env("TMPDIR", self.home.join("tmp"))
            .env("PATH", "/usr/bin:/bin")
            .arg(warp_cli::terminal_server_subcommand());
        if daemon_args {
            command.arg("--daemon");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn start_daemon(&mut self, extra_env: &[(&str, &str)]) -> u32 {
        let mut command = self.command(true);
        command.envs(extra_env.iter().copied());
        let started = Instant::now();
        let status = command.status().unwrap();
        assert!(status.success(), "launcher failed: {status}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the launcher should return immediately, took {:?}",
            started.elapsed()
        );
        let pid = self.daemon_pid();
        self.daemon_pids.push(pid);
        pid
    }

    fn daemon_pid(&self) -> u32 {
        let log = self.paths().log;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(contents) = std::fs::read_to_string(&log)
                && let Some(pid) = contents
                    .lines()
                    .filter_map(|line| line.rsplit_once("running with pid "))
                    .filter_map(|(_, pid)| pid.trim().parse().ok())
                    .next_back()
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "daemon never logged its pid");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn client(&self) -> Arc<DaemonClient> {
        let socket = self.paths().socket;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(client) = DaemonClient::connect_at(&socket).unwrap() {
                return client;
            }
            assert!(Instant::now() < deadline, "daemon is not listening");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn track(&mut self, session: &AttachedSession) {
        self.session_pids.push(session.info.pid);
    }
}

impl Drop for E2eEnv {
    fn drop(&mut self) {
        for pid in self.session_pids.iter().chain(&self.daemon_pids) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(*pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn session_id(n: u128) -> PersistentSessionId {
    PersistentSessionId::from(uuid::Uuid::from_u128(n))
}

fn metadata() -> SessionMetadata {
    SessionMetadata {
        hook_session_id: 7,
        shell_type: "bash".to_owned(),
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ps_field(pid: u32, field: &str) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .unwrap();
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !value.is_empty()).then_some(value)
}

fn is_running(pid: u32) -> bool {
    ps_field(pid, "stat").is_some_and(|stat| !stat.starts_with('Z'))
}

fn stream(fd: OwnedFd) -> UnixStream {
    let stream = UnixStream::from(fd);
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    stream
}

fn read_exact_len(stream: &mut UnixStream, len: u64) -> Vec<u8> {
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).unwrap();
    buf
}

fn read_until_len(stream: &mut UnixStream, min_len: usize) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buf = [0u8; 4096];
    while received.len() < min_len {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "unexpected EOF");
        received.extend_from_slice(&buf[..n]);
    }
    received
}

fn counter_values(bytes: &[u8]) -> Vec<u64> {
    let text = String::from_utf8_lossy(bytes);
    let complete = text.rsplit_once('\n').map(|(head, _)| head).unwrap_or("");
    complete
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn assert_consecutive(values: &[u64]) {
    for pair in values.windows(2) {
        assert_eq!(pair[1], pair[0] + 1, "counter gap in {values:?}");
    }
}

#[test]
fn e2e_helper_client() {
    if std::env::var(ROLE_ENV).as_deref() != Ok("client") {
        return;
    }
    let socket = PathBuf::from(std::env::var_os(SOCKET_ENV).unwrap());
    let ready = PathBuf::from(std::env::var_os(READY_ENV).unwrap());
    let client = DaemonClient::connect_at(&socket).unwrap().unwrap();
    let session = client
        .spawn(session_id(2), metadata(), sh_options(COUNTER_SCRIPT))
        .unwrap();
    std::fs::write(&ready, session.info.pid.to_string()).unwrap();
    std::thread::sleep(Duration::from_secs(300));
}

#[test]
fn daemon_e2e_daemonizes_and_is_private() {
    e2e!(env);
    let pid = env.start_daemon(&[]);

    let ppid: u32 = ps_field(pid, "ppid").unwrap().parse().unwrap();
    let pgid: u32 = ps_field(pid, "pgid").unwrap().parse().unwrap();
    let sid = unsafe { libc::getsid(pid as i32) } as u32;
    let own_sid = unsafe { libc::getsid(0) } as u32;
    eprintln!("daemon pid={pid} ppid={ppid} pgid={pgid} sid={sid} own_sid={own_sid}");
    assert_eq!(ppid, 1, "the daemon should be reparented to init");
    assert_ne!(sid, own_sid, "the daemon should be in its own session");
    assert_eq!(
        sid, pgid,
        "the daemon should sit in the session's first group"
    );
    assert_ne!(
        sid, pid,
        "a double fork leaves the daemon without a controlling tty"
    );

    let paths = env.paths();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&paths.dir), 0o700);
    assert_eq!(mode(&paths.socket), 0o600);
    assert_eq!(mode(&paths.lock), 0o600);

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGHUP,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(is_running(pid), "SIGHUP must be ignored");
    assert!(env.client().list().unwrap().is_empty());

    let second = env.command(true).status().unwrap();
    assert!(second.success());
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        env.daemon_pid(),
        pid,
        "no second daemon should have logged a pid"
    );
    let daemons = Command::new("pgrep")
        .args(["-f", &format!("{} .*--daemon", env.bin.display())])
        .output()
        .unwrap();
    let running: Vec<u32> = String::from_utf8_lossy(&daemons.stdout)
        .lines()
        .filter_map(|line| line.parse().ok())
        .collect();
    assert_eq!(
        running,
        vec![pid],
        "the second daemon should exit on the lock"
    );
}

#[test]
fn daemon_e2e_session_survives_client_exit_and_crash() {
    e2e!(env);
    env.start_daemon(&[]);

    let client = env.client();
    let session = client
        .spawn(session_id(1), metadata(), sh_options(COUNTER_SCRIPT))
        .unwrap();
    env.track(&session);
    let shell_pid = session.info.pid;
    let mut data = stream(session.data.try_clone().unwrap());
    read_until_len(&mut data, 4);

    client.detach_all();
    drop(data);
    drop(session);
    drop(client);
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        is_running(shell_pid),
        "shell must survive a graceful detach"
    );

    let client = env.client();
    let listed = client.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].attached);
    let session = client.attach(session_id(1)).unwrap();
    let mut data = stream(session.data.try_clone().unwrap());
    let replay = read_exact_len(&mut data, session.info.replay_len);
    let values = counter_values(&replay);
    eprintln!(
        "graceful: replay {} bytes, values {:?}",
        replay.len(),
        values
    );
    assert!(
        values.len() >= 12,
        "replay should hold the output produced while detached"
    );
    assert_eq!(values[0], 0);
    assert_consecutive(&values);
    let live = read_until_len(&mut data, 8);
    let live_values = counter_values(&live);
    assert!(!live_values.is_empty());
    assert_eq!(live_values[0], values.last().unwrap() + 1);
    assert_consecutive(&live_values);

    drop(data);
    drop(session);
    client.list().unwrap();
    session_kill(&client, session_id(1));
    drop(client);

    let ready = env.home.join("ready");
    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args(["e2e_helper_client", "--nocapture", "--test-threads=1"])
        .env(ROLE_ENV, "client")
        .env(SOCKET_ENV, env.paths().socket)
        .env(READY_ENV, &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("helper client to spawn a session", || ready.exists());
    let crashed_pid: u32 = std::fs::read_to_string(&ready).unwrap().parse().unwrap();
    env.session_pids.push(crashed_pid);
    helper.kill().unwrap();
    helper.wait().unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert!(is_running(crashed_pid), "shell must survive a client crash");

    let client = env.client();
    let listed = client.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert!(
        !listed[0].attached,
        "crashed client's session should be detached"
    );
    let session = client.attach(session_id(2)).unwrap();
    let mut data = stream(session.data.try_clone().unwrap());
    let replay = read_exact_len(&mut data, session.info.replay_len);
    let values = counter_values(&replay);
    eprintln!("crash: replay {} bytes, values {:?}", replay.len(), values);
    assert!(values.len() >= 12);
    assert_eq!(values[0], 0);
    assert_consecutive(&values);
    let live = read_until_len(&mut data, 8);
    let live_values = counter_values(&live);
    assert_eq!(live_values[0], values.last().unwrap() + 1);
}

fn session_kill(client: &DaemonClient, id: PersistentSessionId) {
    let session = client.attach(id).unwrap();
    let pid = session.info.pid;
    let mut handle = session.handle;
    handle.kill().unwrap();
    wait_until("killed shell to exit", || !is_running(pid));
}

#[test]
fn daemon_e2e_heavy_output_while_detached_does_not_block() {
    e2e!(env);
    env.start_daemon(&[]);
    let client = env.client();
    let started = Instant::now();
    let session = client
        .spawn(
            session_id(3),
            metadata(),
            sh_options("head -c 50000000 /dev/zero | tr '\\0' y; echo done"),
        )
        .unwrap();
    env.track(&session);
    let pid = session.info.pid;
    client.detach_all();
    drop(session);
    drop(client);

    let client = env.client();
    wait_until("the shell to finish while detached", || {
        client
            .list()
            .unwrap()
            .iter()
            .any(|s| s.session_id == session_id(3) && s.exit_status.is_some())
    });
    eprintln!("50 MB drained in {:?}", started.elapsed());
    assert!(!is_running(pid));
    let session = client.attach(session_id(3)).unwrap();
    let mut data = stream(session.data.try_clone().unwrap());
    let replay = read_exact_len(&mut data, session.info.replay_len);
    assert!(replay.len() <= RING_CAPACITY + 64 * 1024);
    assert!(
        replay.windows(4).any(|w| w == b"done"),
        "replay should end with done"
    );
}

#[test]
fn daemon_e2e_kill_prune_and_idle_exit() {
    e2e!(env);
    let daemon = env.start_daemon(&[("WARP_TERMINAL_DAEMON_IDLE_SECS", "2")]);
    let client = env.client();
    let a = client
        .spawn(session_id(10), metadata(), sh_options(COUNTER_SCRIPT))
        .unwrap();
    let b = client
        .spawn(session_id(11), metadata(), sh_options(COUNTER_SCRIPT))
        .unwrap();
    let c = client
        .spawn(session_id(12), metadata(), sh_options(COUNTER_SCRIPT))
        .unwrap();
    for session in [&a, &b, &c] {
        env.track(session);
    }
    let (pid_a, pid_b, pid_c) = (a.info.pid, b.info.pid, c.info.pid);

    let mut handle = c.handle;
    handle.kill().unwrap();
    wait_until("killed shell to exit", || !is_running(pid_c));
    assert!(is_running(pid_a) && is_running(pid_b));
    assert_eq!(client.list().unwrap().len(), 2);

    client.detach_all();
    drop((a.data, a.leader, b.data, b.leader));
    drop(client);
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        is_running(daemon),
        "a daemon with sessions must not idle out"
    );
    assert!(is_running(pid_a) && is_running(pid_b));

    let client = env.client();
    client.prune(vec![session_id(10)]).unwrap();
    wait_until("pruned shell to exit", || !is_running(pid_b));
    assert!(is_running(pid_a));
    client.prune(vec![]).unwrap();
    wait_until("pruned shell to exit", || !is_running(pid_a));
    assert!(client.list().unwrap().is_empty());

    drop(client);
    let socket = env.paths().socket;
    wait_until("idle daemon to exit", || !is_running(daemon));
    assert!(
        !socket.exists(),
        "the socket should be removed on idle exit"
    );
}

#[test]
fn daemon_e2e_rejects_another_protocol_version_and_stays_up() {
    e2e!(env);
    let daemon = env.start_daemon(&[]);
    let client = env.client();
    let session = client
        .spawn(session_id(20), metadata(), sh_options(COUNTER_SCRIPT))
        .unwrap();
    env.track(&session);
    client.detach_all();
    drop(session);
    drop(client);

    let raw = UnixStream::connect(env.paths().socket).unwrap();
    raw.set_read_timeout(Some(TIMEOUT)).unwrap();
    protocol::send_frame(
        raw.as_raw_fd(),
        &ClientMessage::Hello {
            protocol_version: DAEMON_PROTOCOL_VERSION + 1,
            client_pid: std::process::id(),
            app_version: None,
        },
        &[],
    )
    .unwrap();
    let reply = protocol::receive_frame::<DaemonMessage>(raw.as_raw_fd()).unwrap();
    match reply {
        Some((
            DaemonMessage::HelloReject {
                protocol_version,
                reason,
            },
            _,
        )) => {
            eprintln!("HelloReject {{ protocol_version: {protocol_version}, reason: {reason:?} }}");
            assert_eq!(protocol_version, DAEMON_PROTOCOL_VERSION);
        }
        other => panic!("expected HelloReject, got {other:?}"),
    }
    drop(raw);

    assert!(is_running(daemon));
    let client = env.client();
    let listed = client.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert!(
        is_running(listed[0].pid),
        "the old session must be untouched"
    );
}

#[test]
fn daemon_e2e_connect_or_spawn_starts_the_real_binary() {
    e2e!(env);
    let paths = env.paths();
    paths.ensure_private_dir().unwrap();
    let before = std::env::var_os("HOME");
    let tmp_before = std::env::var_os("TMPDIR");
    // SAFETY: serialized by SERIAL; no other test thread reads the environment.
    unsafe {
        std::env::set_var("HOME", &env.home);
        std::env::set_var("TMPDIR", env.home.join("tmp"));
    }
    let bin = env.bin.clone();
    let client = DaemonClient::connect_or_spawn_with(&paths.socket, || spawn_daemon_command(&bin));
    unsafe {
        match before {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match tmp_before {
            Some(value) => std::env::set_var("TMPDIR", value),
            None => std::env::remove_var("TMPDIR"),
        }
    }
    let client = client.unwrap();
    env.daemon_pids.push(env.daemon_pid());
    assert!(client.list().unwrap().is_empty());
}

#[test]
fn daemon_e2e_shell_gets_the_client_environment_and_limits() {
    e2e!(env);
    let mut command = env.command(true);
    command.env("EXAMPLE_STALE", "from-daemon-launch");
    // SAFETY: the closure only calls async-signal-safe functions.
    unsafe {
        command.pre_exec(|| {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit);
            limit.rlim_cur = 256.min(limit.rlim_max);
            libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
            libc::getrlimit(libc::RLIMIT_CORE, &mut limit);
            limit.rlim_cur = limit.rlim_max;
            libc::setrlimit(libc::RLIMIT_CORE, &limit);
            Ok(())
        });
    }
    assert!(command.status().unwrap().success());
    let daemon = env.daemon_pid();
    env.daemon_pids.push(daemon);
    let client = env.client();

    let script = "printf 'stale=%s;client=%s;core=%s;nofile=%s;cwd=%s;\\n' \
                  \"${EXAMPLE_STALE-unset}\" \"$EXAMPLE_CLIENT_VAR\" \
                  \"$(ulimit -c)\" \"$(ulimit -n)\" \"$(pwd -P)\"; read x";
    let environment = SpawnEnvironment {
        vars: vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), env.home.clone().into_os_string()),
            ("EXAMPLE_CLIENT_VAR".into(), "from-client".into()),
        ],
    };
    let session = client
        .spawn_with_environment(session_id(30), metadata(), sh_options(script), environment)
        .unwrap();
    env.track(&session);
    let mut data = stream(session.data.try_clone().unwrap());
    let mut output = Vec::new();
    let mut buf = [0u8; 1024];
    while !output.ends_with(b";\r\n") {
        let n = data.read(&mut buf).unwrap();
        assert!(n > 0, "unexpected EOF");
        output.extend_from_slice(&buf[..n]);
    }
    let output = String::from_utf8_lossy(&output).into_owned();

    let core_hard = {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is valid for writes.
        unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) };
        limit.rlim_max
    };
    assert!(
        output.contains("stale=unset;client=from-client;"),
        "{output}"
    );
    assert!(output.contains("nofile=256;"), "{output}");
    if core_hard == libc::RLIM_INFINITY {
        assert!(output.contains("core=unlimited;"), "{output}");
    } else if core_hard > 0 {
        assert!(!output.contains("core=0;"), "{output}");
    }
    let home = env.home.canonicalize().unwrap();
    assert!(
        output.contains(&format!("cwd={};", home.display())),
        "{output}"
    );
    assert!(is_running(daemon));
}
