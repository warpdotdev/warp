use serde::{Deserialize, Serialize};

/// Unique identifier for a terminal session.
///
/// Each bootstrapped subshell (including SSH sessions) gets its own `SessionId`.
/// This type is defined in `warp_core` so that lower-level crates (e.g. `repo_metadata`)
/// can reference it without depending on the `app` crate.
#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SessionId(u64);

impl SessionId {
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<u64> for SessionId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

impl From<SessionId> for u64 {
    fn from(session_id: SessionId) -> Self {
        session_id.as_u64()
    }
}
