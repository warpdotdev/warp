use std::path::PathBuf;

use super::*;
use crate::terminal::shell::ShellType;

#[test]
fn build_find_command_single_quotes_patterns_and_path() {
    let patterns = vec![
        "$(touch /tmp/warp-poc)*.rs".to_string(),
        "owner's*.rs".to_string(),
    ];

    let command = build_find_command(&patterns, "/tmp/repo path", ShellType::Bash);

    assert_eq!(
        command,
        r#"find '/tmp/repo path' -type f -name '$(touch /tmp/warp-poc)*.rs' -o -name 'owner'"'"'s*.rs'"#
    );
}

#[test]
fn build_git_ls_files_command_single_quotes_joined_patterns() {
    let pattern = "$(touch /tmp/warp-poc)*.rs";
    let patterns = vec![pattern.to_string()];
    let target_path = PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        .join("tmp")
        .join("repo");

    let command = build_git_ls_files_command(
        &patterns,
        target_path.to_str().unwrap(),
        None,
        ShellType::Bash,
    );

    let expected = format!(
        "git ls-files -c -o --exclude-standard -- '{}' '{}'",
        target_path.join(pattern).display(),
        target_path.join("*").join(pattern).display(),
    );
    assert_eq!(command, expected);
}

#[test]
fn build_powershell_get_childitem_command_single_quotes_patterns_and_path() {
    let patterns = vec![
        r#"$(New-Item C:\pwn)*.rs"#.to_string(),
        "owner's*.rs".to_string(),
    ];

    let command = build_powershell_get_childitem_command(&patterns, r#"C:\repo path"#);

    assert_eq!(
        command,
        r#"Get-ChildItem -File -Recurse -Include '$(New-Item C:\pwn)*.rs','owner''s*.rs' -Path 'C:\repo path' | ForEach-Object { $_.FullName }"#
    );
}
