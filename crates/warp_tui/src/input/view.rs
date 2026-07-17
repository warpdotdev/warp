//! [`TuiInputView`] — ratatui-rendered TUI prompt input.
//!
//! Implements [`TuiView`] + [`TypedActionView`]. The view:
//!
//! - Holds a [`ModelHandle<CodeEditorModel>`] constructed in `LayoutMode::CharCell`.
//! - Renders the core [`TuiEditorElement`] verbatim (editable, scroll-windowed).
//! - Owns prompt submission and the `!` shell-mode composition.
//! - Dispatches keystrokes as [`TuiInputAction`] typed actions.
//! - Emits [`TuiInputViewEvent::Submitted`] when the user presses Enter.
//!
//! # Architecture
//!
//! The view works directly with [`CodeEditorModel`] (char-cell mode) so that future
//! TUI features — vim, syntax highlighting, diff, hidden lines — come for free from
//! the shared editor infrastructure. Rendering and mouse interaction come from the
//! shared core element ([`crate::editor_element`]). Editor session mechanisms live
//! model-side, mirroring the GUI split: viewport scroll state on the char-cell
//! render state (`CharCellState`), drag-selection state on the selection model,
//! visual-row kill edits on `CodeEditorModel`. What stays here is input policy:
//! prompt-only keybindings, submit, inline menus, and shell mode.
//!
//! See `specs/tui-input-view/TECH.md` for the full keybinding table.

use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, BlocklistAIInputModel, InputType,
    InputTypeAutoDetectionSource, LLMId, TuiMcpAction,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiText};
use warpui_core::elements::MouseStateHandle;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding};
use warpui_core::{
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, TuiView, TypedActionView,
    ViewContext, ViewHandle,
};

use crate::editor_element::{TuiEditorAction, TuiEditorElement, TuiEditorStyles};
use crate::editor_interaction::{
    apply_editor_action, follow_editor_cursor, TuiEditorBehavior, TuiEditorCommand,
    TuiEditorInteractionOutcome, TuiEditorState,
};
use crate::inline_menu::{active_inline_menu, TuiInlineMenu, TuiInlineMenuAccepted};
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::{
    KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG, PLAN_TOGGLE_AVAILABLE_FLAG, TUI_BINDING_GROUP,
};
use crate::transcript_view::TuiTranscriptView;
use crate::tui_builder::TuiUiBuilder;

/// Keymap-context flag set while the input has contextual Escape behavior.
///
/// The input owns a single Escape binding so modes can arbitrate explicitly in
/// [`TuiInputView::handle_escape`] instead of relying on keymap registration
/// order. Inline menus take priority; later input modes should be handled only
/// after the menu branch.
const INPUT_HANDLES_ESCAPE_FLAG: &str = "TuiInputHandlesEscape";
// ─────────────────────────────────────────────────────────────────────────────
// Keybindings
// ─────────────────────────────────────────────────────────────────────────────

/// Registers the input view's editing keybindings (the readline/chord
/// table). Called once at TUI startup from `keybindings::init` — these
/// bindings exist only in the TUI process; the GUI never registers them.
///
/// Each command is an [`EditableBinding`] named `tui:input:*`, so it is
/// user-remappable by name (via `keybindings.yaml`, once the TUI loads
/// overrides — a follow-up). Commands with multiple default keys register one
/// binding per key under the same name, which the keymap supports directly:
/// it tracks every binding registered under a name, and a custom-trigger
/// override replaces the trigger on all of them. Printable-character
/// insertion is not a binding — it stays element-level in
/// [`TuiEditorElement`]'s event dispatch, matching the GUI.
pub fn init(app: &mut AppContext) {
    app.register_editable_bindings([
        // Submit and contextual Escape are prompt policy, not editor policy.
        EditableBinding::new(
            "tui:input:submit",
            "Submit the input",
            TuiInputAction::Submit,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:input:handle_escape",
            "Handle contextual input escape",
            TuiInputAction::HandleEscape,
        )
        .with_context_predicate(id!("TuiInputView") & id!(INPUT_HANDLES_ESCAPE_FLAG))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("escape"),
    ]);
}

