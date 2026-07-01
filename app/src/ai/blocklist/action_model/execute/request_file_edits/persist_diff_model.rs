//! GUI-less persistence for resolved file edits.
//!
//! [`PersistDiffModel`] is the single writer and result producer for
//! `RequestFileEdits`, shared by every surface (GUI review, TUI/headless, and
//! passive suggestions). Callers hand it fully-resolved [`ResolvedFileEdit`]s
//! (final content already computed) plus the session backend; it writes each
//! file through [`FileModel`] and assembles a [`RequestFileEditsResult`].
use ai::diff_validation::DiffType;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext};
#[cfg(not(target_family = "wasm"))]
use {
    crate::ai::agent::FileLocations,
    crate::code::DiffResult,
    ai::diff_validation::DiffDelta,
    itertools::Itertools,
    similar::{DiffOp, TextDiff},
    std::collections::{HashMap, HashSet},
    std::ops::Range,
    std::path::Path,
    std::rc::Rc,
    warp_files::{FileModel, FileModelEvent},
    warp_util::content_version::ContentVersion,
    warp_util::file::{FileId, FileSaveError},
    warp_util::standardized_path::StandardizedPath,
    warpui::SingletonEntity as _,
};

use crate::ai::agent::RequestFileEditsResult;
use crate::ai::blocklist::diff_types::DiffSessionType;

/// A file edit whose final on-disk content has already been resolved.
///
/// `final_content` is the content to write (from GUI editor buffers or from
/// applying the diff's deltas to `base_content` on headless surfaces).
pub(crate) struct ResolvedFileEdit {
    pub path: String,
    pub base_content: String,
    pub op: DiffType,
    pub final_content: String,
}

pub(crate) struct PersistDiffModel {
    #[cfg(not(target_family = "wasm"))]
    batches: Vec<PersistBatch>,
}

impl Entity for PersistDiffModel {
    type Event = ();
}

impl PersistDiffModel {
    /// Creates the model and subscribes to [`FileModel`] save completion events.
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

    /// Persists the resolved edits via [`FileModel`] and resolves with the
    /// assembled [`RequestFileEditsResult`] once every file has been written.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn persist(
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
    pub(crate) fn persist(
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
    fn handle_file_model_event(&mut self, event: &FileModelEvent, _ctx: &mut ModelContext<Self>) {
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
    let version = ContentVersion::new();

    let file_id = match register_file(file_model, session_type, &path, ctx) {
        Ok(file_id) => file_id,
        Err(error) => {
            return (
                delete_or_update_outcome(&path, &base_content, &final_content, &op),
                Err(error),
            );
        }
    };

    let changed_lines = changed_lines_from_op(&op);
    let outcome = match &op {
        DiffType::Delete { .. } => PersistOutcome {
            diff: diff_result(&base_content, "", &path),
            updated: None,
            deleted: vec![path.clone()],
        },
        DiffType::Create { .. } => PersistOutcome {
            diff: diff_result(&base_content, &final_content, &path),
            updated: Some((
                FileLocations {
                    name: path.clone(),
                    lines: changed_lines,
                },
                final_content.clone(),
            )),
            deleted: Vec::new(),
        },
        DiffType::Update { rename, .. } => {
            // The AI diff flow writes to the original path; a rename is reported
            // to the model as a delete of the source plus an update at the target.
            let target = rename
                .as_ref()
                .map(|rename| rename.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            let deleted = if target != path {
                vec![path.clone()]
            } else {
                Vec::new()
            };
            PersistOutcome {
                diff: diff_result(&base_content, &final_content, &target),
                updated: Some((
                    FileLocations {
                        name: target,
                        lines: changed_lines,
                    },
                    final_content.clone(),
                )),
                deleted,
            }
        }
    };

    let dispatch = match &op {
        DiffType::Delete { .. } => file_model.delete(file_id, version, ctx),
        DiffType::Create { .. } => file_model.save(file_id, final_content, version, ctx),
        DiffType::Update { rename, .. } => match (rename, session_type) {
            // Local renames move the file on disk; remote sessions have no rename
            // primitive, so fall back to writing the (original) registered path.
            (Some(rename), DiffSessionType::Local) => {
                file_model.rename_and_save(file_id, rename.clone(), final_content, version, ctx)
            }
            _ => file_model.save(file_id, final_content, version, ctx),
        },
    };

    (outcome, dispatch.map(|()| file_id))
}

/// Fallback outcome used when registration fails before dispatch.
#[cfg(not(target_family = "wasm"))]
fn delete_or_update_outcome(
    path: &str,
    base_content: &str,
    final_content: &str,
    op: &DiffType,
) -> PersistOutcome {
    match op {
        DiffType::Delete { .. } => PersistOutcome {
            diff: diff_result(base_content, "", path),
            updated: None,
            deleted: vec![path.to_owned()],
        },
        _ => PersistOutcome {
            diff: diff_result(base_content, final_content, path),
            updated: None,
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
        updated_files: super::updated_file_contexts_from_editor_buffers(
            &updated_files,
            &content_map,
        ),
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

/// Derives changed line ranges from a diff's deltas for result file context.
#[cfg(not(target_family = "wasm"))]
fn changed_lines_from_op(diff_type: &DiffType) -> Vec<Range<usize>> {
    match diff_type {
        DiffType::Create { delta } => inserted_content_range(1, &delta.insertion)
            .into_iter()
            .collect(),
        DiffType::Update { deltas, .. } => deltas
            .iter()
            .filter_map(changed_line_range_for_delta)
            .collect(),
        DiffType::Delete { .. } => vec![],
    }
}

/// Maps a single delta to the line range it changed.
#[cfg(not(target_family = "wasm"))]
fn changed_line_range_for_delta(delta: &DiffDelta) -> Option<Range<usize>> {
    let replacement_range = &delta.replacement_line_range;
    if replacement_range.start == replacement_range.end {
        return inserted_content_range(replacement_range.start.max(1), &delta.insertion);
    }
    Some(replacement_range.clone())
}

/// Returns the line range covered by inserted content starting at `start`.
#[cfg(not(target_family = "wasm"))]
fn inserted_content_range(start: usize, content: &str) -> Option<Range<usize>> {
    let line_count = content.lines().count();
    (line_count > 0).then_some(start..start + line_count)
}
