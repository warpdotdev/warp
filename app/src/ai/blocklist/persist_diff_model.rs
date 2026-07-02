//! GUI-less persistence for accepted file edits.
//!
//! [`PersistDiffModel`] is the app-wide singleton writer and result producer
//! for `RequestFileEdits`, shared by every surface (GUI review, TUI/headless,
//! and passive suggestions). Callers hand [`Self::resolve_and_persist`] the
//! prepared [`FileDiff`]s plus any review-surface-supplied final content; it
//! resolves each file's final content, writes it through [`FileModel`], and
//! assembles a [`RequestFileEditsResult`].
use std::collections::HashMap;

use ai::diff_validation::{DiffDelta, DiffType};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext, SingletonEntity};
#[cfg(not(target_family = "wasm"))]
use {
    crate::ai::agent::{AnyFileContent, FileContext, FileLocations, UpdatedFileContext},
    crate::code::DiffResult,
    itertools::Itertools,
    similar::{DiffOp, TextDiff},
    std::collections::HashSet,
    std::ops::Range,
    std::path::{Path, PathBuf},
    std::rc::Rc,
    warp_files::{FileModel, FileModelEvent},
    warp_util::content_version::ContentVersion,
    warp_util::file::{FileId, FileSaveError},
    warp_util::standardized_path::StandardizedPath,
};

use crate::ai::agent::RequestFileEditsResult;
use crate::ai::blocklist::diff_types::{changed_lines_from_op, DiffSessionType, FileDiff};

#[cfg(not(target_family = "wasm"))]
const APPLY_DIFF_RESULT_CONTEXT_LINES: usize = 10;

/// A file edit whose final on-disk content has been resolved.
struct ResolvedFileEdit {
    path: String,
    base_content: String,
    op: DiffType,
    final_content: String,
}

pub(crate) struct PersistDiffModel {
    #[cfg(not(target_family = "wasm"))]
    batches: Vec<PersistBatch>,
}

impl Entity for PersistDiffModel {
    type Event = ();
}

impl SingletonEntity for PersistDiffModel {}