// ─────────────────────────────────────────────────────────────────────────────
// View events
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by [`TuiInputView`].
#[derive(Debug, Clone)]
pub enum TuiInputViewEvent {
    /// The user pressed Enter to submit the current input. Contains the final text.
    Submitted(String),
    /// The user selected a slash command menu item.
    AcceptedSlashCommand(AcceptSlashCommandOrSavedPrompt),
    /// The user selected a conversation menu item.
    AcceptedConversation(warp::tui_export::AgentConversationEntryId),
    /// The user selected a model menu item.
    AcceptedModel(LLMId),
    /// The user selected an action from the MCP menu.
    AcceptedMcp(TuiMcpAction),
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed action enum
// ─────────────────────────────────────────────────────────────────────────────

/// Prompt policy plus shared editor actions dispatched to [`TuiInputView`].
///
/// Each variant corresponds to one or more keybindings from the spec keybinding table.
#[derive(Debug, Clone)]
pub enum TuiInputAction {
    /// Apply input emitted by the shared editor element.
    Editor(TuiEditorAction),
    /// Submit the current input (`Enter`).
    Submit,
    /// Handle contextual input Escape behavior, prioritizing an open inline menu.
    HandleEscape,
    /// Apply an editing command shared with generic TUI editors.
    EditorCommand(TuiEditorCommand),
    /// Place the cursor at `offset` without starting a drag selection
    /// (the `!` gutter click).
    SetCursor { offset: CharOffset },
}

// ─────────────────────────────────────────────────────────────────────────────
// View
// ─────────────────────────────────────────────────────────────────────────────

/// The `TuiView`-implementing entry point for the TUI prompt input.
pub struct TuiInputView {
    /// The backing code editor in char-cell (terminal) mode. Also owns the
    /// editor session state the input drives: viewport scroll (char-cell
    /// render state) and drag-selection state (selection model).
    model: ModelHandle<CodeEditorModel>,
    /// Shared input-mode state driving NLD and explicit shell-mode handling.
    input_mode: ModelHandle<BlocklistAIInputModel>,
    /// Single authoritative menu mode, mirroring the GUI input's suggestions mode.
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    /// Generalized inline menus used to route prioritized menu actions.
    inline_menus: Vec<TuiInlineMenu>,
    /// Shared editor session state, including the single-entry kill buffer.
    editor_state: TuiEditorState,
    /// Multiline insertion and six-row viewport policy.
    editor_behavior: TuiEditorBehavior,
    /// Mouse state for the shell-mode `!` gutter; created once here (not inline
    /// during render) so mouse tracking survives per-frame element rebuilds.
    prefix_mouse_state: MouseStateHandle,
    /// Whether this view is focused, tracked via `on_focus`/`on_blur` like
    /// the GUI's `EditorView::focused`. Snapshotted into the editor element
    /// so it only consumes typed text while the input is focused.
    focused: bool,
    /// Source of truth for whether a rendered plan can be toggled. Production
    /// construction always provides this; isolated input tests omit it.
    transcript: Option<ViewHandle<TuiTranscriptView>>,
    keyboard_enhancement_supported: bool,
    /// The most recently submitted user message, saved at submission time.
    ///
    /// Pressing `↑` in an empty input recalls this text so the user can
    /// re-edit and re-submit their last prompt — matching the shell-history
    /// affordance described in the Figma spec (`↑ to edit`).
    last_submitted_text: Option<String>,
}

impl Entity for TuiInputView {
    type Event = TuiInputViewEvent;
}

impl TuiInputView {
    /// Construct a new `TuiInputView` backed by `model` (must be in char-cell
    /// mode). Construction stays crate-internal because `inline_menu` is the
    /// crate-private active-menu adapter; keeping this as the only constructor
    /// prevents menu and non-menu initialization paths from diverging.
    ///
    /// The model carries the terminal width (set via
    /// [`CodeEditorModel::new_tui`]); the view does not keep its own copy.
    ///
    /// `input_mode` is the shared input-mode model backing detected and explicit shell-mode
    /// handling; the view re-renders whenever the mode changes.
    ///
    /// Subscribes to [`CodeEditorModelEvent::ContentChanged`] to trigger re-renders
    /// whenever the buffer changes from outside `handle_action`.
    pub(crate) fn new(
        model: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        inline_menus: Vec<TuiInlineMenu>,
        transcript: ViewHandle<TuiTranscriptView>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_internal(
            model,
            input_mode,
            suggestions_mode,
            inline_menus,
            Some(transcript),
            ctx,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        model: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        inline_menus: Vec<TuiInlineMenu>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_internal(model, input_mode, suggestions_mode, inline_menus, None, ctx)
    }

    fn new_internal(
        model: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        inline_menus: Vec<TuiInlineMenu>,
        transcript: Option<ViewHandle<TuiTranscriptView>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&model, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                ctx.notify();
            }
        });
        // The model only emits on real config changes, and rendering branches
        // on the config (shell-mode gutter/border), so every event re-renders.
        ctx.subscribe_to_model(&input_mode, |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&suggestions_mode, |_, _, _, ctx| ctx.notify());
        Self {
            model,
            input_mode,
            suggestions_mode,
            inline_menus,
            editor_state: TuiEditorState::default(),
            editor_behavior: TuiEditorBehavior::multiline(6),
            prefix_mouse_state: MouseStateHandle::default(),
            focused: false,
            transcript,
            keyboard_enhancement_supported: false,
            last_submitted_text: None,
        }
    }

    pub(crate) fn with_keyboard_enhancement_supported(
        mut self,
        keyboard_enhancement_supported: bool,
    ) -> Self {
        self.keyboard_enhancement_supported = keyboard_enhancement_supported;
        self
    }

    fn plan_toggle_available(&self, ctx: &AppContext) -> bool {
        self.transcript
            .as_ref()
            .is_some_and(|transcript| transcript.as_ref(ctx).has_toggleable_plan(ctx))
    }
    /// Whether the input is in detected or explicitly locked shell mode.
    pub(crate) fn is_shell_mode(&self, ctx: &AppContext) -> bool {
        input_mode_policy::is_shell_mode(self.input_mode.as_ref(ctx))
    }

    /// Returns a handle to the backing [`CodeEditorModel`].
    pub fn model(&self) -> &ModelHandle<CodeEditorModel> {
        &self.model
    }

    /// Whether the input buffer is empty.
    pub fn is_empty(&self, ctx: &AppContext) -> bool {
        self.model.as_ref(ctx).content().as_ref(ctx).is_empty()
    }

    /// Clears the input buffer, resets to the setting-derived agent mode, and
    /// resets the viewport scroll.
    pub fn clear(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |m, ctx| m.clear_buffer(ctx));
        self.reset_to_default_agent_mode(ctx);
        // The cursor is back at the buffer start, so following it scrolls the
        // viewport back to the top.
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Builds this frame's core editor element: editable, scroll-windowed, and
    /// dispatching [`TuiEditorAction`]s back as [`TuiInputAction`]s. `render`
    /// boxes it (behind the shell-mode `!` gutter when active); tests construct
    /// it directly to exercise mouse dispatch.
    fn render_element(&self, ctx: &AppContext) -> TuiEditorElement {
        let builder = TuiUiBuilder::from_app(ctx);
        let mut styles = TuiEditorStyles::default();
        if let Some(range) = self
            .inline_menus
            .iter()
            .find_map(|inline_menu| inline_menu.input_highlight_range(ctx))
        {
            styles
                .text_overrides
                .push((range, builder.slash_command_text_style()));
        }
        let mut element = TuiEditorElement::new(&self.model, ctx)
            .editable()
            .with_view_focused(self.focused)
            .with_viewport_rows(self.editor_behavior.viewport_rows())
            .with_styles(styles)
            .on_action(|action, event_ctx| {
                event_ctx.dispatch_typed_action(TuiInputAction::Editor(action))
            });
        if let Some(hint_text) = self
            .inline_menus
            .iter()
            .find_map(|inline_menu| inline_menu.input_argument_hint_text(ctx))
        {
            element = element.with_trailing_ghost_text(hint_text, builder.dim_text_style());
        }
        element
    }
    /// Collapses the current text selection to its head without changing text.
    pub(crate) fn clear_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let head = self
            .model
            .as_ref(ctx)
            .buffer_selection_model()
            .as_ref(ctx)
            .first_selection_head();
        self.model.update(ctx, |model, ctx| {
            model.select_at(head, false, ctx);
            model.end_selection(ctx);
        });
        ctx.notify();
    }

    /// The editor element for this frame, boxed for the render tree.
    fn render_input(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        self.render_element(ctx).finish()
    }
    pub(crate) fn set_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let text = self.editor_behavior.normalize_text(text);
        self.model.update(ctx, |m, ctx| {
            m.clear_buffer(ctx);
            m.user_insert(text, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Composes the shell-mode input row: the accent-styled `!` affordance in a
    /// two-column gutter (glyph plus one column of right padding), then the
    /// editor filling the remaining width. The gutter is outside the editable
    /// area; clicking it places the cursor at the start of the buffer.
    fn shell_element(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let prefix_style = TuiUiBuilder::from_app(ctx).shell_mode_accent_style();
        let prefix = TuiHoverable::new(
            self.prefix_mouse_state.clone(),
            TuiContainer::new(TuiText::new("!").with_style(prefix_style).finish())
                .with_padding_right(1)
                .finish(),
        )
        .on_click(|event_ctx, _| {
            event_ctx.dispatch_typed_action(TuiInputAction::SetCursor {
                offset: CharOffset::from(1),
            });
        });
        TuiFlex::row()
            .child(prefix.finish())
            .flex_child(self.render_input(ctx))
            .finish()
    }
}

impl TuiView for TuiInputView {
    fn ui_name() -> &'static str {
        "TuiInputView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        if self.is_shell_mode(ctx) {
            self.shell_element(ctx)
        } else {
            self.render_input(ctx)
        }
    }

    fn keymap_context(&self, ctx: &AppContext) -> keymap::Context {
        input_keymap_context(
            self.active_inline_menu(ctx).is_some() || self.is_shell_mode(ctx),
            self.plan_toggle_available(ctx),
            self.keyboard_enhancement_supported,
        )
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }
}

