use super::*;

fn hex_hook(json: &str) -> Vec<u8> {
    let mut bytes = b"\x1bP$d".to_vec();
    bytes.extend_from_slice(hex::encode(json).as_bytes());
    bytes.extend_from_slice(b"\x1b\\");
    bytes
}

const SESSION: u64 = 7;

fn init_shell(session_id: u64, is_subshell: bool) -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"InitShell","value":{{"session_id":{session_id},"shell":"zsh","is_subshell":{is_subshell}}}}}"#
    ))
}

fn bootstrapped() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"Bootstrapped","value":{{"session_id":{SESSION}}}}}"#
    ))
}

fn precmd() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"Precmd","value":{{"session_id":{SESSION}}}}}"#
    ))
}

fn command_finished() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"CommandFinished","value":{{"exit_code":0,"session_id":{SESSION}}}}}"#
    ))
}

fn preexec() -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"Preexec","value":{{"command":"make","session_id":{SESSION}}}}}"#
    ))
}

fn hook_for(name: &str, session_id: u64) -> Vec<u8> {
    hex_hook(&format!(
        r#"{{"hook":"{name}","value":{{"session_id":{session_id}}}}}"#
    ))
}

#[test]
fn ring_evicts_oldest_bytes_and_tracks_absolute_offsets() {
    let mut ring = Ring::new(10);
    ring.push(b"abcdef");
    assert_eq!((ring.start(), ring.total()), (0, 6));
    ring.push(b"ghijkl");
    assert_eq!((ring.start(), ring.total()), (2, 12));
    assert_eq!(ring.copy_from(2).unwrap(), b"cdefghijkl");
    assert_eq!(ring.copy_from(10).unwrap(), b"kl");
    assert_eq!(ring.copy_from(12).unwrap(), b"");
    assert!(ring.copy_from(1).is_none());
    assert!(ring.copy_from(13).is_none());

    ring.push(b"0123456789ABCDEFGHIJ");
    assert_eq!((ring.start(), ring.total()), (22, 32));
    assert_eq!(ring.copy_from(22).unwrap(), b"ABCDEFGHIJ");
}

#[test]
fn ring_grows_lazily() {
    let mut ring = Ring::new(crate::local_tty::server::RING_CAPACITY);
    ring.push(b"small");
    assert!(ring.buf.capacity() < 1024 * 1024);
}

#[test]
fn prompt_starts_are_pruned_on_eviction() {
    let precmd = precmd();
    let capacity = 3 * (precmd.len() + 8) + 10;
    let mut history = History::new(capacity, SESSION);
    for _ in 0..3 {
        history.record(&precmd);
        history.record(b"output\r\n");
    }
    assert_eq!(history.prompt_starts.len(), 3);
    history.record(&vec![b'x'; precmd.len() + 20]);
    assert!(history.prompt_starts.len() < 3);
    assert!(
        history
            .prompt_starts
            .iter()
            .all(|offset| *offset >= history.ring.start())
    );
    history.record(&vec![b'y'; capacity]);
    assert!(history.prompt_starts.is_empty());
}

#[test]
fn replay_case_one_starts_at_the_last_prompt_before_detach() {
    let mut history = History::new(4096, SESSION);
    let mut preamble = init_shell(SESSION, false);
    history.record(b"login banner\r\n");
    history.record(&preamble);
    history.record(b"bootstrap noise");
    history.record(&bootstrapped());
    preamble.extend_from_slice(&bootstrapped());
    history.record(&precmd());
    history.record(b"$ first\r\n");
    history.record(&precmd());
    history.record(b"$ ");
    history.mark_detached();
    history.record(b"typed while detached");

    let replay = history.replay();
    let mut expected = preamble.clone();
    expected.extend_from_slice(&precmd());
    expected.extend_from_slice(b"$ typed while detached");
    assert!(replay.has_bootstrap_preamble);
    assert_eq!(replay.bytes, expected);
}

