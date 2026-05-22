use std::path::PathBuf;

use simple_logger::manager::resolve_log_path;
use uuid::Uuid;

pub fn relative_log_file_path_from_uuid(uuid: &Uuid) -> PathBuf {
    PathBuf::from(format!("{uuid}.log"))
}

/// The path to the file where an MCP server's log gets written.
pub fn log_file_path_from_uuid(uuid: &Uuid) -> PathBuf {
    resolve_log_path("mcp", relative_log_file_path_from_uuid(uuid))
}
