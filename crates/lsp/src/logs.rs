use simple_logger::RotationConfig;

/// Per-server LSP log rotation policy.
///
/// Caps each LSP server's on-disk log footprint at `LSP_LOG_MAX_FILE_SIZE_BYTES *
/// (1 + LSP_LOG_MAX_ROTATION)` — one active file plus the rotated tail. Matches
/// the MCP rotation policy shipped in #10874 (#7723): 10 MiB × 6 = 60 MiB per
/// LSP server per workspace, well below the multi-GB unbounded-growth observed
/// for verbose servers like `rust-analyzer` and large enough to preserve a
/// useful debugging window across a long-running session (#10877).
const LSP_LOG_MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024;
const LSP_LOG_MAX_ROTATION: usize = 5;

/// Rotation policy applied to every LSP server log writer. Returns `None` only
/// if a future change accidentally sets one of the cap constants to zero;
/// callers can treat `None` as "rotation disabled" and the existing
/// truncate-on-create behavior is preserved.
pub fn lsp_log_rotation_config() -> Option<RotationConfig> {
    RotationConfig::new(LSP_LOG_MAX_FILE_SIZE_BYTES, LSP_LOG_MAX_ROTATION)
}

#[cfg(test)]
mod tests {
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
}
