use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A globally unique ID for the block that is unique across all sessions.
/// For a block created as a result of pty output, it takes the form {WARP_SESSION_ID}-{NUM_ID},
/// where NUM_ID is a monotonically increasing counter for the session.
/// This is because the block ID comes from the precmd in this case, and it is expensive to create a UUID in the bootstrap script.
/// For manually created blocks within the app, it is a UUID.
#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(transparent)]
pub struct BlockId(String);

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl From<String> for BlockId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<BlockId> for String {
    fn from(value: BlockId) -> Self {
        value.0
    }
}

impl BlockId {
    /// Should only be used for manually created blocks.
    /// Blocks created as a result of pty output should get the block ID from the precmd.
    pub fn new() -> Self {
        format!("manual-{}", Uuid::new_v4()).into()
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for BlockId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<BlockId> for session_sharing_protocol::common::BlockId {
    fn from(value: BlockId) -> Self {
        value.to_string().into()
    }
}

impl From<session_sharing_protocol::common::BlockId> for BlockId {
    fn from(value: session_sharing_protocol::common::BlockId) -> Self {
        value.to_string().into()
    }
}

impl From<session_sharing_protocol::common::BufferId> for BlockId {
    fn from(value: session_sharing_protocol::common::BufferId) -> Self {
        let block_id = session_sharing_protocol::common::BlockId::from(value);
        block_id.into()
    }
}

impl From<BlockId> for session_sharing_protocol::common::BufferId {
    fn from(value: BlockId) -> Self {
        let block_id = session_sharing_protocol::common::BlockId::from(value);
        block_id.into()
    }
}
