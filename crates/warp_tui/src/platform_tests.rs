#[cfg(target_os = "windows")]
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use super::*;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn build_reveal_command_uses_platform_file_manager() {
    let path = Path::new("/tmp/warp-logs.zip");
    let command = build_reveal_command(path);

    #[cfg(target_os = "macos")]
    {
        let command = command.unwrap();
        assert_eq!(command.get_program(), "open");
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_owned())
                .collect::<Vec<_>>(),
            vec!["-R".into(), path.as_os_str().to_owned()]
        );
    }
    #[cfg(target_os = "windows")]
    {
        let command = command.unwrap();
        assert_eq!(command.get_program(), "explorer");
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_owned())
                .collect::<Vec<_>>(),
            vec![OsString::from(format!("/select,{}", path.display()))]
        );
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let command = command.unwrap();
        assert_eq!(command.get_program(), "xdg-open");
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_owned())
                .collect::<Vec<_>>(),
            vec![path.parent().unwrap().as_os_str().to_owned()]
        );
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    assert!(command.is_none());
}

#[test]
fn should_attempt_reveal_skips_ssh_sessions() {
    let _guard = env_lock().lock().unwrap();
    let connection = std::env::var_os("SSH_CONNECTION");
    let tty = std::env::var_os("SSH_TTY");

    unsafe {
        std::env::remove_var("SSH_CONNECTION");
        std::env::remove_var("SSH_TTY");
    }
    assert!(should_attempt_reveal());

    unsafe {
        std::env::set_var("SSH_CONNECTION", "host 1234 5678 22");
    }
    assert!(!should_attempt_reveal());
    reveal_path_in_file_manager(Path::new("/tmp/warp-logs.zip"));

    unsafe {
        std::env::remove_var("SSH_CONNECTION");
        std::env::set_var("SSH_TTY", "/dev/pts/0");
    }
    assert!(!should_attempt_reveal());
    reveal_path_in_file_manager(Path::new("/tmp/warp-logs.zip"));

    unsafe {
        std::env::remove_var("SSH_CONNECTION");
        std::env::remove_var("SSH_TTY");
    }
    assert!(should_attempt_reveal());

    match connection {
        Some(value) => unsafe { std::env::set_var("SSH_CONNECTION", value) },
        None => unsafe { std::env::remove_var("SSH_CONNECTION") },
    }
    match tty {
        Some(value) => unsafe { std::env::set_var("SSH_TTY", value) },
        None => unsafe { std::env::remove_var("SSH_TTY") },
    }
}
