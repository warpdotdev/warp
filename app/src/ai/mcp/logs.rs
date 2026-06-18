use std::path::PathBuf;

use simple_logger::manager::resolve_log_path;
use uuid::Uuid;

/// Per-server MCP log rotation policy.
///
/// Caps each server's on-disk log footprint at `MCP_LOG_MAX_FILE_SIZE_BYTES *
/// (1 + MCP_LOG_MAX_ROTATION)` — one active file plus the rotated tail. For
/// the chosen values that's 10 MiB × 6 = 60 MiB per MCP server, which is far
/// below the 20 GB single-server growth observed in #7723 and large enough to
/// preserve a useful debugging window for a misbehaving server.
///
/// Gated to non-wasm: the only consumer is the native MCP manager, which is
/// itself `#[cfg(not(target_family = "wasm"))]`, so these would be dead code
/// on the wasm build.
#[cfg(not(target_family = "wasm"))]
const MCP_LOG_MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024;
#[cfg(not(target_family = "wasm"))]
const MCP_LOG_MAX_ROTATION: usize = 5;

pub fn relative_log_file_path_from_uuid(uuid: &Uuid) -> PathBuf {
    PathBuf::from(format!("{uuid}.log"))
}

/// The path to the file where an MCP server's log gets written.
pub fn log_file_path_from_uuid(uuid: &Uuid) -> PathBuf {
    resolve_log_path("mcp", relative_log_file_path_from_uuid(uuid))
}

/// Rotation policy applied to every MCP server log writer. Returns `None` only
/// if a future change accidentally sets one of the cap constants to zero;
/// callers can treat `None` as "rotation disabled" and the existing
/// truncate-on-create behavior is preserved.
#[cfg(not(target_family = "wasm"))]
pub fn mcp_log_rotation_config() -> Option<simple_logger::RotationConfig> {
    simple_logger::RotationConfig::new(MCP_LOG_MAX_FILE_SIZE_BYTES, MCP_LOG_MAX_ROTATION)
}
