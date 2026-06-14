use super::*;

fn bracketed_paste_payload(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .windows(escape_sequences::BRACKETED_PASTE_START.len())
        .position(|window| window == escape_sequences::BRACKETED_PASTE_START)
        .expect("expected bracketed paste start")
        + escape_sequences::BRACKETED_PASTE_START.len();
    let end = bytes[start..]
        .windows(escape_sequences::BRACKETED_PASTE_END.len())
        .position(|window| window == escape_sequences::BRACKETED_PASTE_END)
        .expect("expected bracketed paste end")
        + start;
    &bytes[start..end]
}

#[test]
fn bracketed_paste_command_normalizes_linefeeds_to_carriage_returns() {
    let command =
        "echo \"A PID=$!\"\n\nps -o pid,etime,command -p $(jobs -p) 2>/dev/null\r\necho \"done\"";

    let bytes = bytes_to_execute_command(command, ShellType::Zsh, true);

    assert_eq!(
        bracketed_paste_payload(&bytes),
        b"echo \"A PID=$!\"\r\rps -o pid,etime,command -p $(jobs -p) 2>/dev/null\recho \"done\"",
    );
}

#[test]
fn bracketed_paste_command_strips_escape_characters_from_payload() {
    let command = format!(
        "printf '{}[31mred{}[0m'\n",
        escape_sequences::C0::ESC as char,
        escape_sequences::C0::ESC as char,
    );

    let bytes = bytes_to_execute_command(&command, ShellType::Zsh, true);

    assert_eq!(bracketed_paste_payload(&bytes), b"printf '[31mred[0m'\r");
}
