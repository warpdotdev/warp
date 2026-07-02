//! Surface-owned storage and the shared persistence flow for `RequestFileEdits`
//! diffs.
//!
//! Every surface that stores pending diffs — the GUI `CodeDiffView` and the
//! TUI's [`TuiDiffStorage`] below — implements [`DiffStorage`], a required-
//! methods-only contract: an accept-time snapshot of per-file state plus the
//! surface-specific write kickoff (the GUI saves through its editor buffers,
//! while [`TuiDiffStorage`] dispatches writes to `FileModel` via the helpers
//! in this module). The shared save-completion flow is [`DiffStorageHelper`],
//! blanket-implemented for every `DiffStorage` so no surface can override it:
//! it joins the per-file save futures, computes each file's result diff, and
//! assembles the final [`RequestFileEditsResult`], so every surface produces
//! results through the same code.
//!
//! The executor knows surfaces only through [`RegisteredDiffStorage`], a small
//! object-safe handle trait, because GUI `ViewHandle`s and model `ModelHandle`s
//! share no common handle type. Each surface's handle type implements it
//! directly, delegating to its entity's [`DiffStorageHelper`] flow. Every
//! surface must register its storage before the action's diffs resolve
//! (`register_requested_edits`); preprocess and execute assume a registered
//! storage.
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use futures::future::{join_all, BoxFuture};
use futures::FutureExt;
use itertools::Itertools;
use warp_editor::multiline::AnyMultilineString;
use warp_util::file::FileSaveError;
use warpui::{AppContext, Entity, ModelHandle};
#[cfg(not(target_family = "wasm"))]
use {
    ai::diff_validation::{DiffDelta, DiffType},
    std::path::{Path, PathBuf},
    warp_files::FileModel,
    warp_util::content_version::ContentVersion,
    warp_util::file::FileId,
    warp_util::standardized_path::StandardizedPath,
    warpui::{ModelContext, SingletonEntity},
};

use crate::ai::agent::{
    AnyFileContent, FileContext, FileLocations, RequestFileEditsResult, UpdatedFileContext,
};
#[cfg(not(target_family = "wasm"))]
use crate::ai::blocklist::diff_types::changed_lines_from_op;
use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff};
use crate::code::editor::compute_unified_diff;
use crate::code::DiffResult;

const APPLY_DIFF_RESULT_CONTEXT_LINES: usize = 10;

/// Resolves with the outcome of one file's dispatched save.
pub type SaveFuture = BoxFuture<'static, Result<(), Arc<FileSaveError>>>;

/// A surface that stores pending file-edit diffs and persists them on accept.
///
/// This trait is only ever implemented, never imported for its methods: every
/// method is required — the accept-time snapshot (the fields live on each
/// impl, since traits cannot hold state) plus the surface-specific write
/// kickoff ([`Self::start_saving`]). The shared save-completion flow lives on
/// [`DiffStorageHelper`]; callers drive an accept solely through
/// [`DiffStorageHelper::accept_and_save`].
pub trait DiffStorage {
    /// Snapshot of per-file state for result assembly, captured as the accept
    /// kicks off: reported paths, changed lines, contents, and user-edit
    /// flags. Snapshotting at kickoff means the result reports exactly the
    /// content handed to [`Self::start_saving`].
    fn snapshot_pending_files(&self, app: &AppContext) -> Vec<FileSnapshot>;

    /// Kicks off persistence for every pending file, returning each
    /// dispatched save's completion future.
    ///
    /// The surface-specific hook invoked by
    /// [`DiffStorageHelper::accept_and_save`] — never called directly by
    /// callers. The GUI saves through its editor buffers; [`TuiDiffStorage`]
    /// dispatches writes to `FileModel`.
    fn start_saving(&mut self, app: &mut AppContext) -> Vec<SaveFuture>;
}

