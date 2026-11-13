//! A Model Context Protocol server that lets an external agent harness drive this computer's
//! mouse, keyboard, and screen through Warp's computer-use implementation.
//!
//! The harness's own model drives the tools directly, so the server owns the translation that
//! warp-server performs for Warp's native computer-use agent: mapping coordinates in the
//! downscaled screenshots the model sees onto native screen pixels, and turning key names into
//! platform key presses.

mod actions;
mod coords;
mod keys;
mod publish;
mod recording;
mod server;
mod session;
mod stdio;
#[cfg(test)]
mod test_util;

use std::fmt;
use std::time::Duration;

pub use coords::ImageConstraints;
pub use keys::KeyDialect;
pub use recording::RecordingDestination;
pub use server::ComputerUseMcpServer;
pub use stdio::run_stdio;

/// The name the server introduces itself by. Transcripts identify its tool calls by this name,
/// and viewers title those calls from the tool and argument names in `tool_contract.json`.
pub const SERVER_NAME: &str = "warp-computer-use";

/// Configuration for a computer-use MCP server.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputerUseMcpConfig {
    /// Size limits for screenshots returned to the model.
    pub image_constraints: ImageConstraints,
    /// The key-name conventions of the model driving the server.
    pub key_dialect: KeyDialect,
    /// How long to wait after an action before capturing its screenshot, so the UI has time to
    /// react to the input.
    pub settle_delay: Duration,
    /// Where finished screen recordings go. `None` disables the recording tools.
    pub recording: Option<RecordingDestination>,
}

/// A failure the model can act on, reported as an error result of the tool call rather than as
/// a protocol error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ToolError(String);

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}
