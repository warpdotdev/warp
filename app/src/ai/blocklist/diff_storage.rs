//! Surface-owned storage and the shared persistence flow for `RequestFileEdits`
//! diffs.
//!
//! Every surface that stores pending diffs — the GUI `CodeDiffView`, the TUI
//! diff view, and the headless fallback below — implements [`DiffStorageView`].
//! The trait's provided methods are the shared save-completion flow: they track
//! per-file progress in [`SavingDiffs`] and assemble the final
//! [`RequestFileEditsResult`], so every surface produces results through the
//! same code. Only the write kickoff ([`DiffStorageView::start_saving`]) is
//! surface-specific: the GUI saves through its editor buffers, while headless
//! surfaces dispatch to `FileModel` via the helpers in this module.
//!
//! The executor knows surfaces only through [`RegisteredDiffStorage`], a small
//! object-safe handle trait, because GUI `ViewHandle`s and model `ModelHandle`s
//! share no common handle type.
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use ai::diff_validation::{DiffDelta, DiffType};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt;
use itertools::Itertools;
use warp_util::file::FileSaveError;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};
#[cfg(not(target_family = "wasm"))]
use {
    similar::{DiffOp, TextDiff},
    std::path::{Path, PathBuf},
    warp_files::{FileModel, FileModelEvent},
    warp_util::content_version::ContentVersion,
    warp_util::file::FileId,
    warp_util::standardized_path::StandardizedPath,
    warpui::SingletonEntity,
};

use crate::ai::agent::{
    AnyFileContent, FileContext, FileLocations, RequestFileEditsResult, UpdatedFileContext,
};
use crate::ai::blocklist::diff_types::{changed_lines_from_op, DiffSessionType, FileDiff};
use crate::code::DiffResult;

const APPLY_DIFF_RESULT_CONTEXT_LINES: usize = 10;

/// A surface that stores pending file-edit diffs and persists them on accept.
///
/// Required methods are state accessors (the fields live on each impl, since
/// traits cannot hold state) plus the surface-specific write kickoff. Provided
/// methods are the shared save-completion flow, so every surface assembles its
/// [`RequestFileEditsResult`] through the same code.
pub trait DiffStorageView {
    /// Per-file save/diff-computation tracking for the in-flight accept.
    fn saving_diffs_mut(&mut self) -> &mut Option<SavingDiffs>;

    /// Delivery channel for the in-flight accept's result.
    fn result_tx_mut(&mut self) -> &mut Option<oneshot::Sender<RequestFileEditsResult>>;

    /// Number of pending file diffs; sizes [`SavingDiffs`].
    fn pending_diff_count(&self) -> usize;

    /// Snapshot of per-file state for result assembly: reported paths, changed
    /// lines, final contents, and user-edit flags.
    fn pending_file_state(&self, app: &AppContext) -> Vec<PendingFileState>;

    /// Kicks off persistence for every pending file.
    ///
    /// Surface-specific: the GUI saves through its editor buffers; headless
    /// surfaces dispatch via [`dispatch_write`]. Each file's completion must be
    /// reported back through [`Self::handle_file_saved`] and
    /// [`Self::handle_diff_computed`].
    fn start_saving(&mut self, app: &mut AppContext);

