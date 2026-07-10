//! TUI view for a `RequestFileEdits` tool call — the diff "wrapper": pure
//! policy and chrome over the core editor element.
//!
//! The view owns a [`TuiDiffStorage`] and registers it with the shared
//! executor as the action's diff storage: the executor seeds it with the
//! resolved diffs when preprocess completes and drives persistence through it
//! at execute time. When the diffs land, the view builds one char-cell
//! [`CodeEditorModel`] per edited file and drives the existing model pipeline
//! (buffer = post-edit content, diff base = pre-edit content, model-side
//! hunk-context hiding, `expand_diffs`); all diff render data — ghost rows,
//! hidden ranges — flows model → render state → [`TuiEditorElement`]. The
//! view renders per-file chrome: a clickable header row
//! (`✓ Updated name +a −r ▾`) over a read-only, gutter-ed, diff-styled core
//! element. It never walks diff hunks, computes hidden ranges, or builds
//! rows. Multi-file edits nest the per-file sections, indented, under one
//! collapsible summary header (`✓ Edited 3 files +a −r ▾`); single-file edits
//! render the file section alone. When the storage was never seeded (failed
//! or cancelled actions, or actions that resolved before this view existed),
//! the view falls back to a one-line label from the action's recorded result.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use ai::agent::action_result::{AIAgentActionResultType, RequestFileEditsResult};
use ai::diff_validation::{DiffDelta, DiffType};
use itertools::Itertools;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    AIAgentActionId, BlocklistAIActionEvent, BlocklistAIActionModel, DiffSessionType, FileDiff,
};
use warp_editor::content::buffer::InitialBufferState;
use warpui_core::elements::tui::{
    tui_disclosure_chevron, Modifier, TuiContainer, TuiElement, TuiFlex, TuiHoverable,
    TuiParentElement, TuiStyle, TuiText,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, ViewContext};

use crate::editor_element::{TuiEditorElement, TuiEditorStyles};
use crate::tool_call_labels::{tool_call_display_state, tool_call_glyph, ToolCallDisplayState};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_diff_storage::{TuiDiffStorage, TuiDiffStorageEvent, TuiDiffStorageHandle};

/// Unchanged context lines rendered on each side of a hunk.
const CONTEXT_LINES: usize = 3;

/// A per-action view backing one `RequestFileEdits` tool call in the transcript.
pub(super) struct TuiFileEditsView {
    /// The storage registered with the executor; only seeded when the action's
    /// diffs resolve while this view exists.
    storage: ModelHandle<TuiDiffStorage>,
    /// The action this view renders.
    action_id: AIAgentActionId,
    /// Consulted for the action's status (header state) and terminal result
    /// (fallback label when the storage was never seeded).
    action_model: ModelHandle<BlocklistAIActionModel>,
    /// One section per resolved file diff, in storage order; empty until the
    /// executor seeds the storage.
    sections: Vec<FileSection>,
    /// Shared per-section UI state (collapse + header hover) for the summary
    /// header and each file, cloned into header click closures — the
    /// thinking-block pattern.
    section_states: SectionStates,
}

/// One edited file's diff: header facts plus the char-cell editor whose
/// buffer/diff models back the rendered body.
struct FileSection {
    /// Buffer = post-edit content; `DiffModel` base = pre-edit content. The
    /// diff recomputes automatically on the seeding edit, and ghost rows land
    /// in the render state's char-cell temporary blocks via `expand_diffs`.
    editor: ModelHandle<CodeEditorModel>,
    /// Header verb: `Updated`, `Created`, or `Deleted`.
    verb: &'static str,
    /// Display name: the file name, or `old → new` for renames.
    name: String,
    /// Whether the diff has been computed and expanded (ghost rows pushed);
    /// the body and header counts render only once this is set.
    diff_ready: bool,
}

impl FileSection {
    /// The header's `(added, removed)` counts, read from the same computed
    /// diff that colors the body so the header can never disagree with the
    /// rendered rows. `None` for the brief window before the diff computes.
    fn line_stats(&self, app: &AppContext) -> Option<(usize, usize)> {
        self.diff_ready.then(|| {
            self.editor
                .as_ref(app)
                .diff()
                .as_ref(app)
                .diff_status()
                .get_diff_lines()
        })
    }
}

