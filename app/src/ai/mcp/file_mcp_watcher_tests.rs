use std::env;
use std::path::PathBuf;
use tempfile::TempDir;

use super::{parse_mcp_config_file, substitute_env_vars};
use crate::ai::mcp::{ConfigParseStage, MCPProvider};

fn cleanup_env_vars(vars: &[&str]) {
    for var in vars {
        env::remove_var(var);
    }
}

#[test]
fn test_substitute_env_vars_success() {
    let test_vars = ["FOO", "BAZ", "REPEATED"];

    // Setup environment variables
    env::set_var("FOO", "bar");
    env::set_var("BAZ", "qux");
    env::set_var("REPEATED", "value");

    // Test 1: Single variable substitution
    let input = r#"{"key": "${FOO}"}"#;
    let result = substitute_env_vars(input).expect("Single variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar"}"#,
        "Single variable FOO should be replaced with 'bar'"
    );

    // Test 2: Multiple different variables
    let input = r#"{"key": "${FOO}", "other": "${BAZ}"}"#;
    let result = substitute_env_vars(input).expect("Multiple variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar", "other": "qux"}"#,
        "Multiple variables FOO and BAZ should be replaced"
    );

    // Test 3: Multiple occurrences of same variable
    let input = r#"{"a": "${REPEATED}", "b": "${REPEATED}", "c": "prefix_${REPEATED}_suffix"}"#;
    let result = substitute_env_vars(input).expect("Repeated variable substitution should succeed");
    assert_eq!(
        result, r#"{"a": "value", "b": "value", "c": "prefix_value_suffix"}"#,
        "All occurrences of REPEATED should be replaced with 'value', including within context"
    );

    // Cleanup
    cleanup_env_vars(&test_vars);
}

#[test]
fn test_substitute_env_vars_missing_or_empty() {
    // Test 1: Missing variable
    // Ensure MISSING_VAR is not set
    env::remove_var("MISSING_VAR");

    let input = r#"{"key": "${MISSING_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: MISSING_VAR"),
        "Error message should mention MISSING_VAR, got: {err_msg}"
    );

    // Test 2: Empty variable
    env::set_var("EMPTY_VAR", "");

    let input = r#"{"key": "${EMPTY_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: EMPTY_VAR"),
        "Error message should mention EMPTY_VAR, got: {err_msg}"
    );

    // Cleanup
    cleanup_env_vars(&["EMPTY_VAR"]);
}

/// Writes `contents` to `name` inside a fresh `TempDir` and returns both. The
/// `TempDir` is kept alive by the caller so the file outlives the parser call.
fn write_temp_file(name: &str, contents: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let path = dir.path().join(name);
    std::fs::write(&path, contents).expect("write temp file");
    (dir, path)
}

#[tokio::test]
async fn parse_missing_file_is_not_an_error() {
    // NotFound is the normal state for an unconfigured provider: parsing returns
    // `Ok(vec![])` so the UI doesn't blame the user for a file they haven't created.
    let dir = tempfile::tempdir().expect("create tempdir");
    let path = dir.path().join("nonexistent.json");

    let result = parse_mcp_config_file(&path, MCPProvider::Claude).await;
    assert!(result.is_ok(), "missing config file should return Ok");
    assert!(
        result.unwrap().is_empty(),
        "missing config file should yield zero servers"
    );
}

#[tokio::test]
async fn parse_valid_json_returns_servers_without_error() {
    let json = r#"{"mcpServers":{"my-server":{"command":"node","args":["server.js"]}}}"#;
    let (_dir, path) = write_temp_file(".mcp.json", json);

    let result = parse_mcp_config_file(&path, MCPProvider::Claude).await;
    let servers = result.expect("valid JSON should parse");
    assert_eq!(servers.len(), 1, "expected one parsed server");
}

#[tokio::test]
async fn parse_unreadable_path_emits_read_stage_error() {
    // Passing the tempdir path itself (rather than a file inside it) trips the
    // read fallback: `read_to_string` on a directory returns an I/O error that
    // is not `NotFound`, so we should surface a `Read`-stage parse error.
    let dir = tempfile::tempdir().expect("create tempdir");
    let dir_as_file = dir.path().to_path_buf();

    let result = parse_mcp_config_file(&dir_as_file, MCPProvider::Claude).await;
    let err = result.expect_err("reading a directory should fail");
    assert_eq!(err.stage, ConfigParseStage::Read);
    assert_eq!(err.provider, MCPProvider::Claude);
    assert_eq!(err.path, dir_as_file);
    assert!(
        !err.message.is_empty(),
        "Read-stage error should carry an underlying I/O message"
    );
}

#[tokio::test]
async fn parse_invalid_codex_toml_emits_toml_normalize_error() {
    // Unterminated table header → toml parser rejects.
    let (_dir, path) = write_temp_file("config.toml", "[[[invalid_toml");

    let result = parse_mcp_config_file(&path, MCPProvider::Codex).await;
    let err = result.expect_err("malformed Codex TOML should fail to parse");
    assert_eq!(err.stage, ConfigParseStage::TomlNormalize);
    assert_eq!(err.provider, MCPProvider::Codex);
    assert_eq!(err.path, path);
}

#[tokio::test]
async fn parse_missing_env_var_emits_env_substitute_error() {
    // Use a deliberately unlikely variable name to avoid colliding with the host
    // environment. `remove_var` is set on the same name for safety.
    const VAR: &str = "WARP_TEST_NONEXISTENT_MCP_VAR_9807";
    env::remove_var(VAR);

    let json =
        format!(r#"{{"mcpServers":{{"x":{{"command":"echo","env":{{"K":"${{{VAR}}}"}}}}}}}}"#);
    let (_dir, path) = write_temp_file(".mcp.json", &json);

    let result = parse_mcp_config_file(&path, MCPProvider::Claude).await;
    let err = result.expect_err("missing env var should fail substitution");
    assert_eq!(err.stage, ConfigParseStage::EnvSubstitute);
    assert!(
        err.message.contains(VAR),
        "EnvSubstitute error should name the missing variable, got: {}",
        err.message
    );
}

#[tokio::test]
async fn parse_malformed_json_emits_json_parse_error() {
    // Trailing brace mismatch — serde_json rejects.
    let (_dir, path) = write_temp_file(".mcp.json", r#"{"mcpServers": { broken"#);

    let result = parse_mcp_config_file(&path, MCPProvider::Claude).await;
    let err = result.expect_err("malformed JSON should fail to parse");
    assert_eq!(err.stage, ConfigParseStage::JsonParse);
    assert_eq!(err.provider, MCPProvider::Claude);
}