    /// Persists all pending diffs, resolving with the assembled result once
    /// every file's save and result-diff computation completes.
    ///
    /// Dropping the surface mid-save drops the stored sender, resolving the
    /// returned future with [`RequestFileEditsResult::Cancelled`].
    fn accept_and_save(
        &mut self,
        app: &mut AppContext,
    ) -> BoxFuture<'static, RequestFileEditsResult> {
        let (tx, rx) = oneshot::channel();
        *self.result_tx_mut() = Some(tx);
        *self.saving_diffs_mut() = Some(SavingDiffs::new(self.pending_diff_count()));
        self.start_saving(app);
        self.try_finish(app);
        async move { rx.await.unwrap_or(RequestFileEditsResult::Cancelled) }.boxed()
    }

    /// Records one file's save completion (or failure).
    fn handle_file_saved(
        &mut self,
        idx: usize,
        error: Option<Rc<FileSaveError>>,
        app: &AppContext,
    ) {
        if let Some(saving) = self.saving_diffs_mut() {
            saving.mark_diff_saved(idx, error);
        }
        self.try_finish(app);
    }

    /// Records one file's computed result diff.
    fn handle_diff_computed(&mut self, idx: usize, diff: Rc<DiffResult>, app: &AppContext) {
        if let Some(saving) = self.saving_diffs_mut() {
            saving.mark_diff_computed(idx, diff);
        }
        self.try_finish(app);
    }

    /// Fails the in-flight accept outright (e.g. the surface cannot save at all).
    fn fail_saving(&mut self, error: String) {
        *self.saving_diffs_mut() = None;
        if let Some(tx) = self.result_tx_mut().take() {
            let _ = tx.send(RequestFileEditsResult::DiffApplicationFailed { error });
        }
    }

    /// Assembles and delivers the result once every file is saved and computed.
    fn try_finish(&mut self, app: &AppContext) {
        let complete = self
            .saving_diffs_mut()
            .as_ref()
            .is_some_and(SavingDiffs::pending_diff_is_complete);
        if !complete {
            return;
        }
        let saving = self
            .saving_diffs_mut()
            .take()
            .expect("checked complete above");
        let Some(tx) = self.result_tx_mut().take() else {
            return;
        };
        let files = self.pending_file_state(app);
        let _ = tx.send(assemble_result(saving, files));
    }
}

/// The executor-facing handle over a registered [`DiffStorageView`] surface.
///
/// GUI views and models share no common handle type, so each surface registers
/// a thin wrapper that delegates through its own handle.
pub trait RegisteredDiffStorage {
    /// Pushes resolved diffs into the surface (called when preprocess resolves).
    fn set_candidate_diffs(
        &self,
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        app: &mut AppContext,
    );

    /// Hands the diffs back out so a newly registered surface can take over.
    /// Returns `None` if this storage keeps ownership (e.g. saving started).
    fn take_candidate_diffs(
        &self,
        app: &mut AppContext,
    ) -> Option<(Vec<FileDiff>, DiffSessionType)>;

    /// Persists all diffs, resolving with the result reported to the LLM.
    fn accept_and_save(&self, app: &mut AppContext) -> BoxFuture<'static, RequestFileEditsResult>;
}

/// Per-file progress for one accepted edit.
///
/// Saving and result-diff computation complete independently per file; the
/// accept finishes when every file has both.
#[derive(Clone, Debug)]
pub struct SavingDiffs {
    pending_diffs: Vec<DiffApplicationState>,
}

impl SavingDiffs {
    /// Initializes tracking for `length` files.
    fn new(length: usize) -> Self {
        Self {
            pending_diffs: vec![DiffApplicationState::default(); length],
        }
    }

    /// Returns true once every file is saved and its result diff computed.
    fn pending_diff_is_complete(&self) -> bool {
        self.pending_diffs
            .iter()
            .all(|diff| diff.computed_diff.is_some() && diff.save_status.is_complete())
    }

    /// Records the save outcome for the file at `idx`.
    fn mark_diff_saved(&mut self, idx: usize, save_error: Option<Rc<FileSaveError>>) {
        if let Some(state) = self.pending_diffs.get_mut(idx) {
            state.save_status = match save_error {
                None => SaveStatus::Success,
                Some(error) => SaveStatus::Failed(error),
            };
        }
    }

    /// Records the computed result diff for the file at `idx`.
    fn mark_diff_computed(&mut self, idx: usize, diff: Rc<DiffResult>) {
        if let Some(state) = self.pending_diffs.get_mut(idx) {
            state.computed_diff = Some(diff);
        }
    }