impl PersistDiffModel {
    /// Creates the model and subscribes to [`FileModel`] save completion events.
    /// Must be registered after [`FileModel`].
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(&FileModel::handle(ctx), |me, _, event, ctx| {
            me.handle_file_model_event(event, ctx);
        });
        Self {
            #[cfg(not(target_family = "wasm"))]
            batches: Vec::new(),
        }
    }

    /// Resolves each diff's final content — review-surface-supplied content per
    /// path when present in `reviewed`, otherwise the diff's deltas applied to
    /// the base content — then persists everything via [`FileModel`], resolving
    /// with the assembled [`RequestFileEditsResult`].
    pub(crate) fn resolve_and_persist(
        &mut self,
        diffs: Vec<FileDiff>,
        reviewed: HashMap<String, String>,
        session_type: DiffSessionType,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, RequestFileEditsResult> {
        match build_resolved_edits(diffs, &reviewed) {
            Ok(resolved) => self.persist(resolved, session_type, ctx),
            Err(error) => {
                futures::future::ready(RequestFileEditsResult::DiffApplicationFailed { error })
                    .boxed()
            }
        }
    }

    /// Persists the resolved edits via [`FileModel`] and resolves with the
    /// assembled [`RequestFileEditsResult`] once every file has been written.
    #[cfg(not(target_family = "wasm"))]
    fn persist(
        &mut self,
        files: Vec<ResolvedFileEdit>,
        session_type: DiffSessionType,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, RequestFileEditsResult> {
        let (tx, rx) = oneshot::channel();
        let file_model = FileModel::handle(ctx);
        let mut remaining: HashSet<FileId> = HashSet::new();
        let mut outcomes: Vec<PersistOutcome> = Vec::new();
        let mut save_errors: Vec<Rc<FileSaveError>> = Vec::new();

        for file in files {
            let (outcome, dispatch) = file_model.update(ctx, |file_model, ctx| {
                dispatch_file(file_model, &session_type, file, ctx)
            });
            match dispatch {
                Ok(file_id) => {
                    remaining.insert(file_id);
                }
                Err(error) => {
                    save_errors.push(Rc::new(error));
                }
            }
            outcomes.push(outcome);
        }

        if remaining.is_empty() {
            let _ = tx.send(assemble_result(outcomes, save_errors));
        } else {
            self.batches.push(PersistBatch {
                remaining,
                outcomes,
                save_errors,
                tx,
            });
        }

        async move { rx.await.unwrap_or(RequestFileEditsResult::Cancelled) }.boxed()
    }

    /// On wasm there is no local/remote file backend, so file edits cannot execute.
    #[cfg(target_family = "wasm")]
    fn persist(
        &mut self,
        _files: Vec<ResolvedFileEdit>,
        _session_type: DiffSessionType,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, RequestFileEditsResult> {
        futures::future::ready(RequestFileEditsResult::DiffApplicationFailed {
            error: "file editing is not supported in this environment".to_string(),
        })
        .boxed()
    }

    /// Records a file's save outcome and, when a batch is fully written,
    /// assembles and delivers its result.
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
            .batches
            .iter()
            .position(|batch| batch.remaining.contains(&file_id))
        else {
            return;
        };

        let batch = &mut self.batches[idx];
        batch.remaining.remove(&file_id);
        if let Some(error) = save_error {
            batch.save_errors.push(error);
        }

        // The write for this file is done either way; release its registration
        // so FileModel's file state doesn't grow unboundedly.
        FileModel::handle(ctx).update(ctx, |file_model, ctx| {
            file_model.unsubscribe(file_id, ctx);
        });

        if batch.remaining.is_empty() {
            let batch = self.batches.remove(idx);
            let _ = batch
                .tx
                .send(assemble_result(batch.outcomes, batch.save_errors));
        }
    }
}

/// An in-flight batch of file writes awaiting their [`FileModel`] events.
#[cfg(not(target_family = "wasm"))]
struct PersistBatch {
    remaining: HashSet<FileId>,
    outcomes: Vec<PersistOutcome>,
    save_errors: Vec<Rc<FileSaveError>>,
    tx: oneshot::Sender<RequestFileEditsResult>,
}

/// The result contribution for a single file, computed up front (independent of
/// save ordering) and combined once the whole batch completes.
#[cfg(not(target_family = "wasm"))]
struct PersistOutcome {
    diff: DiffResult,
    /// `(file location, final content)` for created/updated files; `None` for deletes.
    updated: Option<(FileLocations, String)>,
    /// Paths reported as deleted (the deleted file, or a rename's source path).
    deleted: Vec<String>,
}

/// The actual write operation performed for a file — the single source of truth
/// that both the reported outcome and the [`FileModel`] dispatch derive from,
/// so they cannot drift apart.
#[cfg(not(target_family = "wasm"))]
enum PersistAction {
    /// Write `final_content` at the file's original path.
    Write,
    /// Move the file to the new path and write `final_content` there.
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

/// Registers a file with [`FileModel`] and dispatches its write, returning the
/// per-file result contribution and the registered [`FileId`] on success.
#[cfg(not(target_family = "wasm"))]
fn dispatch_file(
    file_model: &mut FileModel,
    session_type: &DiffSessionType,
    file: ResolvedFileEdit,
    ctx: &mut ModelContext<FileModel>,
) -> (PersistOutcome, Result<FileId, FileSaveError>) {
    let ResolvedFileEdit {
        path,
        base_content,
        op,
        final_content,
    } = file;

    let action = PersistAction::resolve(&op, session_type, &path);
    let changed_lines = changed_lines_from_op(&op);
    let outcome = outcome_for_action(&action, &path, &base_content, &final_content, changed_lines);

    let file_id = match register_file(file_model, session_type, &path, ctx) {
        Ok(file_id) => file_id,
        Err(error) => return (outcome, Err(error)),
    };

    let version = ContentVersion::new();
    let dispatch = match action {
        PersistAction::Delete => file_model.delete(file_id, version, ctx),
        PersistAction::Rename(to) => {
            file_model.rename_and_save(file_id, to, final_content, version, ctx)
        }
        PersistAction::Write => file_model.save(file_id, final_content, version, ctx),
    };
    if dispatch.is_err() {
        // No save event will ever arrive for this file; release it now.
        file_model.unsubscribe(file_id, ctx);
    }

    (outcome, dispatch.map(|()| file_id))
}

/// Builds the result contribution reported for a file, given the write
/// operation that will actually be performed.
#[cfg(not(target_family = "wasm"))]
fn outcome_for_action(
    action: &PersistAction,
    path: &str,
    base_content: &str,
    final_content: &str,
    changed_lines: Vec<Range<usize>>,
) -> PersistOutcome {
    match action {
        PersistAction::Delete => PersistOutcome {
            diff: diff_result(base_content, "", path),
            updated: None,
            deleted: vec![path.to_owned()],
        },
        PersistAction::Rename(to) => {
            let target = to.to_string_lossy().to_string();
            PersistOutcome {
                diff: diff_result(base_content, final_content, &target),
                updated: Some((
                    FileLocations {
                        name: target,
                        lines: changed_lines,
                    },
                    final_content.to_owned(),
                )),
                deleted: vec![path.to_owned()],
            }
        }
        PersistAction::Write => PersistOutcome {
            diff: diff_result(base_content, final_content, path),
            updated: Some((
                FileLocations {
                    name: path.to_owned(),
                    lines: changed_lines,
                },
                final_content.to_owned(),
            )),
            deleted: Vec::new(),
        },
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

/// Combines per-file outcomes into a single [`RequestFileEditsResult`], failing
/// the whole edit if any file failed to save.
#[cfg(not(target_family = "wasm"))]
fn assemble_result(
    outcomes: Vec<PersistOutcome>,
    save_errors: Vec<Rc<FileSaveError>>,
) -> RequestFileEditsResult {
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

    let mut combined = DiffResult::default();
    let mut updated_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut content_map: HashMap<String, String> = HashMap::new();

    for outcome in outcomes {
        combined += &outcome.diff;
        if let Some((file_location, content)) = outcome.updated {
            content_map.insert(file_location.name.clone(), content);
            updated_files.push((file_location, false));
        }
        deleted_files.extend(outcome.deleted);
    }

    RequestFileEditsResult::Success {
        diff: combined.unified_diff,
        updated_files: updated_file_contexts_from_content_map(&updated_files, &content_map),
        deleted_files,
        lines_added: combined.lines_added,
        lines_removed: combined.lines_removed,
    }
}

/// Computes a unified diff and line counts between `before` and `after`.
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

/// Builds resolved file edits, using review-surface-supplied content per path
/// when present and otherwise applying the diff's deltas to the base content.
fn build_resolved_edits(
    diffs: Vec<FileDiff>,
    reviewed: &HashMap<String, String>,
) -> Result<Vec<ResolvedFileEdit>, String> {
    let mut resolved = Vec::with_capacity(diffs.len());
    for diff in diffs {
        let path = diff.file_path();
        let base_content = diff.base.content;
        let op = diff.diff_type;
        let final_content = match reviewed.get(&path) {
            Some(content) => content.clone(),
            None => final_content_from_op(&base_content, &op)?,
        };
        resolved.push(ResolvedFileEdit {
            path,
            base_content,
            op,
            final_content,
        });
    }
    Ok(resolved)
}

/// Derives the final on-disk content for a diff from its base content and deltas.
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

/// Expands each updated file's changed lines with surrounding context and
/// extracts the corresponding fragments from the final file content.
#[cfg(not(target_family = "wasm"))]
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
#[cfg(not(target_family = "wasm"))]
fn clamp_to_file_context_range_start(file_location: &mut FileLocations) {
    for range in &mut file_location.lines {
        range.start = range.start.max(1);
        range.end = range.end.max(range.start);
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "persist_diff_model_tests.rs"]
mod tests;