fn input_keymap_context(
    input_handles_escape: bool,
    plan_toggle_available: bool,
    keyboard_enhancement_supported: bool,
) -> keymap::Context {
    let mut context = keymap::Context::default();
    context.set.insert(TuiInputView::ui_name());
    if input_handles_escape {
        context.set.insert(INPUT_HANDLES_ESCAPE_FLAG);
    }
    if plan_toggle_available {
        context.set.insert(PLAN_TOGGLE_AVAILABLE_FLAG);
    }
    if keyboard_enhancement_supported {
        context.set.insert(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG);
    }
    context
}
impl TypedActionView for TuiInputView {
    type Action = TuiInputAction;

    fn handle_action(&mut self, action: &TuiInputAction, ctx: &mut ViewContext<Self>) {
        if self.handle_inline_menu_action(action, ctx) {
            return;
        }
        let outcome = match action {
            TuiInputAction::Editor(editor_action) => {
                // A `!` typed at the very start of the input enters shell mode
                // instead of inserting (matching the GUI's typed-only trigger).
                if matches!(editor_action, TuiEditorAction::InsertChar('!'))
                    && !self.is_shell_mode(ctx)
                    && self.is_cursor_at_start(ctx)
                    && !self
                        .input_mode
                        .as_ref(ctx)
                        .is_terminal_use_active_or_pending()
                {
                    self.enter_shell_mode(ctx);
                    TuiEditorInteractionOutcome::FollowCursor
                } else {
                    apply_editor_action(&self.model, editor_action, self.editor_behavior, ctx)
                }
            }
            TuiInputAction::Submit => {
                self.submit(ctx);
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiInputAction::HandleEscape => {
                self.handle_escape(ctx);
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiInputAction::EditorCommand(command) => {
                // Only open the conversation list from normal agent input; in
                // `!` shell mode the `!` prefix is not part of `plain_text`, so
                // an empty shell command would otherwise trip this branch and
                // open the picker while the input stayed shell-mode.
                if matches!(*command, TuiEditorCommand::MoveLeft)
                    && !self.is_shell_mode(ctx)
                    && self.plain_text(ctx).is_empty()
                    && self.is_cursor_at_start(ctx)
                {
                    if let Some(menu) = self
                        .inline_menus
                        .iter()
                        .find(|menu| menu.mode() == TuiInputSuggestionsMode::ConversationMenu)
                    {
                        menu.open(ctx);
                    }
                    TuiEditorInteractionOutcome::FollowCursor
                // With nothing left to delete, backspace removes the `!`
                // affordance instead; typed text is preserved.
                } else if matches!(*command, TuiEditorCommand::Backspace)
                    && self.is_shell_mode(ctx)
                    && self.is_cursor_at_start(ctx)
                {
                    self.exit_shell_mode(ctx);
                    TuiEditorInteractionOutcome::FollowCursor
                // When the input is empty, ↑ recalls the last submitted message
                // for re-editing — the shell-history affordance shown in the
                // Figma footer as "↑ to edit". Non-empty buffers get the normal
                // cursor-up behavior unchanged.
                } else if matches!(*command, TuiEditorCommand::MoveUp) && self.is_empty(ctx) {
                    if let Some(recalled) = self.last_submitted_text.clone() {
                        self.set_text(&recalled, ctx);
                    }
                    TuiEditorInteractionOutcome::FollowCursor
                } else {
                    self.editor_state.apply_command(
                        &self.model,
                        *command,
                        self.editor_behavior,
                        ctx,
                    )
                }
            }
            TuiInputAction::SetCursor { offset } => {
                self.model.update(ctx, |m, ctx| {
                    m.select_at(*offset, false, ctx);
                    m.end_selection(ctx);
                });
                TuiEditorInteractionOutcome::FollowCursor
            }
        };
        if outcome == TuiEditorInteractionOutcome::FollowCursor {
            self.follow_cursor(ctx);
        }
        ctx.notify();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// View-level TUI helpers
// ─────────────────────────────────────────────────────────────────────────────

impl TuiInputView {
    // ── Read helpers ──────────────────────────────────────────────────────────

    fn plain_text(&self, ctx: &AppContext) -> String {
        let inner = self.model.as_ref(ctx);
        let buffer = inner.content().as_ref(ctx);
        if buffer.is_empty() {
            return String::new();
        }
        buffer.text().into_string()
    }

    fn cursor_offset(&self, ctx: &AppContext) -> CharOffset {
        self.model
            .as_ref(ctx)
            .selection_model()
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// The selection as a 1-based gap range, or `None` when the selection is
    /// empty. Rendering reads the selection through the editor element; this
    /// backs cursor-position checks (e.g. shell-mode entry) and tests.
    fn selection_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let inner = self.model.as_ref(ctx);
        let sel = inner.buffer_selection_model().as_ref(ctx);
        let head = sel.first_selection_head();
        let tail = sel.first_selection_tail();
        if head == tail {
            None
        } else {
            let start = head.min(tail);
            let end = head.max(tail);
            Some(start..end)
        }
    }

    /// Whether the cursor sits at the very start of the buffer with no active
    /// selection (the position where `!` toggles shell mode).
    fn is_cursor_at_start(&self, ctx: &AppContext) -> bool {
        self.cursor_offset(ctx).as_usize() <= 1 && self.selection_range(ctx).is_none()
    }

    // ── Scroll ─────────────────────────────────────────────────────────────
    //
    // The scroll offset and its clamping/follow policy live on the char-cell
    // render state (`CharCellState`); these helpers gather the inputs the
    // mechanism needs — the primary cursor and the model-derived hidden line
    // ranges — and apply the input's viewport policy.

    /// Scrolls the viewport the minimal amount needed to keep the cursor
    /// visible.
    fn follow_cursor(&self, ctx: &AppContext) {
        follow_editor_cursor(&self.model, self.editor_behavior, ctx);
    }

    // ── Shell mode ────────────────────────────────────────────────────────────

    /// Locks the shared input mode to shell with the `!` shell-prefix source.
    fn enter_shell_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let is_input_buffer_empty = self.plain_text(ctx).is_empty();
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                SHELL_LOCKED_CONFIG,
                is_input_buffer_empty,
                Some(InputTypeAutoDetectionSource::ShellPrefix),
                ctx,
            );
        });
    }

