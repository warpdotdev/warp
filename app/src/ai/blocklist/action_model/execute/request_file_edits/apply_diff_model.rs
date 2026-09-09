//! Entity submodel that encapsulates all filesystem access for diff application.
//!
//! The executor holds a [`ModelHandle<ApplyDiffModel>`] and calls
//! [`ApplyDiffModel::apply_diffs`] without knowing whether the session is local
//! or remote. Internally the method resolves the session context and remote
//! client from the model context, then dispatches:
//!
//! - **Local**: calls [`apply_edits`] with a `std::fs`-backed closure.
//! - **Remote**: calls [`apply_edits`] with a [`RemoteServerClient`]-backed closure.

use ai::diff_validation::{AIRequestedCodeDiff, ParsedDiff};
use futures::FutureExt;
use vec1::Vec1;
use warpui::r#async::BoxFuture;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity as _};

use super::super::file_revisions::FileRevisionTracker;
use super::diff_application::{DiffApplicationError, FileReadResult, apply_edits};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIIdentifiers, FileEdit};
use crate::ai::blocklist::SessionContext;
use crate::ai::blocklist::diff_storage::FileSnapshot;
use crate::auth::AuthStateProvider;
use crate::terminal::model::session::active_session::ActiveSession;

/// Entity submodel that encapsulates filesystem access for diff application.
///
/// Held as a [`ModelHandle`] by the [`super::RequestFileEditsExecutor`].
pub(crate) struct ApplyDiffModel {
    active_session: ModelHandle<ActiveSession>,
    file_revision_tracker: FileRevisionTracker,
}

fn revisions_from_persisted_files(
    files: &[FileSnapshot],
) -> Vec<(String, super::super::file_revisions::FileRevision)> {
    files
        .iter()
        .flat_map(|file| {
            let updated = file.updated.as_ref().map(|updated| {
                (
                    updated.path.clone(),
                    super::super::file_revisions::FileRevision::present(
                        &updated.final_content,
                        None,
                    ),
                )
            });
            updated.into_iter().chain(
                file.deleted_paths
                    .iter()
                    .cloned()
                    .map(|path| (path, super::super::file_revisions::FileRevision::Missing)),
            )
        })
        .collect()
}

fn absolute_edit_paths(edits: &[FileEdit], session_context: &SessionContext) -> Vec<String> {
    let mut paths = Vec::new();
    for edit in edits {
        if let Some(path) = edit.file() {
            paths.push(crate::ai::paths::host_native_absolute_path(
                path,
                session_context.shell(),
                session_context.current_working_directory(),
            ));
        }
        if let FileEdit::Edit(ParsedDiff::V4AEdit {
            move_to: Some(path),
            ..
        }) = edit
        {
            paths.push(crate::ai::paths::host_native_absolute_path(
                path,
                session_context.shell(),
                session_context.current_working_directory(),
            ));
        }
    }
    paths.sort_unstable();
    paths.dedup();
    paths
}

impl Entity for ApplyDiffModel {
    type Event = ();
}

