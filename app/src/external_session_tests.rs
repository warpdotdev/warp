use std::ffi::OsString;
use std::path::{Path, PathBuf};

use warp_cli::ExternalCommandSpec;

use super::*;
use crate::terminal::shell::ShellType;

#[test]
fn sanitize_caller_env_vars_drops_internal_vars() {
    let vars = vec![
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("TERM_PROGRAM"), OsString::from("vscode")),
        (
            OsString::from("TERM_PROGRAM_VERSION"),
            OsString::from("1.100.0"),
        ),
        (OsString::from("COLORTERM"), OsString::from("truecolor")),
        (OsString::from("WARP_SESSION_ID"), OsString::from("12345")),
        (
            OsString::from("WARP_INITIAL_WORKING_DIR"),
            OsString::from("/old"),
        ),
        (OsString::from("ZDOTDIR"), OsString::from("/tmp/zsh")),
        (OsString::from("PWD"), OsString::from("/old")),
        (OsString::from("SHLVL"), OsString::from("3")),
        (
            OsString::from("__CFBundleIdentifier"),
            OsString::from("com.microsoft.VSCode"),
        ),
        (
            OsString::from("DESKTOP_STARTUP_ID"),
            OsString::from("startup-1"),
        ),
    ];

    let sanitized = sanitize_caller_env_vars(&vars);

    assert_eq!(sanitized, Vec::<(OsString, OsString)>::new());
}

#[test]
fn sanitize_caller_env_vars_keeps_user_vars() {
    let vars = vec![
        (
            OsString::from("PATH"),
            OsString::from("/usr/local/bin:/usr/bin"),
        ),
        (
            OsString::from("Path"),
            OsString::from("C:\\Windows\\System32"),
        ),
        (OsString::from("PYTHONPATH"), OsString::from("/repo/src")),
        (OsString::from("VIRTUAL_ENV"), OsString::from("/repo/.venv")),
        (
            OsString::from("DEBUGPY_LOG_DIR"),
            OsString::from("/tmp/debugpy"),
        ),
    ];

    let sanitized = sanitize_caller_env_vars(&vars);

    assert_eq!(sanitized, vars);
}

#[test]
fn format_external_command_quotes_special_chars_for_zsh() {
    let spec = ExternalCommandSpec::Argv(vec![
        "/path/with spaces/python".to_owned(),
        "-c".to_owned(),
        "print('hello $USER'); exit".to_owned(),
        "a\nb".to_owned(),
    ]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Zsh, None);

    assert_eq!(
        formatted.as_deref(),
        Some(
            r#"'/path/with spaces/python' '-c' 'print('"'"'hello $USER'"'"'); exit' 'a
b'"#
        )
    );
}

#[test]
fn format_external_command_quotes_special_chars_for_bash() {
    let spec = ExternalCommandSpec::Argv(vec![
        "/path/with spaces/python".to_owned(),
        "-c".to_owned(),
        "print('hello $USER'); exit".to_owned(),
        "a\nb".to_owned(),
    ]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Bash, None);

    assert_eq!(
        formatted.as_deref(),
        Some(
            r#"'/path/with spaces/python' '-c' 'print('"'"'hello $USER'"'"'); exit' 'a
b'"#
        )
    );
}

#[test]
fn format_external_command_quotes_special_chars_for_fish() {
    let spec = ExternalCommandSpec::Argv(vec![
        "/path/with spaces/python".to_owned(),
        "-c".to_owned(),
        "print('hello $USER'); exit".to_owned(),
        "a\nb".to_owned(),
    ]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Fish, None);

    assert_eq!(
        formatted.as_deref(),
        Some("'/path/with spaces/python' '-c' 'print(\\'hello $USER\\'); exit' 'a\nb'")
    );
}

#[test]
fn format_external_command_uses_call_operator_for_powershell() {
    let spec = ExternalCommandSpec::Argv(vec![
        r"C:\Program Files\Python312\python.exe".to_owned(),
        "-m".to_owned(),
        "debugpy".to_owned(),
        "it's".to_owned(),
    ]);

    let formatted = format_external_command_for_shell(&spec, ShellType::PowerShell, None);

    assert_eq!(
        formatted.as_deref(),
        Some(r"& 'C:\Program Files\Python312\python.exe' '-m' 'debugpy' 'it''s'")
    );
}