#[test]
fn replay_case_two_falls_back_to_the_oldest_retained_prompt() {
    let precmd = precmd();
    let mut history = History::new(precmd.len() * 2 + 40, SESSION);
    history.record(&precmd);
    history.record(b"$ cmd one\r\n");
    history.mark_detached();
    history.record(&[b'.'; 30]);
    history.record(&precmd);
    history.record(b"$ two");
    assert!(history.replay_from < history.ring.start());

    let replay = history.replay();
    let mut expected = precmd.clone();
    expected.extend_from_slice(b"$ two");
    assert!(!replay.has_bootstrap_preamble);
    assert_eq!(replay.bytes, expected);
}

#[test]
fn replay_case_three_reconstructs_the_running_command() {
    let precmd = precmd();
    let preexec = preexec();
    let mut history = History::new(64, SESSION);
    history.record(&init_shell(SESSION, false));
    history.record(&bootstrapped());
    history.record(&precmd);
    history.record(b"$ make\r\n");
    history.record(&preexec);
    history.mark_detached();
    let tail: Vec<u8> = (0..200u32).map(|i| b'a' + (i % 26) as u8).collect();
    history.record(&tail);

    let replay = history.replay();
    let mut expected = init_shell(SESSION, false);
    expected.extend_from_slice(&bootstrapped());
    expected.extend_from_slice(&precmd);
    expected.extend_from_slice(&preexec);
    expected.extend_from_slice(TRUNCATION_MARKER);
    expected.extend_from_slice(&tail[tail.len() - 64..]);
    assert!(replay.has_bootstrap_preamble);
    assert_eq!(replay.bytes, expected);
}

#[test]
fn replay_case_three_skips_a_preexec_older_than_the_last_prompt() {
    let precmd = precmd();
    let mut history = History::new(32, SESSION);
    history.record(&preexec());
    history.record(&precmd);
    history.record(&[b'z'; 100]);

    let mut expected = precmd.clone();
    expected.extend_from_slice(TRUNCATION_MARKER);
    expected.extend_from_slice(&[b'z'; 32]);
    assert_eq!(history.replay().bytes, expected);
}

#[test]
fn replay_without_a_prompt_returns_the_whole_retained_stream_once() {
    let mut history = History::new(4096, SESSION);
    history.record(b"before ");
    history.record(&init_shell(SESSION, false));
    history.record(&bootstrapped());
    history.record(b"after");
    history.mark_detached();

    let replay = history.replay();
    let mut expected = b"before ".to_vec();
    expected.extend_from_slice(&init_shell(SESSION, false));
    expected.extend_from_slice(&bootstrapped());
    expected.extend_from_slice(b"after");
    assert!(replay.has_bootstrap_preamble);
    assert_eq!(replay.bytes, expected);
}

#[test]
fn preamble_requires_a_login_init_shell_for_this_session() {
    let mut history = History::new(4096, SESSION);
    history.record(&init_shell(SESSION, true));
    history.record(&bootstrapped());
    assert!(!history.has_bootstrap_preamble());
    history.record(&init_shell(SESSION + 1, false));
    history.record(&bootstrapped());
    assert!(!history.has_bootstrap_preamble());
    history.record(&bootstrapped());
    history.record(&init_shell(SESSION, false));
    assert!(!history.has_bootstrap_preamble());
    history.record(&bootstrapped());
    assert!(history.has_bootstrap_preamble());
}

#[test]
fn a_new_login_init_shell_replaces_the_preamble() {
    let mut history = History::new(4096, SESSION);
    history.record(&init_shell(SESSION, false));
    history.record(&bootstrapped());
    history.record(&precmd());
    history.record(b"exec zsh\r\n");
    let second = hex_hook(&format!(
        r#"{{"hook":"InitShell","value":{{"session_id":{SESSION},"shell":"bash"}}}}"#
    ));
    history.record(&second);
    assert!(!history.has_bootstrap_preamble());
    history.record(&bootstrapped());
    history.record(&precmd());
    history.mark_detached();

    let replay = history.replay();
    let mut expected = second.clone();
    expected.extend_from_slice(&bootstrapped());
    expected.extend_from_slice(&precmd());
    assert_eq!(replay.bytes, expected);
}

