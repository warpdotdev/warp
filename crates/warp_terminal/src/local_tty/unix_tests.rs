use super::*;

fn shell_starter(shell_type: ShellType, shell_path: &str) -> DirectShellStarter {
    DirectShellStarter::new_for_test(shell_type, PathBuf::from(shell_path), Vec::new())
}

fn env_value(command: &Command, key: &str) -> Option<Option<String>> {
    command
        .get_envs()
        .find(|(env_key, _)| *env_key == std::ffi::OsStr::new(key))
        .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
}

#[test]
fn host_bash_command_sets_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Bash, "/bin/bash"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
        None,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

#[test]
fn host_non_bash_command_does_not_set_history_size_sentinels() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
        None,
    );

    assert_eq!(env_value(&command, "HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "HISTSIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTFILESIZE"), None);
    assert_eq!(env_value(&command, "WARP_INITIAL_HISTSIZE"), None);
}

#[test]
fn docker_sandbox_command_sets_history_size_sentinels() {
    let docker_starter =
        DockerSandboxShellStarter::new(shell_starter(ShellType::Bash, "sbx"), None);
    let command = build_docker_sandbox_command(
        &docker_starter,
        None,
        HashMap::new(),
        false,
        false,
        false,
        false,
        true,
        None,
    );

    assert_eq!(
        env_value(&command, "HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTFILESIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
    assert_eq!(
        env_value(&command, "WARP_INITIAL_HISTSIZE"),
        Some(Some(BASH_HISTORY_SIZE_SENTINEL.to_owned()))
    );
}

struct FakeDaemonHandle {
    data_peer: std::os::unix::net::UnixStream,
    saw_closed_data: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl PtyHandle for FakeDaemonHandle {
    fn pid(&self) -> u32 {
        4242
    }

    fn process_termination_status(&mut self) -> Result<Option<crate::event::ObservedExitStatus>> {
        Ok(None)
    }

    fn kill(&mut self) -> Result<()> {
        let mut buf = [0u8; 1];
        let closed = matches!(io::Read::read(&mut self.data_peer, &mut buf), Ok(0));
        self.saw_closed_data
            .store(closed, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

struct AttachedPtyFixture {
    pty: Pty,
    follower: File,
    peer: std::os::unix::net::UnixStream,
    saw_closed_data: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

fn attached_pty(replay: &[u8], size: &SizeInfo) -> AttachedPtyFixture {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    let ends = openpty(None, None).unwrap();
    let leader = unsafe { OwnedFd::from_raw_fd(ends.master) };
    let follower = unsafe { File::from_raw_fd(ends.slave) };
    let (data, mut peer) = UnixStream::pair().unwrap();
    io::Write::write_all(&mut peer, replay).unwrap();
    let saw_closed_data = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let session = AttachedSession {
        info: AttachInfo {
            session_id: PersistentSessionId([1; 16]),
            pid: 4242,
            metadata: SessionMetadata {
                hook_session_id: 7,
                shell_type: "zsh".to_owned(),
            },
            replay_len: replay.len() as u64,
            has_bootstrap_preamble: true,
            exit_status: None,
        },
        data: OwnedFd::from(data),
        leader: Some(leader),
        handle: Box::new(FakeDaemonHandle {
            data_peer: {
                let data_peer = peer.try_clone().unwrap();
                data_peer.set_nonblocking(true).unwrap();
                data_peer
            },
            saw_closed_data: saw_closed_data.clone(),
        }),
    };
    AttachedPtyFixture {
        pty: Pty::from_attached(session, size).unwrap(),
        follower,
        peer,
        saw_closed_data,
    }
}

fn window_size(file: &File) -> (u16, u16) {
    let mut win: winsize = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCGWINSZ, &mut win as *mut _) };
    assert_eq!(res, 0);
    (win.ws_row, win.ws_col)
}

#[test]
fn attached_pty_resizes_through_the_leader() {
    let fixture = attached_pty(b"", &SizeInfo::new_without_font_metrics(24, 80));
    let mut pty = fixture.pty;
    assert_eq!(window_size(&fixture.follower), (24, 80));
    assert_ne!(pty.get_fd(), pty.fd.as_raw_fd());

    pty.on_resize(&SizeInfo::new_without_font_metrics(30, 100));

    assert_eq!(window_size(&fixture.follower), (30, 100));
    assert!(pty.take_replay().is_none());
}

#[test]
fn attached_pty_reads_replay_once() {
    let replay = b"\x1bP$d7b7d\x1b\\tick 1\r\n";
    let mut fixture = attached_pty(replay, &SizeInfo::new_without_font_metrics(24, 80));
    io::Write::write_all(&mut fixture.peer, b"live").unwrap();

    assert_eq!(fixture.pty.take_replay().as_deref(), Some(&replay[..]));
    assert!(fixture.pty.take_replay().is_none());
    assert_eq!(window_size(&fixture.follower), (24, 80));

    let mut live = [0u8; 4];
    io::Read::read_exact(&mut fixture.pty.fd, &mut live).unwrap();
    assert_eq!(&live, b"live");
}

#[test]
fn attached_pty_without_leader_ignores_resize() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    let (data, peer) = UnixStream::pair().unwrap();
    let session = AttachedSession {
        info: AttachInfo {
            session_id: PersistentSessionId([2; 16]),
            pid: 4242,
            metadata: SessionMetadata {
                hook_session_id: 7,
                shell_type: "zsh".to_owned(),
            },
            replay_len: 0,
            has_bootstrap_preamble: true,
            exit_status: Some(crate::event::ObservedExitStatus::Code(0)),
        },
        data: OwnedFd::from(data),
        leader: None,
        handle: Box::new(FakeDaemonHandle {
            data_peer: peer,
            saw_closed_data: Default::default(),
        }),
    };
    let mut pty = Pty::from_attached(session, &SizeInfo::new_without_font_metrics(24, 80)).unwrap();

    pty.on_resize(&SizeInfo::new_without_font_metrics(30, 100));

    assert_eq!(pty.get_fd(), pty.fd.as_raw_fd());
}

#[test]
fn attached_pty_kill_closes_fds_before_killing() {
    let fixture = attached_pty(b"", &SizeInfo::new_without_font_metrics(24, 80));

    fixture.pty.kill().unwrap();

    assert!(
        fixture
            .saw_closed_data
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[test]
fn attached_pty_drains_output_until_eof_after_exit() {
    let mut fixture = attached_pty(b"", &SizeInfo::new_without_font_metrics(24, 80));
    io::Write::write_all(&mut fixture.peer, b"last words\r\n").unwrap();
    fixture.peer.shutdown(std::net::Shutdown::Write).unwrap();

    assert_eq!(
        fixture.pty.drain_after_exit().as_deref(),
        Some(&b"last words\r\n"[..])
    );
}

#[test]
fn attached_pty_drain_gives_up_without_eof() {
    let mut fixture = attached_pty(b"", &SizeInfo::new_without_font_metrics(24, 80));
    io::Write::write_all(&mut fixture.peer, b"partial").unwrap();

    let started = Instant::now();
    let drained = fixture.pty.drain_after_exit();

    assert_eq!(drained.as_deref(), Some(&b"partial"[..]));
    assert!(started.elapsed() < EXIT_DRAIN_TIMEOUT * 3);
}

#[test]
fn close_fds_limit_falls_back_and_is_capped() {
    assert_eq!(close_fds_limit(None), CLOSE_FDS_FALLBACK_LIMIT);
    assert_eq!(close_fds_limit(Some(256)), 256);
    assert_eq!(close_fds_limit(Some(u64::MAX)), CLOSE_FDS_MAX_LIMIT);
    assert_eq!(close_fds_limit(Some(0)), 3);
}

#[test]
fn close_fds_upper_bound_is_sane() {
    let limit = close_fds_upper_bound();
    assert!((3..=CLOSE_FDS_MAX_LIMIT).contains(&limit));
}

#[test]
fn base_env_replaces_the_inherited_environment() {
    let base_env = vec![
        ("HOME".into(), "/tmp/example-home".into()),
        ("EXAMPLE_BASE".into(), "from-base".into()),
        ("EXAMPLE_OVERRIDDEN".into(), "from-base".into()),
        ("EXAMPLE=INVALID".into(), "skipped".into()),
    ];
    let env_vars = HashMap::from([("EXAMPLE_OVERRIDDEN".into(), "from-env-vars".into())]);
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        env_vars,
        None,
        false,
        false,
        false,
        false,
        true,
        Some(base_env),
    );

    assert_eq!(
        env_value(&command, "HOME"),
        Some(Some("/tmp/example-home".to_owned()))
    );
    assert_eq!(
        env_value(&command, "EXAMPLE_BASE"),
        Some(Some("from-base".to_owned()))
    );
    assert_eq!(
        env_value(&command, "EXAMPLE_OVERRIDDEN"),
        Some(Some("from-env-vars".to_owned()))
    );
    assert_eq!(env_value(&command, "EXAMPLE=INVALID"), None);
    assert_eq!(
        env_value(&command, "TERM_PROGRAM"),
        Some(Some("WarpTerminal".to_owned()))
    );
}

#[test]
fn without_base_env_the_environment_is_inherited() {
    let command = build_host_shell_command(
        shell_starter(ShellType::Zsh, "/bin/zsh"),
        None,
        HashMap::new(),
        None,
        false,
        false,
        false,
        false,
        true,
        None,
    );

    assert_eq!(env_value(&command, "EXAMPLE_BASE"), None);
    assert_eq!(
        env_value(&command, "HOME").flatten(),
        std::env::var("HOME").ok()
    );
}

#[test]
fn spawned_shell_gets_the_base_env_and_child_rlimits() {
    let mut nofile = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `nofile` is valid for writes.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut nofile) };
    let wanted_nofile: u64 = 200.min(nofile.rlim_max);
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    let home = std::env::temp_dir();
    let mut options = crate::local_tty::server::test_util::sh_options(
        "printf 'base=%s;inherited=%s;nofile=%s;core=%s;\\n' \"$EXAMPLE_BASE\" \"${CARGO_MANIFEST_DIR-unset}\" \"$(ulimit -n)\" \"$(ulimit -c)\"",
    );
    options.base_env = Some(vec![
        ("HOME".into(), home.into_os_string()),
        ("EXAMPLE_BASE".into(), "from-base".into()),
    ]);
    options.child_rlimits = Some(ChildResourceLimits {
        nofile_soft: wanted_nofile,
        core_soft: 0,
    });
    let info = spawn(options).unwrap();
    let mut leader = unsafe { File::from_raw_fd(info.result.leader_fd) };
    let mut output = Vec::new();
    let mut buf = [0u8; 1024];
    let deadline = Instant::now() + Duration::from_secs(10);
    while !output.ends_with(b";\r\n") && Instant::now() < deadline {
        if !wait_readable(leader.as_raw_fd(), Duration::from_millis(100)) {
            continue;
        }
        match io::Read::read(&mut leader, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
        }
    }
    let mut child = info.child;
    let _ = child.wait();
    let output = String::from_utf8_lossy(&output).into_owned();

    assert!(output.contains("base=from-base;"), "{output}");
    assert!(output.contains("inherited=unset;"), "{output}");
    assert!(
        output.contains(&format!("nofile={wanted_nofile};")),
        "{output}"
    );
    assert!(output.contains("core=0;"), "{output}");
}
