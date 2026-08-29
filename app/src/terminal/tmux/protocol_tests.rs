use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use command::blocking::Command;
use instant::Instant;
use warp_core::SessionId;
use warp_terminal::shell::ShellType;

use super::{
    PaneBootstrap, capture_pane_command, cleanup_unspawned_dedicated_files, control_client_argv,
    detach_client_command, in_band_init_bytes, in_place_pane_spawn_argv, kill_pane_command,
    kill_server_argv, kill_server_command, kill_window_command, new_window_command,
    pane_bootstrap_for_shell, pipe_pane_journal_command, refresh_client_command,
    register_dedicated_server, resize_pane_command, schedule_kill_dedicated_server,
    select_pane_command, select_window_command, send_keys_commands, silent_bootstrap_bytes,
    split_stage_complete, split_window_command, zsh_init_bytes,
};
use crate::terminal::tmux::parser::{PaneId, WindowId};

#[test]
fn refresh_client_uses_columns_x_rows() {
    assert_eq!(refresh_client_command(80, 24), "refresh-client -C 80x24\n");
}

#[test]
fn split_window_encodes_orientation_and_prints_pane_id() {
    let pane = PaneId::from("%0");
    assert_eq!(
        split_window_command(&pane, true),
        "split-window -h -t %0 -P -F '#{pane_id}'\n"
    );
    assert_eq!(
        split_window_command(&pane, false),
        "split-window -v -t %0 -P -F '#{pane_id}'\n"
    );
}

#[test]
fn pane_and_window_commands_target_tmux_ids() {
    let pane = PaneId::from("%1");
    let window = WindowId::from("@2");
    assert_eq!(select_pane_command(&pane), "select-pane -t %1\n");
    assert_eq!(kill_pane_command(&pane), "kill-pane -t %1\n");
    assert_eq!(
        resize_pane_command(&pane, 40, 12),
        "resize-pane -t %1 -x 40 -y 12\n"
    );
    assert_eq!(new_window_command(), "new-window -P -F '#{window_id}'\n");
    assert_eq!(select_window_command(&window), "select-window -t @2\n");
    assert_eq!(kill_window_command(&window), "kill-window -t @2\n");
    assert_eq!(detach_client_command(), "detach-client\n");
    assert_ne!(detach_client_command(), kill_server_command());
    assert_eq!(
        pipe_pane_journal_command(&pane, "/tmp/warp-%1.journal"),
        "pipe-pane -t %1 -O 'cat >> /tmp/warp-%1.journal'\n"
    );
    assert_eq!(capture_pane_command(&pane), "capture-pane -p -t %1\n");
}

#[test]
fn kill_server_argv_targets_dedicated_socket() {
    assert_eq!(kill_server_command(), "kill-server\n");
    let argv = kill_server_argv(
        PathBuf::from("/usr/bin/tmux").as_path(),
        PathBuf::from("/tmp/warp.sock").as_path(),
    );
    let argv: Vec<String> = argv
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        vec!["/usr/bin/tmux", "-S", "/tmp/warp.sock", "kill-server"]
    );
}

static TEST_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        TEST_PATH_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write_placeholder_socket() -> PathBuf {
    let socket = unique_temp_path("warp-tmux-kill-test.sock");
    std::fs::write(&socket, b"keep-me").expect("write placeholder socket");
    socket
}

fn write_placeholder_socket_and_config() -> (PathBuf, PathBuf) {
    let socket = unique_temp_path("warp-tmux-stale").with_extension("sock");
    let config = socket.with_extension("conf");
    std::fs::write(&socket, b"keep-me").expect("write placeholder socket");
    std::fs::write(&config, b"keep-me").expect("write placeholder config");
    (socket, config)
}

fn wait_until(pred: impl Fn() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    pred()
}

#[cfg(unix)]
fn chmod_script(script: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(script)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(script, permissions).expect("chmod script");
}

#[test]
fn kill_dedicated_server_terminates_tmux_on_socket() {
    let Some(tmux_path) = super::resolve_tmux_binary() else {
        return;
    };
    let socket = unique_temp_path("warp-tmux-kill-test.sock");
    let _ = std::fs::remove_file(&socket);
    let started = Command::new(&tmux_path)
        .args([
            "-S",
            socket.to_str().expect("socket utf8"),
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "warp-kill-test",
        ])
        .status()
        .expect("spawn dedicated tmux server");
    assert!(started.success());
    let listed = Command::new(&tmux_path)
        .args(["-S", socket.to_str().expect("socket utf8"), "list-sessions"])
        .status()
        .expect("list dedicated tmux sessions");
    assert!(listed.success());
    super::kill_dedicated_server(&socket);
    let listed_after = Command::new(&tmux_path)
        .args(["-S", socket.to_str().expect("socket utf8"), "list-sessions"])
        .status()
        .expect("list dedicated tmux sessions after kill");
    assert!(!listed_after.success());
    assert!(!socket.exists());
}

#[test]
fn kill_dedicated_server_preserves_socket_when_tmux_is_missing() {
    let socket = write_placeholder_socket();
    super::kill_dedicated_server_with(None, &socket, Duration::from_secs(1));
    assert!(socket.exists());
    let _ = std::fs::remove_file(&socket);
}

#[test]
fn kill_dedicated_server_preserves_socket_when_kill_fails() {
    let false_bin = Path::new("/bin/false");
    if !false_bin.exists() {
        return;
    }
    let socket = write_placeholder_socket();
    super::kill_dedicated_server_with(Some(false_bin), &socket, Duration::from_secs(1));
    assert!(socket.exists());
    let _ = std::fs::remove_file(&socket);
}

#[test]
fn kill_dedicated_server_removes_socket_when_no_server_is_running() {
    let script = unique_temp_path("warp-tmux-no-server.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write no-server script");
    #[cfg(unix)]
    chmod_script(&script);
    let socket = write_placeholder_socket();
    let config = socket.with_extension("conf");
    std::fs::write(&config, b"keep-me").expect("write placeholder config");
    super::kill_dedicated_server_with(Some(&script), &socket, Duration::from_secs(1));
    assert!(!socket.exists());
    assert!(!config.exists());
    let _ = std::fs::remove_file(&script);
}

