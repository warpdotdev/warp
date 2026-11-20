use std::ffi::OsStr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

use super::*;

fn run_sh(script: &str, args: &[&str]) -> ExitStatus {
    let set_disclaim = set_disclaim_fn().expect("the disclaim symbol is available");
    match spawn_disclaimed_and_wait(Path::new("/bin/sh"), &sh_args(script, args), set_disclaim) {
        DisclaimedSpawn::Exited(status) => status,
        other => panic!("expected the shell to run, got {other:?}"),
    }
}

fn sh_args<'a>(script: &'a str, args: &'a [&'a str]) -> Vec<&'a OsStr> {
    let mut all_args: Vec<&OsStr> = vec![OsStr::new("-c"), OsStr::new(script), OsStr::new("sh")];
    all_args.extend(args.iter().map(OsStr::new));
    all_args
}

unsafe extern "C" fn failing_set_disclaim(
    _attr: *mut libc::posix_spawnattr_t,
    _disclaim: c_int,
) -> c_int {
    libc::ENOTSUP
}

#[test]
fn the_disclaim_symbol_resolves() {
    assert!(set_disclaim_fn().is_some());
}

#[test]
fn disclaimed_spawn_passes_args_and_reports_the_exit_status() {
    assert!(run_sh(r#"test "$1" = --daemon"#, &["--daemon"]).success());
    assert_eq!(run_sh("exit 3", &[]).code(), Some(3));
}

#[test]
fn disclaimed_spawn_inherits_the_environment() {
    let key = "WARP_DISCLAIM_SPAWN_TEST";
    // SAFETY: no other test reads or writes this variable.
    unsafe { std::env::set_var(key, "example-value") };
    let status = run_sh(r#"test "$WARP_DISCLAIM_SPAWN_TEST" = example-value"#, &[]);
    unsafe { std::env::remove_var(key) };
    assert!(status.success());
}

#[test]
fn a_failing_disclaim_launches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let script = format!("touch {}", marker.display());
    let outcome = spawn_disclaimed_and_wait(
        Path::new("/bin/sh"),
        &sh_args(&script, &[]),
        failing_set_disclaim,
    );
    assert!(
        matches!(&outcome, DisclaimedSpawn::NotLaunched(err)
            if err.to_string().contains("responsibility_spawnattrs_setdisclaim")),
        "{outcome:?}"
    );
    assert!(!marker.exists());
}

#[test]
fn a_missing_program_launches_nothing() {
    let set_disclaim = set_disclaim_fn().expect("the disclaim symbol is available");
    let outcome =
        spawn_disclaimed_and_wait(Path::new("/nonexistent/example-daemon"), &[], set_disclaim);
    assert!(
        matches!(outcome, DisclaimedSpawn::NotLaunched(_)),
        "{outcome:?}"
    );
}

#[test]
fn disclaimed_spawn_has_null_stdin_and_no_inherited_fds() {
    // SAFETY: `dup` returns a new fd without close-on-exec, owned here.
    let inherited = unsafe { OwnedFd::from_raw_fd(libc::dup(libc::STDERR_FILENO)) };
    let fd = inherited.as_raw_fd().to_string();
    let status = run_sh(
        r#"if read -r _; then exit 1; fi; if [ -e "/dev/fd/$1" ]; then exit 2; fi; exit 0"#,
        &[&fd],
    );
    assert_eq!(status.code(), Some(0));
}
