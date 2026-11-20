use super::*;

fn hex_hook(json: &str) -> Vec<u8> {
    let mut bytes = b"\x1bP$d".to_vec();
    bytes.extend_from_slice(hex::encode(json).as_bytes());
    bytes.extend_from_slice(b"\x1b\\");
    bytes
}

fn scan_byte_by_byte(input: &[u8]) -> Vec<Hook> {
    let mut scanner = DcsScanner::new();
    let mut hooks = Vec::new();
    for byte in input {
        scanner.feed(std::slice::from_ref(byte), &mut hooks);
    }
    hooks
}

fn scan(input: &[u8]) -> Vec<Hook> {
    let mut scanner = DcsScanner::new();
    let mut hooks = Vec::new();
    scanner.feed(input, &mut hooks);
    hooks
}

#[test]
fn recognizes_lifecycle_hooks_split_byte_by_byte() {
    let init_shell =
        hex_hook(r#"{"hook":"InitShell","value":{"session_id":7,"shell":"zsh","user":"acme"}}"#);
    let bootstrapped = hex_hook(r#"{"hook":"Bootstrapped","value":{"session_id":7,"aliases":""}}"#);
    let precmd = hex_hook(r#"{"hook":"Precmd","value":{"session_id":7,"pwd":"/home/acme"}}"#);
    let preexec = hex_hook(r#"{"hook":"Preexec","value":{"command":"ls"}}"#);

    let mut input = b"hello ".to_vec();
    let init_offset = input.len() as u64;
    input.extend_from_slice(&init_shell);
    input.extend_from_slice(b"\r\n");
    let bootstrapped_offset = input.len() as u64;
    input.extend_from_slice(&bootstrapped);
    let precmd_offset = input.len() as u64;
    input.extend_from_slice(&precmd);
    input.extend_from_slice(b"$ ");
    let preexec_offset = input.len() as u64;
    input.extend_from_slice(&preexec);
    input.extend_from_slice(b"output");

    let hooks = scan_byte_by_byte(&input);
    assert_eq!(hooks, scan(&input));
    let summary: Vec<_> = hooks
        .iter()
        .map(|hook| (hook.kind, hook.session_id, hook.offset))
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                HookKind::InitShell { is_subshell: false },
                Some(7),
                init_offset
            ),
            (HookKind::Bootstrapped, Some(7), bootstrapped_offset),
            (HookKind::Precmd, Some(7), precmd_offset),
            (HookKind::Preexec, None, preexec_offset),
        ]
    );
    assert_eq!(hooks[0].raw, init_shell);
    assert_eq!(hooks[2].raw, precmd);
}

#[test]
fn recognizes_subshell_init_and_unencoded_json() {
    let mut input = hex_hook(r#"{"hook":"InitShell","value":{"session_id":8,"is_subshell":true}}"#);
    input.extend_from_slice(b"\x1bP$f{\"hook\":\"Precmd\",\"value\":{\"session_id\":8}}\x9c");
    let hooks = scan_byte_by_byte(&input);
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[0].kind, HookKind::InitShell { is_subshell: true });
    assert_eq!(hooks[1].kind, HookKind::Precmd);
    assert!(hooks[1].raw.ends_with(b"\x1b\\"));
}

#[test]
fn ignores_foreign_dcs_and_unknown_hooks() {
    let mut input = b"\x1bPq#0;2;0;0;0#0~~@@\x1b\\".to_vec();
    input.extend_from_slice(b"\x1bP$qm\x1b\\");
    input.extend_from_slice(&hex_hook(r#"{"hook":"InputBuffer","value":{"buffer":""}}"#));
    input.extend_from_slice(b"\x1bP$dnot-hex\x1b\\");
    input.extend_from_slice(b"\x1b[31mred\x1b[0m");
    assert!(scan_byte_by_byte(&input).is_empty());

    input.extend_from_slice(&hex_hook(r#"{"hook":"Precmd","value":{}}"#));
    let hooks = scan_byte_by_byte(&input);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].kind, HookKind::Precmd);
}

#[test]
fn foreign_dcs_payload_cannot_smuggle_a_hook_start() {
    let mut input = b"\x1bPqdata".to_vec();
    input.extend_from_slice(&hex_hook(r#"{"hook":"Precmd","value":{}}"#));
    let hooks = scan(&input);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].offset, 7);
}

#[test]
fn can_and_sub_abort_a_hook() {
    let precmd = hex_hook(r#"{"hook":"Precmd","value":{}}"#);
    for abort in [0x18u8, 0x1a] {
        let mut input = precmd[..precmd.len() - 4].to_vec();
        input.push(abort);
        input.extend_from_slice(&precmd[precmd.len() - 4..]);
        assert!(scan_byte_by_byte(&input).is_empty());

        input.extend_from_slice(&precmd);
        assert_eq!(scan_byte_by_byte(&input).len(), 1);
    }
}

#[test]
fn escape_inside_payload_restarts_scanning() {
    let precmd = hex_hook(r#"{"hook":"Precmd","value":{}}"#);
    let mut input = b"\x1bP$d7b22".to_vec();
    let offset = input.len() as u64;
    input.extend_from_slice(&precmd);
    let hooks = scan_byte_by_byte(&input);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].offset, offset);
}

#[test]
fn oversized_payloads_are_dropped() {
    let mut input = b"\x1bP$d".to_vec();
    input.extend(std::iter::repeat_n(b'0', MAX_HOOK_PAYLOAD + 10));
    input.extend_from_slice(b"\x1b\\");
    let precmd = hex_hook(r#"{"hook":"Precmd","value":{}}"#);
    input.extend_from_slice(&precmd);

    let mut scanner = DcsScanner::new();
    let mut hooks = Vec::new();
    for chunk in input.chunks(4096) {
        scanner.feed(chunk, &mut hooks);
    }
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].raw, precmd);
}

#[test]
fn recognizes_command_finished() {
    let input = hex_hook(r#"{"hook":"CommandFinished","value":{"exit_code":1,"session_id":3}}"#);
    let hooks = scan_byte_by_byte(&input);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].kind, HookKind::CommandFinished);
    assert_eq!(hooks[0].session_id, Some(3));
    assert_eq!(hooks[0].raw, input);
}
