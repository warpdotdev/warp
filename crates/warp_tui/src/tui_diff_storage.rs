//! The TUI surface's diff storage for `RequestFileEdits` actions.
//!
//! [`TuiDiffStorage`] implements the app's surface-agnostic [`DiffStorage`]
//! contract: it holds the resolved diffs and persists them on accept by
//! writing straight through [`FileModel`]. The TUI has no review UI or editor
//! buffers, so final content is derived by applying each diff's deltas to its
//! base. The file-edits view registers one per action with the shared executor
//! and renders a compact summary over it.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai::agent::action_result::RequestFileEditsResult;
use ai::diff_validation::{DiffDelta, DiffType};
use futures::future::BoxFuture;
use futures::FutureExt;
use warp::tui_export::{
    changed_lines_from_op, DiffSessionType, DiffStorage, DiffStorageHelper, FileDiff, FileSnapshot,
    RegisteredDiffStorage, SaveFuture, UpdatedFileState,
};
use warp_files::FileModel;
use warp_util::content_version::ContentVersion;
use warp_util::file::{FileId, FileSaveError};
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

/// Derives the final file content for a diff from its base content and deltas.
///
/// Used because the TUI has no editor buffers; the GUI reads final content
/// from its buffers instead.
fn final_content_from_op(base_content: &str, op: &DiffType) -> Result<String, String> {
    match op {
        DiffType::Create { delta } => Ok(delta.insertion.clone()),
        DiffType::Update { deltas, .. } => apply_deltas_to_content(base_content, deltas),
        DiffType::Delete { .. } => Ok(String::new()),
    }
}

/// Applies line-range replacement deltas to `content`, producing the new content.
fn apply_deltas_to_content(content: &str, deltas: &[DiffDelta]) -> Result<String, String> {
    let mut lines = split_lines_preserving_newlines(content);
    let mut deltas = deltas.to_vec();
    deltas.sort_by_key(|delta| delta.replacement_line_range.start);

    for delta in deltas.into_iter().rev() {
        let start = delta.replacement_line_range.start.saturating_sub(1);
        let end = delta.replacement_line_range.end.saturating_sub(1);
        if start > lines.len() || end > lines.len() || start > end {
            return Err(format!(
                "Diff range {:?} is out of bounds for file with {} lines",
                delta.replacement_line_range,
                lines.len()
            ));
        }
        let mut insertion = delta.insertion;
        // Insertions often lack a trailing newline; without one the splice
        // would run into the next line (mirrors `EditorModel::apply_diffs`).
        if !insertion.is_empty() && !insertion.ends_with('\n') && !content.is_empty() {
            insertion.push('\n');
        }
        let replacement = split_lines_preserving_newlines(&insertion);
        lines.splice(start..end, replacement);
    }

    Ok(lines.concat())
}

/// Splits content into lines while keeping trailing newlines, so reassembly via
/// `concat` reproduces the original byte-for-byte.
fn split_lines_preserving_newlines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.split_inclusive('\n').map(str::to_string).collect()
    }
}

/// The actual write operation performed for a file.
enum PersistAction {
    /// Write the final content at the file's original path.
    Write,
    /// Move the file to the new path and write the final content there.
    Rename(PathBuf),
    /// Delete the file.
    Delete,
}

impl PersistAction {
    /// Decides the write operation once from the diff op and backend. Remote
    /// sessions have no rename primitive, so remote renames fall back to an
    /// in-place write at the original path (and are reported as such).
    fn resolve(op: &DiffType, session_type: &DiffSessionType, path: &str) -> Self {
        match (op, session_type) {
            (DiffType::Delete { .. }, _) => PersistAction::Delete,
            (
                DiffType::Update {
                    rename: Some(to), ..
                },
                DiffSessionType::Local,
            ) if to.to_string_lossy() != path => PersistAction::Rename(to.clone()),
            _ => PersistAction::Write,
        }
    }
}

/// Registers `path` with [`FileModel`] using the session's local/remote backend.
fn register_file(
    file_model: &mut FileModel,
    session_type: &DiffSessionType,
    path: &str,
    ctx: &mut ModelContext<FileModel>,
) -> Result<FileId, FileSaveError> {
    match session_type {
        DiffSessionType::Local => Ok(file_model.register_file_path(Path::new(path), false, ctx)),
        DiffSessionType::Remote(host_id) => {
            let standardized = StandardizedPath::try_new(path)
                .map_err(|_| FileSaveError::RemoteError(format!("Invalid remote path: {path}")))?;
            Ok(file_model.register_remote_file(host_id.clone(), standardized))
        }
    }
}

/// Registers a file with [`FileModel`] and dispatches its write, returning the
/// write's completion future.
fn dispatch_write(
    file_model: &mut FileModel,
    session_type: &DiffSessionType,
    action: &PersistAction,
    path: &str,
    final_content: String,
    ctx: &mut ModelContext<FileModel>,
) -> Result<SaveFuture, FileSaveError> {
    let file_id = register_file(file_model, session_type, path, ctx)?;
    let version = ContentVersion::new();
    let dispatch = match action {
        PersistAction::Delete => file_model.delete(file_id, version, ctx),
        PersistAction::Rename(to) => {
            file_model.rename_and_save(file_id, to.clone(), final_content, version, ctx)
        }
        PersistAction::Write => file_model.save(file_id, final_content, version, ctx),
    };
    // The write future captures its target path up front; release the temporary
    // registration either way so FileModel state doesn't grow unboundedly.
    file_model.unsubscribe(file_id, ctx);
    dispatch
}

