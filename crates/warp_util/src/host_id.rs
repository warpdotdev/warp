use std::fmt;

use serde::{Deserialize, Serialize};

/// Opaque identifier for a remote host.
///
/// Returned by the server in `InitializeResponse`. Used by
/// `RemoteServerManager` and downstream features to deduplicate
/// host-scoped models (e.g. `RepoMetadataModel`) across sessions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostId(String);

impl HostId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
