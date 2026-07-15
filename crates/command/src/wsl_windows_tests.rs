use std::ffi::{OsStr, OsString};
use std::path::Path;

use super::{
    TranslatedCommand, WslUncPath, build_wslenv, parse_wsl_unc_path, translate_for_wsl_unc_cwd,
};

/// Builds the owned `OsString` argument vector the translator expects
/// from a slice of string literals.
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Builds the owned `(key, value)` environment slice the translator
/// expects from a slice of string-literal pairs.
fn env_pairs(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect()
}

/// Asserts that `input` parses into the given distribution and Linux
/// path.
fn assert_parses(input: &str, distro: &str, linux_path: &str) {
    assert_eq!(
        parse_wsl_unc_path(Path::new(input)),
        Some(WslUncPath {
            distro: distro.to_string(),
            linux_path: linux_path.to_string(),
        }),
        "input: {input:?}"
    );
}

#[test]
fn parses_backslash_form() {
    assert_parses(r"\\wsl$\Ubuntu\home\user", "Ubuntu", "/home/user");
}

#[test]
fn parses_wsl_localhost_form() {
    assert_parses(r"\\wsl.localhost\Ubuntu\home", "Ubuntu", "/home");
}

#[test]
fn parses_verbatim_form() {
    assert_parses(r"\\?\UNC\wsl$\Ubuntu\home\user", "Ubuntu", "/home/user");
}

#[test]
fn parses_verbatim_wsl_localhost_form() {
    assert_parses(
        r"\\?\UNC\wsl.localhost\Ubuntu\srv\repo",
        "Ubuntu",
        "/srv/repo",
    );
}

#[test]
fn parses_forward_slash_form() {
    assert_parses("//wsl$/Ubuntu/home/user", "Ubuntu", "/home/user");
}

#[test]
fn parses_host_case_insensitively() {
    assert_parses(r"\\WSL$\Ubuntu\src", "Ubuntu", "/src");
    assert_parses(r"\\Wsl.LocalHost\Ubuntu\src", "Ubuntu", "/src");
}

#[test]
fn parses_verbatim_prefix_case_insensitively() {
    assert_parses(r"\\?\unc\wsl$\Ubuntu\src", "Ubuntu", "/src");
}

#[test]
fn drops_trailing_separator() {
    assert_parses(r"\\wsl$\Ubuntu\home\user\", "Ubuntu", "/home/user");
    assert_parses("//wsl$/Ubuntu/home/user/", "Ubuntu", "/home/user");
}

#[test]
fn preserves_dotted_and_dashed_distro_names() {
    assert_parses(
        r"\\wsl.localhost\Ubuntu-24.04\home\krag",
        "Ubuntu-24.04",
        "/home/krag",
    );
}

#[test]
fn maps_distribution_root_to_slash() {
    assert_parses(r"\\wsl$\Ubuntu", "Ubuntu", "/");
    assert_parses(r"\\wsl$\Ubuntu\", "Ubuntu", "/");
    assert_parses("//wsl$/archlinux", "archlinux", "/");
}

#[test]
fn rejects_non_wsl_unc_path() {
    assert_eq!(parse_wsl_unc_path(Path::new(r"\\server\share\dir")), None);
}

#[test]
fn rejects_drive_letter_path() {
    assert_eq!(parse_wsl_unc_path(Path::new(r"C:\Users\foo")), None);
}

#[test]
fn rejects_relative_path() {
    assert_eq!(parse_wsl_unc_path(Path::new(r"foo\bar")), None);
    assert_eq!(parse_wsl_unc_path(Path::new("foo/bar")), None);
}

#[test]
fn rejects_wsl_host_without_distro() {
    assert_eq!(parse_wsl_unc_path(Path::new(r"\\wsl$")), None);
    assert_eq!(parse_wsl_unc_path(Path::new(r"\\wsl$\")), None);
}

#[test]
fn translates_bare_git_in_unc_cwd() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["status", "--short"]),
        Some(Path::new(r"\\wsl$\Ubuntu\home\user\repo")),
        &[],
    );

    assert_eq!(
        translated,
        Some(TranslatedCommand {
            program: OsString::from("wsl.exe"),
            args: args(&[
                "--distribution",
                "Ubuntu",
                "--cd",
                "/home/user/repo",
                "--exec",
                "git",
                "status",
                "--short",
            ]),
            wslenv: None,
        })
    );
}

#[test]
fn does_not_translate_gh() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("gh"),
        &args(&["pr", "list"]),
        Some(Path::new(r"\\wsl$\Ubuntu\home\user\repo")),
        &[],
    );
    assert_eq!(translated, None);
}

#[test]
fn does_not_translate_path_qualified_git() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("/usr/bin/git"),
        &args(&["status"]),
        Some(Path::new(r"\\wsl$\Ubuntu\home\user\repo")),
        &[],
    );
    assert_eq!(translated, None);
}

#[test]
fn does_not_translate_non_unc_cwd() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["status"]),
        Some(Path::new(r"C:\Users\user\repo")),
        &[],
    );
    assert_eq!(translated, None);
}

