use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::ai::agent::conversation::AIConversationId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileRevision {
    Present {
        last_modified: Option<SystemTime>,
        content_digest: [u8; 32],
    },
    Missing,
    Uneditable,
}

impl FileRevision {
    pub(super) fn present(content: impl AsRef<[u8]>, last_modified: Option<SystemTime>) -> Self {
        Self::Present {
            last_modified,
            content_digest: Sha256::digest(content).into(),
        }
    }

    pub(super) fn from_digest(content_digest: [u8; 32]) -> Self {
        Self::Present {
            last_modified: None,
            content_digest,
        }
    }

    pub(super) fn persistence_revision(self) -> remote_server::ExpectedFileRevision {
        match self {
            Self::Present {
                last_modified,
                content_digest,
            } => remote_server::ExpectedFileRevision::Present {
                content_digest,
                last_modified,
            },
            Self::Missing => remote_server::ExpectedFileRevision::Missing,
            Self::Uneditable => remote_server::ExpectedFileRevision::Uneditable,
        }
    }

    pub(super) fn matches(self, current: Self) -> bool {
        match (self, current) {
            (
                Self::Present {
                    content_digest: expected_digest,
                    last_modified: expected_modified,
                },
                Self::Present {
                    content_digest: current_digest,
                    last_modified: current_modified,
                },
            ) => {
                expected_digest == current_digest
                    && expected_modified.is_none_or(|expected| Some(expected) == current_modified)
            }
            (Self::Missing, Self::Missing) => true,
            (Self::Uneditable, _) | (_, Self::Uneditable) => false,
            _ => false,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct FileRevisionTracker {
    revisions: Arc<Mutex<HashMap<AIConversationId, HashMap<String, FileRevision>>>>,
}

impl FileRevisionTracker {
    pub fn expected_revisions(
        &self,
        conversation_id: AIConversationId,
        paths: impl IntoIterator<Item = String>,
    ) -> HashMap<String, FileRevision> {
        let revisions = self.revisions.lock();
        let Some(conversation_revisions) = revisions.get(&conversation_id) else {
            return HashMap::new();
        };

        paths
            .into_iter()
            .filter_map(|path| {
                conversation_revisions
                    .get(&path)
                    .copied()
                    .map(|revision| (path, revision))
            })
            .collect()
    }

    pub fn record_revisions(
        &self,
        conversation_id: AIConversationId,
        revisions_to_record: impl IntoIterator<Item = (String, FileRevision)>,
    ) {
        self.revisions
            .lock()
            .entry(conversation_id)
            .or_default()
            .extend(revisions_to_record);
    }
}

pub(super) fn revision_from_remote_context(
    context: remote_server::proto::FileContextProto,
) -> FileRevision {
    context
        .content_sha256
        .try_into()
        .map(FileRevision::from_digest)
        .unwrap_or(FileRevision::Uneditable)
}

pub(super) fn is_missing_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not found") || message.contains("does not exist")
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "file_revisions_tests.rs"]
mod tests;