    /// Splits into the combined result diff and any save errors.
    fn into_parts(self) -> (DiffResult, Vec<Rc<FileSaveError>>) {
        let mut combined = DiffResult::default();
        let mut save_errors = Vec::new();
        for state in self.pending_diffs {
            if let Some(diff) = state.computed_diff {
                combined += diff.as_ref();
            }
            if let SaveStatus::Failed(error) = state.save_status {
                save_errors.push(error);
            }
        }
        (combined, save_errors)
    }
}

/// The status of saving a single file.
#[derive(Clone, Debug, Default)]
enum SaveStatus {
    #[default]
    Pending,
    Success,
    Failed(Rc<FileSaveError>),
}

impl SaveStatus {
    fn is_complete(&self) -> bool {
        !matches!(self, SaveStatus::Pending)
    }
}

/// The save/diff-computation state for a single file.
#[derive(Clone, Debug, Default)]
struct DiffApplicationState {
    computed_diff: Option<Rc<DiffResult>>,
    save_status: SaveStatus,
}

/// One file's contribution to the assembled result.
#[derive(Clone)]
pub struct PendingFileState {
    /// The updated file, reported at its final path; `None` for deletions.
    pub updated: Option<UpdatedFileState>,
    /// Paths reported as deleted (the deleted file, or a rename's source path).
    pub deleted_paths: Vec<String>,
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

/// Combines per-file progress and report state into one
/// [`RequestFileEditsResult`], failing the whole edit if any file failed to save.
fn assemble_result(saving: SavingDiffs, files: Vec<PendingFileState>) -> RequestFileEditsResult {
    let (combined, save_errors) = saving.into_parts();
    if !save_errors.is_empty() {
        let error = save_errors
            .iter()
            .map(|error| match error.as_ref() {
                FileSaveError::IOError { error, path } => {
                    format!("Failed to save file {path:?}: {error}")
                }
                other => other.to_string(),
            })
            .join("\n");
        return RequestFileEditsResult::DiffApplicationFailed { error };
    }

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
/// Used by surfaces without editor buffers (headless, TUI); the GUI reads final
/// content from its buffers instead.
pub(crate) fn final_content_from_op(base_content: &str, op: &DiffType) -> Result<String, String> {
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
        let replacement = split_lines_preserving_newlines(&delta.insertion);
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
/// registered [`FileId`] on success.
#[cfg(not(target_family = "wasm"))]
fn dispatch_write(
    file_model: &mut FileModel,
    session_type: &DiffSessionType,
    action: &PersistAction,
    path: &str,
    final_content: String,
    ctx: &mut ModelContext<FileModel>,
) -> Result<FileId, FileSaveError> {
    let file_id = register_file(file_model, session_type, path, ctx)?;
    let version = ContentVersion::new();
    let dispatch = match action {
        PersistAction::Delete => file_model.delete(file_id, version, ctx),
        PersistAction::Rename(to) => {
            file_model.rename_and_save(file_id, to.clone(), final_content, version, ctx)
        }
        PersistAction::Write => file_model.save(file_id, final_content, version, ctx),
    };
    if dispatch.is_err() {
        // No save event will ever arrive for this file; release it now.
        file_model.unsubscribe(file_id, ctx);
    }
    dispatch.map(|()| file_id)
}

/// Computes a similar-based unified diff and line counts between `before` and
/// `after`, for surfaces without an editor-computed diff.
#[cfg(not(target_family = "wasm"))]
fn diff_result(before: &str, after: &str, file_name: &str) -> DiffResult {
    if before == after {
        return DiffResult::default();
    }

    let text_diff = TextDiff::from_lines(before, after);
    let mut lines_added = 0;
    let mut lines_removed = 0;
    for op in text_diff.ops() {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete { old_len, .. } => lines_removed += old_len,
            DiffOp::Insert { new_len, .. } => lines_added += new_len,
            DiffOp::Replace {
                old_len, new_len, ..
            } => {
                lines_removed += old_len;
                lines_added += new_len;
            }
        }
    }

    DiffResult {
        unified_diff: text_diff
            .unified_diff()
            .context_radius(3)
            .header(file_name, file_name)
            .missing_newline_hint(false)
            .to_string(),
        lines_added,
        lines_removed,
    }
}

/// Builds a file's report state and similar-based result diff to mirror the
/// write that `action` will actually perform.
#[cfg(not(target_family = "wasm"))]
fn headless_file_outcome(
    action: &PersistAction,
    diff: &FileDiff,
    path: &str,
    final_content: &str,
) -> (PendingFileState, DiffResult) {
    let changed_lines = changed_lines_from_op(&diff.diff_type);
    match action {
        PersistAction::Delete => (
            PendingFileState {
                updated: None,
                deleted_paths: vec![path.to_owned()],
            },
            diff_result(&diff.base.content, "", path),
        ),
        PersistAction::Rename(to) => {
            let target = to.to_string_lossy().to_string();
            (
                PendingFileState {
                    updated: Some(UpdatedFileState {
                        path: target.clone(),
                        changed_lines,
                        final_content: final_content.to_owned(),
                        was_edited: false,
                    }),
                    deleted_paths: vec![path.to_owned()],
                },
                diff_result(&diff.base.content, final_content, &target),
            )
        }
        PersistAction::Write => (
            PendingFileState {
                updated: Some(UpdatedFileState {
                    path: path.to_owned(),
                    changed_lines,
                    final_content: final_content.to_owned(),
                    was_edited: false,
                }),
                deleted_paths: Vec::new(),
            },
            diff_result(&diff.base.content, final_content, path),
        ),
    }
}

/// GUI-less [`DiffStorageView`]: plain diff storage with no review UI.
///
/// The executor creates one per action when diffs resolve with no registered
/// surface (autoexecution racing view creation, or headless/TUI-driven
/// conversations), so file edits stay executable everywhere. Final content is
/// derived by applying each diff's deltas to its base. This is also the
/// template the TUI's diff storage builds on.
pub(crate) struct HeadlessDiffStorageModel {
    diffs: Vec<FileDiff>,
    session_type: DiffSessionType,
    saving_diffs: Option<SavingDiffs>,
    result_tx: Option<oneshot::Sender<RequestFileEditsResult>>,
    /// `FileModel` registration for each in-flight write, index-aligned with `diffs`.
    #[cfg(not(target_family = "wasm"))]
    in_flight_file_ids: Vec<Option<FileId>>,
    /// Report state built when saving starts, consumed by result assembly.
    resolved: Vec<PendingFileState>,
}

impl HeadlessDiffStorageModel {
    /// Creates storage over resolved diffs and subscribes to [`FileModel`] save
    /// events for its own writes.
    pub(crate) fn new(
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(&FileModel::handle(ctx), |me, _, event, ctx| {
            me.handle_file_model_event(event, ctx);
        });
        #[cfg(target_family = "wasm")]
        let _ = ctx;

        Self {
            diffs,
            session_type,
            saving_diffs: None,
            result_tx: None,
            #[cfg(not(target_family = "wasm"))]
            in_flight_file_ids: Vec::new(),
            resolved: Vec::new(),
        }
    }

    /// Replaces the stored diffs and session backend.
    fn set_candidate_diffs(&mut self, diffs: Vec<FileDiff>, session_type: DiffSessionType) {
        self.diffs = diffs;
        self.session_type = session_type;
    }

    /// Relinquishes the diffs to a newly registered surface, unless saving has
    /// already started (or finished — `resolved` outlives the save).
    fn take_candidate_diffs(&mut self) -> Option<(Vec<FileDiff>, DiffSessionType)> {
        if self.saving_diffs.is_some() || !self.resolved.is_empty() || self.diffs.is_empty() {
            return None;
        }
        Some((std::mem::take(&mut self.diffs), self.session_type.clone()))
    }

    /// Routes this model's own [`FileModel`] save events into the shared flow.
    #[cfg(not(target_family = "wasm"))]
    fn handle_file_model_event(&mut self, event: &FileModelEvent, ctx: &mut ModelContext<Self>) {
        let file_id = event.file_id();
        let save_error = match event {
            FileModelEvent::FileSaved { .. } => None,
            FileModelEvent::FailedToSave { error, .. } => Some(error.clone()),
            // Other file events (loads, external updates) are unrelated to our writes.
            _ => return,
        };
        let Some(idx) = self
            .in_flight_file_ids
            .iter()
            .position(|id| *id == Some(file_id))
        else {
            return;
        };
        self.in_flight_file_ids[idx] = None;

        // The write is done either way; release the registration so FileModel
        // state doesn't grow unboundedly.
        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.unsubscribe(file_id, ctx);
        });

        self.handle_file_saved(idx, save_error, ctx);
    }
}

