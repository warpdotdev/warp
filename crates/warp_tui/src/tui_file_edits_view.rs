//! TUI view for a `RequestFileEdits` tool call.
//!
//! The view owns a [`TuiDiffStorage`] and registers it with the
//! shared executor as the action's diff storage: the executor seeds it with
//! the resolved diffs when preprocess completes and drives persistence through
//! it at execute time. The TUI does not yet offer in-place editing, so
//! execution applies the diffs' deltas unmodified; this view renders a compact
//! summary over the stored diffs. When the storage was never seeded (failed
//! or cancelled actions, or actions that resolved before this view existed),
//! the summary falls back to the action's recorded result.
use ai::agent::action_result::{AIAgentActionResultType, RequestFileEditsResult};
use itertools::Itertools;
use warp::tui_export::{
    AIAgentActionId, BlocklistAIActionEvent, BlocklistAIActionModel, DiffSessionType, FileDiff,
};
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiText};
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, ViewContext};

use crate::tui_builder::TuiUiBuilder;
use crate::tui_diff_storage::{TuiDiffStorage, TuiDiffStorageEvent, TuiDiffStorageHandle};

/// A per-action view backing one `RequestFileEdits` tool call in the transcript.
pub(super) struct TuiFileEditsView {
    /// The storage registered with the executor; only seeded when the action's
    /// diffs resolve while this view exists.
    storage: ModelHandle<TuiDiffStorage>,
    /// The action this view renders.
    action_id: AIAgentActionId,
    /// Consulted for the action's terminal result when the storage was never
    /// seeded.
    action_model: ModelHandle<BlocklistAIActionModel>,
}

impl TuiFileEditsView {
    pub(super) fn new(
        action_id: AIAgentActionId,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let storage = ctx.add_model(|_| TuiDiffStorage::new(Vec::new(), DiffSessionType::Local));

        ctx.subscribe_to_model(&storage, |_me, _, event, ctx| match event {
            TuiDiffStorageEvent::CandidateDiffsSet => ctx.notify(),
        });

        // Failed and cancelled actions never seed the storage; re-render on
        // the terminal result so the row doesn't stay pending.
        ctx.subscribe_to_model(action_model, |me, _, event, ctx| {
            if let BlocklistAIActionEvent::FinishedAction { action_id, .. } = event {
                if *action_id == me.action_id {
                    ctx.notify();
                }
            }
        });

        // An already-resolved action (e.g. on a restored transcript) renders
        // from its recorded result; registering a storage for it would leave
        // a stale entry in the executor.
        if action_model
            .as_ref(ctx)
            .get_action_result(&action_id)
            .is_none()
        {
            let executor = action_model.as_ref(ctx).request_file_edits_executor(ctx);
            executor.update(ctx, |executor, _| {
                let handle = TuiDiffStorageHandle::new(storage.clone());
                executor.register_requested_edits(&action_id, Box::new(handle));
            });
        }

        Self {
            storage,
            action_id,
            action_model: action_model.clone(),
        }
    }

    /// Total `(files, lines_added, lines_removed)` across the stored diffs.
    fn summary_stats(&self, app: &AppContext) -> Option<(usize, usize, usize)> {
        let diffs = self.storage.as_ref(app).diffs();
        if diffs.is_empty() {
            return None;
        }
        let (added, removed) = diffs
            .iter()
            .map(FileDiff::line_stats)
            .fold((0, 0), |(a, r), (da, dr)| (a + da, r + dr));
        Some((diffs.len(), added, removed))
    }

    /// The row label: a summary over the seeded diffs, else a terminal label
    /// from the action's recorded result, else a pending label.
    fn label(&self, app: &AppContext) -> String {
        if let Some((files, added, removed)) = self.summary_stats(app) {
            return summary_label(files, added, removed);
        }
        let result = self
            .action_model
            .as_ref(app)
            .get_action_result(&self.action_id);
        match result.and_then(|result| match &result.result {
            AIAgentActionResultType::RequestFileEdits(result) => Some(result),
            _ => None,
        }) {
            Some(RequestFileEditsResult::Success {
                updated_files,
                deleted_files,
                lines_added,
                lines_removed,
                ..
            }) => {
                // Updated entries are per-fragment, so de-dupe by file name.
                let files = updated_files
                    .iter()
                    .map(|file| file.file_context.file_name.as_str())
                    .chain(deleted_files.iter().map(String::as_str))
                    .unique()
                    .count();
                summary_label(files, *lines_added, *lines_removed)
            }
            Some(RequestFileEditsResult::Cancelled) => "File edits cancelled".to_string(),
            Some(RequestFileEditsResult::DiffApplicationFailed { .. }) => {
                "File edits failed".to_string()
            }
            None => "Preparing edits…".to_string(),
        }
    }
}

/// Formats the summary row over edited file and line counts.
fn summary_label(files: usize, added: usize, removed: usize) -> String {
    let files_label = if files == 1 { "file" } else { "files" };
    format!("Edited {files} {files_label} (+{added} −{removed})")
}

impl Entity for TuiFileEditsView {
    type Event = ();
}

impl TuiView for TuiFileEditsView {
    fn ui_name() -> &'static str {
        "TuiFileEditsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let label = self.label(app);
        TuiContainer::new(Box::new(
            TuiText::new(label).with_style(TuiUiBuilder::from_app(app).dim_text_style()),
        ))
        .finish()
    }
}
