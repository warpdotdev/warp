use super::*;

#[test]
fn bracketed_paste_command_execution_normalizes_crlf_to_lf_for_posix_shells() {
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
fn powershell_bracketed_paste_command_execution_preserves_crlf() {
    let command = "Write-Output 'hello'\r\nWrite-Output 'world'";

    let bytes = bytes_to_execute_command(command, ShellType::PowerShell, true);

    let mut expected = ShellType::PowerShell.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"Write-Output 'hello'\r\nWrite-Output 'world'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::PowerShell.execute_command_bytes());

    assert_eq!(bytes, expected);
}
