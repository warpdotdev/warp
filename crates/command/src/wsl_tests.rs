use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::fs;
#[cfg(windows)]
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use futures_lite::future;
#[cfg(windows)]
use instant::Instant;

#[cfg(windows)]
use super::{BACKGROUND_COMMAND_BACKOFF, BackgroundWslCommandError, output_background_command};
use super::{known_bare_name, resolve_binary_in_wsl_safe_path};
#[cfg(windows)]
use crate::r#async::{Command, OutputError};

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(unix)]
fn write_exec(path: &std::path::Path) {
    fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
    make_executable(path);
}

#[cfg(unix)]
fn join(parts: &[PathBuf]) -> OsString {
    std::env::join_paths(parts).unwrap()
}

#[cfg(unix)]
#[test]
fn picks_first_linux_path_when_wsl() {
    let linux_dir = tempfile::tempdir().unwrap();
    write_exec(&linux_dir.path().join("git"));

    // `/mnt/c/...` paths in the synthetic PATH that don't exist on
    // disk simulate the WSL-with-Windows-git layout: the resolver
    // should skip them on the prefix and never stat them.
    let path_env = join(&[
        PathBuf::from("/mnt/c/Program Files/Git/cmd"),
        linux_dir.path().to_path_buf(),
    ]);

    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, linux_dir.path().join("git"));
    assert!(!resolved.starts_with("/mnt"));
}

#[cfg(unix)]
#[test]
fn passes_through_first_match_when_not_wsl() {
    // When not WSL, `/mnt/...` is just another directory; the resolver
    // should pick the first dir on PATH that contains an exec match.
    let mnt_dir = tempfile::tempdir().unwrap();
    write_exec(&mnt_dir.path().join("git"));
    let other_dir = tempfile::tempdir().unwrap();
    write_exec(&other_dir.path().join("git"));

    let path_env = join(&[mnt_dir.path().to_path_buf(), other_dir.path().to_path_buf()]);

    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), false).unwrap();
    assert_eq!(resolved, mnt_dir.path().join("git"));
}

#[cfg(unix)]
#[test]
fn falls_back_to_none_when_only_mnt_has_git() {
    let path_env = join(&[
        PathBuf::from("/mnt/c/Program Files/Git/cmd"),
        PathBuf::from("/mnt/c/Windows/System32"),
    ]);
    assert!(resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).is_none());
}

#[cfg(unix)]
#[test]
fn picks_user_local_bin() {
    let bin_dir = tempfile::tempdir().unwrap();
    let empty_dir = bin_dir.path().join("empty");
    fs::create_dir_all(&empty_dir).unwrap();
    let local_bin = bin_dir.path().join("home/.local/bin");
    fs::create_dir_all(&local_bin).unwrap();
    write_exec(&local_bin.join("git"));

    // PATH order: an empty dir, then a `/mnt/...` candidate that
    // should be skipped on WSL, then the directory with the real
    // exec. The resolver must walk past the first two and land on
    // the third.
    let path_env = join(&[
        empty_dir,
        PathBuf::from("/mnt/c/Program Files/Git/cmd"),
        local_bin.clone(),
    ]);
    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, local_bin.join("git"));
}

#[cfg(unix)]
#[test]
fn follows_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("git-wrapper");
    write_exec(&real);
    let link = dir.path().join("git");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let path_env = join(&[dir.path().to_path_buf()]);
    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, link);
}

#[cfg(unix)]
#[test]
fn skips_non_executable() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("git"), b"not exec").unwrap();
    // Intentionally do NOT chmod +x.

    let path_env = join(&[dir.path().to_path_buf()]);
    assert!(resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).is_none());
}

#[test]
fn handles_empty_path_env() {
    assert!(resolve_binary_in_wsl_safe_path("git", None, true).is_none());
    assert!(resolve_binary_in_wsl_safe_path("git", None, false).is_none());
}

#[cfg(unix)]
#[test]
fn handles_non_utf8_path_components() {
    use std::os::unix::ffi::OsStringExt as _;

    let dir = tempfile::tempdir().unwrap();
    write_exec(&dir.path().join("git"));

    // Build a PATH whose first component is a non-UTF-8 byte sequence,
    // followed by a real directory. The resolver must walk past the
    // garbage entry without panicking and find the valid one.
    let mut bytes = b"/\xff\xfe/bad:".to_vec();
    bytes.extend_from_slice(dir.path().as_os_str().as_encoded_bytes());
    let path_env = OsString::from_vec(bytes);

    let resolved =
        resolve_binary_in_wsl_safe_path("git", Some(path_env.as_os_str()), true).unwrap();
    assert_eq!(resolved, dir.path().join("git"));
}

#[test]
fn known_bare_name_recognizes_git_and_gh() {
    assert_eq!(known_bare_name(OsStr::new("git")), Some("git"));
    assert_eq!(known_bare_name(OsStr::new("gh")), Some("gh"));
}