/// Keys the shared collapse/hover state map: the multi-file summary header or
/// one file section by index. File states are independent of the summary's,
/// so inner collapse choices survive outer toggles.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SectionKey {
    Summary,
    File(usize),
}

/// Per-section UI state shared with header click closures. Lives outside the
/// view (like `ThinkingBlockStates`) because click handlers only get a
/// `TuiEventContext`, not the view.
#[derive(Clone, Default)]
struct SectionStates {
    states: Rc<RefCell<HashMap<SectionKey, SectionUiState>>>,
}

/// UI state for a single collapsible section.
#[derive(Default)]
struct SectionUiState {
    collapsed: bool,
    /// Hover state for the header row. Owned here so it survives element-tree
    /// rebuilds (the GUI `MouseStateHandle` pattern).
    hover_state: MouseStateHandle,
}

impl SectionStates {
    /// Whether the keyed section is collapsed (default: expanded).
    fn is_collapsed(&self, key: SectionKey) -> bool {
        self.states
            .borrow()
            .get(&key)
            .map(|state| state.collapsed)
            .unwrap_or(false)
    }

    /// Flips the collapse state of the keyed section.
    fn toggle_collapsed(&self, key: SectionKey) {
        let mut states = self.states.borrow_mut();
        let state = states.entry(key).or_default();
        state.collapsed = !state.collapsed;
    }

    /// The persistent hover state handle for the keyed section.
    fn hover_state(&self, key: SectionKey) -> MouseStateHandle {
        self.states
            .borrow_mut()
            .entry(key)
            .or_default()
            .hover_state
            .clone()
    }
}