#[test]
fn kill_dedicated_server_preserves_socket_when_kill_times_out() {
    let script = unique_temp_path("warp-tmux-hang-kill-server.sh");
    std::fs::write(&script, b"#!/bin/sh\nexec sleep 30\n").expect("write hang script");
    #[cfg(unix)]
    chmod_script(&script);
    let socket = write_placeholder_socket();
    let started = Instant::now();
    super::kill_dedicated_server_with(Some(&script), &socket, Duration::from_millis(80));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(socket.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn schedule_kill_dedicated_server_does_not_block_caller() {
    let socket = write_placeholder_socket();
    let started = Instant::now();
    schedule_kill_dedicated_server(socket.clone());
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(50));
    let _ = std::fs::remove_file(&socket);
}

#[test]
fn kill_dedicated_server_preserves_socket_on_permission_error() {
    let script = unique_temp_path("warp-tmux-permission.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'error connecting to /tmp/x (Permission denied)' >&2\nexit 1\n",
    )
    .expect("write permission script");
    #[cfg(unix)]
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    super::kill_dedicated_server_with(Some(&script), &socket, Duration::from_secs(1));
    assert!(socket.exists());
    assert!(config.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn cleanup_unspawned_dedicated_files_removes_socket_and_config() {
    let (socket, config) = write_placeholder_socket_and_config();
    cleanup_unspawned_dedicated_files(&socket);
    assert!(!socket.exists());
    assert!(!config.exists());
}

#[cfg(unix)]
#[test]
fn detached_kill_helper_unlinks_socket_and_config_when_server_is_gone() {
    let script = unique_temp_path("warp-tmux-detached-no-server.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write detached no-server script");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    super::spawn_detached_kill_helper(Some(&script), &socket);
    assert!(wait_until(
        || !socket.exists() && !config.exists(),
        Duration::from_secs(3)
    ));
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn successful_kill_helper_prunes_registered_socket() {
    let _guard = super::registry_test_lock();
    let script = unique_temp_path("warp-tmux-prune-no-server.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write prune no-server script");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let before = super::registered_dedicated_server_count();
    register_dedicated_server(socket.clone());
    assert!(super::spawn_detached_kill_helper(Some(&script), &socket));
    assert!(wait_until(
        || {
            !socket.exists()
                && !config.exists()
                && super::registered_dedicated_server_count() == before
        },
        Duration::from_secs(3)
    ));
    let list = std::fs::read_to_string(super::registry_list_path()).unwrap_or_default();
    assert!(!list.contains(&socket.to_string_lossy().into_owned()));
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn detached_kill_helper_times_out_hanging_tmux_and_keeps_files() {
    let script = unique_temp_path("warp-tmux-hang-helper.sh");
    std::fs::write(&script, b"#!/bin/sh\nexec sleep 30\n").expect("write hang helper");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let started = Instant::now();
    super::spawn_detached_kill_helper(Some(&script), &socket);
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(socket.exists());
    assert!(config.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn failed_kill_helper_keeps_unix_sockets_registered_for_retry() {
    let _guard = super::registry_test_lock();
    let script = unique_temp_path("warp-tmux-permission-retain.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'error connecting to /tmp/x (Permission denied)' >&2\nexit 1\n",
    )
    .expect("write permission retain script");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let before = super::registered_dedicated_server_count();
    register_dedicated_server(socket.clone());
    assert!(super::spawn_detached_kill_helper(Some(&script), &socket));
    assert!(wait_until(
        || {
            let list = std::fs::read_to_string(super::registry_list_path()).unwrap_or_default();
            socket.exists()
                && config.exists()
                && super::registered_dedicated_server_count() == before + 1
                && list.contains(&socket.to_string_lossy().into_owned())
        },
        Duration::from_secs(3)
    ));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    assert!(socket.exists());
    assert!(config.exists());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_file(&script);
}

#[test]
fn registry_list_contains_only_this_process_registered_sockets() {
    let _guard = super::registry_test_lock();
    let foreign = super::dedicated_server_dir().join("warp-foreign.sock");
    let _ = std::fs::create_dir_all(super::dedicated_server_dir());
    std::fs::write(&foreign, b"other-process").expect("write foreign socket");
    let socket = write_placeholder_socket();
    register_dedicated_server(socket.clone());
    let list = std::fs::read_to_string(super::registry_list_path()).expect("read registry list");
    let socket_s = socket.to_string_lossy().into_owned();
    let foreign_s = foreign.to_string_lossy().into_owned();
    assert!(list.contains(&socket_s));
    assert!(!list.contains(&foreign_s));
    assert!(
        super::registry_list_path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(&std::process::id().to_string()))
    );
    schedule_kill_dedicated_server(socket.clone());
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&foreign);
}

#[cfg(unix)]
#[test]
fn app_exit_reaper_script_kills_listed_sockets_after_stdin_eof() {
    let script = unique_temp_path("warp-tmux-reaper-tmux.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write reaper tmux script");
    chmod_script(&script);
    let (listed, listed_conf) = write_placeholder_socket_and_config();
    let (unlisted, unlisted_conf) = write_placeholder_socket_and_config();
    let list = unique_temp_path("warp-tmux-reaper.list");
    std::fs::write(&list, format!("{}\n", listed.display())).expect("write list");
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair().expect("pipe");
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(super::APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(&list)
        .arg(&script)
        // SAFETY: child_end is the unique owner of this socket.
        .stdin(unsafe {
            use std::os::fd::{FromRawFd as _, IntoRawFd as _};
            Stdio::from_raw_fd(child_end.into_raw_fd())
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reaper script");
    drop(parent_end);
    let _ = child.wait();
    assert!(wait_until(
        || !listed.exists() && !listed_conf.exists(),
        Duration::from_secs(3)
    ));
    assert!(unlisted.exists());
    assert!(unlisted_conf.exists());
    let _ = std::fs::remove_file(&unlisted);
    let _ = std::fs::remove_file(&unlisted_conf);
    let _ = std::fs::remove_file(&script);
}

#[cfg(unix)]
#[test]
fn spawn_app_exit_reaper_marks_started_only_on_success() {
    let _guard = super::registry_test_lock();
    if super::app_exit_reaper_has_started() {
        assert!(super::ensure_app_exit_reaper_with(None));
        return;
    }
    assert!(!super::ensure_app_exit_reaper_with(None));
    assert!(!super::app_exit_reaper_has_started());
    let Some(tmux_path) = super::resolve_tmux_binary() else {
        return;
    };
    assert!(super::ensure_app_exit_reaper_with(Some(&tmux_path)));
    assert!(super::app_exit_reaper_has_started());
}

#[cfg(unix)]
#[test]
fn last_tab_exit_does_not_drop_sockets_before_cleanup_is_secured() {
    let _guard = super::registry_test_lock();
    let before = super::registered_dedicated_server_count();
    let socket = write_placeholder_socket();
    register_dedicated_server(socket.clone());
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    assert!(!super::spawn_detached_kill_helper(None, &socket));
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    super::schedule_kill_registered_dedicated_servers();
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    let list = std::fs::read_to_string(super::registry_list_path()).expect("read registry list");
    assert!(list.contains(&socket.to_string_lossy().into_owned()));
    schedule_kill_dedicated_server(socket.clone());
    let _ = std::fs::remove_file(&socket);
}

#[cfg(unix)]
#[test]
fn timed_out_helper_keeps_socket_registered_for_app_exit() {
    let _guard = super::registry_test_lock();
    let script = unique_temp_path("warp-tmux-hang-then-reaper.sh");
    std::fs::write(&script, b"#!/bin/sh\nexec sleep 30\n").expect("write hang helper");
    chmod_script(&script);
    let (socket, config) = write_placeholder_socket_and_config();
    let before = super::registered_dedicated_server_count();
    register_dedicated_server(socket.clone());
    let started = Instant::now();
    assert!(super::spawn_detached_kill_helper(Some(&script), &socket));
    assert!(started.elapsed() < Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(socket.exists());
    assert!(config.exists());
    assert_eq!(super::registered_dedicated_server_count(), before + 1);
    let list = std::fs::read_to_string(super::registry_list_path()).expect("read registry list");
    assert!(list.contains(&socket.to_string_lossy().into_owned()));

    let reaper_tmux = unique_temp_path("warp-tmux-reaper-after-timeout.sh");
    std::fs::write(
        &reaper_tmux,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write reaper tmux script");
    chmod_script(&reaper_tmux);
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair().expect("pipe");
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(super::APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(super::registry_list_path())
        .arg(&reaper_tmux)
        .stdin(unsafe {
            use std::os::fd::{FromRawFd as _, IntoRawFd as _};
            Stdio::from_raw_fd(child_end.into_raw_fd())
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reaper script");
    drop(parent_end);
    let _ = child.wait();
    assert!(wait_until(
        || !socket.exists() && !config.exists(),
        Duration::from_secs(3)
    ));
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&reaper_tmux);
}

#[test]
fn registry_persist_failure_does_not_mark_reaper_ready() {
    let _guard = super::registry_test_lock();
    let blocker = unique_temp_path("warp-tmux-registry-blocker");
    std::fs::write(&blocker, b"not-a-dir").expect("write blocker file");
    let path = blocker.join("active.list");
    let socket = write_placeholder_socket();
    let sockets = HashSet::from([socket.clone()]);
    assert!(super::persist_registry_list_at(&path, &sockets).is_err());
    #[cfg(unix)]
    {
        if !super::app_exit_reaper_has_started() {
            assert!(!super::ensure_app_exit_reaper_with(None));
            assert!(!super::app_exit_reaper_has_started());
        }
    }
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&blocker);
}

#[test]
fn registry_list_filename_isolates_pid_reuse_with_token() {
    let name = super::registry_list_filename();
    let pid = std::process::id().to_string();
    assert!(name.starts_with(&format!("active-{pid}-")));
    assert!(name.ends_with(".list"));
    assert_ne!(name, format!("active-{pid}.list"));
}

#[cfg(unix)]
#[test]
fn app_exit_reaper_ignores_same_pid_list_without_instance_token() {
    let script = unique_temp_path("warp-tmux-token-reaper.sh");
    std::fs::write(
        &script,
        b"#!/bin/sh\necho 'no server running on /tmp/missing' >&2\nexit 1\n",
    )
    .expect("write token reaper tmux");
    chmod_script(&script);
    let (ours, ours_conf) = write_placeholder_socket_and_config();
    let (theirs, theirs_conf) = write_placeholder_socket_and_config();
    let tokenized = unique_temp_path("warp-tmux-tokenized.list");
    let colliding = unique_temp_path(&format!("active-{}.list", std::process::id()));
    std::fs::write(&tokenized, format!("{}\n", ours.display())).expect("write tokenized list");
    std::fs::write(&colliding, format!("{}\n", theirs.display())).expect("write colliding list");
    let (parent_end, child_end) = std::os::unix::net::UnixStream::pair().expect("pipe");
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(super::APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(&tokenized)
        .arg(&script)
        .stdin(unsafe {
            use std::os::fd::{FromRawFd as _, IntoRawFd as _};
            Stdio::from_raw_fd(child_end.into_raw_fd())
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn reaper script");
    drop(parent_end);
    let _ = child.wait();
    assert!(wait_until(
        || !ours.exists() && !ours_conf.exists(),
        Duration::from_secs(3)
    ));
    assert!(theirs.exists());
    assert!(theirs_conf.exists());
    let _ = std::fs::remove_file(&theirs);
    let _ = std::fs::remove_file(&theirs_conf);
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&colliding);
}

#[test]
fn send_keys_hex_encodes_control_and_printable_bytes() {
    let commands = send_keys_commands(&PaneId::from("%0"), b"a\x03\n");
    assert_eq!(commands, vec!["send-keys -t %0 -H 61 03 0a\n".to_owned()]);
}

#[test]
fn send_keys_chunks_large_payloads() {
    let bytes = vec![b'x'; 200];
    let commands = send_keys_commands(&PaneId::from("%3"), &bytes);
    assert_eq!(commands.len(), 2);
    assert!(commands[0].starts_with("send-keys -t %3 -H "));
    assert!(commands[0].ends_with('\n'));
    assert_eq!(commands[0].matches(' ').count(), 3 + 128);
    assert_eq!(commands[1].matches(' ').count(), 3 + 72);
}

#[test]
fn control_client_argv_starts_detached_control_mode_session() {
    let bootstrap = PaneBootstrap {
        session_id: SessionId::from(42),
        shell_type: ShellType::Bash,
        shell_path: PathBuf::from("/bin/bash"),
        args: vec!["-c".into(), "echo hi".into()],
        init_script: None,
    };
    let argv = control_client_argv(
        PathBuf::from("/usr/bin/tmux").as_path(),
        PathBuf::from("/tmp/warp.sock").as_path(),
        PathBuf::from("/tmp/warp.conf").as_path(),
        &bootstrap,
        80,
        24,
    );
    let argv: Vec<String> = argv
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        vec![
            "/usr/bin/tmux",
            "-S",
            "/tmp/warp.sock",
            "-f",
            "/tmp/warp.conf",
            "-CC",
            "new-session",
            "-s",
            "warp-42",
            "-n",
            "warp",
            "-x",
            "80",
            "-y",
            "24",
            "--",
            "/bin/bash",
            "-c",
            "echo hi",
        ]
    );
}

#[test]
fn bash_pane_bootstrap_embeds_session_id_in_args() {
    let bootstrap = pane_bootstrap_for_shell(PathBuf::from("/bin/bash"), ShellType::Bash);
    assert!(bootstrap.init_script.is_none());
    let joined = bootstrap
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains(&bootstrap.session_id.as_u64().to_string()));
}

#[test]
fn zsh_pane_bootstrap_keeps_init_script_for_send_keys() {
    let bootstrap = pane_bootstrap_for_shell(PathBuf::from("/bin/zsh"), ShellType::Zsh);
    let init_script = bootstrap.init_script.expect("zsh needs an init script");
    assert!(init_script.contains(&bootstrap.session_id.as_u64().to_string()));
    let bytes = zsh_init_bytes(&init_script, ShellType::Zsh, bootstrap.session_id);
    assert!(bytes.ends_with(b"\n"));
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains(r#"'"'"'"#),
        "send-keys zsh init must not be eval-escaped: {text}"
    );
    let init_at = text.find("InitShell").expect("InitShell");
    let ack_at = text.rfind("9278;t;").expect("stage-complete ack");
    assert!(init_at < ack_at);
    assert_eq!(text.matches('\n').count(), 1);
    assert!(
        !text[..ack_at].contains('\n'),
        "stage ack must run on the same line as init: {text}"
    );
}

#[test]
fn in_place_pane_spawn_uses_path_name_and_no_rcs() {
    let (argv, session_id) = in_place_pane_spawn_argv(ShellType::Zsh);
    assert_eq!(argv[0], "zsh");
    assert!(argv.iter().any(|arg| arg.contains("--no-rcs")));
    assert!(!argv[0].starts_with('/'));
    assert_ne!(session_id.as_u64(), 0);
    let zsh = super::in_place_pane_spawn(ShellType::Zsh);
    let init_script = zsh
        .init_script
        .expect("zsh no-rcs pane keeps retained init");
    assert!(init_script.contains(&zsh.session_id.as_u64().to_string()));
    assert!(init_script.contains("InitShell"));
    let (bash_argv, _) = in_place_pane_spawn_argv(ShellType::Bash);
    assert_eq!(bash_argv[0], "bash");
    assert!(
        super::in_place_pane_spawn(ShellType::Bash)
            .init_script
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn product_path_tmux_cc_no_rcs_pane_survives_init_ack_bootstrap_and_detach() {
    let Some(tmux) = super::resolve_tmux_binary() else {
        return;
    };
    if optional_shell("zsh").is_none() {
        return;
    };
    let Ok(python) = Command::new("python3").arg("-c").arg("import pty").status() else {
        return;
    };
    if !python.success() {
        return;
    }
    let (mut argv, session_id, zsh_init) =
        crate::terminal::tmux::transport::tmux_cc_argv(&[], "warp", 80, 24, Some(ShellType::Zsh))
            .expect("bare /tmux argv");
    let session_id = session_id.expect("bare /tmux always includes a pane spawn");
    let zsh_init = zsh_init.expect("zsh no-rcs pane keeps retained init");
    argv[0] = tmux.display().to_string();
    argv.insert(1, "-f".to_owned());
    argv.insert(2, "/dev/null".to_owned());
    let init = zsh_init_bytes(&zsh_init, ShellType::Zsh, session_id);
    let bootstrap_bytes = silent_bootstrap_bytes(ShellType::Zsh);
    let socket_name = warp_terminal::tmux::WARP_CONTROL_SOCKET_NAME;
    let home = unique_temp_path("warp-tmux-cc-home");
    std::fs::create_dir_all(&home).expect("tmux cc home");
    let tmpdir = unique_temp_path("warp-tmux-cc-tmp");
    std::fs::create_dir_all(&tmpdir).expect("tmux cc tmp");
    let init_path = home.join("init.bin");
    let boot_path = home.join("boot.bin");
    std::fs::write(&init_path, &init).expect("write init payload");
    std::fs::write(&boot_path, &bootstrap_bytes).expect("write bootstrap payload");
    let runner = format!(
        r##"
import os, pty, select, signal, subprocess, sys, time
tmux = {tmux:?}
argv = {argv:?}
socket = {socket_name:?}
home = {home:?}
tmpdir = {tmpdir:?}
init_path = {init_path:?}
boot_path = {boot_path:?}
os.environ["HOME"] = home
os.environ["TMUX_TMPDIR"] = tmpdir
os.environ.pop("TMUX", None)
def to_hex(data):
    return " ".join(f"{{b:02x}}" for b in data)
def chunks(hexs, n=128):
    parts = hexs.split()
    for i in range(0, len(parts), n):
        yield " ".join(parts[i:i+n])
init_hex = to_hex(open(init_path, "rb").read())
boot_hex = to_hex(open(boot_path, "rb").read())
pid, fd = pty.fork()
if pid == 0:
    os.chdir(home)
    os.execv(tmux, argv)
out = b""
deadline = time.time() + 8
entered = False
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 65536)
    except OSError:
        break
    if not chunk:
        break
    out += chunk
    if b"\x1bP1000p" in out:
        entered = True
        break
if not entered:
    sys.stdout.buffer.write(out)
    sys.exit(2)
os.write(fd, b"set -s exit-empty off\n")
time.sleep(0.2)
def drain(timeout):
    global out
    end = time.time() + timeout
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return
        if not chunk:
            return
        out += chunk
        end = time.time() + timeout
for part in chunks(init_hex):
    os.write(fd, f"send-keys -t %0 -H {{part}}\n".encode())
    drain(0.05)
drain(1.5)
saw_init = b"InitShell" in out or b"496e69745368656c6c" in out
saw_ack = b"9278;t;" in out
for part in chunks(boot_hex):
    os.write(fd, f"send-keys -t %0 -H {{part}}\n".encode())
    drain(0.05)
drain(8)
saw_boot = b"Bootstrapped" in out or b"426f6f747374726170706564" in out
listed = subprocess.run([tmux, "-L", socket, "list-sessions"], capture_output=True)
panes = subprocess.run([tmux, "-L", socket, "list-panes", "-a", "-F", "#{{pane_dead}}"], capture_output=True, text=True)
os.write(fd, b"detach-client\n")
time.sleep(0.4)
listed_after = subprocess.run([tmux, "-L", socket, "list-sessions"], capture_output=True)
subprocess.run([tmux, "-L", socket, "kill-server"], capture_output=True)
try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
sys.stdout.buffer.write(out)
print("LISTED", listed.returncode, file=sys.stderr)
print("PANES", panes.stdout.strip(), file=sys.stderr)
print("AFTER", listed_after.returncode, file=sys.stderr)
print("INIT", int(saw_init), "ACK", int(saw_ack), "BOOT", int(saw_boot), file=sys.stderr)
sys.exit(0 if saw_init and saw_ack and saw_boot and listed.returncode == 0 and listed_after.returncode == 0 and "0" in panes.stdout else 1)
"##,
        tmux = tmux.display(),
        argv = argv,
        socket_name = socket_name,
        home = home.display(),
        tmpdir = tmpdir.display(),
        init_path = init_path.display(),
        boot_path = boot_path.display(),
    );
    let child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python tmux cc runner");
    let output = child.wait_with_output().expect("wait python tmux cc");
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&tmpdir);
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let text = String::from_utf8_lossy(&combined);
    assert!(
        output.status.success(),
        "product-path tmux -CC must keep pane/server through InitShell, ack, Bootstrapped, and detach: {text}"
    );
}

#[cfg(unix)]
#[test]
fn zsh_retained_init_bytes_reach_prompt_without_quote_ps2() {
    let Some(zsh) = optional_shell("zsh") else {
        return;
    };
    let Ok(python) = Command::new("python3").arg("-c").arg("import pty").status() else {
        return;
    };
    if !python.success() {
        return;
    }
    let bootstrap = pane_bootstrap_for_shell(zsh.clone(), ShellType::Zsh);
    let init_script = bootstrap.init_script.expect("zsh needs an init script");
    let init = zsh_init_bytes(&init_script, ShellType::Zsh, bootstrap.session_id);
    let home = unique_temp_path("warp-tmux-zsh-retained-home");
    std::fs::create_dir_all(&home).expect("zsh retained home");
    let runner = format!(
        r#"
import os, pty, select, signal, sys, termios, time
zsh = {zsh:?}
script = sys.stdin.buffer.read()
pid, fd = pty.fork()
if pid == 0:
    os.chdir({home:?})
    os.environ["HOME"] = {home:?}
    os.environ.pop("HISTFILE", None)
    os.execv(zsh, [zsh, "--no-rcs", "-i"])
attrs = termios.tcgetattr(fd)
attrs[3] = attrs[3] & ~termios.ECHO
termios.tcsetattr(fd, termios.TCSANOW, attrs)
for i in range(0, len(script), 128):
    os.write(fd, script[i:i+128])
out = b""
deadline = time.time() + 6
saw_dcs = False
saw_ok = False
saw_eof = False
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        saw_eof = True
        break
    if not chunk:
        saw_eof = True
        break
    out += chunk
    if b"\x1bP$d" in out and b"9278;t;" in out:
        saw_dcs = True
        break
if saw_dcs:
    os.write(fd, b"printf 'WARP_INIT_OK\\n'\n")
    os.write(fd, b"exit\n")
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.2)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            saw_eof = True
            break
        if not chunk:
            saw_eof = True
            break
        out += chunk
        if b"WARP_INIT_OK" in out:
            saw_ok = True
            break
def reap(until):
    while time.time() < until:
        try:
            wpid, _ = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            return True
        if wpid == pid:
            return True
        time.sleep(0.05)
    return False
reaped = reap(time.time() + 2)
if not reaped:
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except OSError:
        pass
    reaped = reap(time.time() + 2)
eof_until = time.time() + 2
while time.time() < eof_until and not saw_eof:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        saw_eof = True
        break
    if not chunk:
        saw_eof = True
        break
    out += chunk
sys.stdout.buffer.write(out)
sys.exit(0 if saw_dcs and saw_ok and reaped and saw_eof else 1)
"#,
        zsh = zsh.display(),
        home = home.display(),
    );
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python pty runner");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, &init);
    }
    let output = child.wait_with_output().expect("wait python pty");
    let _ = std::fs::remove_dir_all(&home);
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let text = String::from_utf8_lossy(&combined);
    assert!(
        output.status.success(),
        "python pty runner failed: {text:?}"
    );
    assert!(
        combined.windows(4).any(|w| w == [0x1b, b'P', b'$', b'd']),
        "retained zsh init must emit InitShell DCS: {text:?}"
    );
    assert!(
        text.contains("WARP_INIT_OK"),
        "non-echo completion marker missing: {text:?}"
    );
    assert!(
        !text.contains("quote>")
            && !text.contains("heredoc>")
            && !text.contains("function>")
            && !text.contains("then>"),
        "retained zsh init must not stall on PS2: {text:?}"
    );
    assert!(
        !text.contains("fc -p") && !text.contains("fc -P"),
        "retained zsh init must not inject in-band history setup: {text:?}"
    );
}

#[test]
fn in_band_init_bytes_are_remote_safe_and_session_scoped() {
    let session_id = SessionId::from(7);
    let bytes = in_band_init_bytes(ShellType::Zsh, session_id).expect("zsh init");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("7"));
    assert!(text.contains("InitShell"));
    assert!(bytes.ends_with(b"\n"));
    let bash = in_band_init_bytes(ShellType::Bash, session_id).expect("bash init");
    assert_ne!(bytes, bash);
    assert!(text.contains("9278;t;7"));
    assert!(text.find("InitShell").unwrap() < text.rfind("9278;t;").unwrap());
}

#[test]
fn split_stage_complete_strips_osc_and_returns_session() {
    let mut bytes = b"keep-before".to_vec();
    bytes.extend_from_slice(b"\x1b]9278;t;42\x07");
    bytes.extend_from_slice(b"keep-after");
    let (kept, sessions) = split_stage_complete(&bytes);
    assert_eq!(kept, b"keep-beforekeep-after");
    assert_eq!(sessions, vec![SessionId::from(42)]);
    let (kept, sessions) = split_stage_complete(b"no-ack-here");
    assert_eq!(kept, b"no-ack-here");
    assert!(sessions.is_empty());
}

#[test]
fn split_stage_complete_strips_coalesced_stale_and_live_markers() {
    let mut bytes = b"pre".to_vec();
    bytes.extend_from_slice(b"\x1b]9278;t;1\x07");
    bytes.extend_from_slice(b"mid");
    bytes.extend_from_slice(b"\x1b]9278;t;2\x07");
    bytes.extend_from_slice(b"post");
    let (kept, sessions) = split_stage_complete(&bytes);
    assert_eq!(kept, b"premidpost");
    assert_eq!(sessions, vec![SessionId::from(1), SessionId::from(2)]);
}

#[test]
fn silent_bootstrap_framing_hides_setup_and_clears_before_prompt() {
    let body =
        b"warp_bootstrapped() {\nread -r -d '' WARP_BOOTSTRAP_VAR << 'EOM'\npayload\nEOM\n\n";
    let framed = super::silent_history_isolated_script(ShellType::Zsh, body);
    let text = String::from_utf8_lossy(&framed);
    let echo_off = text.find("stty -echo").expect("echo off");
    let hist = text.find("HISTFILE=/dev/null").expect("history isolate");
    let marker = text.find("warp_bootstrapped").expect("bootstrap body");
    let trap_off = text.find("trap - EXIT INT TERM").expect("disable traps");
    let unset_snaps = text
        .rfind("unset __warp_histfile")
        .expect("snapshot unset after traps");
    let unfunction = text
        .rfind("unfunction __warp_silent_cleanup")
        .expect("unfunction after unset");
    let clear = text.rfind("printf").expect("clear");
    let echo_on = text.rfind("stty echo").expect("echo restore");
    assert!(text.contains("setopt NO_BANG_HIST"));
    let fcntl_snap = text.find("__warp_hist_fcntl").expect("fcntl snapshot");
    let fcntl = text
        .find("unsetopt HIST_FCNTL_LOCK")
        .expect("fcntl lock off");
    assert!(fcntl_snap < fcntl);
    assert!(fcntl < hist);
    assert!(text.contains("if (( __warp_hist_fcntl )); then setopt HIST_FCNTL_LOCK"));
    assert!(text.contains("__warp_histfile_set"));
    assert!(text.contains("__warp_banghist"));
    assert!(text.contains("(( ${+__warp_silent_cleaned} )) && return"));
    assert!(!text.contains("fc -P") && !text.contains("fc -p"));
    assert!(echo_off < hist);
    assert!(hist < marker);
    assert!(marker < trap_off);
    assert!(trap_off < unset_snaps);
    assert!(unset_snaps < unfunction);
    assert!(unfunction < clear);
    assert!(clear < echo_on);
    assert!(text.trim_end().ends_with("stty echo 2>/dev/null || true"));
}

#[cfg(unix)]
#[test]
fn silent_bootstrap_clear_printf_emits_csi_bytes() {
    let framed = super::silent_history_isolated_script(ShellType::Bash, b":");
    let text = String::from_utf8(framed).expect("utf8");
    let printf_line = text
        .lines()
        .find(|line| line.contains("printf"))
        .expect("printf clear line");
    let output = Command::new("bash")
        .args(["-c", printf_line])
        .output()
        .expect("run printf");
    assert_eq!(
        output.stdout.as_slice(),
        [0x1b, 0x5b, 0x48, 0x1b, 0x5b, 0x32, 0x4a]
    );
}

#[cfg(unix)]
#[test]
fn zsh_silent_framed_bootstrap_exits_to_prompt_once() {
    let Some(zsh) = optional_shell("zsh") else {
        return;
    };
    let Ok(python) = Command::new("python3").arg("-c").arg("import pty").status() else {
        return;
    };
    if !python.success() {
        return;
    }
    let home = unique_temp_path("warp-tmux-zsh-silent-home");
    std::fs::create_dir_all(&home).expect("zsh silent home");
    let hist = home.join(".zsh_history");
    std::fs::write(&hist, b"prior-user-cmd\n").expect("seed histfile");
    let body = b"warp_silent_hook() { :; }\n";
    let framed = super::silent_history_isolated_script(ShellType::Zsh, body);
    let runner = format!(
        r#"
import os, pty, select, sys, time
zsh = {zsh:?}
script = sys.stdin.buffer.read()
pid, fd = pty.fork()
if pid == 0:
    os.chdir({home:?})
    os.environ["HOME"] = {home:?}
    os.environ["HISTFILE"] = {hist:?}
    os.environ["SAVEHIST"] = "1000"
    os.execv(zsh, [zsh, "--no-rcs", "-i"])
os.write(fd, script)
time.sleep(0.2)
os.write(fd, b"typeset -f warp_silent_hook >/dev/null && echo WARP_HOOK_OK\n")
time.sleep(0.1)
os.write(fd, b"stty -a 2>/dev/null | tr ' ' '\n' | grep -E '^-?echo$' | head -1 | sed 's/^-echo/ECHO_OFF/;s/^echo/ECHO_ON/'\n")
os.write(fd, b"SAVEHIST=0\necho WARP_SILENT_DONE\nexit\n")
out = b""
deadline = time.time() + 5
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        break
    if not chunk:
        break
    out += chunk
    if b"WARP_SILENT_DONE" in out:
        break
os.write(fd, b"exit\n")
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
sys.stdout.buffer.write(out)
"#,
        zsh = zsh.display(),
        home = home.display(),
        hist = hist.display(),
    );
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python pty runner");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, &framed);
    }
    let output = child.wait_with_output().expect("wait python pty");
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let text = String::from_utf8_lossy(&combined);
    let fc_p_count = text.matches("fc -P").count();
    assert!(
        fc_p_count <= 1,
        "fc -P must not loop, saw {fc_p_count}: {text:?}"
    );
    assert!(
        !text.contains("warp_bootstrapped") && !text.contains("<< 'EOM'"),
        "bootstrap markers must be absent: {text:?}"
    );
    assert!(
        text.contains("WARP_HOOK_OK"),
        "hook must be installed: {text:?}"
    );
    assert!(
        text.contains("WARP_SILENT_DONE"),
        "interactive zsh must reach a prompt and exit once: {text:?}"
    );
    assert!(
        text.contains("ECHO_ON"),
        "echo must be restored after silent bootstrap: {text:?}"
    );
    let csi = [0x1b, 0x5b, 0x48, 0x1b, 0x5b, 0x32, 0x4a];
    let Some(clear_at) = combined.windows(csi.len()).position(|w| w == csi) else {
        panic!("CSI clear missing: {text:?}");
    };
    let after_clear = String::from_utf8_lossy(&combined[clear_at + csi.len()..]);
    for marker in [
        "trap - EXIT INT TERM",
        "unset __warp_histfile",
        "unfunction __warp_silent_cleanup",
        "__warp_silent_cleanup",
    ] {
        assert!(
            !after_clear.contains(marker),
            "cleanup {marker:?} must not appear after clear: {after_clear:?}"
        );
    }
    let hist_bytes = std::fs::read(&hist).unwrap_or_default();
    let entries = ShellType::Zsh.parse_history(&hist_bytes);
    let joined = entries.join("\n");
    assert!(
        joined.contains("prior-user-cmd"),
        "prior history must be restored: {joined:?}"
    );
    assert!(
        !joined.contains("warp_silent_hook") && !joined.contains("HISTFILE=/dev/null"),
        "bootstrap must not persist in history: {joined:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn zsh_chunked_stage_then_bootstrap_reaches_prompt_without_ps2() {
    let Some(zsh) = optional_shell("zsh") else {
        return;
    };
    let Ok(python) = Command::new("python3").arg("-c").arg("import pty").status() else {
        return;
    };
    if !python.success() {
        return;
    }
    let home = unique_temp_path("warp-tmux-zsh-chunked-home");
    std::fs::create_dir_all(&home).expect("zsh chunked home");
    let session_id = SessionId::from(11);
    let init = zsh_init_bytes(":\n", ShellType::Zsh, session_id);
    let bootstrap =
        super::silent_history_isolated_script(ShellType::Zsh, b"warp_silent_hook() { :; }\n");
    let runner = format!(
        r#"
import os, pty, select, sys
zsh = {zsh:?}
payload = sys.stdin.buffer.read()
init_len = {init_len}
init, bootstrap = payload[:init_len], payload[init_len:]
pid, fd = pty.fork()
if pid == 0:
    os.chdir({home:?})
    os.environ["HOME"] = {home:?}
    os.environ.pop("HISTFILE", None)
    os.execv(zsh, [zsh, "--no-rcs", "-i"])
def write_chunks(data):
    for i in range(0, len(data), 128):
        os.write(fd, data[i:i+128])
write_chunks(init)
write_chunks(bootstrap)
os.write(fd, b"typeset -f warp_silent_hook >/dev/null && echo WARP_HOOK_OK\n")
os.write(fd, b"SAVEHIST=0\necho WARP_SILENT_DONE\nexit\n")
out = b""
import time
deadline = time.time() + 6
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        break
    if not chunk:
        break
    out += chunk
    if b"WARP_SILENT_DONE" in out:
        break
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
sys.stdout.buffer.write(out)
"#,
        zsh = zsh.display(),
        home = home.display(),
        init_len = init.len(),
    );
    let mut payload = init;
    payload.extend_from_slice(&bootstrap);
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python pty runner");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, &payload);
    }
    let output = child.wait_with_output().expect("wait python pty");
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let text = String::from_utf8_lossy(&combined);
    assert!(
        !text.contains("heredoc>") && !text.contains("function>") && !text.contains("then>"),
        "chunked staging must not leak PS2: {text:?}"
    );
    let csi = [0x1b, 0x5b, 0x48, 0x1b, 0x5b, 0x32, 0x4a];
    if let Some(clear_at) = combined.windows(csi.len()).position(|w| w == csi) {
        let after_clear = String::from_utf8_lossy(&combined[clear_at + csi.len()..]);
        for marker in [
            "__warp_silent_cleanup",
            "unset __warp_histfile",
            "HISTFILE=/dev/null",
        ] {
            assert!(
                !after_clear.contains(marker),
                "cleanup {marker:?} must not remain after clear: {after_clear:?}"
            );
        }
    }
    assert!(
        text.contains("WARP_HOOK_OK") && text.contains("WARP_SILENT_DONE"),
        "hooks must be accepted and prompt reached: {text:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
fn zsh_silent_probe(preamble: &[u8], probe: &[u8]) -> Option<String> {
    let zsh = optional_shell("zsh")?;
    let python = Command::new("python3")
        .arg("-c")
        .arg("import pty")
        .status()
        .ok()?;
    if !python.success() {
        return None;
    }
    let home = unique_temp_path("warp-tmux-zsh-probe-home");
    std::fs::create_dir_all(&home).ok()?;
    let framed = super::silent_history_isolated_script(ShellType::Zsh, b":\n");
    let runner = format!(
        r#"
import os, pty, select, sys, time
zsh = {zsh:?}
preamble = bytes({preamble:?})
probe = bytes({probe:?})
script = sys.stdin.buffer.read()
pid, fd = pty.fork()
if pid == 0:
    os.chdir({home:?})
    os.environ["HOME"] = {home:?}
    os.environ.pop("HISTFILE", None)
    os.environ.pop("SAVEHIST", None)
    os.execv(zsh, [zsh, "--no-rcs", "-i"])
os.write(fd, preamble)
time.sleep(0.15)
os.write(fd, script)
time.sleep(0.2)
os.write(fd, probe)
os.write(fd, b"SAVEHIST=0\necho WARP_SILENT_DONE\nexit\n")
out = b""
deadline = time.time() + 5
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        break
    if not chunk:
        break
    out += chunk
    if b"WARP_SILENT_DONE" in out:
        break
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
sys.stdout.buffer.write(out)
"#,
        zsh = zsh.display(),
        home = home.display(),
        preamble = preamble,
        probe = probe,
    );
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, &framed);
    }
    let output = child.wait_with_output().ok()?;
    let _ = std::fs::remove_dir_all(&home);
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    Some(String::from_utf8_lossy(&combined).into_owned())
}

#[cfg(unix)]
#[test]
fn zsh_silent_restores_unset_hist_and_banghist_on() {
    let Some(text) = zsh_silent_probe(
        b"unsetopt BANG_HIST\nsetopt BANG_HIST\nsetopt HIST_FCNTL_LOCK\nunset HISTFILE\nunset SAVEHIST\n",
        b"[[ -o banghist ]] && echo BANG_ON || echo BANG_OFF\n[[ -o histfcntllock ]] && echo FCNTL_ON || echo FCNTL_OFF\n[[ -v HISTFILE ]] && echo HIST_SET || echo HIST_UNSET\n[[ -v SAVEHIST ]] && echo SAVE_SET || echo SAVE_UNSET\n[[ -v __warp_hist_fcntl ]] && echo FCNTL_VAR_SET || echo FCNTL_VAR_UNSET\n",
    ) else {
        return;
    };
    assert!(
        text.contains("WARP_SILENT_DONE"),
        "script must complete: {text:?}"
    );
    assert!(
        text.contains("BANG_ON"),
        "BANG_HIST must be restored on: {text:?}"
    );
    assert!(
        text.contains("FCNTL_ON"),
        "HIST_FCNTL_LOCK must be restored on: {text:?}"
    );
    assert!(
        text.contains("FCNTL_VAR_UNSET"),
        "fcntl snapshot must be unset: {text:?}"
    );
    assert!(
        text.contains("HIST_UNSET"),
        "unset HISTFILE must stay unset: {text:?}"
    );
    assert!(
        text.contains("SAVE_UNSET"),
        "unset SAVEHIST must stay unset: {text:?}"
    );
}

#[cfg(unix)]
#[test]
fn zsh_silent_restores_set_hist_and_banghist_off() {
    let Some(text) = zsh_silent_probe(
        b"unsetopt BANG_HIST\nunsetopt HIST_FCNTL_LOCK\nHISTFILE=/tmp/warp-silent-hist\nSAVEHIST=42\n",
        b"[[ -o banghist ]] && echo BANG_ON || echo BANG_OFF\n[[ -o histfcntllock ]] && echo FCNTL_ON || echo FCNTL_OFF\n[[ -v HISTFILE ]] && echo HIST_SET:$HISTFILE || echo HIST_UNSET\n[[ -v SAVEHIST ]] && echo SAVE_SET:$SAVEHIST || echo SAVE_UNSET\n[[ -v __warp_hist_fcntl ]] && echo FCNTL_VAR_SET || echo FCNTL_VAR_UNSET\n",
    ) else {
        return;
    };
    assert!(
        text.contains("WARP_SILENT_DONE"),
        "script must complete: {text:?}"
    );
    assert!(
        text.contains("BANG_OFF"),
        "BANG_HIST must stay off: {text:?}"
    );
    assert!(
        text.contains("FCNTL_OFF"),
        "HIST_FCNTL_LOCK must stay off: {text:?}"
    );
    assert!(
        text.contains("FCNTL_VAR_UNSET"),
        "fcntl snapshot must be unset: {text:?}"
    );
    assert!(
        text.contains("HIST_SET:/tmp/warp-silent-hist"),
        "HISTFILE must be restored: {text:?}"
    );
    assert!(
        text.contains("SAVE_SET:42"),
        "SAVEHIST must be restored: {text:?}"
    );
}

#[cfg(unix)]
fn zsh_silent_signal_probe(signum: i32, preamble: &[u8], probe: &[u8]) -> Option<String> {
    let zsh = optional_shell("zsh")?;
    let python = Command::new("python3")
        .arg("-c")
        .arg("import pty")
        .status()
        .ok()?;
    if !python.success() {
        return None;
    }
    let home = unique_temp_path("warp-tmux-zsh-signal-home");
    std::fs::create_dir_all(&home).ok()?;
    let framed = super::silent_history_isolated_script(ShellType::Zsh, b"sleep 0.4\n");
    let runner = format!(
        r#"
import os, pty, select, signal, sys, time
zsh = {zsh:?}
preamble = bytes({preamble:?})
probe = bytes({probe:?})
signum = {signum}
script = sys.stdin.buffer.read()
pid, fd = pty.fork()
if pid == 0:
    os.chdir({home:?})
    os.environ["HOME"] = {home:?}
    os.environ.pop("HISTFILE", None)
    os.environ.pop("SAVEHIST", None)
    os.execv(zsh, [zsh, "--no-rcs", "-i"])
os.write(fd, preamble)
time.sleep(0.15)
os.write(fd, script)
time.sleep(0.12)
os.kill(pid, signum)
time.sleep(0.5)
os.write(fd, probe)
os.write(fd, b"stty -a 2>/dev/null | tr ' ' '\n' | grep -E '^-?echo$' | head -1 | sed 's/^-echo/ECHO_OFF/;s/^echo/ECHO_ON/'\n")
os.write(fd, b"echo WARP_SILENT_DONE\n")
out = b""
deadline = time.time() + 6
alive = True
while time.time() < deadline:
    wpid, status = os.waitpid(pid, os.WNOHANG)
    if wpid == pid:
        alive = False
        break
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        break
    if not chunk:
        break
    out += chunk
    if b"WARP_SILENT_DONE" in out:
        break
if alive:
    out += b"\nWARP_SHELL_ALIVE\n"
    os.write(fd, b"exit\n")
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
sys.stdout.buffer.write(out)
"#,
        zsh = zsh.display(),
        home = home.display(),
        preamble = preamble,
        probe = probe,
        signum = signum,
    );
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, &framed);
    }
    let output = child.wait_with_output().ok()?;
    let _ = std::fs::remove_dir_all(&home);
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    Some(String::from_utf8_lossy(&combined).into_owned())
}

#[cfg(unix)]
fn assert_zsh_silent_signal_restored(signum: i32, name: &str) {
    let Some(text) = zsh_silent_signal_probe(
        signum,
        b"unsetopt BANG_HIST\nsetopt BANG_HIST\nsetopt HIST_FCNTL_LOCK\nunset HISTFILE\nunset SAVEHIST\n",
        b"[[ -o banghist ]] && echo BANG_ON || echo BANG_OFF\n[[ -o histfcntllock ]] && echo FCNTL_ON || echo FCNTL_OFF\n[[ -v HISTFILE ]] && echo HIST_SET || echo HIST_UNSET\n[[ -v SAVEHIST ]] && echo SAVE_SET || echo SAVE_UNSET\n[[ -v __warp_hist_fcntl ]] && echo FCNTL_VAR_SET || echo FCNTL_VAR_UNSET\n",
    ) else {
        return;
    };
    assert!(
        text.contains("WARP_SHELL_ALIVE"),
        "{name}: shell must stay alive: {text:?}"
    );
    assert!(
        text.contains("WARP_SILENT_DONE"),
        "{name}: queued epilogue must complete: {text:?}"
    );
    let done = text.matches("WARP_SILENT_DONE").count();
    assert!(
        (1..=2).contains(&done),
        "{name}: prompt/done once (echo may duplicate): {text:?}"
    );
    assert!(
        text.contains("ECHO_ON"),
        "{name}: echo must be restored: {text:?}"
    );
    assert!(
        text.contains("BANG_ON"),
        "{name}: BANG_HIST must be restored on: {text:?}"
    );
    assert!(
        text.contains("FCNTL_ON"),
        "{name}: HIST_FCNTL_LOCK must be restored on: {text:?}"
    );
    assert!(
        text.contains("FCNTL_VAR_UNSET"),
        "{name}: fcntl snapshot must be unset: {text:?}"
    );
    assert!(
        text.contains("HIST_UNSET"),
        "{name}: unset HISTFILE must stay unset: {text:?}"
    );
    assert!(
        text.contains("SAVE_UNSET"),
        "{name}: unset SAVEHIST must stay unset: {text:?}"
    );
}

#[cfg(unix)]
#[test]
fn zsh_silent_int_during_wrapper_restores_exact_state() {
    assert_zsh_silent_signal_restored(libc::SIGINT, "INT");
}

#[cfg(unix)]
#[test]
fn zsh_silent_term_during_wrapper_restores_exact_state() {
    assert_zsh_silent_signal_restored(libc::SIGTERM, "TERM");
}

#[cfg(unix)]
#[test]
fn zsh_silent_second_wrapper_after_int_restores_without_stale_guard() {
    let Some(zsh) = optional_shell("zsh") else {
        return;
    };
    let Ok(python) = Command::new("python3").arg("-c").arg("import pty").status() else {
        return;
    };
    if !python.success() {
        return;
    }
    let home = unique_temp_path("warp-tmux-zsh-second-home");
    std::fs::create_dir_all(&home).expect("zsh second home");
    let first = super::silent_history_isolated_script(ShellType::Zsh, b"sleep 0.4\n");
    let second = super::silent_history_isolated_script(ShellType::Zsh, b":\n");
    let runner = format!(
        r#"
import os, pty, select, signal, sys, time
zsh = {zsh:?}
first = bytes({first:?})
second = bytes({second:?})
pid, fd = pty.fork()
if pid == 0:
    os.chdir({home:?})
    os.environ["HOME"] = {home:?}
    os.environ.pop("HISTFILE", None)
    os.environ.pop("SAVEHIST", None)
    os.execv(zsh, [zsh, "--no-rcs", "-i"])
os.write(fd, b"unsetopt BANG_HIST\nsetopt BANG_HIST\nunset HISTFILE\nunset SAVEHIST\n")
time.sleep(0.15)
os.write(fd, first)
time.sleep(0.12)
os.kill(pid, signal.SIGINT)
time.sleep(0.5)
os.write(fd, second)
time.sleep(0.25)
os.write(fd, b"[[ -o banghist ]] && echo BANG_ON || echo BANG_OFF\n")
os.write(fd, b"[[ -v HISTFILE ]] && echo HIST_SET || echo HIST_UNSET\n")
os.write(fd, b"[[ -v SAVEHIST ]] && echo SAVE_SET || echo SAVE_UNSET\n")
os.write(fd, b"[[ -v __warp_silent_cleaned ]] && echo GUARD_SET || echo GUARD_UNSET\n")
os.write(fd, b"typeset -f __warp_silent_cleanup >/dev/null && echo HELPER_SET || echo HELPER_UNSET\n")
os.write(fd, b"stty -a 2>/dev/null | tr ' ' '\n' | grep -E '^-?echo$' | head -1 | sed 's/^-echo/ECHO_OFF/;s/^echo/ECHO_ON/'\n")
os.write(fd, b"echo WARP_SILENT_DONE\nexit\n")
out = b""
deadline = time.time() + 8
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if not ready:
        continue
    try:
        chunk = os.read(fd, 4096)
    except OSError:
        break
    if not chunk:
        break
    out += chunk
    if b"WARP_SILENT_DONE" in out:
        break
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
sys.stdout.buffer.write(out)
"#,
        zsh = zsh.display(),
        home = home.display(),
        first = first,
        second = second,
    );
    let child = Command::new("python3")
        .arg("-c")
        .arg(&runner)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python pty runner");
    let output = child.wait_with_output().expect("wait python pty");
    let _ = std::fs::remove_dir_all(&home);
    let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    let text = String::from_utf8_lossy(&combined);
    assert!(
        text.contains("WARP_SILENT_DONE"),
        "second wrapper must complete: {text:?}"
    );
    assert!(text.contains("ECHO_ON"), "echo restored: {text:?}");
    assert!(text.contains("BANG_ON"), "BANG_HIST restored: {text:?}");
    assert!(
        text.contains("HIST_UNSET"),
        "HISTFILE stays unset: {text:?}"
    );
    assert!(
        text.contains("SAVE_UNSET"),
        "SAVEHIST stays unset: {text:?}"
    );
    assert!(
        text.contains("GUARD_UNSET"),
        "guard must not leak: {text:?}"
    );
    assert!(
        text.contains("HELPER_UNSET"),
        "helper must not leak: {text:?}"
    );
}

#[test]
fn in_band_init_saves_and_restores_history_around_injected_body() {
    let bytes = in_band_init_bytes(ShellType::Bash, SessionId::from(3)).expect("bash init");
    let text = String::from_utf8_lossy(&bytes);
    let history_off = text.find("set +o history").expect("history off");
    let setup_newline = text[history_off..].find('\n').expect("setup command");
    let stty = text.find("stty raw").expect("stty");
    let restore = text.rfind("set -o history").expect("history restore");
    assert!(history_off + setup_newline < stty);
    assert!(text[history_off..stty].contains("HISTCONTROL=ignorespace"));
    assert!(text[..history_off + setup_newline].contains("__warp_histfile"));
    assert!(stty < restore);
    let ack = text.find("9278;t;3").expect("stage ack");
    assert!(stty < ack);
    assert!(ack < restore);
    let zsh = in_band_init_bytes(ShellType::Zsh, SessionId::from(3)).expect("zsh init");
    let zsh_text = String::from_utf8_lossy(&zsh);
    let push = zsh_text.find("fc -p /dev/null").expect("zsh hist push");
    let pop = zsh_text.rfind("fc -P").expect("zsh hist pop");
    let zsh_ack = zsh_text.find("9278;t;3").expect("zsh stage ack");
    assert!(push < zsh_ack);
    assert!(zsh_ack < pop);
    assert!(push < pop);
    let fish = in_band_init_bytes(ShellType::Fish, SessionId::from(3)).expect("fish init");
    let fish_text = String::from_utf8_lossy(&fish);
    let disable = fish_text
        .find("set -g fish_history ''")
        .expect("fish hist off");
    let enable = fish_text
        .rfind("set -g fish_history $__warp_fish_history")
        .expect("fish hist restore");
    let fish_ack = fish_text.find("9278;t;3").expect("fish stage ack");
    assert!(disable < fish_ack);
    assert!(fish_ack < enable);
    assert!(disable < enable);
}

#[cfg(unix)]
fn write_history_script(init: &[u8], user_command: &str, flush: &str) -> Vec<u8> {
    let mut script = init.to_vec();
    script.extend_from_slice(user_command.as_bytes());
    script.push(b'\n');
    script.extend_from_slice(flush.as_bytes());
    script.push(b'\n');
    script.extend_from_slice(b"exit\n");
    script
}

#[cfg(unix)]
fn optional_shell(name: &str) -> Option<PathBuf> {
    warp_util::path::resolve_executable(name).map(|path| path.into_owned())
}

#[cfg(unix)]
fn run_shell_history_script(
    program: &Path,
    args: &[&str],
    env: &[(&str, &Path)],
    script: &[u8],
) -> bool {
    let mut command = Command::new(program);
    command.args(args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env_remove("HISTCONTROL");
    command.env_remove("HISTIGNORE");
    command.env("HISTSIZE", "1000");
    command.env("HISTFILESIZE", "1000");
    command.env("SAVEHIST", "1000");
    for (key, value) in env {
        command.env(key, value);
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, script);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.wait();
    true
}

#[cfg(unix)]
#[test]
fn bash_in_band_init_does_not_persist_bootstrap_in_history() {
    let bash = Path::new("/bin/bash");
    if !bash.exists() {
        return;
    }
    let home = unique_temp_path("warp-tmux-bash-hist-home");
    std::fs::create_dir_all(&home).expect("bash hist home");
    let hist = home.join(".bash_history");
    std::fs::write(&hist, b"").expect("create bash histfile");
    let Some(init) = in_band_init_bytes(ShellType::Bash, SessionId::from(3)) else {
        return;
    };
    let script = write_history_script(&init, "echo warp-tmux-user-cmd", "history -a");
    if !run_shell_history_script(
        bash,
        &["--noprofile", "--norc", "-i"],
        &[("HOME", home.as_path()), ("HISTFILE", hist.as_path())],
        &script,
    ) {
        return;
    }
    let hist_bytes = std::fs::read(&hist).unwrap_or_default();
    let entries = ShellType::Bash.parse_history(&hist_bytes);
    let joined = entries.join("\n");
    assert!(
        !joined.contains("__warp_histfile")
            && !joined.contains("InitShell")
            && !joined.contains("9278;t;"),
        "bootstrap must not be saved: {joined:?}"
    );
    assert!(
        joined.contains("warp-tmux-user-cmd"),
        "following user command must be saved: {joined:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn zsh_in_band_init_does_not_persist_bootstrap_in_history() {
    let Some(zsh) = optional_shell("zsh") else {
        return;
    };
    let home = unique_temp_path("warp-tmux-zsh-hist-home");
    std::fs::create_dir_all(&home).expect("zsh hist home");
    let hist = home.join(".zsh_history");
    std::fs::write(&hist, b"").expect("create zsh histfile");
    let Some(init_bytes) = in_band_init_bytes(ShellType::Zsh, SessionId::from(3)) else {
        return;
    };
    let init = String::from_utf8_lossy(&init_bytes);
    let command = format!(
        "HISTFILE={hist};SAVEHIST=1000;{init}echo warp-tmux-user-cmd;fc -W",
        hist = hist.display(),
        init = init,
    );
    let status = Command::new(&zsh)
        .args(["--no-rcs", "-c", &command])
        .env("HOME", &home)
        .env("HISTFILE", &hist)
        .env("SAVEHIST", "1000")
        .status();
    if !status.map(|s| s.success()).unwrap_or(false) {
        let _ = std::fs::remove_dir_all(&home);
        return;
    }
    let hist_bytes = std::fs::read(&hist).unwrap_or_default();
    if hist_bytes.is_empty() {
        let _ = std::fs::remove_dir_all(&home);
        return;
    }
    let entries = ShellType::Zsh.parse_history(&hist_bytes);
    let joined = entries.join("\n");
    assert!(
        !joined.contains("__warp_histfile")
            && !joined.contains("InitShell")
            && !joined.contains("9278;t;"),
        "bootstrap must not be saved: {joined:?}"
    );
    assert!(
        joined.contains("warp-tmux-user-cmd"),
        "following user command must be saved: {joined:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn fish_in_band_init_does_not_persist_bootstrap_in_history() {
    let Some(fish) = optional_shell("fish") else {
        return;
    };
    let home = unique_temp_path("warp-tmux-fish-hist-home");
    let data = home.join("data");
    std::fs::create_dir_all(data.join("fish")).expect("fish hist home");
    let Some(init_bytes) = in_band_init_bytes(ShellType::Fish, SessionId::from(3)) else {
        return;
    };
    let init = String::from_utf8_lossy(&init_bytes);
    let command = format!("{init}; echo warp-tmux-user-cmd; history save");
    let status = Command::new(&fish)
        .args(["--no-config", "-c", &command])
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &data)
        .status();
    if !status.map(|s| s.success()).unwrap_or(false) {
        let _ = std::fs::remove_dir_all(&home);
        return;
    }
    let hist = data.join("fish").join("fish_history");
    let hist_bytes = std::fs::read(&hist).unwrap_or_default();
    if hist_bytes.is_empty() {
        let _ = std::fs::remove_dir_all(&home);
        return;
    }
    let entries = ShellType::Fish.parse_history(&hist_bytes);
    let joined = entries.join("\n");
    assert!(
        !joined.contains("__warp_fish_history")
            && !joined.contains("InitShell")
            && !joined.contains("9278;t;"),
        "bootstrap must not be saved: {joined:?}"
    );
    assert!(
        joined.contains("warp-tmux-user-cmd"),
        "following user command must be saved: {joined:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
fn run_shell_capture(
    program: &Path,
    args: &[&str],
    env: &[(&str, String)],
    script: &[u8],
) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = std::io::Write::write_all(&mut stdin, script);
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(unix)]
#[test]
fn bash_history_restore_preserves_enabled_and_disabled_state() {
    let bash = Path::new("/bin/bash");
    if !bash.exists() {
        return;
    }
    let init = super::history_isolated_script(ShellType::Bash, b":");
    let dump = b"if shopt -qo history; then echo hist_on=1; else echo hist_on=0; fi; if [ \"${HISTFILE+x}\" ]; then echo histfile_set=1 histfile=$HISTFILE; else echo histfile_set=0; fi\n";
    let mut enabled = b"set -o history\nHISTFILE=/tmp/warp-tmux-hist-enabled\n".to_vec();
    enabled.extend_from_slice(&init);
    enabled.extend_from_slice(dump);
    let out =
        run_shell_capture(bash, &["--noprofile", "--norc"], &[], &enabled).unwrap_or_default();
    assert!(
        out.contains("hist_on=1"),
        "enabled history must be restored: {out:?}"
    );
    assert!(
        out.contains("histfile_set=1"),
        "HISTFILE set must be restored: {out:?}"
    );
    assert!(
        out.contains("histfile=/tmp/warp-tmux-hist-enabled"),
        "{out:?}"
    );

    let mut disabled = b"set +o history\nunset HISTFILE\n".to_vec();
    disabled.extend_from_slice(&init);
    disabled.extend_from_slice(dump);
    let out =
        run_shell_capture(bash, &["--noprofile", "--norc"], &[], &disabled).unwrap_or_default();
    assert!(
        out.contains("hist_on=0"),
        "disabled history must stay off: {out:?}"
    );
    assert!(
        out.contains("histfile_set=0"),
        "unset HISTFILE must stay unset: {out:?}"
    );
}

#[cfg(unix)]
#[test]
fn zsh_history_restore_preserves_histfile_and_savehist() {
    let Some(zsh) = optional_shell("zsh") else {
        return;
    };
    let init =
        String::from_utf8_lossy(&super::history_isolated_script(ShellType::Zsh, b":")).into_owned();
    let dump = "if (( ${+HISTFILE} )); then echo histfile_set=1 histfile=$HISTFILE; else echo histfile_set=0; fi; if (( ${+SAVEHIST} )); then echo savehist_set=1 savehist=$SAVEHIST; else echo savehist_set=0; fi\n";
    let set_cmd = format!("HISTFILE=/tmp/warp-tmux-zsh-hist; SAVEHIST=42; {init}{dump}");
    let out = run_shell_capture(&zsh, &["--no-rcs", "-c", &set_cmd], &[], b"").unwrap_or_default();
    assert!(
        out.contains("histfile_set=1"),
        "HISTFILE set must be restored: {out:?}"
    );
    assert!(out.contains("histfile=/tmp/warp-tmux-zsh-hist"), "{out:?}");
    assert!(
        out.contains("savehist_set=1"),
        "SAVEHIST set must be restored: {out:?}"
    );
    assert!(out.contains("savehist=42"), "{out:?}");

    let unset_cmd = format!("unset HISTFILE; unset SAVEHIST; {init}{dump}");
    let out =
        run_shell_capture(&zsh, &["--no-rcs", "-c", &unset_cmd], &[], b"").unwrap_or_default();
    assert!(
        out.contains("histfile_set=0"),
        "unset HISTFILE must stay unset: {out:?}"
    );
    assert!(
        out.contains("savehist_set=0"),
        "unset SAVEHIST must stay unset: {out:?}"
    );
}

#[cfg(unix)]
#[test]
fn fish_history_restore_preserves_unset_and_global_set() {
    let Some(fish) = optional_shell("fish") else {
        return;
    };
    let init = String::from_utf8_lossy(&super::history_isolated_script(ShellType::Fish, b"true"))
        .into_owned();
    let dump = "if set -q fish_history; echo fish_history_set=1 fish_history=$fish_history; else; echo fish_history_set=0; end\n";
    let set_cmd = format!("set -g fish_history default; {init}; {dump}");
    let out =
        run_shell_capture(&fish, &["--no-config", "-c", &set_cmd], &[], b"").unwrap_or_default();
    assert!(
        out.contains("fish_history_set=1"),
        "set fish_history must be restored: {out:?}"
    );
    assert!(out.contains("fish_history=default"), "{out:?}");

    let unset_cmd = format!("set -e fish_history; {init}; {dump}");
    let out =
        run_shell_capture(&fish, &["--no-config", "-c", &unset_cmd], &[], b"").unwrap_or_default();
    assert!(
        out.contains("fish_history_set=0"),
        "unset fish_history must stay unset: {out:?}"
    );
}