impl DiffStorageView for HeadlessDiffStorageModel {
    fn saving_diffs_mut(&mut self) -> &mut Option<SavingDiffs> {
        &mut self.saving_diffs
    }

    fn result_tx_mut(&mut self) -> &mut Option<oneshot::Sender<RequestFileEditsResult>> {
        &mut self.result_tx
    }

    fn pending_diff_count(&self) -> usize {
        self.diffs.len()
    }

    fn pending_file_state(&self, _app: &AppContext) -> Vec<PendingFileState> {
        self.resolved.clone()
    }

    #[cfg(not(target_family = "wasm"))]
    fn start_saving(&mut self, app: &mut AppContext) {
        self.resolved.clear();
        self.in_flight_file_ids = vec![None; self.diffs.len()];
        let diffs = self.diffs.clone();
        let session_type = self.session_type.clone();
        let file_model = FileModel::handle(app);

        for (idx, diff) in diffs.into_iter().enumerate() {
            let path = diff.file_path();
            let final_content = match final_content_from_op(&diff.base.content, &diff.diff_type) {
                Ok(content) => content,
                Err(error) => {
                    self.fail_saving(error);
                    return;
                }
            };

            let action = PersistAction::resolve(&diff.diff_type, &session_type, &path);
            let (state, result_diff) = headless_file_outcome(&action, &diff, &path, &final_content);
            self.resolved.push(state);
            // No editor computes the result diff here; the similar-based diff
            // is available immediately.
            self.handle_diff_computed(idx, Rc::new(result_diff), app);

            let dispatch = file_model.update(app, |file_model, ctx| {
                dispatch_write(
                    file_model,
                    &session_type,
                    &action,
                    &path,
                    final_content,
                    ctx,
                )
            });
            match dispatch {
                Ok(file_id) => self.in_flight_file_ids[idx] = Some(file_id),
                Err(error) => self.handle_file_saved(idx, Some(Rc::new(error)), app),
            }
        }
    }

