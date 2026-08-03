use std::path::Path;

use super::{WslGitCommand, build_wslenv, translate_for_wsl_unc_cwd};

/// Translates a git command in `cwd`, asserting that the working directory
/// qualified for the WSL rewrite.
fn translate(args: &[&str], cwd: &str, env: &[(&str, &str)]) -> WslGitCommand {
    translate_for_wsl_unc_cwd(args, Path::new(cwd), env).expect("expected translation")
}

#[test]
fn translates_git_in_unc_cwd() {
    let translated = translate(&["status", "--short"], r"\\wsl$\Ubuntu\home\user\repo", &[]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "status",
            "--short",
        ]
    );
    assert_eq!(translated.wslenv, None);
}

#[test]
fn does_not_translate_non_unc_cwd() {
    assert_eq!(
        translate_for_wsl_unc_cwd(&["status"], Path::new(r"C:\Users\user\repo"), &[]),
        None
    );
    assert_eq!(
        translate_for_wsl_unc_cwd(&["status"], Path::new("/home/user/repo"), &[]),
        None
    );
}

#[test]
fn rewrites_same_distro_unc_argument_to_linux_path() {
    let translated = translate(
        &["-C", r"\\wsl$\Ubuntu\home\user\other"],
        r"\\wsl$\Ubuntu\home\user\repo",
        &[],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            "/home/user/other",
        ]
    );
}

#[test]
fn rewrites_argument_with_case_insensitive_distro_match() {
    // The cwd distribution is `Ubuntu`; an argument spelled `ubuntu` refers to
    // the same distribution and must still be converted to its Linux path.
    let translated = translate(
        &["-C", r"\\wsl$\ubuntu\home\user\other"],
        r"\\wsl$\Ubuntu\home\user\repo",
        &[],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            "/home/user/other",
        ]
    );
}

#[test]
fn leaves_other_distro_unc_argument_unchanged() {
    let other = r"\\wsl$\Debian\home\user\other";
    let translated = translate(&["-C", other], r"\\wsl$\Ubuntu\home\user\repo", &[]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            other,
        ]
    );
}

#[test]
fn build_wslenv_excludes_path_case_insensitively() {
    // `PATH` in any spelling is dropped so a Linux-form `PATH` is never handed
    // to `wsl.exe` through `WSLENV`; other keys are kept and suffixed with `/u`.
    assert_eq!(
        build_wslenv(&[("PATH", "/usr/bin"), ("GIT_OPTIONAL_LOCKS", "0")]),
        Some("GIT_OPTIONAL_LOCKS/u".to_string())
    );
    assert_eq!(
        build_wslenv(&[("Path", "/usr/bin"), ("GIT_AUTHOR_NAME", "Ada")]),
        Some("GIT_AUTHOR_NAME/u".to_string())
    );
    assert_eq!(build_wslenv(&[("path", "/usr/bin")]), None);
    assert_eq!(build_wslenv(&[]), None);
}

#[test]
fn builds_wslenv_from_env_keys() {
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_AUTHOR_NAME", "Ada"), ("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(
        translated.wslenv,
        Some("GIT_AUTHOR_NAME/u:GIT_OPTIONAL_LOCKS/u".to_string())
    );
}

#[test]
fn omits_wslenv_when_no_env_keys() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[]);

    assert_eq!(translated.wslenv, None);
}

#[test]
fn carries_explicit_path_through_argv() {
    // A caller-supplied `PATH` is threaded into the distribution as
    // `env PATH=<value>` in front of `git`, and must not leak into `WSLENV`.
    // The `PATH` already resolves `git`, so no login shell is needed.
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("PATH", "/usr/local/bin:/usr/bin")],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/usr/local/bin:/usr/bin",
            "git",
            "commit",
        ]
    );
    assert_eq!(translated.wslenv, None);
}

#[test]
fn carries_case_insensitive_path_through_argv() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[("Path", "/opt/bin")]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/opt/bin",
            "git",
            "status",
        ]
    );
}

#[test]
fn routes_through_login_shell_when_no_path() {
    // Without an explicit `PATH`, `git` is resolved by a login shell inside the
    // distribution — `wsl.exe --exec` alone only searches a minimal default
    // `PATH`. Other variables still travel via `WSLENV`.
    let translated = translate(
        &["status"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "status",
        ]
    );
    assert_eq!(translated.wslenv, Some("GIT_OPTIONAL_LOCKS/u".to_string()));
}
