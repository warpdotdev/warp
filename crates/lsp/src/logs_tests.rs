use super::*;

#[test]
fn lsp_log_rotation_config_uses_expected_caps() {
    let cfg = lsp_log_rotation_config().expect("config should be Some");
    assert_eq!(cfg.max_file_size_bytes(), LSP_LOG_MAX_FILE_SIZE_BYTES);
    assert_eq!(cfg.max_rotation(), LSP_LOG_MAX_ROTATION);
}

#[test]
fn lsp_log_rotation_caps_match_mcp_namespace() {
    // The LSP rotation policy intentionally mirrors the MCP policy
    // shipped in #10874. If MCP's constants ever change, this test
    // will need an explicit update — that's the point.
    let cfg = lsp_log_rotation_config().unwrap();
    assert_eq!(cfg.max_file_size_bytes(), 10 * 1024 * 1024);
    assert_eq!(cfg.max_rotation(), 5);
}
