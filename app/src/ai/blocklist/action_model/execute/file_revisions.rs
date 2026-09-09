use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;

use crate::ai::agent::FileContext;
use crate::ai::agent::conversation::AIConversationId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FileRevision {
    Modified(SystemTime),
    Missing,
}

#[derive(Clone, Default)]
pub(super) struct FileRevisionTracker {
    revisions: Arc<Mutex<HashMap<AIConversationId, HashMap<String, FileRevision>>>>,
}

impl FileRevisionTracker {
    pub fn record_file_contexts(&self, conversation_id: AIConversationId, files: &[FileContext]) {
        let mut revisions = self.revisions.lock();
        let conversation_revisions = revisions.entry(conversation_id).or_default();
        for file in files {
            if let Some(last_modified) = file.last_modified {
                conversation_revisions.insert(
                    file.file_name.clone(),
                    FileRevision::Modified(last_modified),
                );
            } else {
                conversation_revisions.remove(&file.file_name);
            }
        }
    }

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
        revisions_to_record: impl IntoIterator<Item = (String, SystemTime)>,
    ) {
        self.revisions
            .lock()
            .entry(conversation_id)
            .or_default()
            .extend(
                revisions_to_record
                    .into_iter()
                    .map(|(path, revision)| (path, FileRevision::Modified(revision))),
            );
    }

    pub fn record_missing_files(
        &self,
        conversation_id: AIConversationId,
        paths: impl IntoIterator<Item = String>,
    ) {
        self.revisions
            .lock()
            .entry(conversation_id)
            .or_default()
            .extend(paths.into_iter().map(|path| (path, FileRevision::Missing)));
    }
}