impl ApplyDiffModel {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        file_revision_tracker: FileRevisionTracker,
    ) -> Self {
        Self {
            active_session,
            file_revision_tracker,
        }
    }

    pub fn persistence_revisions(
        &self,
        edits: &[FileEdit],
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> std::collections::HashMap<String, warp_files::ExpectedFileRevision> {
        let session_context = SessionContext::from_session(self.active_session.as_ref(ctx), ctx);
        self.file_revision_tracker
            .expected_revisions(
                conversation_id,
                absolute_edit_paths(edits, &session_context),
            )
            .into_iter()
            .map(|(path, revision)| (path, revision.persistence_revision()))
            .collect()
    }

    /// Resolves session context and remote client from the model context, then
    /// returns a future that applies the edits locally or remotely.
    pub fn apply_diffs(
        &self,
        edits: Vec<FileEdit>,
        conversation_id: AIConversationId,
        ai_identifiers: &AIIdentifiers,
        passive_diff: bool,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, Result<Vec<AIRequestedCodeDiff>, Vec1<DiffApplicationError>>> {
        let session_context = SessionContext::from_session(self.active_session.as_ref(ctx), ctx);
        let background_executor = ctx.background_executor();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let ai_identifiers = ai_identifiers.clone();
        let expected_revisions = self.file_revision_tracker.expected_revisions(
            conversation_id,
            absolute_edit_paths(&edits, &session_context),
        );

        let host_request_handle = session_context.host_id().map(|host_id| {
            remote_server::manager::RemoteServerManager::as_ref(ctx).host_request_handle(host_id)
        });

        let is_remote = session_context.is_remote();
        let fut = async move {
            if is_remote {
                match host_request_handle {
                    Some(handle) => {
                        apply_edits(
                            edits,
                            &session_context,
                            &ai_identifiers,
                            background_executor,
                            auth_state,
                            passive_diff,
                            |path| {
                                let handle = &handle;
                                let expected_revision = expected_revisions.get(&path).copied();
                                async move {
                                    read_remote_file(handle, &path)
                                        .await
                                        .with_expected_revision(expected_revision)
                                }
                            },
                        )
                        .await
                    }
                    None => Err(vec1::vec1![
                        DiffApplicationError::RemoteFileOperationsUnsupported
                    ]),
                }
            } else {
                apply_edits(
                    edits,
                    &session_context,
                    &ai_identifiers,
                    background_executor,
                    auth_state,
                    passive_diff,
                    |path| {
                        let expected_revision = expected_revisions.get(&path).copied();
                        async move {
                            read_local_file(path).with_expected_revision(expected_revision)
                        }
                    },
                )
                .await
            }
        };
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                fut.boxed_local()
            } else {
                fut.boxed()
            }
        }
    }

    pub fn track_persisted_revisions(
        &self,
        files: &[FileSnapshot],
        conversation_id: AIConversationId,
    ) {
        self.file_revision_tracker
            .record_revisions(conversation_id, revisions_from_persisted_files(files));
    }
}

// ── Remote file reading ──────────────────────────────────────────────────────────

/// Per-file byte limit for remote diff application (10 MB).
const MAX_DIFF_READ_BYTES: u32 = 10_000_000;

fn read_local_file(path: String) -> FileReadResult {
    match std::fs::read_to_string(&path) {
        Ok(content) => FileReadResult::Found {
            content,
            last_modified: std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok(),
        },
        Err(err) => FileReadResult::from(Err(err)),
    }
}

async fn read_remote_file(
    handle: &remote_server::manager::HostRequestHandle,
    path: &str,
) -> FileReadResult {
    let request = remote_server::proto::ReadFileContextRequest {
        files: vec![remote_server::proto::ReadFileContextFile {
            path: path.to_string(),
            line_ranges: vec![],
        }],
        max_file_bytes: Some(MAX_DIFF_READ_BYTES),
        max_batch_bytes: None,
    };
    match handle.read_file_context(request).await {
        Ok(response) => {
            if let Some(fc) = response.file_contexts.into_iter().next() {
                // A whole-file read that was truncated by the byte limit will
                // have line_range_start/end set even though no ranges were
                // requested. Detect this and fail explicitly rather than
                // applying the diff to partial content.
                if fc.line_range_start.is_some() || fc.line_range_end.is_some() {
                    return FileReadResult::ReadError(format!(
                        "File exceeds the {MAX_DIFF_READ_BYTES}-byte limit for remote diff \
                         application and was truncated. The diff cannot be applied safely."
                    ));
                }
                match fc.content {
                    Some(remote_server::proto::file_context_proto::Content::TextContent(
                        content,
                    )) => FileReadResult::Found {
                        content,
                        last_modified: fc.last_modified_epoch_millis.map(|millis| {
                            std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis)
                        }),
                    },
                    Some(remote_server::proto::file_context_proto::Content::BinaryContent(_)) => {
                        // apply-diff only works with text files
                        FileReadResult::ReadError("File is binary".to_string())
                    }
                    None => FileReadResult::Found {
                        content: String::new(),
                        last_modified: fc.last_modified_epoch_millis.map(|millis| {
                            std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis)
                        }),
                    },
                }
            } else if let Some(failed) = response.failed_files.into_iter().next() {
                let message = failed
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "Unknown error".to_string());
                if message.contains("not found") || message.contains("Not found") {
                    FileReadResult::NotFound
                } else {
                    FileReadResult::ReadError(message)
                }
            } else {
                FileReadResult::NotFound
            }
        }
        Err(err) => FileReadResult::ReadError(format!("{err}")),
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "apply_diff_model_tests.rs"]
mod tests;
