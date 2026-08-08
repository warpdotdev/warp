//! Tracks which file contents the model has verifiably observed in each
//! conversation, so `create_file` requests over existing files can be coerced
//! into reviewable replacements exactly when the overwrite is informed.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use warpui::{AppContext, Entity, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AnyFileContent, FileContext};

/// A compact equality fingerprint of a file's full text content.
///
/// CRLF line endings are normalized to LF before hashing because the file-read
/// tooling applies the same normalization before content reaches the model, so
/// raw disk content must fingerprint equal to its normalized form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ContentFingerprint {
    normalized_len: usize,
    hash: u64,
}

impl ContentFingerprint {
    pub(crate) fn of(content: &str) -> Self {
        let normalized = if content.contains('\r') {
            Cow::Owned(content.replace("\r\n", "\n"))
        } else {
            Cow::Borrowed(content)
        };
        let mut hasher = DefaultHasher::new();
        normalized.hash(&mut hasher);
        Self {
            normalized_len: normalized.len(),
            hash: hasher.finish(),
        }
    }
}

/// The full file contents observed within a single conversation, keyed by
/// host-native absolute path.
#[derive(Clone, Default)]
pub(crate) struct ConversationObservedContents {
    by_path: HashMap<String, HashSet<ContentFingerprint>>,
}

impl ConversationObservedContents {
    pub(crate) fn record(&mut self, path: String, fingerprint: ContentFingerprint) {
        self.by_path.entry(path).or_default().insert(fingerprint);
    }

    pub(crate) fn contains(&self, path: &str, fingerprint: ContentFingerprint) -> bool {
        self.by_path
            .get(path)
            .is_some_and(|fingerprints| fingerprints.contains(&fingerprint))
    }
}

/// Singleton recording, per conversation, fingerprints of file contents that
/// have been reported to the model: whole-file reads and contents it authored
/// via accepted file edits. Diff application consults a snapshot of this to
/// decide whether a create over an existing file is an informed overwrite.
#[derive(Default)]
pub struct ObservedFileContents {
    by_conversation: HashMap<AIConversationId, ConversationObservedContents>,
}

impl ObservedFileContents {
    pub(crate) fn record(
        &mut self,
        conversation_id: AIConversationId,
        path: String,
        fingerprint: ContentFingerprint,
    ) {
        self.by_conversation
            .entry(conversation_id)
            .or_default()
            .record(path, fingerprint);
    }

    pub(crate) fn snapshot(
        &self,
        conversation_id: Option<AIConversationId>,
    ) -> ConversationObservedContents {
        conversation_id
            .and_then(|id| self.by_conversation.get(&id))
            .cloned()
            .unwrap_or_default()
    }
}

impl Entity for ObservedFileContents {
    type Event = ();
}

impl SingletonEntity for ObservedFileContents {}

/// Records fingerprints for whole-file text reads reported to the model.
/// A `line_range` of `None` marks a complete read; the line-count check
/// additionally guards against truncated reads that carry no range.
pub(crate) fn record_whole_file_reads<'a>(
    conversation_id: AIConversationId,
    files: impl IntoIterator<Item = &'a FileContext>,
    app: &mut AppContext,
) {
    let fingerprints: Vec<(String, ContentFingerprint)> = files
        .into_iter()
        .filter(|file| file.line_range.is_none())
        .filter_map(|file| match &file.content {
            AnyFileContent::StringContent(content) => (content.lines().count() == file.line_count)
                .then(|| (file.file_name.clone(), ContentFingerprint::of(content))),
            AnyFileContent::BinaryContent(_) => None,
        })
        .collect();
    if fingerprints.is_empty() {
        return;
    }
    ObservedFileContents::handle(app).update(app, |model, _| {
        for (path, fingerprint) in fingerprints {
            model.record(conversation_id, path, fingerprint);
        }
    });
}
