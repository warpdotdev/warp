use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum PaletteMode {
    Command,
    Navigation,
    LaunchConfig,
    WarpDrive,
    Files,
    Conversations,
    /// Search every known CLI-agent session by task, project or directory, and
    /// resume the one that is picked.
    SessionSearch,
}
