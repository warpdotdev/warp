use super::*;

/// Serialized block JSON with byte fields encoded as base64 strings, as produced by `to_json`.
const BASE64_JSON: &str = r#"{
    "id": "test-block-1",
    "stylized_command": "ZWNobyBoZWxsbw==",
    "stylized_output": "aGVsbG8gd29ybGQ=",
    "pwd": null,
    "git_head": null,
    "virtual_env": null,
    "conda_env": null,
    "node_version": null,
    "exit_code": 0,
    "did_execute": true,
    "completed_ts": null,
    "start_ts": null,
    "ps1": null,
    "rprompt": null,
    "honor_ps1": false,
    "is_background": false,
    "session_id": null,
    "shell_host": null,
    "prompt_snapshot": null,
    "ai_metadata": null
}"#;

/// Serialized block JSON with byte fields as integer arrays, as produced by plain `serde_json`.
const ARRAY_JSON: &str = r#"{
    "id": "test-block-1",
    "stylized_command": [101,99,104,111,32,104,101,108,108,111],
    "stylized_output": [104,101,108,108,111,32,119,111,114,108,100],
    "pwd": null,
    "git_head": null,
    "virtual_env": null,
    "conda_env": null,
    "node_version": null,
    "exit_code": 0,
    "did_execute": true,
    "completed_ts": null,
    "start_ts": null,
    "ps1": null,
    "rprompt": null,
    "honor_ps1": false,
    "is_background": false,
    "session_id": null,
    "shell_host": null,
    "prompt_snapshot": null,
    "ai_metadata": null
}"#;

#[test]
fn from_json_accepts_base64_encoded_bytes() {
    let block = SerializedBlock::from_json(BASE64_JSON.as_bytes()).unwrap();
    assert_eq!(block.stylized_command, b"echo hello");
    assert_eq!(block.stylized_output, b"hello world");
}

#[test]
fn from_json_accepts_integer_array_bytes() {
    let block = SerializedBlock::from_json(ARRAY_JSON.as_bytes()).unwrap();
    assert_eq!(block.stylized_command, b"echo hello");
    assert_eq!(block.stylized_output, b"hello world");
}

/// APP-5257: `plain_text_command_preview` is used to give a background tab a
/// usable title before its real restoration has ever run. It must handle
/// plain commands, ones wrapped in CSI (color) escapes, and ones interleaved
/// with an OSC (e.g. title-setting) escape sequence, matching what a shell
/// with syntax highlighting or a title-setting hook would actually emit.
#[test]
fn plain_text_command_preview_strips_plain_command() {
    let block = SerializedBlock::new_for_test(b"pwd".to_vec(), b"/home/user\n".to_vec());
    assert_eq!(block.plain_text_command_preview().as_deref(), Some("pwd"));
}

#[test]
fn plain_text_command_preview_strips_csi_color_codes() {
    let stylized = b"\x1b[32mecho\x1b[0m TAB2_MARKER_BRAVO".to_vec();
    let block = SerializedBlock::new_for_test(stylized, Vec::new());
    assert_eq!(
        block.plain_text_command_preview().as_deref(),
        Some("echo TAB2_MARKER_BRAVO")
    );
}

#[test]
fn plain_text_command_preview_strips_osc_title_sequence() {
    // A shell configured to set the terminal title to the running command
    // (a common preexec hook) could plausibly interleave an OSC sequence
    // with the echoed command bytes.
    let stylized = b"\x1b]0;uname -a\x07uname -a".to_vec();
    let block = SerializedBlock::new_for_test(stylized, Vec::new());
    assert_eq!(
        block.plain_text_command_preview().as_deref(),
        Some("uname -a")
    );
}

#[test]
fn plain_text_command_preview_takes_only_the_first_line() {
    let block = SerializedBlock::new_for_test(b"echo one\necho two".to_vec(), Vec::new());
    assert_eq!(
        block.plain_text_command_preview().as_deref(),
        Some("echo one")
    );
}

#[test]
fn plain_text_command_preview_is_none_for_a_block_that_never_executed() {
    let mut block = SerializedBlock::new_for_test(b"pwd".to_vec(), Vec::new());
    block.did_execute = false;
    assert_eq!(block.plain_text_command_preview(), None);
}

#[test]
fn plain_text_command_preview_is_none_for_empty_command() {
    let block = SerializedBlock::new_for_test(Vec::new(), Vec::new());
    assert_eq!(block.plain_text_command_preview(), None);
}
