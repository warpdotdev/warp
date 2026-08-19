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
//! render the file section alone. Blocked edits use the in-progress `Editing`
//! verb while awaiting approval. Failed and cancelled actions fall back to a
//! one-line label from the action's recorded result; restored successful
//! actions are hydrated from their original `FileEdit` request.
use std::collections::HashMap;
use std::path::Path;

use ai::agent::action::FileEdit;
use ai::agent::action_result::{AIAgentActionResultType, RequestFileEditsResult};
use ai::diff_validation::{DiffDelta, DiffType};
use itertools::Itertools;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    AIActionStatus, AIAgentActionId, AIConversationId, BlocklistAIActionEvent,
    BlocklistAIActionModel, CancellationReason, DiffSessionType, FileDiff,
    convert_file_edits_to_file_diffs,
};
use warp_editor::content::buffer::InitialBufferState;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    Modifier, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiStyle, TuiText,
    tui_collapsible,
};
use warpui_core::keymap::EditableBinding;
use warpui_core::keymap::macros::*;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::editor_element::{TuiEditorElement, TuiEditorStyles};
use crate::keybindings::{TUI_BINDING_GROUP, is_tui_owned_binding};
use crate::tool_call_labels::{
    ToolCallDisplayState, styled_tool_call_label_spans, tool_call_display_state,
};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_diff_storage::{TuiDiffStorage, TuiDiffStorageEvent, TuiDiffStorageHandle};
use crate::tui_permission_prompt::{
    TuiPermissionPrompt, TuiPermissionPromptEvent, render_permission_card,
};

/// Keymap context set on `TuiFileEditsView` while a file-edits permission card
/// is active and the option list (yes/no/Other) owns focus, gating the `e`
/// expand/collapse-all binding.
const FILE_EDITS_PERMISSION_ACTIVE: &str = "TuiFileEditsPermissionActive";

/// Registers `TuiFileEditsView`-specific keybindings.
pub(crate) fn init(app: &mut AppContext) {
    let predicate = id!(TuiFileEditsView::ui_name()) & id!(FILE_EDITS_PERMISSION_ACTIVE);
    app.register_editable_bindings([EditableBinding::new(
        "tui:file-edits-permission:toggle-expand-all",
        "Expand or collapse all diffs",
        TuiFileEditsViewAction::ToggleExpandAll,
    )
    .with_context_predicate(predicate)
    .with_group(TUI_BINDING_GROUP)
    .with_key_binding("e")]);
    app.register_tui_binding_validator::<TuiFileEditsView>(is_tui_owned_binding);
}

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
    conversation_id: AIConversationId,
    permission_prompt: ViewHandle<TuiPermissionPrompt>,
    /// One section per resolved file diff, in storage order; empty until the
    /// executor seeds the storage.
    sections: Vec<FileSection>,
    /// Shared per-section UI state (collapse + header hover) for the summary
    /// header and each file.
    section_states: SectionStates,
}

/// Events emitted to the owning agent block.
pub(super) enum TuiFileEditsViewEvent {
    BlockingStateChanged,
    LayoutChanged,
    ReplacementGuidanceSubmitted(String),
}

/// User interactions handled by the file-edits view.
#[derive(Clone, Debug)]
pub(super) enum TuiFileEditsViewAction {
    ToggleSection(SectionKey),
    /// Toggles all diff sections between expanded and collapsed together.
    ToggleExpandAll,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SectionKey {
    Summary,
    File(usize),
}

/// Persistent collapse and hover state for each section.
#[derive(Default)]
struct SectionStates {
    states: HashMap<SectionKey, SectionUiState>,
}

impl SectionStates {
    fn expand_all(&mut self, keys: &[SectionKey]) {
        self.states.clear();
        for key in keys {
            self.states.insert(
                *key,
                SectionUiState {
                    collapsed: false,
                    ..Default::default()
                },
            );
        }
    }

    fn collapse_all(&mut self, keys: &[SectionKey]) {
        self.states.clear();
        for key in keys {
            self.states.insert(
                *key,
                SectionUiState {
                    collapsed: true,
                    ..Default::default()
                },
            );
        }
    }

    /// Whether the keyed section is collapsed.
    fn is_collapsed(&self, key: SectionKey) -> bool {
        self.states
            .get(&key)
            .expect("file-edit section state initialized before render")
            .collapsed
    }

    /// Flips the collapse state of the keyed section.
    fn toggle_collapsed(&mut self, key: SectionKey) {
        let state = self
            .states
            .get_mut(&key)
            .expect("file-edit section state initialized before toggle");
        state.collapsed = !state.collapsed;
    }