    /// On wasm there is no local/remote file backend, so file edits cannot execute.
    #[cfg(target_family = "wasm")]
    fn start_saving(&mut self, _app: &mut AppContext) {
        self.fail_saving("file editing is not supported in this environment".to_string());
    }
}

impl Entity for HeadlessDiffStorageModel {
    type Event = ();
}

/// [`RegisteredDiffStorage`] wrapper over a [`HeadlessDiffStorageModel`].
pub(crate) struct HeadlessDiffStorage(pub(crate) ModelHandle<HeadlessDiffStorageModel>);

impl RegisteredDiffStorage for HeadlessDiffStorage {
    fn set_candidate_diffs(
        &self,
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        app: &mut AppContext,
    ) {
        self.0.update(app, |model, _| {
            model.set_candidate_diffs(diffs, session_type)
        });
    }

    fn take_candidate_diffs(
        &self,
        app: &mut AppContext,
    ) -> Option<(Vec<FileDiff>, DiffSessionType)> {
        self.0.update(app, |model, _| model.take_candidate_diffs())
    }

    fn accept_and_save(&self, app: &mut AppContext) -> BoxFuture<'static, RequestFileEditsResult> {
        self.0.update(app, |model, ctx| {
            DiffStorageView::accept_and_save(model, ctx)
        })
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "diff_storage_tests.rs"]
mod tests;