impl TuiFileEditsView {
    pub(super) fn new(
        action_id: AIAgentActionId,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let storage = ctx.add_model(|_| TuiDiffStorage::new(Vec::new(), DiffSessionType::Local));

        ctx.subscribe_to_model(&storage, |me, _, event, ctx| match event {
            TuiDiffStorageEvent::CandidateDiffsSet => me.rebuild_sections(ctx),
        });

        // Failed and cancelled actions never seed the storage; re-render on
        // the terminal result so the row doesn't stay pending. Successful
        // actions also update their header glyph from this event.
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
            sections: Vec::new(),
            section_states: SectionStates::default(),
        }
    }

    /// Rebuilds one [`FileSection`] per stored diff. Called when the executor
    /// seeds the storage (diffs resolve once, atomically, at preprocess time).
    fn rebuild_sections(&mut self, ctx: &mut ViewContext<Self>) {
        self.sections.clear();
        let diffs = self.storage.as_ref(ctx).diffs().to_vec();

        for (index, diff) in diffs.into_iter().enumerate() {
            let editor = ctx.add_model(|ctx| CodeEditorModel::new_tui(0, ctx));
            editor.update(ctx, |editor, ctx| {
                // Buffer starts as the pre-edit content and doubles as the
                // diff base; applying the deltas produces the post-edit
                // buffer and auto-triggers the diff computation against it.
                editor.reset_content(InitialBufferState::plain_text(&diff.base.content), ctx);
                editor.apply_diffs(deltas_for(&diff.diff_type), ctx);
                // Model-side hunk-context hiding; when the in-flight diff
                // computes, the model recalculates the hidden line ranges
                // (hunks ± context) on its own.
                editor.hide_lines_outside_of_active_diff(CONTEXT_LINES, ctx);
                // Expanded diff navigation; when the diff computes, the
                // model's refresh pushes removed-line ghost blocks into the
                // char-cell render state.
                editor.expand_diffs(ctx);
            });

            // The diff computes asynchronously; re-render when it lands (and
            // start showing header counts, which read the computed diff).
            ctx.subscribe_to_model(&editor, move |me, _, event, ctx| {
                if matches!(event, CodeEditorModelEvent::DiffUpdated) {
                    if let Some(section) = me.sections.get_mut(index) {
                        section.diff_ready = true;
                    }
                    ctx.notify();
                }
            });

            let (verb, name) = verb_and_name(&diff);
            self.sections.push(FileSection {
                editor,
                verb,
                name,
                diff_ready: false,
            });
        }
        ctx.notify();
    }

    /// The action's display state, driving the header glyph and styling.
    fn display_state(&self, app: &AppContext) -> ToolCallDisplayState {
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);
        tool_call_display_state(status.as_ref(), false, None)
    }

    /// The one-line fallback shown before diffs resolve (or when they never
    /// will): a terminal label from the action's recorded result when there is
    /// one, else a pending label.
    fn fallback_label(&self, app: &AppContext) -> String {
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
                let files_label = if files == 1 { "file" } else { "files" };
                format!("Edited {files} {files_label} (+{lines_added} −{lines_removed})")
            }
            Some(RequestFileEditsResult::Cancelled) => "File edits cancelled".to_string(),
            Some(RequestFileEditsResult::DiffApplicationFailed { .. }) => {
                "File edits failed".to_string()
            }
            None => "Preparing edits…".to_string(),
        }
    }

    /// The summed `(added, removed)` counts across all sections, available
    /// only once every file's diff has computed so the summary totals never
    /// tick up as async diffs land.
    fn aggregate_stats(&self, app: &AppContext) -> Option<(usize, usize)> {
        self.sections
            .iter()
            .try_fold((0, 0), |(added, removed), section| {
                section
                    .line_stats(app)
                    .map(|(a, r)| (added + a, removed + r))
            })
    }

    /// Renders one collapsible section: the keyed header over `body`. The
    /// body is built lazily, only when the section is expanded; sections
    /// without a body (`None`) render no chevron.
    fn render_section(
        &self,
        key: SectionKey,
        label: &str,
        line_stats: Option<(usize, usize)>,
        builder: &TuiUiBuilder,
        app: &AppContext,
        body: Option<impl FnOnce() -> Box<dyn TuiElement>>,
    ) -> Box<dyn TuiElement> {
        let has_body = body.is_some();
        let mut column = TuiFlex::column()
            .child(self.render_header(key, label, line_stats, has_body, builder, app));
        if let Some(body) = body {
            if !self.section_states.is_collapsed(key) {
                column.add_child(body());
            }
        }
        column.finish()
    }

    /// Renders a collapsible header row as one styled-span paragraph: a state
    /// glyph (colored like `render_tool_call_section`'s rows), `label` in
    /// bold, colored `+a −r` counts, and the collapse chevron, clickable to
    /// toggle the keyed section. The counts are omitted while `line_stats` is
    /// `None` (diff(s) not yet computed).
    fn render_header(
        &self,
        key: SectionKey,
        label: &str,
        line_stats: Option<(usize, usize)>,
        show_chevron: bool,
        builder: &TuiUiBuilder,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let state = self.display_state(app);
        let hovered = self
            .section_states
            .hover_state(key)
            .lock()
            .unwrap()
            .is_hovered();

        // State lives in the glyph, mirroring `render_tool_call_section`.
        let glyph_style = match state {
            ToolCallDisplayState::Constructing | ToolCallDisplayState::Pending => {
                builder.dim_text_style()
            }
            ToolCallDisplayState::AwaitingApproval | ToolCallDisplayState::Running => {
                builder.attention_glyph_style()
            }
            ToolCallDisplayState::Succeeded => builder.success_glyph_style(),
            ToolCallDisplayState::Failed => builder.error_text_style(),
            ToolCallDisplayState::Cancelled => builder.muted_text_style(),
        };
        let name_style = match state {
            ToolCallDisplayState::Constructing | ToolCallDisplayState::Pending => {
                builder.dim_text_style()
            }
            ToolCallDisplayState::AwaitingApproval
            | ToolCallDisplayState::Running
            | ToolCallDisplayState::Succeeded
            | ToolCallDisplayState::Failed
            | ToolCallDisplayState::Cancelled => builder.primary_text_style(),
        };
        let bold = |style: TuiStyle| style.add_modifier(Modifier::BOLD);
        let embolden = |style: TuiStyle| if hovered { bold(style) } else { style };

        let mut spans = vec![
            (format!("{} ", tool_call_glyph(state)), glyph_style),
            (format!("{label} "), embolden(bold(name_style))),
        ];
        if let Some((added, removed)) = line_stats {
            spans.push((
                format!("+{added}"),
                embolden(bold(builder.diff_added_style())),
            ));
            spans.push((
                format!(" −{removed}"),
                embolden(bold(builder.diff_removed_style())),
            ));
        }
        if show_chevron {
            let chevron = tui_disclosure_chevron(self.section_states.is_collapsed(key));
            spans.push((format!("  {chevron}"), embolden(name_style)));
        }
        let row = TuiText::from_spans(spans).truncate();

        let states = self.section_states.clone();
        TuiHoverable::new(self.section_states.hover_state(key), row.finish())
            .on_click(move |event_ctx, _app| {
                states.toggle_collapsed(key);
                event_ctx.notify();
            })
            .finish()
    }

    /// Renders the per-file sections as a column of collapsible sections with
    /// a blank row between files.
    fn render_file_sections(
        &self,
        builder: &TuiUiBuilder,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let last_index = self.sections.len() - 1;
        let mut column = TuiFlex::column();
        for (index, section) in self.sections.iter().enumerate() {
            let line_stats = section.line_stats(app);
            // Zero-change (and not-yet-computed) diffs have no body to toggle.
            let has_body = line_stats.is_some_and(|stats| stats != (0, 0));
            let file_section = self.render_section(
                SectionKey::File(index),
                &format!("{} {}", section.verb, section.name),
                line_stats,
                builder,
                app,
                has_body.then_some(|| self.render_body(section, builder, app)),
            );
            // Blank row between files; the block composer pads after the last.
            let padding_bottom = if index == last_index { 0 } else { 1 };
            column.add_child(
                TuiContainer::new(file_section)
                    .with_padding_bottom(padding_bottom)
                    .finish(),
            );
        }
        column.finish()
    }

    /// Builds the body for one file section: the core editor element,
    /// read-only (no action handler), with a line-number gutter and diff
    /// styles. Ghost rows and hidden ranges reach the element through the
    /// render state; the only diff data read here is the added/changed line
    /// classification that drives the green line style.
    fn render_body(
        &self,
        section: &FileSection,
        builder: &TuiUiBuilder,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let added_style = builder.diff_added_style();
        let line_overrides = section
            .editor
            .as_ref(app)
            .diff()
            .as_ref(app)
            .added_or_changed_lines()
            .map(|range| (range, added_style))
            .collect();

        TuiEditorElement::new(&section.editor, app)
            .with_line_number_gutter()
            .with_styles(TuiEditorStyles {
                text: builder.muted_text_style(),
                ghost: builder.diff_removed_style(),
                gap: builder.dim_text_style(),
                line_overrides,
            })
            // A file's conventional trailing newline must not render as a
            // blank numbered row (the body ends at the outermost context line).
            .hide_trailing_empty_line()
            .finish()
    }
}