    /// Toggles all sections between fully expanded and fully collapsed:
    /// if any section is currently expanded, collapse all; otherwise expand
    /// all.
    fn toggle_expand_all(&mut self, keys: &[SectionKey]) {
        let any_expanded = keys.iter().any(|key| !self.is_collapsed(*key));
        let target_collapsed = any_expanded;
        for key in keys {
            let state = self
                .states
                .get_mut(key)
                .expect("file-edit section state initialized before toggle");
            state.collapsed = target_collapsed;
        }
    }

    /// The persistent hover state handle for the keyed section.
    fn hover_state(&self, key: SectionKey) -> MouseStateHandle {
        self.states
            .get(&key)
            .expect("file-edit section state initialized before render")
            .hover_state
            .clone()
    }
}

/// UI state for a single collapsible section.
#[derive(Default)]
struct SectionUiState {
    collapsed: bool,
    /// Hover state for the header row. Owned here so it survives element-tree
    /// rebuilds (the GUI `MouseStateHandle` pattern).
    hover_state: MouseStateHandle,
}

impl TuiFileEditsView {
    pub(super) fn new(
        action_id: AIAgentActionId,
        conversation_id: AIConversationId,
        file_edits: Vec<FileEdit>,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // A recorded result means this is a restored (already-finished) action;
        // live actions have no result yet and stay executor-backed below. The
        // borrow into the action model is scoped here so it is released before
        // the `add_model` / `subscribe_to_model` calls below.
        let (is_restored, is_restored_success) = {
            let restored_result = action_model
                .as_ref(ctx)
                .get_action_result(&action_id)
                .and_then(|result| match &result.result {
                    AIAgentActionResultType::RequestFileEdits(result) => Some(result),
                    _ => None,
                });
            let is_restored = restored_result.is_some();
            // Only successful restored edits rehydrate their originally-requested
            // diffs. Cancelled and failed actions keep their terminal fallback
            // label ("File edits cancelled" / "File edits failed"), mirroring
            // the GUI's `set_restored_file_edits` which marks non-success
            // results `CodeDiffState::Rejected` rather than showing the diff.
            let is_restored_success = matches!(
                restored_result,
                Some(RequestFileEditsResult::Success { .. })
            );
            (is_restored, is_restored_success)
        };
        let initial_diffs = if is_restored_success {
            // Legacy persisted results do not carry line counts, but the
            // original request can be converted into the display-only diff
            // ranges that drive TUI headers and bodies.
            convert_file_edits_to_file_diffs(file_edits, &None, &None)
        } else {
            Default::default()
        };
        let storage = ctx.add_model(|_| TuiDiffStorage::new(initial_diffs, DiffSessionType::Local));

        ctx.subscribe_to_model(&storage, |me, _, event, ctx| match event {
            TuiDiffStorageEvent::CandidateDiffsSet => me.rebuild_sections(ctx),
        });

        // Failed and cancelled actions never seed the storage; re-render on
        // the terminal result so the row doesn't stay pending. Successful
        // actions also update their header glyph from this event.
        ctx.subscribe_to_model(action_model, |me, _, event, ctx| {
            if event.action_id() != &me.action_id {
                return;
            }
            if matches!(
                event,
                BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
            ) {
                me.expand_all_sections();
                ctx.notify();
            } else if matches!(
                event,
                BlocklistAIActionEvent::ExecutingAction(_)
                    | BlocklistAIActionEvent::FinishedAction { .. }
            ) {
                me.collapse_all_sections();
                ctx.notify();
            }
        });

        // An already-resolved action (e.g. on a restored transcript) must
        // rehydrate the same lossy FileDiff representation used by the GUI.
        // Legacy persisted ApplyFileDiffs results contain updated file metadata
        // but no line counts, so rendering only the recorded result produces
        // the incorrect +0/-0 fallback.
        if !is_restored {
            // Live actions stay executor-backed; registering a storage here
            // lets preprocessing seed the authoritative resolved diffs.
            let executor = action_model.as_ref(ctx).request_file_edits_executor(ctx);
            executor.update(ctx, |executor, _| {
                let handle = TuiDiffStorageHandle::new(storage.clone());
                executor.register_requested_edits(&action_id, Box::new(handle));
            });
        }

        let prompt_action_id = action_id.clone();
        let prompt_action_model = action_model.clone();
        let permission_prompt = ctx.add_typed_action_tui_view(move |ctx| {
            TuiPermissionPrompt::new(prompt_action_model, prompt_action_id, None, ctx)
        });
        ctx.subscribe_to_view(&permission_prompt, |view, _, event, ctx| match event {
            TuiPermissionPromptEvent::AcceptRequested => view.accept(ctx),
            TuiPermissionPromptEvent::ReplacementGuidanceSubmitted(text) => {
                ctx.emit(TuiFileEditsViewEvent::ReplacementGuidanceSubmitted(
                    text.clone(),
                ));
            }
            TuiPermissionPromptEvent::RejectRequested => view.reject(ctx),
            TuiPermissionPromptEvent::BlockingStateChanged => {
                ctx.emit(TuiFileEditsViewEvent::BlockingStateChanged);
                view.invalidate_layout(ctx);
            }
            TuiPermissionPromptEvent::LayoutChanged => view.invalidate_layout(ctx),
        });

        let mut view = Self {
            storage,
            action_id,
            action_model: action_model.clone(),
            conversation_id,
            permission_prompt,
            sections: Vec::new(),
            section_states: SectionStates::default(),
        };
        if is_restored_success {
            view.rebuild_sections(ctx);
        }
        view
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
                    ctx.emit(TuiFileEditsViewEvent::LayoutChanged);
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
        let is_blocked = self
            .action_model
            .as_ref(ctx)
            .get_action_status(&self.action_id)
            .is_some_and(|status| status.is_blocked());
        if is_blocked {
            self.expand_all_sections();
        } else {
            self.collapse_all_sections();
        }
        ctx.emit(TuiFileEditsViewEvent::LayoutChanged);
        ctx.notify();
    }

