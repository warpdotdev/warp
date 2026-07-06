//! Surface-owned storage and the shared persistence flow for `RequestFileEdits`
//! diffs.
//!
//! Every surface that stores pending diffs — the GUI `CodeDiffView` and the
//! up-stack TUI diff storage — implements [`DiffStorage`]. The trait's
//! provided methods are the shared save-completion flow: they track per-file
//! progress in a [`DiffSaveState`] and assemble the final
//! [`RequestFileEditsResult`], so every surface produces results through the
//! same code. Only the write kickoff ([`DiffStorage::start_saving`]) is
//! surface-specific: the GUI saves through its editor buffers.
//!
//! The executor knows surfaces only through [`RegisteredDiffStorage`], a small
//! object-safe handle trait, because GUI `ViewHandle`s and model `ModelHandle`s
//! share no common handle type. Each surface's handle type implements it
//! directly, delegating to the entity's [`DiffStorage`]. Every surface must
//! register its storage before the action's diffs resolve
//! (`register_requested_edits`); preprocess and execute assume a registered
//! storage.
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt;
use itertools::Itertools;
use warp_util::file::FileSaveError;
use warpui::AppContext;

use crate::ai::agent::{
    AnyFileContent, FileContext, FileLocations, RequestFileEditsResult, UpdatedFileContext,
};
use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff};
use crate::code::DiffResult;

const APPLY_DIFF_RESULT_CONTEXT_LINES: usize = 10;

/// A surface that stores pending file-edit diffs and persists them on accept.
///
/// Required methods are state accessors (the fields live on each impl, since
/// traits cannot hold state) plus the surface-specific write kickoff
/// ([`Self::start_saving`]). Provided methods are the shared save-completion
/// flow, so every surface assembles its [`RequestFileEditsResult`] through the
/// same code. Callers drive an accept solely through [`Self::accept_and_save`].
pub trait DiffStorage {
    /// The in-flight accept's progress and result channel. Impls just store a
    /// [`DiffSaveState`]; only the provided methods below act on it.
    fn save_state_mut(&mut self) -> &mut DiffSaveState;

    /// Number of pending file diffs; sizes the per-file save tracking.
    fn pending_diff_count(&self) -> usize;

    /// Snapshot of per-file state for result assembly: reported paths, changed
    /// lines, final contents, and user-edit flags.
    fn pending_file_state(&self, app: &AppContext) -> Vec<PendingFileState>;

    /// Kicks off persistence for every pending file.
    ///
    /// The surface-specific hook invoked by [`Self::accept_and_save`] — never
    /// called directly by callers. The GUI saves through its editor buffers;
    /// surfaces without editor buffers dispatch writes to `FileModel`. Each
    /// file's completion must be reported back through
    /// [`Self::handle_file_saved`] and [`Self::handle_diff_computed`].
    fn start_saving(&mut self, app: &mut AppContext);

    /// The entry point for accepting a surface's diffs: persists them all,
    /// resolving with the assembled result once every file's save and
    /// result-diff computation completes.
    ///
    /// Dropping the surface mid-save drops the stored sender, resolving the
    /// returned future with [`RequestFileEditsResult::Cancelled`].
    fn accept_and_save(
        &mut self,
        app: &mut AppContext,
    ) -> BoxFuture<'static, RequestFileEditsResult> {
        let count = self.pending_diff_count();
        let result = self.save_state_mut().begin(count);
        self.start_saving(app);
        self.try_finish(app);
        result
    }

    /// Records one file's save completion (or failure).
    fn handle_file_saved(
        &mut self,
        idx: usize,
        error: Option<Rc<FileSaveError>>,
        app: &AppContext,
    ) {
        self.save_state_mut().mark_diff_saved(idx, error);
        self.try_finish(app);
    }

    /// Records one file's computed result diff.
    fn handle_diff_computed(&mut self, idx: usize, diff: Rc<DiffResult>, app: &AppContext) {
        self.save_state_mut().mark_diff_computed(idx, diff);
        self.try_finish(app);
    }

    /// Assembles and delivers the result once every file is saved and computed.
    fn try_finish(&mut self, app: &AppContext) {
        if !self.save_state_mut().is_complete() {
            return;
        }
        let files = self.pending_file_state(app);
        self.save_state_mut().finish(files);
    }
}

/// Progress and result delivery for one in-flight accept.
///
/// Each [`DiffStorage`] impl stores one of these and exposes it via
/// [`DiffStorage::save_state_mut`]; the trait's provided methods drive it.
#[derive(Default)]
pub struct DiffSaveState {
    /// Per-file save/diff-computation tracking; `Some` while an accept is in flight.
    saving: Option<SavingDiffs>,
    /// Delivery channel for the in-flight accept's result.
    result_tx: Option<oneshot::Sender<RequestFileEditsResult>>,
}

impl DiffSaveState {
    /// Starts tracking an accept over `count` files, returning the future that
    /// resolves with the delivered result.
    fn begin(&mut self, count: usize) -> BoxFuture<'static, RequestFileEditsResult> {
        let (tx, rx) = oneshot::channel();
        self.result_tx = Some(tx);
        self.saving = Some(SavingDiffs::new(count));
        async move { rx.await.unwrap_or(RequestFileEditsResult::Cancelled) }.boxed()
    }

    /// True while an accept is being persisted.
    pub fn is_saving(&self) -> bool {
        self.saving.is_some()
    }

    /// Records the save outcome for the file at `idx`.
    fn mark_diff_saved(&mut self, idx: usize, error: Option<Rc<FileSaveError>>) {
        if let Some(saving) = &mut self.saving {
            saving.mark_diff_saved(idx, error);
        }
    }

    /// Records the computed result diff for the file at `idx`.
    fn mark_diff_computed(&mut self, idx: usize, diff: Rc<DiffResult>) {
        if let Some(saving) = &mut self.saving {
            saving.mark_diff_computed(idx, diff);
        }
    }

    /// True once every file is saved and its result diff computed.
    fn is_complete(&self) -> bool {
        self.saving
            .as_ref()
            .is_some_and(SavingDiffs::pending_diff_is_complete)
    }

    /// Delivers the assembled result and clears the in-flight state.
    fn finish(&mut self, files: Vec<PendingFileState>) {
        let Some(saving) = self.saving.take() else {
            return;
        };
        let Some(tx) = self.result_tx.take() else {
            return;
        };
        let _ = tx.send(assemble_result(saving, files));
    }
}

/// The executor-facing handle over a registered [`DiffStorage`] surface.
///
/// A separate trait from [`DiffStorage`] because the executor holds
/// surfaces by handle, and GUI view handles and model handles share no common
/// type. Each surface's handle type (e.g. `WeakViewHandle<CodeDiffView>`)
/// implements this directly, delegating each call to its entity's
/// [`DiffStorage`].
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

/// Per-file progress for one accepted edit.
///
/// Saving and result-diff computation complete independently per file; the
/// accept finishes when every file has both.
#[derive(Clone, Debug)]
struct SavingDiffs {
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

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "diff_storage_tests.rs"]
mod tests;
