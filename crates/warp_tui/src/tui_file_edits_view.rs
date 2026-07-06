//! TUI view for a `RequestFileEdits` tool call.
//!
//! The view owns a [`TuiDiffStorage`] and registers it with the
//! shared executor as the action's diff storage: the executor seeds it with
//! the resolved diffs when preprocess completes and drives persistence through
//! it at execute time. The TUI does not yet offer in-place editing, so
//! execution applies the diffs' deltas unmodified; this view renders a compact
//! summary over the stored diffs.
use warp::tui_export::{
    AIAgentActionId, Appearance, BlocklistAIActionModel, DiffSessionType, FileDiff,
};
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{Modifier, TuiContainer, TuiElement, TuiStyle, TuiText};
use warpui_core::elements::Fill;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, ViewContext};

use crate::tui_diff_storage::{TuiDiffStorage, TuiDiffStorageHandle};

/// A per-action view backing one `RequestFileEdits` tool call in the transcript.
pub(super) struct TuiFileEditsView {
    /// The storage registered with the executor; empty until preprocess seeds
    /// it (and stays empty if this view was created after the action already
    /// executed through another storage).
    storage: ModelHandle<TuiDiffStorage>,
}

impl TuiFileEditsView {
    pub(super) fn new(
        action_id: AIAgentActionId,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let storage =
            ctx.add_model(|_| TuiDiffStorage::new(Vec::new(), DiffSessionType::Local));

        // The storage emits when the executor seeds it with resolved diffs;
        // re-render the summary.
        ctx.subscribe_to_model(&storage, |_me, _, (), ctx| {
            ctx.notify();
        });

        let executor = action_model.as_ref(ctx).request_file_edits_executor(ctx);
        executor.update(ctx, |executor, _| {
            let handle = TuiDiffStorageHandle::new(storage.clone());
            executor.register_requested_edits(&action_id, Box::new(handle));
        });

        Self { storage }
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
}

impl Entity for TuiFileEditsView {
    type Event = ();
}

impl TuiView for TuiFileEditsView {
    fn ui_name() -> &'static str {
        "TuiFileEditsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        let text_color = Fill::from(ThemeFill::from(theme.terminal_colors().bright.black)).into();
        let label = match self.summary_stats(app) {
            Some((files, added, removed)) => {
                let files_label = if files == 1 { "file" } else { "files" };
                format!("Edited {files} {files_label} (+{added} −{removed})")
            }
            None => "Preparing edits…".to_string(),
        };
        TuiContainer::new(Box::new(
            TuiText::new(label).with_style(
                TuiStyle::default()
                    .fg(text_color)
                    .add_modifier(Modifier::DIM),
            ),
        ))
        .finish()
    }
}
