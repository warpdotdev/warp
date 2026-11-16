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