#[test]
fn does_not_translate_without_cwd() {
    let translated = translate_for_wsl_unc_cwd(OsStr::new("git"), &args(&["status"]), None, &[]);
    assert_eq!(translated, None);
}

#[test]
fn rewrites_same_distro_unc_argument_to_linux_path() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["-C", r"\\wsl$\Ubuntu\home\user\other"]),
        Some(Path::new(r"\\wsl$\Ubuntu\home\user\repo")),
        &[],
    )
    .expect("expected translation");

    assert_eq!(
        translated.args,
        args(&[
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "git",
            "-C",
            "/home/user/other",
        ])
    );
}

#[test]
fn rewrites_argument_with_case_insensitive_distro_match() {
    // The cwd distribution is `Ubuntu`; an argument spelled `ubuntu` refers to the same
    // distribution and must still be converted to its Linux path.
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["-C", r"\\wsl$\ubuntu\home\user\other"]),
        Some(Path::new(r"\\wsl$\Ubuntu\home\user\repo")),
        &[],
    )
    .expect("expected translation");

    assert_eq!(
        translated.args,
        args(&[
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "git",
            "-C",
            "/home/user/other",
        ])
    );
}

#[test]
fn leaves_other_distro_unc_argument_unchanged() {
    let other = r"\\wsl$\Debian\home\user\other";
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["-C", other]),
        Some(Path::new(r"\\wsl$\Ubuntu\home\user\repo")),
        &[],
    )
    .expect("expected translation");

    assert_eq!(
        translated.args,
        args(&[
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "git",
            "-C",
            other,
        ])
    );
}

#[test]
fn build_wslenv_excludes_path_case_insensitively() {
    // `PATH` in any spelling is dropped so a Linux-form `PATH` is never handed to `wsl.exe` through
    // `WSLENV`; other keys are kept and suffixed with `/u`.
    assert_eq!(
        build_wslenv(&env_pairs(&[
            ("PATH", "/usr/bin"),
            ("GIT_OPTIONAL_LOCKS", "0")
        ])),
        Some("GIT_OPTIONAL_LOCKS/u".to_string())
    );
    assert_eq!(
        build_wslenv(&env_pairs(&[
            ("Path", "/usr/bin"),
            ("GIT_AUTHOR_NAME", "Ada")
        ])),
        Some("GIT_AUTHOR_NAME/u".to_string())
    );
    assert_eq!(build_wslenv(&env_pairs(&[("path", "/usr/bin")])), None);
    assert_eq!(build_wslenv(&[]), None);
}

#[test]
fn builds_wslenv_from_env_keys() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["commit"]),
        Some(Path::new(r"\\wsl$\Ubuntu\repo")),
        &env_pairs(&[("GIT_AUTHOR_NAME", "Ada"), ("GIT_OPTIONAL_LOCKS", "0")]),
    )
    .expect("expected translation");

    assert_eq!(
        translated.wslenv,
        Some("GIT_AUTHOR_NAME/u:GIT_OPTIONAL_LOCKS/u".to_string())
    );
}

#[test]
fn omits_wslenv_when_no_env_keys() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["status"]),
        Some(Path::new(r"\\wsl$\Ubuntu\repo")),
        &[],
    )
    .expect("expected translation");

    assert_eq!(translated.wslenv, None);
}

#[test]
fn carries_explicit_path_through_argv() {
    // A caller-supplied `PATH` is threaded into the distribution as `env PATH=<value>` in front of
    // `git`, and must not leak into `WSLENV`.
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["commit"]),
        Some(Path::new(r"\\wsl$\Ubuntu\repo")),
        &env_pairs(&[("PATH", "/usr/local/bin:/usr/bin")]),
    )
    .expect("expected translation");

    assert_eq!(
        translated.args,
        args(&[
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/usr/local/bin:/usr/bin",
            "git",
            "commit",
        ])
    );
    assert_eq!(translated.wslenv, None);
}

#[test]
fn carries_case_insensitive_path_through_argv() {
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["status"]),
        Some(Path::new(r"\\wsl$\Ubuntu\repo")),
        &env_pairs(&[("Path", "/opt/bin")]),
    )
    .expect("expected translation");

    assert_eq!(
        translated.args,
        args(&[
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/opt/bin",
            "git",
            "status",
        ])
    );
}

#[test]
fn omits_env_wrapper_when_no_path() {
    // Without an explicit `PATH`, `git` is executed directly with no `env` wrapper; other variables
    // still travel via `WSLENV`.
    let translated = translate_for_wsl_unc_cwd(
        OsStr::new("git"),
        &args(&["status"]),
        Some(Path::new(r"\\wsl$\Ubuntu\repo")),
        &env_pairs(&[("GIT_OPTIONAL_LOCKS", "0")]),
    )
    .expect("expected translation");

    assert_eq!(
        translated.args,
        args(&[
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "git",
            "status",
        ])
    );
    assert_eq!(translated.wslenv, Some("GIT_OPTIONAL_LOCKS/u".to_string()));
}