#[test]
fn format_external_command_wraps_windows_cmdline() {
    let spec = ExternalCommandSpec::WindowsCmdLine(
        r#""C:\Python\python.exe" "launcher.py" & pause"#.to_owned(),
    );

    let formatted = format_external_command_for_shell(&spec, ShellType::PowerShell, None);

    assert_eq!(
        formatted.as_deref(),
        Some(r#"cmd.exe /c '"C:\Python\python.exe" "launcher.py" & pause'"#)
    );
}

#[test]
fn format_external_command_appends_exit_when_wait_is_false_for_argv() {
    let spec = ExternalCommandSpec::Argv(vec!["python3".to_owned(), "main.py".to_owned()]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Bash, Some(false));

    assert_eq!(formatted.as_deref(), Some("'python3' 'main.py'; exit"));
}

#[test]
fn format_external_command_appends_exit_when_wait_is_false_for_windows_cmdline() {
    let spec = ExternalCommandSpec::WindowsCmdLine("python.exe main.py".to_owned());

    let formatted = format_external_command_for_shell(&spec, ShellType::PowerShell, Some(false));

    assert_eq!(
        formatted.as_deref(),
        Some("cmd.exe /c 'python.exe main.py'; exit")
    );
}

#[test]
fn format_external_command_omits_exit_when_wait_is_true() {
    let spec = ExternalCommandSpec::Argv(vec!["python3".to_owned(), "main.py".to_owned()]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Bash, Some(true));

    assert_eq!(formatted.as_deref(), Some("'python3' 'main.py'"));
}

#[test]
fn format_external_command_unwraps_double_single_quotes_for_bash_dash_c() {
    let spec = ExternalCommandSpec::Argv(vec![
        "bash".to_owned(),
        "-c".to_owned(),
        "''python main.py; read -n1''".to_owned(),
    ]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Bash, None);

    assert_eq!(
        formatted.as_deref(),
        Some("'bash' '-c' 'python main.py; read -n1'")
    );
}

#[test]
fn format_external_command_unwraps_double_single_quotes_for_bin_bash_dash_c() {
    let spec = ExternalCommandSpec::Argv(vec![
        "/bin/bash".to_owned(),
        "-c".to_owned(),
        "''python main.py; read -n1''".to_owned(),
    ]);

    let formatted = format_external_command_for_shell(&spec, ShellType::Zsh, None);

    assert_eq!(
        formatted.as_deref(),
        Some("'/bin/bash' '-c' 'python main.py; read -n1'")
    );
}

#[test]
fn format_external_command_returns_none_for_empty_argv() {
    let spec = ExternalCommandSpec::Argv(Vec::new());

    let formatted = format_external_command_for_shell(&spec, ShellType::Bash, Some(false));

    assert_eq!(formatted, None);
}

#[test]
fn format_external_command_returns_none_for_blank_windows_cmdline() {
    let spec = ExternalCommandSpec::WindowsCmdLine("   ".to_owned());

    let formatted = format_external_command_for_shell(&spec, ShellType::PowerShell, Some(false));

    assert_eq!(formatted, None);
}

#[test]
fn resolve_launch_working_directory_expands_tilde() {
    let expected = PathBuf::from(shellexpand::tilde("~/projects").into_owned());

    let resolved = resolve_launch_working_directory(
        Some(Path::new("~/projects")),
        Some(Path::new("/base")),
        false,
    );

    assert_eq!(resolved, Some(expected));
}

#[test]
fn resolve_launch_working_directory_joins_relative_path_onto_current_dir() {
    let base = Path::new("/workspace");

    let resolved = resolve_launch_working_directory(Some(Path::new("sub/dir")), Some(base), false);

    assert_eq!(resolved, Some(base.join("sub/dir")));
}

#[test]
fn resolve_launch_working_directory_omits_implicit_root_on_macos() {
    let resolved = resolve_launch_working_directory(None, Some(Path::new("/")), true);

    assert_eq!(resolved, None);
}

#[test]
fn resolve_launch_working_directory_keeps_explicit_root_on_macos() {
    let resolved =
        resolve_launch_working_directory(Some(Path::new("/")), Some(Path::new("/workspace")), true);

    assert_eq!(resolved, Some(PathBuf::from("/")));
}

#[test]
fn resolve_launch_working_directory_keeps_implicit_root_on_non_macos() {
    let resolved = resolve_launch_working_directory(None, Some(Path::new("/")), false);

    assert_eq!(resolved, Some(PathBuf::from("/")));
}

#[test]
fn external_session_launch_request_roundtrips_through_json() {
    let request = ExternalSessionLaunchRequest {
        working_directory: Some(PathBuf::from("/tmp/project")),
        command: Some(ExternalCommandSpec::Argv(vec![
            "python3".to_owned(),
            "-m".to_owned(),
            "debugpy".to_owned(),
        ])),
        env_vars: vec![(OsString::from("PYTHONPATH"), OsString::from("/tmp/project"))],
        title: Some("VS Code Console".to_owned()),
        new_window: true,
        wait_after_command: Some(true),
    };

    let json = serde_json::to_string(&request).unwrap();
    let from_json: ExternalSessionLaunchRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(from_json, request);
}

#[test]
fn external_session_launch_request_roundtrips_through_bincode() {
    let request = ExternalSessionLaunchRequest {
        working_directory: Some(PathBuf::from("/tmp/project")),
        command: Some(ExternalCommandSpec::Argv(vec![
            "python3".to_owned(),
            "-m".to_owned(),
            "debugpy".to_owned(),
        ])),
        env_vars: vec![(OsString::from("PYTHONPATH"), OsString::from("/tmp/project"))],
        title: Some("VS Code Console".to_owned()),
        new_window: true,
        wait_after_command: Some(true),
    };

    let bin = bincode::serialize(&request).unwrap();
    let from_bin: ExternalSessionLaunchRequest = bincode::deserialize(&bin).unwrap();

    assert_eq!(from_bin, request);
}