/// The shared save-completion flow over an impl of [`DiffStorage`].
///
/// Defined within a separate trait rather than a default implementation of
/// `DiffStorage` so implementations cannot errantly override it (the same
/// convention as `AIBlockModelHelper`).
pub trait DiffStorageHelper {
    /// The entry point for accepting a surface's diffs: snapshots per-file
    /// state, persists every file, and resolves with the assembled result
    /// once every save completes.
    fn accept_and_save(
        &mut self,
        app: &mut AppContext,
    ) -> BoxFuture<'static, RequestFileEditsResult>;
}

impl<T: DiffStorage> DiffStorageHelper for T {
    fn accept_and_save(
        &mut self,
        app: &mut AppContext,
    ) -> BoxFuture<'static, RequestFileEditsResult> {
        let files = self.snapshot_pending_files(app);
        let saves = self.start_saving(app);
        async move {
            let save_errors = join_all(saves)
                .await
                .into_iter()
                .filter_map(Result::err)
                .collect_vec();
            if !save_errors.is_empty() {
                return save_failure_result(&save_errors);
            }

            let mut combined = DiffResult::default();
            for file in &files {
                let base = AnyMultilineString::infer(file.diff_base.clone());
                let new = AnyMultilineString::infer(file.diff_new.clone());
                combined += &compute_unified_diff(
                    base.to_format().as_ref(),
                    new.to_format().as_ref(),
                    &file.diff_name,
                )
                .await;
            }
            assemble_result(combined, files)
        }
        .boxed()
    }
}

/// The executor-facing handle over a registered [`DiffStorage`] surface.
///
/// A separate trait from [`DiffStorage`] because the executor holds
/// surfaces by handle, and GUI view handles and model handles share no common
/// type. Each surface's handle type (e.g. `WeakViewHandle<CodeDiffView>`)
/// implements this directly, delegating each call to its entity's
/// [`DiffStorageHelper`] flow.
pub trait RegisteredDiffStorage {
    /// Pushes resolved diffs into the surface (called when preprocess resolves).
    fn set_candidate_diffs(
        &self,
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        app: &mut AppContext,
    );

    /// Persists all diffs, resolving with the result reported to the LLM.
    fn accept_and_save(&self, app: &mut AppContext) -> BoxFuture<'static, RequestFileEditsResult>;
}

/// One file's contribution to the assembled result, snapshotted at accept
/// time.
#[derive(Clone)]
pub struct FileSnapshot {
    /// The updated file, reported at its final path; `None` for deletions.
    pub updated: Option<UpdatedFileState>,
    /// Paths reported as deleted (the deleted file, or a rename's source path).
    pub deleted_paths: Vec<String>,
    /// Base content the result diff is computed against.
    pub diff_base: String,
    /// Final content the result diff is computed from.
    pub diff_new: String,
    /// Path used for the result diff's header.
    pub diff_name: String,
}

/// Report state for a created or updated file.
#[derive(Clone)]
pub struct UpdatedFileState {
    /// Path the update is reported at (the rename target when renamed).
    pub path: String,
    /// 1-indexed changed line ranges.
    pub changed_lines: Vec<Range<usize>>,
    /// Final file content.
    pub final_content: String,
    /// Whether the user hand-edited the content during review.
    pub was_edited: bool,
}

/// Fails the whole edit when any file failed to save.
fn save_failure_result(errors: &[Arc<FileSaveError>]) -> RequestFileEditsResult {
    let error = errors
        .iter()
        .map(|error| match error.as_ref() {
            FileSaveError::IOError { error, path } => {
                format!("Failed to save file {path:?}: {error}")
            }
            other => other.to_string(),
        })
        .join("\n");
    RequestFileEditsResult::DiffApplicationFailed { error }
}

