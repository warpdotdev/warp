pub mod actions;
mod context_menu;
pub mod editor;
pub mod file;
pub mod link;
mod styles;

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use warpui::AppContext;

use warp_server_client::ids::{HashableId, SyncId};

/// This is the notebook_id in the database associated with this notebook.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct NotebookId(String);

#[cfg(any(test, feature = "test-util"))]
impl From<i64> for NotebookId {
    fn from(id: i64) -> Self {
        Self(format!("test_uid{}", id.abs()))
    }
}

impl From<String> for NotebookId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<NotebookId> for String {
    fn from(id: NotebookId) -> String {
        id.0
    }
}

impl std::fmt::Display for NotebookId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.0)
    }
}

impl HashableId for NotebookId {
    fn to_hash(&self) -> String {
        format!("Notebook-{}", self)
    }

    fn from_hash(hash: &str) -> Option<Self> {
        hash.strip_prefix("Notebook-")
            .map(|id| Self(id.to_string()))
    }
}

impl From<NotebookId> for SyncId {
    fn from(id: NotebookId) -> Self {
        Self::LegacyObjectId(id.0)
    }
}

/// A notebook location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum NotebookLocation {
    /// A notebook backed by a local file.
    LocalFile,
    /// A notebook backed by a remote file.
    RemoteFile,
}

/// Initialize notebooks-related keybindings.
pub fn init(app: &mut AppContext) {
    self::file::init(app);
    self::editor::view::init(app);
}

/// Post process a notebook's content read from an external system. This cleans up extra
/// whitespace, and, in the future, may filter out unsupported syntax extensions.
///
/// See CLD-944.
pub fn post_process_notebook(data: &str) -> String {
    // TODO(kevin): We should not strip out newlines in the code block.
    data.lines().filter(|line| !line.is_empty()).join("\n")
}