#[test]
fn replay_starts_at_the_command_finished_before_the_last_prompt() {
    let mut history = History::new(4096, SESSION);
    history.record(&init_shell(SESSION, false));
    history.record(&bootstrapped());
    history.record(&command_finished());
    history.record(&precmd());
    history.record(b"$ sleep 100\r\n");
    history.record(&preexec());
    history.record(b"tick 1\r\n");
    history.record(&command_finished());
    history.record(b"\r\n");
    history.record(&precmd());
    history.record(b"$ ");
    history.mark_detached();
    history.record(b"tick 2");

    let mut expected = init_shell(SESSION, false);
    expected.extend_from_slice(&bootstrapped());
    expected.extend_from_slice(&command_finished());
    expected.extend_from_slice(b"\r\n");
    expected.extend_from_slice(&precmd());
    expected.extend_from_slice(b"$ tick 2");
    assert_eq!(history.replay().bytes, expected);
}

#[test]
fn a_command_finished_followed_by_preexec_is_not_paired_with_the_next_prompt() {
    let mut history = History::new(4096, SESSION);
    history.record(&command_finished());
    history.record(&preexec());
    history.record(b"noise");
    history.record(&precmd());
    history.record(b"$ ");
    history.mark_detached();

    let mut expected = precmd();
    expected.extend_from_slice(b"$ ");
    assert_eq!(history.replay().bytes, expected);
}

#[test]
fn replay_case_two_starts_at_the_oldest_retained_command_finished() {
    let command_finished = command_finished();
    let precmd = precmd();
    let prompt_len = command_finished.len() + precmd.len();
    let mut history = History::new(prompt_len * 2 + 40, SESSION);
    history.record(&command_finished);
    history.record(&precmd);
    history.record(b"$ cmd one\r\n");
    history.mark_detached();
    history.record(&[b'.'; 30]);
    history.record(&command_finished);
    history.record(&precmd);
    history.record(b"$ two");

    let mut expected = command_finished.clone();
    expected.extend_from_slice(&precmd);
    expected.extend_from_slice(b"$ two");
    assert_eq!(history.replay().bytes, expected);
}

#[test]
fn replay_case_three_prepends_command_finished_before_the_prompt() {
    let command_finished = command_finished();
    let precmd = precmd();
    let preexec = preexec();
    let mut history = History::new(64, SESSION);
    history.record(&command_finished);
    history.record(&precmd);
    history.record(b"$ make\r\n");
    history.record(&preexec);
    history.mark_detached();
    history.record(&[b'q'; 200]);

    let mut expected = command_finished.clone();
    expected.extend_from_slice(&precmd);
    expected.extend_from_slice(&preexec);
    expected.extend_from_slice(TRUNCATION_MARKER);
    expected.extend_from_slice(&[b'q'; 64]);
    assert_eq!(history.replay().bytes, expected);
}

#[test]
fn hooks_from_another_session_do_not_move_the_replay_start() {
    let mut history = History::new(4096, SESSION);
    history.record(&command_finished());
    history.record(&precmd());
    history.record(b"$ ssh example.com\r\n");
    history.record(&preexec());
    history.record(&hook_for("CommandFinished", 99));
    history.record(&hook_for("Precmd", 99));
    history.record(b"remote$ ");
    history.record(&hook_for("Preexec", 99));
    history.mark_detached();

    let mut expected = command_finished();
    expected.extend_from_slice(&precmd());
    expected.extend_from_slice(b"$ ssh example.com\r\n");
    assert!(history.replay().bytes.starts_with(&expected));
}

#[test]
fn hooks_without_a_session_id_are_ignored() {
    let mut history = History::new(4096, SESSION);
    history.record(b"before");
    history.record(&hex_hook(r#"{"hook":"Precmd","value":{}}"#));
    history.record(b"$ ");
    history.mark_detached();

    assert!(history.replay().bytes.starts_with(b"before"));
}
