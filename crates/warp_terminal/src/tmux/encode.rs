use super::parser::PaneId;

const SEND_KEYS_CHUNK_BYTES: usize = 128;

/// Correlated query for the initial window/layout dump. tmux 3.6a `new-session -A`
/// enters control mode without `%layout-change` or `%window-pane-changed`.
pub const LIST_WINDOWS_LAYOUT_COMMAND: &str = "list-windows -F '#{window_id} #{window_layout}'\n";

/// Dedicated socket name for Warp-managed `/tmux` (`tmux -CC -L warp-control-v1 …`).
pub const WARP_CONTROL_SOCKET_NAME: &str = "warp-control-v1";

/// Keep the isolated Warp server alive if the last pane exits while a control client is attached.
pub const EXIT_EMPTY_OFF_COMMAND: &str = "set -s exit-empty off\n";

pub fn refresh_client_command(columns: usize, rows: usize) -> String {
    format!("refresh-client -C {columns}x{rows}\n")
}

pub fn send_keys_command(pane_id: &PaneId, bytes: &[u8]) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for chunk in bytes.chunks(SEND_KEYS_CHUNK_BYTES) {
        out.extend_from_slice(format!("send-keys -t {} -H", pane_id.as_str()).as_bytes());
        for byte in chunk {
            out.extend_from_slice(format!(" {byte:02x}").as_bytes());
        }
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
#[path = "encode_tests.rs"]
mod tests;