/// Combines per-file report state and the combined result diff into one
/// [`RequestFileEditsResult`].
fn assemble_result(combined: DiffResult, files: Vec<FileSnapshot>) -> RequestFileEditsResult {
    let mut updated_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut content_map: HashMap<String, String> = HashMap::new();
    for file in files {
        if let Some(updated) = file.updated {
            content_map.insert(updated.path.clone(), updated.final_content);
            updated_files.push((
                FileLocations {
                    name: updated.path,
                    lines: updated.changed_lines,
                },
                updated.was_edited,
            ));
        }
        deleted_files.extend(file.deleted_paths);
    }

    RequestFileEditsResult::Success {
        diff: combined.unified_diff,
        updated_files: updated_file_contexts_from_content_map(&updated_files, &content_map),
        deleted_files,
        lines_added: combined.lines_added,
        lines_removed: combined.lines_removed,
    }
}

/// Expands each updated file's changed lines with surrounding context and
/// extracts the corresponding fragments from the final file content.
fn updated_file_contexts_from_content_map(
    updated_files: &[(FileLocations, bool)],
    content_map: &HashMap<String, String>,
) -> Vec<UpdatedFileContext> {
    updated_files
        .iter()
        .flat_map(|(file_location, was_edited)| {
            let content = content_map
                .get(&file_location.name)
                .cloned()
                .unwrap_or_default();
            let line_count = content.lines().count();

            let mut file_location = file_location.clone();
            file_location.expand_surrounding_context(APPLY_DIFF_RESULT_CONTEXT_LINES);
            clamp_to_file_context_range_start(&mut file_location);

            if file_location.lines.is_empty() {
                return vec![UpdatedFileContext {
                    was_edited_by_user: *was_edited,
                    file_context: FileContext {
                        file_name: file_location.name,
                        content: AnyFileContent::StringContent(content),
                        line_range: None,
                        last_modified: None,
                        line_count,
                    },
                }];
            }

            let lines = content.lines().collect_vec();
            file_location
                .lines
                .into_iter()
                .map(|range| {
                    let start = range.start.saturating_sub(1).min(lines.len());
                    let end = range.end.saturating_sub(1).min(lines.len());
                    let fragment = if start >= end {
                        String::new()
                    } else {
                        lines[start..end].join("\n")
                    };

                    UpdatedFileContext {
                        was_edited_by_user: *was_edited,
                        file_context: FileContext {
                            file_name: file_location.name.clone(),
                            content: AnyFileContent::StringContent(fragment),
                            line_range: Some(range),
                            last_modified: None,
                            line_count,
                        },
                    }
                })
                .collect_vec()
        })
        .collect()
}

/// Clamps line ranges to the 1-indexed space used by file contexts.
fn clamp_to_file_context_range_start(file_location: &mut FileLocations) {
    for range in &mut file_location.lines {
        range.start = range.start.max(1);
        range.end = range.end.max(range.start);
    }
}

/// Derives the final file content for a diff from its base content and deltas.
///
/// Used by surfaces without editor buffers; the GUI reads final content from
/// its buffers instead.
#[cfg(not(target_family = "wasm"))]
fn final_content_from_op(base_content: &str, op: &DiffType) -> Result<String, String> {
    match op {
        DiffType::Create { delta } => Ok(delta.insertion.clone()),
        DiffType::Update { deltas, .. } => apply_deltas_to_content(base_content, deltas),
        DiffType::Delete { .. } => Ok(String::new()),
    }
}

/// Applies line-range replacement deltas to `content`, producing the new content.
#[cfg(not(target_family = "wasm"))]
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
        let replacement = split_lines_preserving_newlines(&delta.insertion);
        lines.splice(start..end, replacement);
    }

    Ok(lines.concat())
}

/// Splits content into lines while keeping trailing newlines, so reassembly via
/// `concat` reproduces the original byte-for-byte.
#[cfg(not(target_family = "wasm"))]
fn split_lines_preserving_newlines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.split_inclusive('\n').map(str::to_string).collect()
    }
}

