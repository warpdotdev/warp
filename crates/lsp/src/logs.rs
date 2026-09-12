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
#[path = "logs_tests.rs"]
mod tests;