    /// Explicitly forces agent mode for the current buffer; any typed text is
    /// preserved. Clearing or submitting the buffer resumes setting-derived
    /// autodetection.
    pub(crate) fn exit_shell_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let is_input_buffer_empty = self.plain_text(ctx).is_empty();
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(AI_LOCKED_CONFIG, is_input_buffer_empty, None, ctx);
        });
    }

    fn reset_to_default_agent_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let is_autodetection_enabled = self
            .input_mode
            .as_ref(ctx)
            .is_autodetection_enabled_for_current_context(ctx);
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            if is_autodetection_enabled {
                input_mode.enable_autodetection(InputType::AI, ctx);
            } else {
                input_mode.set_input_config(AI_LOCKED_CONFIG, true, None, ctx);
            }
        });
    }

    // ── Submit ────────────────────────────────────────────────────────────────

    /// Emits [`TuiInputViewEvent::Submitted`] without clearing the buffer; the
    /// owner decides whether the submission is accepted and calls [`Self::clear`].
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.plain_text(ctx);
        if !text.trim().is_empty() {
            self.last_submitted_text = Some(text.clone());
        }
        ctx.emit(TuiInputViewEvent::Submitted(text));
    }

    fn handle_inline_menu_action(
        &mut self,
        action: &TuiInputAction,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !matches!(
            action,
            TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp | TuiEditorCommand::MoveDown)
                | TuiInputAction::Submit
                | TuiInputAction::HandleEscape
        ) {
            return false;
        }
        let Some(inline_menu) = self.active_inline_menu(ctx) else {
            return false;
        };

        match action {
            TuiInputAction::EditorCommand(TuiEditorCommand::MoveUp) => {
                inline_menu.select_previous(ctx);
            }
            TuiInputAction::EditorCommand(TuiEditorCommand::MoveDown) => {
                inline_menu.select_next(ctx);
            }
            TuiInputAction::Submit => {
                if let Some(accepted) = inline_menu.accept(ctx) {
                    match accepted {
                        TuiInlineMenuAccepted::SlashCommand(action) => {
                            ctx.emit(TuiInputViewEvent::AcceptedSlashCommand(action));
                        }
                        TuiInlineMenuAccepted::Conversation(entry_id) => {
                            ctx.emit(TuiInputViewEvent::AcceptedConversation(entry_id));
                        }
                        TuiInlineMenuAccepted::Model(id) => {
                            ctx.emit(TuiInputViewEvent::AcceptedModel(id));
                        }
                        TuiInlineMenuAccepted::Mcp(action) => {
                            ctx.emit(TuiInputViewEvent::AcceptedMcp(action));
                        }
                    }
                }
            }
            TuiInputAction::HandleEscape => return self.handle_escape(ctx),
            _ => return false,
        }
        ctx.notify();
        true
    }

    /// Handles the input's contextual Escape behavior in explicit priority
    /// order. New input modes should be added after the inline-menu branch so
    /// one Escape always closes the most local surface first.
    fn handle_escape(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if let Some(inline_menu) = self.active_inline_menu(ctx) {
            inline_menu.dismiss(ctx);
            ctx.notify();
            return true;
        }

        if self.is_shell_mode(ctx) {
            self.exit_shell_mode(ctx);
            return true;
        }
        false
    }

    fn active_inline_menu(&self, ctx: &AppContext) -> Option<TuiInlineMenu> {
        active_inline_menu(
            &self.inline_menus,
            self.suggestions_mode.as_ref(ctx).mode(),
            ctx,
        )
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