/// Builds a file's report state and result-diff inputs to mirror the write
/// that `action` will actually perform.
fn persist_outcome(
    action: &PersistAction,
    diff: &FileDiff,
    path: &str,
    final_content: &str,
) -> FileSnapshot {
    let changed_lines = changed_lines_from_op(&diff.diff_type);
    match action {
        PersistAction::Delete => FileSnapshot {
            updated: None,
            deleted_paths: vec![path.to_owned()],
            diff_base: diff.base.content.clone(),
            diff_new: String::new(),
            diff_name: path.to_owned(),
        },
        PersistAction::Rename(to) => {
            let target = to.to_string_lossy().to_string();
            FileSnapshot {
                updated: Some(UpdatedFileState {
                    path: target.clone(),
                    changed_lines,
                    final_content: final_content.to_owned(),
                    was_edited: false,
                }),
                deleted_paths: vec![path.to_owned()],
                diff_base: diff.base.content.clone(),
                diff_new: final_content.to_owned(),
                diff_name: target,
            }
        }
        PersistAction::Write => FileSnapshot {
            updated: Some(UpdatedFileState {
                path: path.to_owned(),
                changed_lines,
                final_content: final_content.to_owned(),
                was_edited: false,
            }),
            deleted_paths: Vec::new(),
            diff_base: diff.base.content.clone(),
            diff_new: final_content.to_owned(),
            diff_name: path.to_owned(),
        },
    }
}

/// A save future that fails immediately with `error`.
fn ready_save_failure(error: FileSaveError) -> SaveFuture {
    futures::future::ready(Err(Arc::new(error))).boxed()
}

/// Events emitted by [`TuiDiffStorage`].
pub(crate) enum TuiDiffStorageEvent {
    /// The executor seeded the storage with resolved diffs.
    CandidateDiffsSet,
}

/// The TUI surface's diff storage: holds the resolved diffs and persists them
/// by writing straight through [`FileModel`], with no review UI or editor
/// buffers of its own.
pub(crate) struct TuiDiffStorage {
    diffs: Vec<FileDiff>,
    session_type: DiffSessionType,
}

impl TuiDiffStorage {
    /// Creates storage over resolved diffs.
    pub(crate) fn new(diffs: Vec<FileDiff>, session_type: DiffSessionType) -> Self {
        Self {
            diffs,
            session_type,
        }
    }

    /// The stored diffs (for views rendering a summary over this storage).
    pub(crate) fn diffs(&self) -> &[FileDiff] {
        &self.diffs
    }

    /// Replaces the stored diffs and session backend.
    fn set_candidate_diffs(&mut self, diffs: Vec<FileDiff>, session_type: DiffSessionType) {
        self.diffs = diffs;
        self.session_type = session_type;
    }
}

impl DiffStorage for TuiDiffStorage {
    fn snapshot_pending_files(&self, _app: &AppContext) -> Vec<FileSnapshot> {
        self.diffs
            .iter()
            .map(|diff| {
                let path = diff.file_path();
                let action = PersistAction::resolve(&diff.diff_type, &self.session_type, &path);
                // A derivation failure is surfaced by `start_saving`; the
                // snapshot is unused when the accept fails.
                let final_content =
                    final_content_from_op(&diff.base.content, &diff.diff_type).unwrap_or_default();
                persist_outcome(&action, diff, &path, &final_content)
            })
            .collect()
    }

    fn start_saving(&mut self, app: &mut AppContext) -> Vec<SaveFuture> {
        let file_model = FileModel::handle(app);
        let session_type = self.session_type.clone();
        self.diffs
            .iter()
            .map(|diff| {
                let path = diff.file_path();
                let final_content = match final_content_from_op(&diff.base.content, &diff.diff_type)
                {
                    Ok(content) => content,
                    Err(error) => return ready_save_failure(FileSaveError::Other(error)),
                };
                let action = PersistAction::resolve(&diff.diff_type, &session_type, &path);
                file_model
                    .update(app, |file_model, ctx| {
                        dispatch_write(
                            file_model,
                            &session_type,
                            &action,
                            &path,
                            final_content,
                            ctx,
                        )
                    })
                    .unwrap_or_else(ready_save_failure)
            })
            .collect()
    }
}

impl Entity for TuiDiffStorage {
    type Event = TuiDiffStorageEvent;
}

/// The handle the TUI registers as the executor's storage.
///
/// Wraps the model handle because [`RegisteredDiffStorage`] and
/// [`ModelHandle`] are both foreign to this crate, so the orphan rule forbids
/// implementing the trait on the handle directly.
pub(crate) struct TuiDiffStorageHandle {
    storage: ModelHandle<TuiDiffStorage>,
}

impl TuiDiffStorageHandle {
    /// Wraps a storage handle for registration with the executor.
    pub(crate) fn new(storage: ModelHandle<TuiDiffStorage>) -> Self {
        Self { storage }
    }
}

impl RegisteredDiffStorage for TuiDiffStorageHandle {
    fn set_candidate_diffs(
        &self,
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        app: &mut AppContext,
    ) {
        self.storage.update(app, |model, ctx| {
            model.set_candidate_diffs(diffs, session_type);
            ctx.emit(TuiDiffStorageEvent::CandidateDiffsSet);
        });
    }

    fn accept_and_save(&self, app: &mut AppContext) -> BoxFuture<'static, RequestFileEditsResult> {
        self.storage.update(app, |model, ctx| {
            DiffStorageHelper::accept_and_save(model, ctx)
        })
    }
}

#[cfg(test)]
#[path = "tui_diff_storage_tests.rs"]
mod tests;
