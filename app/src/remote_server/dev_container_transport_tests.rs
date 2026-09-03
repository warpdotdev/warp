use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use remote_server::manager::RemoteServerExitStatus;
use remote_server::transport::RemoteTransport;
use warpui::r#async::BoxFuture;

use super::*;

fn static_auth_context() -> Arc<RemoteServerAuthContext> {
    Arc::new(RemoteServerAuthContext::new(
        || -> BoxFuture<'static, Option<String>> { Box::pin(async { None }) },
        || "user id/with spaces".to_string(),
        String::new(),
        String::new(),
        true,
    ))
}

fn test_transport() -> DevContainerTransport {
    DevContainerTransport::new(
        PathBuf::from("/usr/bin/docker"),
        "abc123".to_string(),
        Some("vscode".to_string()),
        "/workspaces/project".to_string(),
        static_auth_context(),
    )
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn command_args_propagate_user_and_workdir_without_tty() {
    let transport = test_transport();
    assert_eq!(
        transport.command_args("uname -sm"),
        os(&[
            "exec",
            "-u",
            "vscode",
            "-w",
            "/workspaces/project",
            "abc123",
            "sh",
            "-c",
            "uname -sm",
        ])
    );
}

#[test]
fn command_args_omit_user_when_unset() {
    let transport = DevContainerTransport::new(
        PathBuf::from("/usr/bin/docker"),
        "abc123".to_string(),
        None,
        "/workspaces/project".to_string(),
        static_auth_context(),
    );
    let args = transport.command_args("uname -sm");
    assert!(!args.iter().any(|arg| arg == "-u"));
    assert_eq!(
        args,
        os(&[
            "exec",
            "-w",
            "/workspaces/project",
            "abc123",
            "sh",
            "-c",
            "uname -sm",
        ])
    );
}

#[test]
fn proxy_args_are_interactive_without_tty_and_quote_identity_key() {
    let transport = test_transport();
    let args = transport.proxy_args();
    assert!(args.contains(&OsString::from("-i")));
    assert!(!args.iter().any(|arg| arg == "-t" || arg == "-it"));
    let command = args
        .last()
        .expect("proxy args include a command")
        .to_string_lossy();
    assert!(command.contains("remote-server-proxy --identity-key"));
    assert!(command.contains("'user id/with spaces'"));
}

#[test]
fn script_args_use_interactive_bash_stdin() {
    let transport = test_transport();
    assert_eq!(
        transport.script_args(),
        os(&[
            "exec",
            "-i",
            "-u",
            "vscode",
            "-w",
            "/workspaces/project",
            "abc123",
            "bash",
            "-s",
        ])
    );
}

#[test]
fn cp_args_target_container_path_without_credentials() {
    let args = DevContainerTransport::cp_args(
        "abc123",
        Path::new("/tmp/oz.tar.gz"),
        "/home/vscode/.warp/remote-server/oz-upload.tar.gz",
    );
    assert_eq!(
        args,
        os(&[
            "cp",
            "/tmp/oz.tar.gz",
            "abc123:/home/vscode/.warp/remote-server/oz-upload.tar.gz",
        ])
    );
    let joined = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!joined.contains("user id/with spaces"));
    assert!(!joined.contains("Bearer"));
}

#[test]
fn debug_fmt_omits_auth_context() {
    let debug = format!("{:?}", test_transport());
    assert!(debug.contains("abc123"));
    assert!(!debug.contains("user id/with spaces"));
}

#[test]
fn exec_argv_passes_home_and_tarball_paths_without_shell() {
    let transport = test_transport();
    let install_dir = "/home/user with spaces/o'brien/.warp/remote-server";
    let tarball = "/home/user with spaces/o'brien/.warp/remote-server/oz-upload-;rm.tar.gz";
    let mkdir_args = transport.exec_argv(["mkdir", "-p", "--", install_dir]);
    assert_eq!(
        mkdir_args,
        os(&[
            "exec",
            "-u",
            "vscode",
            "-w",
            "/workspaces/project",
            "abc123",
            "mkdir",
            "-p",
            "--",
            install_dir,
        ])
    );
    assert!(!mkdir_args.iter().any(|arg| arg == "sh" || arg == "-c"));

    let rm_args = transport.exec_argv(["rm", "-f", "--", tarball]);
    assert_eq!(
        rm_args.last().map(|arg| arg.as_os_str()),
        Some(std::ffi::OsStr::new(tarball))
    );
    assert!(!rm_args.iter().any(|arg| arg == "sh" || arg == "-c"));

    let script_args = transport.script_args_with_positional([tarball]);
    assert_eq!(
        script_args,
        os(&[
            "exec",
            "-i",
            "-u",
            "vscode",
            "-w",
            "/workspaces/project",
            "abc123",
            "bash",
            "-s",
            "--",
            tarball,
        ])
    );
}

#[test]
fn cp_args_preserve_spaces_and_metacharacters() {
    let container_path = "/home/user with spaces/.warp/remote-server/oz-upload-$HOME.tar.gz";
    let args = DevContainerTransport::cp_args(
        "abc123",
        Path::new("/tmp/oz with spaces.tar.gz"),
        container_path,
    );
    assert_eq!(
        args,
        os(&[
            "cp",
            "/tmp/oz with spaces.tar.gz",
            &format!("abc123:{container_path}"),
        ])
    );
}

#[test]
fn is_reconnectable_rejects_docker_cli_failure_and_signal_kill() {
    let transport = test_transport();
    assert!(!transport.is_reconnectable(Some(&RemoteServerExitStatus {
        code: Some(125),
        signal_killed: false,
    })));
    assert!(!transport.is_reconnectable(Some(&RemoteServerExitStatus {
        code: Some(1),
        signal_killed: true,
    })));
    assert!(transport.is_reconnectable(Some(&RemoteServerExitStatus {
        code: Some(1),
        signal_killed: false,
    })));
    assert!(transport.is_reconnectable(None));
}