#[test]
fn known_bare_name_skips_paths() {
    assert_eq!(known_bare_name(OsStr::new("/usr/bin/git")), None);
    assert_eq!(known_bare_name(OsStr::new("./git")), None);
    assert_eq!(known_bare_name(OsStr::new("bin/git")), None);
    #[cfg(windows)]
    assert_eq!(known_bare_name(OsStr::new("C:\\git\\git.exe")), None);
}

#[test]
fn known_bare_name_skips_unknowns() {
    assert_eq!(known_bare_name(OsStr::new("ls")), None);
    assert_eq!(known_bare_name(OsStr::new("python")), None);
    assert_eq!(known_bare_name(OsStr::new("")), None);
}

#[cfg(windows)]
fn hanging_command(pid_file: &Path) -> Command {
    let pid_file = pid_file.to_string_lossy().replace('\'', "''");
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!("Set-Content -LiteralPath '{pid_file}' -Value $PID; Start-Sleep -Seconds 300"),
    ]);
    command
}

#[cfg(windows)]
fn marker_command(marker_file: &Path) -> Command {
    let marker_file = marker_file.to_string_lossy().replace('\'', "''");
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!("Set-Content -LiteralPath '{marker_file}' -Value spawned"),
    ]);
    command
}

#[cfg(windows)]
async fn wait_for_pid_file(pid_file: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(pid) = std::fs::read_to_string(pid_file)
            && let Ok(pid) = pid.trim().parse()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for helper process to record its pid"
        );
        async_io::Timer::after(Duration::from_millis(25)).await;
    }
}

#[cfg(windows)]
async fn wait_for_process_exit(process_id: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut command = Command::new("tasklist.exe");
        command.args(["/FI", &format!("PID eq {process_id}"), "/NH", "/FO", "CSV"]);
        let output = command.output().await.expect("inspect helper process");
        if !String::from_utf8_lossy(&output.stdout).contains(&process_id.to_string()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "helper process {process_id} was not terminated"
        );
        async_io::Timer::after(Duration::from_millis(25)).await;
    }
}

#[cfg(windows)]
async fn assert_backoff_then_recovery(distribution: &str, temp_dir: &Path) {
    let blocked_marker = temp_dir.join("blocked");
    let mut blocked_command = marker_command(&blocked_marker);
    assert!(matches!(
        output_background_command(&mut blocked_command, distribution).await,
        Err(BackgroundWslCommandError::BackingOff)
    ));
    assert!(!blocked_marker.exists());

    async_io::Timer::after(BACKGROUND_COMMAND_BACKOFF + Duration::from_millis(100)).await;

    let admitted_marker = temp_dir.join("admitted");
    let mut admitted_command = marker_command(&admitted_marker);
    let output = output_background_command(&mut admitted_command, distribution)
        .await
        .expect("command should be admitted after backoff");
    assert!(output.status.success());
    assert!(admitted_marker.exists());
}

#[cfg(windows)]
#[test]
fn public_wrapper_suppresses_overlap_times_out_reaps_and_recovers() {
    future::block_on(async {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let distribution = format!("timeout-{}", std::process::id());
        let pid_file = temp_dir.path().join("pid");
        let mut hanging_command = hanging_command(&pid_file);
        let mut pending = Box::pin(output_background_command(
            &mut hanging_command,
            &distribution,
        ));

        let immediate = future::poll_once(&mut pending).await;
        assert!(
            immediate.is_none(),
            "hanging command completed immediately: {immediate:?}"
        );
        let process_id = wait_for_pid_file(&pid_file).await;

        let overlap_marker = temp_dir.path().join("overlap");
        let mut overlap_command = marker_command(&overlap_marker);
        assert!(matches!(
            output_background_command(&mut overlap_command, &distribution).await,
            Err(BackgroundWslCommandError::AlreadyRunning)
        ));
        assert!(!overlap_marker.exists());

        let error = pending.await.expect_err("hanging command should time out");
        assert!(matches!(
            error,
            BackgroundWslCommandError::Output(OutputError::TimedOut {
                process_id: timed_out_process_id,
                ..
            }) if timed_out_process_id == process_id
        ));
        wait_for_process_exit(process_id).await;
        assert_backoff_then_recovery(&distribution, temp_dir.path()).await;
    });
}

#[cfg(windows)]
#[test]
fn public_wrapper_cancellation_reaps_and_backs_off() {
    future::block_on(async {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let distribution = format!("canceled-{}", std::process::id());
        let pid_file = temp_dir.path().join("pid");
        let mut hanging_command = hanging_command(&pid_file);
        let mut pending = Box::pin(output_background_command(
            &mut hanging_command,
            &distribution,
        ));

        let immediate = future::poll_once(&mut pending).await;
        assert!(
            immediate.is_none(),
            "hanging command completed immediately: {immediate:?}"
        );
        let process_id = wait_for_pid_file(&pid_file).await;
        drop(pending);

        wait_for_process_exit(process_id).await;
        assert_backoff_then_recovery(&distribution, temp_dir.path()).await;
    });
}