/// The buffer edits that turn a diff's base content into its final content.
fn deltas_for(diff_type: &DiffType) -> Vec<DiffDelta> {
    match diff_type {
        DiffType::Create { delta } | DiffType::Delete { delta } => vec![delta.clone()],
        DiffType::Update { deltas, .. } => deltas.clone(),
    }
}

/// The header verb and display name for a diff: file names only (no
/// directories), with renames shown as `old → new`.
fn verb_and_name(diff: &FileDiff) -> (&'static str, String) {
    let file_name = |path: &str| {
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned())
    };
    let name = file_name(&diff.base.file_path);
    match &diff.diff_type {
        DiffType::Create { .. } => ("Created", name),
        DiffType::Delete { .. } => ("Deleted", name),
        DiffType::Update {
            rename: Some(to), ..
        } => {
            let to_name = file_name(&to.to_string_lossy());
            if to_name == name {
                ("Updated", name)
            } else {
                ("Updated", format!("{name} → {to_name}"))
            }
        }
        DiffType::Update { rename: None, .. } => ("Updated", name),
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
        let builder = TuiUiBuilder::from_app(app);

        if self.sections.is_empty() {
            let label = self.fallback_label(app);
            return TuiContainer::new(Box::new(
                TuiText::new(label).with_style(builder.dim_text_style()),
            ))
            .finish();
        }

        // Single-file edits render the file section alone; multi-file edits
        // nest the sections, indented, under one collapsible summary header.
        if self.sections.len() == 1 {
            return self.render_file_sections(&builder, app);
        }

        self.render_section(
            SectionKey::Summary,
            &format!("Edited {} files", self.sections.len()),
            self.aggregate_stats(app),
            &builder,
            app,
            Some(|| {
                TuiContainer::new(self.render_file_sections(&builder, app))
                    .with_padding_left(2)
                    .finish()
            }),
        )
    }
}

#[cfg(test)]
#[path = "tui_file_edits_view_tests.rs"]
mod tests;