/// The actual write operation performed for a file — the single source of truth
/// that both the reported outcome and the [`FileModel`] dispatch derive from,
/// so they cannot drift apart.
#[cfg(not(target_family = "wasm"))]
enum PersistAction {
    /// Write the final content at the file's original path.
    Write,
    /// Move the file to the new path and write the final content there.
    Rename(PathBuf),
    /// Delete the file.
    Delete,
}

#[cfg(not(target_family = "wasm"))]
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
#[cfg(not(target_family = "wasm"))]
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
#[cfg(not(target_family = "wasm"))]
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
    // The registration is only needed to dispatch (the write captures its path
    // up front); release it either way so FileModel state doesn't grow
    // unboundedly.
    file_model.unsubscribe(file_id, ctx);
    dispatch
}

/// Builds a file's report state and result-diff inputs to mirror the write
/// that `action` will actually perform.
#[cfg(not(target_family = "wasm"))]
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

/// The TUI surface's diff storage: holds the resolved diffs and persists them
/// by writing straight through [`FileModel`], with no review UI or editor
/// buffers of its own — final content is derived by applying each diff's
/// deltas to its base.
///
/// It lives app-side (rather than in `warp_tui`) because persistence belongs
/// next to [`FileModel`]; the TUI registers one per `RequestFileEdits` action
/// via `tui_export` and renders a summary over it.
pub struct TuiDiffStorage {
    diffs: Vec<FileDiff>,
    session_type: DiffSessionType,
}

impl TuiDiffStorage {
    /// Creates storage over resolved diffs.
    pub fn new(diffs: Vec<FileDiff>, session_type: DiffSessionType) -> Self {
        Self {
            diffs,
            session_type,
        }
    }

    /// The stored diffs (for surfaces rendering a summary over this storage).
    pub fn diffs(&self) -> &[FileDiff] {
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
        #[cfg(not(target_family = "wasm"))]
        {
            self.diffs
                .iter()
                .map(|diff| {
                    let path = diff.file_path();
                    let action =
                        PersistAction::resolve(&diff.diff_type, &self.session_type, &path);
                    // A derivation failure is surfaced by `start_saving`; the
                    // snapshot is unused when the accept fails.
                    let final_content =
                        final_content_from_op(&diff.base.content, &diff.diff_type)
                            .unwrap_or_default();
                    persist_outcome(&action, diff, &path, &final_content)
                })
                .collect()
        }
        // On wasm the accept always fails (see `start_saving`); no snapshot needed.
        #[cfg(target_family = "wasm")]
        {
            Vec::new()
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn start_saving(&mut self, app: &mut AppContext) -> Vec<SaveFuture> {
        let file_model = FileModel::handle(app);
        let session_type = self.session_type.clone();
        self.diffs
            .iter()
            .map(|diff| {
                let path = diff.file_path();
                let final_content =
                    match final_content_from_op(&diff.base.content, &diff.diff_type) {
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

    /// On wasm there is no local/remote file backend, so file edits cannot execute.
    #[cfg(target_family = "wasm")]
    fn start_saving(&mut self, _app: &mut AppContext) -> Vec<SaveFuture> {
        vec![ready_save_failure(FileSaveError::Other(
            "file editing is not supported in this environment".to_string(),
        ))]
    }
}

impl Entity for TuiDiffStorage {
    type Event = ();
}

/// The TUI registers its model handle directly as the executor's storage.
impl RegisteredDiffStorage for ModelHandle<TuiDiffStorage> {
    fn set_candidate_diffs(
        &self,
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        app: &mut AppContext,
    ) {
        self.update(app, |model, ctx| {
            model.set_candidate_diffs(diffs, session_type);
            // Wake subscribers (e.g. the TUI summary view) now that diffs exist.
            ctx.emit(());
        });
    }

    fn accept_and_save(&self, app: &mut AppContext) -> BoxFuture<'static, RequestFileEditsResult> {
        self.update(app, |model, ctx| {
            DiffStorageHelper::accept_and_save(model, ctx)
        })
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "diff_storage_tests.rs"]
mod tests;