    fn section_keys(&self) -> Vec<SectionKey> {
        let mut keys = Vec::with_capacity(self.sections.len() + 1);
        if self.sections.len() > 1 {
            keys.push(SectionKey::Summary);
        }
        keys.extend((0..self.sections.len()).map(SectionKey::File));
        keys
    }

    fn expand_all_sections(&mut self) {
        let keys = self.section_keys();
        self.section_states.expand_all(&keys);
    }

    fn collapse_all_sections(&mut self) {
        let keys = self.section_keys();
        self.section_states.collapse_all(&keys);
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
        let file_edits_result = result.and_then(|result| match &result.result {
            AIAgentActionResultType::RequestFileEdits(result) => Some(result),
            _ => None,
        });
        match file_edits_result {
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
                match file_edit_stats_label(*lines_added, *lines_removed) {
                    Some(stats) => format!("Edited {files} {files_label} ({stats})"),
                    None => format!("Edited {files} {files_label}"),
                }
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
        let Some(body) = body else {
            let (header_spans, _) = self.header_spans(label, line_stats, false, builder, app);
            return TuiText::from_spans(header_spans).truncate().finish();
        };

        let collapsed = self.section_states.is_collapsed(key);
        let hover_state = self.section_states.hover_state(key);
        let hovered = hover_state.lock().unwrap().is_hovered();
        let (mut header_spans, chevron_style) =
            self.header_spans(label, line_stats, hovered, builder, app);
        // The helper contributes one separating space with the chevron; add
        // another here to preserve the existing two-space disclosure gap.
        header_spans.push((" ".to_owned(), chevron_style));
        tui_collapsible(
            collapsed,
            header_spans,
            chevron_style,
            hover_state,
            body,
            move |event_ctx, _app| {
                event_ctx.dispatch_typed_action(TuiFileEditsViewAction::ToggleSection(key));
            },
        )
    }

    /// Builds a section header's styled spans: a state glyph (colored like
    /// `render_tool_call_section`'s rows), a bold action with neutral details,
    /// and colored `+a −r` counts. [`tui_collapsible`] appends the shared
    /// chevron for sections with bodies; the counts are omitted while
    /// `line_stats` is `None` (diff(s) not yet computed).
    fn header_spans(
        &self,
        label: &str,
        line_stats: Option<(usize, usize)>,
        hovered: bool,
        builder: &TuiUiBuilder,
        app: &AppContext,
    ) -> (Vec<(String, TuiStyle)>, TuiStyle) {
        file_edit_header_spans(self.display_state(app), label, line_stats, hovered, builder)
    }

    /// Renders the per-file sections as a column of collapsible sections with
    /// a blank row between files.
    fn render_file_sections(
        &self,
        builder: &TuiUiBuilder,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let state = self.display_state(app);
        let last_index = self.sections.len() - 1;
        let mut column = TuiFlex::column();
        for (index, section) in self.sections.iter().enumerate() {
            let line_stats = section.line_stats(app);
            // Zero-change (and not-yet-computed) diffs have no body to toggle.
            let has_body = line_stats.is_some_and(|stats| stats != (0, 0));
            let label = file_edit_header_label(state, section.verb, &section.name);
            let file_section = self.render_section(
                SectionKey::File(index),
                &label,
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
                text_overrides: Vec::new(),
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

fn file_edit_header_label(
    state: ToolCallDisplayState,
    completed_verb: &str,
    subject: &str,
) -> String {
    let verb = if state == ToolCallDisplayState::Blocked {
        "Editing"
    } else {
        completed_verb
    };
    format!("{verb} {subject}")
}

fn file_edit_stat_labels(added: usize, removed: usize) -> [Option<String>; 2] {
    [
        (added > 0).then(|| format!("+{added}")),
        (removed > 0).then(|| format!("−{removed}")),
    ]
}

fn file_edit_stats_label(added: usize, removed: usize) -> Option<String> {
    let label = file_edit_stat_labels(added, removed)
        .into_iter()
        .flatten()
        .join(" ");
    (!label.is_empty()).then_some(label)
}

fn file_edit_header_spans(
    state: ToolCallDisplayState,
    label: &str,
    line_stats: Option<(usize, usize)>,
    hovered: bool,
    builder: &TuiUiBuilder,
) -> (Vec<(String, TuiStyle)>, TuiStyle) {
    let mut spans = vec![(format!("{} ", state.glyph()), state.glyph_style(builder))];
    spans.extend(styled_tool_call_label_spans(label, builder));
    if let Some((added, removed)) = line_stats {
        let [added_label, removed_label] = file_edit_stat_labels(added, removed);
        if let Some(added_label) = added_label {
            spans.push((format!(" {added_label}"), builder.diff_added_style()));
        }
        if let Some(removed_label) = removed_label {
            spans.push((format!(" {removed_label}"), builder.diff_removed_style()));
        }
    }
    let chevron_style = if hovered {
        state.label_style(builder).add_modifier(Modifier::BOLD)
    } else {
        state.label_style(builder)
    };
    (spans, chevron_style)
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
    type Event = TuiFileEditsViewEvent;
}
impl TuiFileEditsView {
    pub(super) fn active_permission_prompt(
        &self,
        app: &AppContext,
    ) -> Option<ViewHandle<TuiPermissionPrompt>> {
        self.permission_prompt
            .as_ref(app)
            .is_active(app)
            .then(|| self.permission_prompt.clone())
    }

    fn accept(&self, ctx: &mut ViewContext<Self>) {
        let action_id = self.action_id.clone();
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.execute_action(&action_id, self.conversation_id, ctx);
        });
    }

    fn reject(&self, ctx: &mut ViewContext<Self>) {
        let action_id = self.action_id.clone();
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_action_with_id(
                self.conversation_id,
                &action_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
    }

    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiFileEditsViewEvent::LayoutChanged);
        ctx.notify();
    }
}
impl TuiView for TuiFileEditsView {
    fn ui_name() -> &'static str {
        "TuiFileEditsView"
    }
    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        vec![self.permission_prompt.id()]
    }

