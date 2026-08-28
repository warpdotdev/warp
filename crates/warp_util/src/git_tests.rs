use std::path::Path;

use command::blocking::Command;

use super::{
    WslGitCommand, build_wslenv, run_git_command, run_git_command_bytes, translate_for_wsl_unc_cwd,
};

/// Translates a git command in `cwd`, asserting that the working directory qualified for the WSL
/// rewrite.
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
    assert_eq!(translated.wslenv, "");
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
    assert_eq!(
        build_wslenv(&[("PATH", "/usr/bin"), ("GIT_OPTIONAL_LOCKS", "0")]),
        "GIT_OPTIONAL_LOCKS/u"
    );
    assert_eq!(
        build_wslenv(&[("Path", "/usr/bin"), ("GIT_AUTHOR_NAME", "Ada")]),
        "GIT_AUTHOR_NAME/u"
    );
    assert_eq!(build_wslenv(&[("path", "/usr/bin")]), "");
    assert_eq!(build_wslenv(&[]), "");
}

#[test]
fn builds_wslenv_from_env_keys() {
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_AUTHOR_NAME", "Ada"), ("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(translated.wslenv, "GIT_AUTHOR_NAME/u:GIT_OPTIONAL_LOCKS/u");
}

#[test]
fn omits_wslenv_when_no_env_keys() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[]);

    assert_eq!(translated.wslenv, "");
}

#[test]
fn carries_explicit_path_through_argv() {
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
    assert_eq!(translated.wslenv, "");
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
    assert_eq!(translated.wslenv, "GIT_OPTIONAL_LOCKS/u");
}

/// Bytes that are intentionally not valid UTF-8 (PNG magic followed by
/// continuation bytes), so a lossy string decode would corrupt them.
const BINARY_BYTES: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFE, 0x00, 0x80, 0xC3, 0x28,
];

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn run_git_command_bytes_round_trips_binary_blobs() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let repo = dir.path();

    git(repo, &["init", "--quiet"]);
    std::fs::write(repo.join("img.png"), BINARY_BYTES).expect("write binary file");
    git(repo, &["add", "img.png"]);
    git(
        repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "add binary",
        ],
    );

    let bytes =
        futures_lite::future::block_on(run_git_command_bytes(repo, &["show", "HEAD:img.png"]))
            .expect("git show should succeed");
    assert_eq!(bytes, BINARY_BYTES);

    // The string variant lossily decodes the same blob, corrupting it — the
    // reason image bytes must go through `run_git_command_bytes`.
    let lossy = futures_lite::future::block_on(run_git_command(repo, &["show", "HEAD:img.png"]))
        .expect("git show should succeed");
    assert_ne!(lossy.as_bytes(), BINARY_BYTES);
}

#[test]
fn run_git_command_bytes_errors_on_missing_blob() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let repo = dir.path();
    git(repo, &["init", "--quiet"]);

    let result =
        futures_lite::future::block_on(run_git_command_bytes(repo, &["show", "HEAD:missing.png"]));
    assert!(result.is_err());
}
