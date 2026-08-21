use super::*;

#[test]
#[cfg(windows)]
fn bracketed_paste_command_execution_normalizes_crlf_to_lf_for_posix_shells_on_windows() {
    let command = "curl 'https://google.com' \\\r\n  -H 'accept: application/json'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, true);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"curl 'https://google.com' \\\n  -H 'accept: application/json'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\r'));
}

#[test]
#[cfg(not(windows))]
fn bracketed_paste_command_execution_preserves_crlf_for_posix_shells_off_windows() {
    let command = "curl 'https://google.com' \\\r\n  -H 'accept: application/json'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, true);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"curl 'https://google.com' \\\r\n  -H 'accept: application/json'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(bytes.contains(&b'\r'));
}

#[test]
fn unbracketed_paste_command_execution_preserves_lf_for_posix_shells() {
    let command = "printf 'hello'\nprintf 'world'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, false);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(b"printf 'hello'\nprintf 'world'");
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\r'));
}

#[test]
fn powershell_command_execution_normalizes_linefeeds_to_carriage_returns() {
    let command = "Write-Output 'hello'\r\nWrite-Output 'world'\nWrite-Output 'again'";

    let bytes = bytes_to_execute_command(command, ShellType::PowerShell, false);

    let mut expected = ShellType::PowerShell.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(b"Write-Output 'hello'\rWrite-Output 'world'\rWrite-Output 'again'");
    expected.extend_from_slice(ShellType::PowerShell.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\n'));
}

#[test]
fn split_kill_buffer_write_splits_powershell_off_from_the_rest() {
    let bytes = bytes_to_execute_command("Get-ChildItem", ShellType::PowerShell, false);

    let (kill_buffer, rest) =
        split_kill_buffer_write(&bytes, ShellType::PowerShell).expect("PowerShell should split");

    assert_eq!(kill_buffer, ShellType::PowerShell.kill_buffer_bytes());
    let mut expected_rest = b"Get-ChildItem".to_vec();
    expected_rest.extend_from_slice(ShellType::PowerShell.execute_command_bytes());
    assert_eq!(rest, expected_rest.as_slice());
}

#[test]
fn split_kill_buffer_write_does_not_split_the_other_three_shells() {
    for shell_type in [ShellType::Zsh, ShellType::Bash, ShellType::Fish] {
        let bytes = bytes_to_execute_command("echo hi", shell_type, false);
        assert!(
            split_kill_buffer_write(&bytes, shell_type).is_none(),
            "expected no split for {shell_type:?}, which uses a single unambiguous control byte"
        );
    }
}

#[test]
fn split_kill_buffer_write_handles_a_command_with_no_content_gracefully() {
    // Defensive: even if bytes_to_execute_command somehow ever returned just the kill-buffer
    // bytes and nothing else, splitting must not panic or produce an empty second write.
    let kill_buffer_only = ShellType::PowerShell.kill_buffer_bytes().to_vec();
    assert!(split_kill_buffer_write(&kill_buffer_only, ShellType::PowerShell).is_none());
}