    fn keymap_context(&self, app: &AppContext) -> warpui_core::keymap::Context {
        let mut context = Self::default_keymap_context();
        // Activate the `e` expand/collapse-all binding only when the
        // permission card is active and the option list (yes/no/Other) owns
        // focus — not while the user is typing in the Other custom-text editor.
        let is_blocked = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id)
            .is_some_and(|s| s.is_blocked());
        if is_blocked && self.permission_prompt.as_ref(app).list_is_focused(app) {
            context.set.insert(FILE_EDITS_PERMISSION_ACTIVE);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let content = self.render_diff_content(app);
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);
        if !matches!(status, Some(AIActionStatus::Blocked)) {
            return content;
        }

        let builder = TuiUiBuilder::from_app(app);
        let expand_collapse_hint = TuiText::from_spans([
            ("e".to_owned(), builder.primary_text_style()),
            (" to expand/collapse".to_owned(), builder.muted_text_style()),
        ])
        .truncate()
        .finish();

        render_permission_card(
            &self.permission_prompt,
            "Is it OK if I make these file edits?",
            Some(content),
            Some(expand_collapse_hint),
            app,
        )
    }
}

impl TuiFileEditsView {
    fn render_diff_content(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);

        if self.sections.is_empty() {
            let label = self.fallback_label(app);
            return TuiContainer::new(
                TuiText::from_spans(styled_tool_call_label_spans(&label, &builder)).finish(),
            )
            .finish();
        }

        // Single-file edits render the file section alone; multi-file edits
        // nest the sections, indented, under one collapsible summary header.
        if self.sections.len() == 1 {
            return self.render_file_sections(&builder, app);
        }

        self.render_section(
            SectionKey::Summary,
            &file_edit_header_label(
                self.display_state(app),
                "Edited",
                &format!("{} files", self.sections.len()),
            ),
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

impl TypedActionView for TuiFileEditsView {
    type Action = TuiFileEditsViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiFileEditsViewAction::ToggleSection(key) => {
                self.section_states.toggle_collapsed(*key);
                ctx.emit(TuiFileEditsViewEvent::LayoutChanged);
                ctx.notify();
            }
            TuiFileEditsViewAction::ToggleExpandAll => {
                let keys = self.section_keys();
                self.section_states.toggle_expand_all(&keys);
                ctx.emit(TuiFileEditsViewEvent::LayoutChanged);
                ctx.notify();
            }
        }
    }
}

#[cfg(test)]
#[path = "tui_file_edits_view_tests.rs"]
mod tests;
