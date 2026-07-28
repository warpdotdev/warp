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

use std::ops::Range;
use std::rc::Rc;

use string_offset::{ByteOffset, CharOffset};
use vim::vim::{InsertPosition, VimMode};
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::AppEditorSettings;
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, BlocklistAIInputModel, InputType,
    InputTypeAutoDetectionSource, LLMId, TuiMcpAction,
};
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiText};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding};
use warpui_core::text::{byte_offset_for_char_offset, count_chars_up_to_byte};
use warpui_core::{
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, TuiView, TypedActionView,
    ViewContext,
};

use crate::completion_menu::TuiCompletionAcceptance;
use crate::editor_element::{TuiEditorAction, TuiEditorElement, TuiEditorStyles};
use crate::editor_interaction::{
    TuiEditorBehavior, TuiEditorCommand, TuiEditorInteractionOutcome, TuiEditorState,
    apply_editor_action, apply_editor_clipboard_action, follow_editor_cursor,
};
use crate::inline_menu::{TuiInlineMenu, TuiInlineMenuAccepted, active_inline_menu};
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::{
    KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG, PLAN_TOGGLE_AVAILABLE_FLAG, TUI_BINDING_GROUP,
};
use crate::terminal_session_view::state::TuiTerminalSessionStateModel;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_vim_input::{TuiVimAction, TuiVimInputModel};
use crate::voice_input::{TuiVoiceInputModel, TuiVoiceInputState, VoiceInputStartSource};

/// Keymap-context flag set while the input has contextual Escape behavior.
///
/// The input owns a single Escape binding so modes can arbitrate explicitly in
/// [`TuiInputView::handle_escape`] instead of relying on keymap registration
/// order. Inline menus take priority; later input modes should be handled only
/// after the menu branch.
const INPUT_HANDLES_ESCAPE_FLAG: &str = "TuiInputHandlesEscape";
const SHELL_COMPLETION_AVAILABLE_FLAG: &str = "TuiShellCompletionAvailable";
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
        EditableBinding::new(
            "tui:input:complete_shell_command",
            "Complete the shell command",
            TuiInputAction::Complete,
        )
        .with_context_predicate(id!("TuiInputView") & id!(SHELL_COMPLETION_AVAILABLE_FLAG))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("tab"),
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
    /// The terminal delivered one complete bracketed-paste payload.
    Pasted(String),
    /// Backspace was pressed at the start of an empty agent input. Empty shell
    /// input consumes Backspace to exit shell mode instead.
    BackspaceAtEmptyInput,
    /// The user selected a slash command menu item.
    AcceptedSlashCommand(AcceptSlashCommandOrSavedPrompt),
    /// The user selected a conversation menu item.
    AcceptedConversation(warp::tui_export::AgentConversationEntryId),
    /// The user selected a model menu item.
    AcceptedModel(LLMId),
    /// The user selected an action from the MCP menu.
    AcceptedMcp(TuiMcpAction),
    /// Shift+Up should move focus from the first visual row to the region above.
    MoveFocusUp,
    /// The user accepted a prompt from the up-arrow prompt-history menu. Carries
    /// the prompt text to fill into the input and submit.
    AcceptedPromptHistory(String),
    /// Tab requested shell completion for the current input snapshot.
    RequestShellCompletion,
    /// Selected prompt text was copied to the host clipboard.
    ClipboardCopySucceeded,
    /// Selected prompt text could not be copied to the host clipboard.
    ClipboardCopyFailed,
    /// The vim mode changed (Insert↔Normal↔Visual↔Replace). Emitted so the
    /// parent session view can re-render its footer vim-mode indicator.
    VimModeChanged,
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed action enum
// ─────────────────────────────────────────────────────────────────────────────

/// Prompt policy plus shared editor actions dispatched to [`TuiInputView`].
///
/// Each variant corresponds to one or more keybindings.
#[derive(Debug, Clone)]
pub enum TuiInputAction {
    /// Apply input emitted by the shared editor element.
    Editor(TuiEditorAction),
    /// Submit the current input (`Enter`).
    Submit,
    /// Handle contextual input Escape behavior, prioritizing an open inline menu.
    HandleEscape,
    /// Request or advance shell-command completion.
    Complete,
    /// Apply an editing command shared with generic TUI editors.
    EditorCommand(TuiEditorCommand),
    /// Place the cursor at `offset` without starting a drag selection
    /// (the prompt gutter click).
    SetCursor { offset: CharOffset },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiCompletionInputSnapshot {
    pub(crate) buffer_text: String,
    pub(crate) cursor_byte_offset: usize,
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
    /// Mouse state for the input prompt gutter; created once here (not inline
    /// during render) so mouse tracking survives per-frame element rebuilds.
    prefix_mouse_state: MouseStateHandle,
    /// Whether this view is focused, tracked via `on_focus`/`on_blur` like
    /// the GUI's `EditorView::focused`. Snapshotted into the editor element
    /// so it only consumes typed text while the input is focused.
    focused: bool,
    /// Session-owned source for hints and additive capabilities.
    session_state: ModelHandle<TuiTerminalSessionStateModel>,
    keyboard_enhancement_supported: bool,
    /// Consults the owner live before an inline-menu Enter can accept an item.
    can_accept_inline_menu: Rc<dyn Fn(&AppContext) -> bool>,
    /// TUI voice state used for Escape routing and shell-gutter suppression.
    voice_input: ModelHandle<TuiVoiceInputModel>,
    /// Vim-mode state machine. Always present but only active when
    /// `AppEditorSettings::vim_mode_enabled()` returns `true`.
    vim: TuiVimInputModel,
    /// The cursor offset at which visual mode was entered, used to track the
    /// visual selection range. `None` when not in visual mode.
    visual_selection_anchor: Option<CharOffset>,
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
        session_state: ModelHandle<TuiTerminalSessionStateModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_internal(
            model,
            input_mode,
            suggestions_mode,
            inline_menus,
            session_state,
            ctx,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        model: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        inline_menus: Vec<TuiInlineMenu>,
        orchestration_tabs_available: impl Fn(&AppContext) -> bool + 'static,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let session_state = ctx.add_model(|_| {
            TuiTerminalSessionStateModel::new_for_input(
                &input_mode,
                &suggestions_mode,
                orchestration_tabs_available,
            )
        });
        Self::new_internal(
            model,
            input_mode,
            suggestions_mode,
            inline_menus,
            session_state,
            ctx,
        )
    }

    fn new_internal(
        model: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        inline_menus: Vec<TuiInlineMenu>,
        session_state: ModelHandle<TuiTerminalSessionStateModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let voice_input = ctx.add_model(TuiVoiceInputModel::new);
        ctx.subscribe_to_model(&model, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                ctx.notify();
            }
        });
        // The model only emits on real config changes, and rendering branches
        // on the config (shell-mode gutter/border), so every event re-renders.
        ctx.subscribe_to_model(&input_mode, |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&suggestions_mode, |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&voice_input, |_, _, _, ctx| ctx.notify());
        Self {
            model,
            input_mode,
            suggestions_mode,
            inline_menus,
            editor_state: TuiEditorState::default(),
            editor_behavior: TuiEditorBehavior::multiline(6).with_copy_on_mouse_highlight(),
            prefix_mouse_state: MouseStateHandle::default(),
            focused: false,
            session_state,
            keyboard_enhancement_supported: false,
            can_accept_inline_menu: Rc::new(|_| true),
            voice_input,
            vim: TuiVimInputModel::new(),
            visual_selection_anchor: None,
        }
    }

    pub(crate) fn with_inline_menu_actions_allowed(
        mut self,
        can_accept_inline_menu: impl Fn(&AppContext) -> bool + 'static,
    ) -> Self {
        self.can_accept_inline_menu = Rc::new(can_accept_inline_menu);
        self
    }

    pub(crate) fn with_keyboard_enhancement_supported(
        mut self,
        keyboard_enhancement_supported: bool,
    ) -> Self {
        self.keyboard_enhancement_supported = keyboard_enhancement_supported;
        self
    }

    fn plan_toggle_available(&self, ctx: &AppContext) -> bool {
        self.session_state
            .as_ref(ctx)
            .resolve(ctx)
            .is_ok_and(|state| state.plan_available())
    }
    /// Whether vim mode is enabled in settings.
    ///
    /// Returns `false` when [`AppEditorSettings`] has not been registered in
    /// the context (e.g. in lightweight test fixtures that don't boot the full
    /// settings stack).
    pub(crate) fn vim_mode_enabled(&self, ctx: &AppContext) -> bool {
        ctx.has_singleton_model::<AppEditorSettings>()
            && AppEditorSettings::as_ref(ctx).vim_mode_enabled()
    }

    /// Reset the vim state machine to insert mode. Called when vim mode is
    /// enabled (so the user starts in insert mode, not whatever mode they
    /// were in previously).
    pub(crate) fn reset_vim_to_insert(&mut self) {
        self.vim.reset_to_insert();
    }

    /// The current vim mode, or `None` when vim mode is disabled.
    pub(crate) fn vim_mode(&self, ctx: &AppContext) -> Option<VimMode> {
        if self.vim_mode_enabled(ctx) {
            Some(self.vim.mode())
        } else {
            None
        }
    }

    /// Whether the input is in detected or explicitly locked shell mode.
    pub(crate) fn is_shell_mode(&self, ctx: &AppContext) -> bool {
        input_mode_policy::is_shell_mode(self.input_mode.as_ref(ctx))
    }

    pub(crate) fn voice_is_active(&self, ctx: &AppContext) -> bool {
        self.voice_input.as_ref(ctx).is_active()
    }

    pub(crate) fn voice_input_model(&self) -> &ModelHandle<TuiVoiceInputModel> {
        &self.voice_input
    }

    pub(crate) fn voice_state(&self, ctx: &AppContext) -> TuiVoiceInputState {
        self.voice_input.as_ref(ctx).state()
    }

    pub(crate) fn voice_animation_clock(&self, ctx: &AppContext) -> AnimationClock {
        self.voice_input.as_ref(ctx).animation_clock()
    }

    pub(crate) fn start_voice_input(
        &mut self,
        available: bool,
        source: VoiceInputStartSource,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        self.voice_input.update(ctx, |voice_input, ctx| {
            voice_input.start(available, source, ctx)
        })
    }

    pub(crate) fn stop_voice_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.voice_input
            .update(ctx, |voice_input, ctx| voice_input.stop(ctx));
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

    /// Inserts normalized text at the current cursor without submitting it.
    pub(crate) fn insert_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let text = self.editor_behavior.normalize_text(text);
        if !text.is_empty() {
            self.model.update(ctx, |m, ctx| m.user_insert(text, ctx));
            self.follow_cursor(ctx);
            ctx.notify();
        }
    }

    /// Builds this frame's core editor element: editable, scroll-windowed, and
    /// dispatching [`TuiEditorAction`]s back as [`TuiInputAction`]s. `render`
    /// boxes it (behind the mode-specific prompt gutter when active); tests
    /// construct it directly to exercise mouse dispatch.
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
        // Empty-buffer placeholder hints depend on state that changes without
        // this view re-rendering (transcript emptiness flips when blocks land
        // via history events or PTY wakeups), so the hint is resolved by a
        // provider on every layout pass instead of being snapshotted here.
        // Shell mode teaches how to exit; agent mode adapts to the transcript
        // state.
        let session_state = self.session_state.clone();
        element.with_placeholder_ghost_text(move |app| {
            session_state
                .as_ref(app)
                .resolve(app)
                .ok()
                .and_then(|state| state.hint_text())
                .map(|hint| (hint, TuiUiBuilder::from_app(app).muted_text_style()))
        })
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

    pub(crate) fn insert_typeahead_text(
        &mut self,
        previously_inserted: CharOffset,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            model.replace_first_n_characters(previously_inserted, text, ctx);
            let end = model.content().as_ref(ctx).max_charoffset();
            model.cursor_at(end, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Inserts a paste payload after the parent declines to consume it as
    /// structured input.
    pub(crate) fn insert_pasted_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        apply_editor_action(
            &self.model,
            &TuiEditorAction::PasteText(text.to_owned()),
            self.editor_behavior,
            ctx,
        );
        self.follow_cursor(ctx);
        ctx.notify();
    }
}

impl TuiView for TuiInputView {
    fn ui_name() -> &'static str {
        "TuiInputView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        if self.voice_is_active(ctx) {
            return self.render_input(ctx);
        }
        let (prefix, prefix_style) = if self.is_shell_mode(ctx) {
            ("!", builder.shell_command_accent_style())
        } else {
            (">", builder.accent_text_style())
        };
        let prefix = TuiHoverable::new(
            self.prefix_mouse_state.clone(),
            TuiContainer::new(TuiText::new(prefix).with_style(prefix_style).finish())
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

    fn keymap_context(&self, ctx: &AppContext) -> keymap::Context {
        let suggestions_mode = self.suggestions_mode.as_ref(ctx).mode();
        // In vim mode, escape is handled only when vim actually needs it:
        // - Non-Normal modes (Insert→Normal, Visual→Normal, Replace→Normal)
        // - Normal mode with pending input (clear the partial command)
        // In Normal mode with no pending input, escape is a no-op for vim;
        // passing it through allows session-level bindings (e.g.
        // orchestration focus-main, cancel-restore) to fire instead.
        let vim_mode_enabled = self.vim_mode_enabled(ctx);
        input_keymap_context(InputKeymapContextConfig {
            input_handles_escape: self.active_inline_menu(ctx).is_some()
                || matches!(suggestions_mode, TuiInputSuggestionsMode::Shortcuts)
                || self.is_shell_mode(ctx)
                || self.voice_is_active(ctx)
                || (vim_mode_enabled
                    && (!matches!(self.vim.mode(), VimMode::Normal) || self.vim.has_pending())),
            plan_toggle_available: self.plan_toggle_available(ctx),
            keyboard_enhancement_supported: self.keyboard_enhancement_supported,
            shell_completion_available: self.is_shell_mode(ctx),
        })
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

#[derive(Clone, Copy, Debug, Default)]
struct InputKeymapContextConfig {
    input_handles_escape: bool,
    plan_toggle_available: bool,
    keyboard_enhancement_supported: bool,
    shell_completion_available: bool,
}

fn input_keymap_context(config: InputKeymapContextConfig) -> keymap::Context {
    let mut context = keymap::Context::default();
    context.set.insert(TuiInputView::ui_name());
    if config.input_handles_escape {
        context.set.insert(INPUT_HANDLES_ESCAPE_FLAG);
    }
    if config.plan_toggle_available {
        context.set.insert(PLAN_TOGGLE_AVAILABLE_FLAG);
    }
    if config.keyboard_enhancement_supported {
        context.set.insert(KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG);
    }
    if config.shell_completion_available {
        context.set.insert(SHELL_COMPLETION_AVAILABLE_FLAG);
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
                if let TuiEditorAction::PasteText(text) = editor_action {
                    self.close_shortcuts(ctx);
                    ctx.emit(TuiInputViewEvent::Pasted(text.clone()));
                    return;
                }
                if self.close_shortcuts(ctx) {
                    if matches!(editor_action, TuiEditorAction::InsertChar('?')) {
                        return;
                    }
                } else if matches!(editor_action, TuiEditorAction::InsertChar('?'))
                    && self.plain_text(ctx).is_empty()
                    && self.is_cursor_at_start(ctx)
                    && matches!(
                        self.suggestions_mode.as_ref(ctx).mode(),
                        TuiInputSuggestionsMode::Closed
                    )
                {
                    self.suggestions_mode.update(ctx, |mode, ctx| {
                        mode.set_mode(TuiInputSuggestionsMode::Shortcuts, ctx);
                    });
                    return;
                }
                // Route through vim FSA when vim mode is enabled and the
                // FSA is NOT in Insert mode. In Insert mode the character
                // falls through to normal editor handling so that `!` at
                // the start still enters shell mode regardless of vim mode.
                if let TuiEditorAction::InsertChar(c) = *editor_action
                    && self.vim_mode_enabled(ctx)
                    && !matches!(self.vim.mode(), VimMode::Insert)
                {
                    // Capture mode BEFORE the FSA advances so apply_vim_action can
                    // detect a mode transition and emit VimModeChanged.
                    let prev_mode = self.vim.mode();
                    let vim_action = self.vim.process_char(c);
                    return self.apply_vim_action(vim_action, prev_mode, ctx);
                }
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
                self.close_shortcuts(ctx);
                // In vim normal/visual/replace mode, Enter still submits so the
                // prompt behaves like a command line (same as bash/zsh vi-mode).
                if !self.handle_voice_submit(ctx) {
                    self.submit(ctx);
                }
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiInputAction::HandleEscape => {
                self.handle_escape(ctx);
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiInputAction::Complete => {
                if self.is_shell_mode(ctx) {
                    ctx.emit(TuiInputViewEvent::RequestShellCompletion);
                }
                TuiEditorInteractionOutcome::PreserveViewport
            }
            TuiInputAction::EditorCommand(command) => {
                self.close_shortcuts(ctx);
                if matches!(*command, TuiEditorCommand::SelectUp) && self.can_focus_above(ctx) {
                    ctx.emit(TuiInputViewEvent::MoveFocusUp);
                    return;
                }
                // In vim normal/visual mode, backspace is a leftward motion.
                if matches!(*command, TuiEditorCommand::Backspace)
                    && self.vim_mode_enabled(ctx)
                    && !matches!(self.vim.mode(), VimMode::Insert)
                {
                    let prev_mode = self.vim.mode();
                    let vim_action = self.vim.process_special_key("backspace");
                    return self.apply_vim_action(vim_action, prev_mode, ctx);
                }
                // Only open the conversation list from normal agent input; in
                // `!` shell mode the `!` prefix is not part of `plain_text`, so
                // an empty shell command would otherwise trip this branch and
                // open the picker while the input stayed shell-mode.
                if matches!(*command, TuiEditorCommand::MoveLeft)
                    && !self.is_shell_mode(ctx)
                    && self.plain_text(ctx).is_empty()
                    && self.is_cursor_at_start(ctx)
                {
                    self.open_inline_menu(TuiInputSuggestionsMode::ConversationMenu, ctx);
                    TuiEditorInteractionOutcome::FollowCursor
                } else if matches!(*command, TuiEditorCommand::MoveUp)
                    && !self.is_shell_mode(ctx)
                    && self.single_cursor_on_first_row(ctx)
                {
                    self.open_inline_menu(TuiInputSuggestionsMode::PromptHistory, ctx);
                    TuiEditorInteractionOutcome::FollowCursor
                // With nothing left to delete, backspace removes the `!`
                // affordance instead; typed text is preserved.
                } else if matches!(*command, TuiEditorCommand::Backspace)
                    && self.is_shell_mode(ctx)
                    && self.is_cursor_at_start(ctx)
                {
                    self.exit_shell_mode(ctx);
                    TuiEditorInteractionOutcome::FollowCursor
                } else if matches!(*command, TuiEditorCommand::Backspace)
                    && self.plain_text(ctx).is_empty()
                    && self.is_cursor_at_start(ctx)
                {
                    ctx.emit(TuiInputViewEvent::BackspaceAtEmptyInput);
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
                // Clicking in the input switches to insert mode in vim.
                if self.vim_mode_enabled(ctx) && !matches!(self.vim.mode(), VimMode::Insert) {
                    self.vim.force_insert_mode();
                }
                self.close_shortcuts(ctx);
                self.model.update(ctx, |m, ctx| {
                    m.select_at(*offset, false, ctx);
                    m.end_selection(ctx);
                });
                TuiEditorInteractionOutcome::FollowCursor
            }
        };
        let outcome = match outcome {
            TuiEditorInteractionOutcome::Clipboard(action) => {
                match apply_editor_clipboard_action(&self.model, action, ctx) {
                    Ok(true) => ctx.emit(TuiInputViewEvent::ClipboardCopySucceeded),
                    Ok(false) => {}
                    Err(error) => {
                        log::error!("Failed to copy TUI input selection: {error}");
                        ctx.emit(TuiInputViewEvent::ClipboardCopyFailed);
                    }
                }
                TuiEditorInteractionOutcome::FollowCursor
            }
            outcome => outcome,
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
    fn open_inline_menu(&self, mode: TuiInputSuggestionsMode, ctx: &mut ViewContext<Self>) {
        if let Some(menu) = self.inline_menus.iter().find(|menu| menu.mode() == mode) {
            menu.open(ctx);
        }
    }

    fn plain_text(&self, ctx: &AppContext) -> String {
        let inner = self.model.as_ref(ctx);
        let buffer = inner.content().as_ref(ctx);
        if buffer.is_empty() {
            return String::new();
        }
        buffer.text().into_string()
    }

    pub(crate) fn completion_snapshot(
        &self,
        ctx: &AppContext,
    ) -> Option<TuiCompletionInputSnapshot> {
        if !self.is_shell_mode(ctx) || !self.model.as_ref(ctx).selection_is_single_cursor(ctx) {
            return None;
        }
        let buffer_text = self.plain_text(ctx);
        let cursor_char_offset = self
            .model
            .as_ref(ctx)
            .buffer_selection_model()
            .as_ref(ctx)
            .first_selection_head()
            .as_usize()
            .saturating_sub(1);
        let cursor_byte_offset =
            byte_offset_for_char_offset(&buffer_text, CharOffset::from(cursor_char_offset))?
                .as_usize();
        Some(TuiCompletionInputSnapshot {
            buffer_text,
            cursor_byte_offset,
        })
    }

    pub(crate) fn apply_shell_completion(
        &mut self,
        acceptance: TuiCompletionAcceptance,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let buffer_text = self.plain_text(ctx);
        let replacement_range = acceptance.replacement_range;
        if replacement_range.start > replacement_range.end {
            return false;
        }
        let Some(replacement_start) =
            count_chars_up_to_byte(&buffer_text, ByteOffset::from(replacement_range.start))
        else {
            return false;
        };
        let Some(replacement_end) =
            count_chars_up_to_byte(&buffer_text, ByteOffset::from(replacement_range.end))
        else {
            return false;
        };
        let selection_range = replacement_start + 1..replacement_end + 1;
        let mut replacement = acceptance.replacement;
        if acceptance.append_space {
            replacement.push(' ');
        }
        self.model.update(ctx, |model, ctx| {
            model.select_at(selection_range.start, false, ctx);
            model.set_last_selection_head(selection_range.end, ctx);
            model.end_selection(ctx);
            model.user_insert(&replacement, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
        true
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

    /// Whether Shift+Up should leave the input instead of extending selection.
    fn can_focus_above(&self, ctx: &AppContext) -> bool {
        self.session_state
            .as_ref(ctx)
            .resolve(ctx)
            .is_ok_and(|state| state.orchestration_available())
            && self.single_cursor_on_first_row(ctx)
    }

    /// Whether the single caret sits on the first visual row of the input with
    /// no active selection — the position where Up opens the prompt-history
    /// menu. Accounts for soft-wrapping via the char-cell display lattice,
    /// mirroring the GUI editor view's `single_cursor_on_first_row`.
    fn single_cursor_on_first_row(&self, ctx: &AppContext) -> bool {
        if self.selection_range(ctx).is_some() {
            return false;
        }

        let model = self.model.as_ref(ctx);
        let render = model.render_state().as_ref(ctx);
        let Some(char_cell) = render.char_cell() else {
            return false;
        };

        let cursor_offset = CharOffset::from(self.cursor_offset(ctx).as_usize().saturating_sub(1));
        let hidden = char_cell.hidden_line_ranges(ctx);
        char_cell
            .display_lattice(&hidden)
            .offset_to_display_point(cursor_offset)
            .is_some_and(|point| point.row == 0)
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
            input_mode.set_input_config(
                AI_LOCKED_CONFIG,
                is_input_buffer_empty,
                Some(InputTypeAutoDetectionSource::ManualToggle),
                ctx,
            );
        });
    }

    /// Locks the input to Agent while a CLI subagent owns terminal control. The
    /// `AgentTerminalControl` autodetection source marks the lock as
    /// agent-installed so the post-agent reset can distinguish it from a
    /// user-forced `ManualToggle` lock.
    pub(crate) fn lock_for_agent_control(&mut self, ctx: &mut ViewContext<Self>) {
        let is_input_buffer_empty = self.plain_text(ctx).is_empty();
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(
                AI_LOCKED_CONFIG,
                is_input_buffer_empty,
                Some(InputTypeAutoDetectionSource::AgentTerminalControl),
                ctx,
            );
        });
    }

    /// Restores the setting-derived agent-first mode while preserving the
    /// current input buffer.
    pub(crate) fn reset_to_default_agent_mode(&mut self, ctx: &mut ViewContext<Self>) {
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

    /// Restores the setting-derived mode only when the current AI lock was
    /// installed for agent terminal control. Reuses the shared model's
    /// `last_ai_autodetection_source` rather than a parallel bool: an explicit
    /// user lock carries a `ManualToggle` (or `ShellPrefix`) source, so it is
    /// left untouched.
    pub(crate) fn reset_after_agent_control(&mut self, ctx: &mut ViewContext<Self>) {
        let is_agent_controlled = self.input_mode.as_ref(ctx).last_ai_autodetection_source()
            == Some(InputTypeAutoDetectionSource::AgentTerminalControl);
        if !is_agent_controlled {
            return;
        }
        self.reset_to_default_agent_mode(ctx);
    }

    // ── Submit ────────────────────────────────────────────────────────────────

    /// Emits [`TuiInputViewEvent::Submitted`] without clearing the buffer; the
    /// owner decides whether the submission is accepted and calls [`Self::clear`].
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.plain_text(ctx);
        ctx.emit(TuiInputViewEvent::Submitted(text));
    }

    fn handle_voice_submit(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        match self.voice_input.as_ref(ctx).state() {
            TuiVoiceInputState::Listening => {
                self.voice_input
                    .update(ctx, |voice_input, ctx| voice_input.stop(ctx));
                true
            }
            TuiVoiceInputState::Transcribing => true,
            TuiVoiceInputState::Idle => false,
        }
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
                | TuiInputAction::Complete
        ) {
            return false;
        }
        let Some(inline_menu) = self.active_inline_menu(ctx) else {
            return false;
        };
        if matches!(action, TuiInputAction::Submit) && !(self.can_accept_inline_menu)(ctx) {
            // The session can render a disabled editor while the shell is still
            // bootstrapping. Consume Enter without accepting a hidden menu item;
            // otherwise the accepted-menu event bypasses the session's normal
            // submission guard and can execute or clear the draft.
            return true;
        }
        if matches!(action, TuiInputAction::Complete) {
            if inline_menu.mode() == TuiInputSuggestionsMode::CompletionSuggestions {
                inline_menu.select_next(ctx);
                ctx.notify();
            }
            return true;
        }

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
                        TuiInlineMenuAccepted::PromptHistory(text) => {
                            ctx.emit(TuiInputViewEvent::AcceptedPromptHistory(text));
                        }
                        TuiInlineMenuAccepted::Completion(acceptance) => {
                            self.apply_shell_completion(acceptance, ctx);
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
        if self.close_shortcuts(ctx) {
            return true;
        }
        if let Some(inline_menu) = self.active_inline_menu(ctx) {
            inline_menu.dismiss(ctx);
            ctx.notify();
            return true;
        }

        match self.voice_input.as_ref(ctx).state() {
            TuiVoiceInputState::Listening => {
                self.voice_input
                    .update(ctx, |voice_input, ctx| voice_input.stop(ctx));
                return true;
            }
            TuiVoiceInputState::Transcribing => {
                self.voice_input
                    .update(ctx, |voice_input, ctx| voice_input.cancel(ctx));
                return true;
            }
            TuiVoiceInputState::Idle => {}
        }

        // In vim mode, Escape transitions between modes (Insert→Normal,
        // Visual/Replace→Normal, Normal→clear pending). This takes priority
        // over shell-mode exit so that `<Esc>` is always a vim command first.
        // Exception: when the FSA is already in Normal mode with no pending
        // input, a second Escape should exit shell mode if active (matching
        // bash/zsh vi-mode behaviour where `<Esc><Esc>` exits shell mode).
        if self.vim_mode_enabled(ctx) {
            if matches!(self.vim.mode(), VimMode::Normal)
                && !self.vim.has_pending()
                && self.is_shell_mode(ctx)
            {
                self.exit_shell_mode(ctx);
                return true;
            }
            // Capture mode BEFORE the FSA advances so apply_vim_action can
            // detect the transition and emit VimModeChanged.
            let prev_mode = self.vim.mode();
            let vim_action = self.vim.process_special_key("escape");
            self.apply_vim_action(vim_action, prev_mode, ctx);
            ctx.notify();
            return true;
        }

        if self.is_shell_mode(ctx) {
            self.exit_shell_mode(ctx);
            return true;
        }
        false
    }

    /// Applies a [`TuiVimAction`] — returned by the vim FSA — to the backing
    /// editor model and re-renders.
    ///
    /// `prev_vim_mode` must be captured by the caller **before** it calls the
    /// FSA (`process_char`/`process_special_key`), because the FSA advances its
    /// internal mode as part of that call.  Comparing `self.vim.mode()` inside
    /// this function would always see the post-transition mode and would never
    /// detect a change.
    fn apply_vim_action(
        &mut self,
        action: TuiVimAction,
        prev_vim_mode: VimMode,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            TuiVimAction::InsertChar(c) => {
                // Normal character insert (insert mode or insert-mode char from FSA).
                let c_str = c.to_string();
                self.model.update(ctx, |m, ctx| m.user_insert(&c_str, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::InsertText(text) => {
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::Backspace => {
                self.model.update(ctx, |m, ctx| m.backspace(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::DeleteForward => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::DeleteForward,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::DeleteWordBackward => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::DeleteWordBackward,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::DeleteWordForward => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::DeleteWordForward,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveLeft => {
                self.model.update(ctx, |m, ctx| m.move_left(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveRight => {
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveUp => {
                self.model.update(ctx, |m, ctx| m.move_up(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveDown => {
                self.model.update(ctx, |m, ctx| m.move_down(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveWordLeft => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::MoveWordLeft,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            // Both `w` (start of next word) and `e` (end of current word) map
            // to the single `MoveWordRight` editor command; the TUI model does
            // not yet expose a separate end-of-word cursor stop.
            TuiVimAction::MoveWordRightStart | TuiVimAction::MoveWordRightEnd => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::MoveWordRight,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToLineStart => {
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToLineEnd => {
                self.model.update(ctx, |m, ctx| m.move_to_line_end(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToFirstNonWhitespace => {
                self.model.update(ctx, |m, ctx| {
                    m.vim_move_to_first_nonwhitespace(false, ctx);
                });
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToBufferStart => {
                // `gg` — jump to the start of the buffer. Use paragraph
                // navigation which moves past all content to the very beginning.
                self.model
                    .update(ctx, |m, ctx| m.move_to_paragraph_start(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToBufferEnd => {
                // `G` — jump to the end of the buffer.
                self.model
                    .update(ctx, |m, ctx| m.move_to_paragraph_end(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::KillToLineEnd => {
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                {
                    self.vim.set_yank_buffer(killed);
                }
                self.follow_cursor(ctx);
            }
            TuiVimAction::KillToLineStart => {
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_start(ctx))
                {
                    self.vim.set_yank_buffer(killed);
                }
                self.follow_cursor(ctx);
            }
            TuiVimAction::KillLine => {
                // `dd` — delete the whole current line regardless of cursor column.
                // Move to the start of the visual row first, then kill to the end.
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                {
                    self.vim.set_yank_buffer(killed);
                }
                self.follow_cursor(ctx);
            }
            TuiVimAction::ReplaceChar(c) => {
                // `r<char>` — replace the character at the cursor in-place.
                // `replace_char` atomically replaces without changing the cursor
                // position, matching vim's behaviour of staying on the new char.
                self.model.update(ctx, |m, ctx| m.replace_char(c, 1, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::YankToLineEnd => {
                // `y$` — yank from cursor to end of the line, non-destructively.
                // Read the buffer text directly so the undo stack is untouched;
                // the kill+re-insert approach previously broke `u` by leaving
                // the kill/re-insert pair on the undo stack.
                let cursor = self.cursor_offset(ctx);
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                // cursor is a 1-based gap offset; char index = as_usize() - 1
                let char_idx = cursor.as_usize().saturating_sub(1);
                // Yank to end of the current line (’\n’ is the line separator).
                let yanked: String = buffer_text
                    .chars()
                    .skip(char_idx)
                    .take_while(|&c| c != '\n')
                    .collect();
                if !yanked.is_empty() {
                    self.vim.set_yank_buffer(yanked);
                }
                // No buffer mutation — undo stack is unchanged.
                self.follow_cursor(ctx);
            }
            TuiVimAction::YankWordForward => {
                // `yw` — yank one word forward from the cursor, non-destructively.
                // Uses vim's `w`-motion word boundary: skip the current token
                // (word chars or punctuation) then include trailing whitespace.
                let cursor = self.cursor_offset(ctx);
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                let char_idx = cursor.as_usize().saturating_sub(1);
                let yanked = yank_word_from_offset(&buffer_text, char_idx);
                if !yanked.is_empty() {
                    self.vim.set_yank_buffer(yanked);
                }
                // Non-destructive: no cursor or buffer mutation needed.
            }
            TuiVimAction::YankBuffer => {
                // `yy` / visual `y` — yank the full buffer content.
                let text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    if buffer.is_empty() {
                        String::new()
                    } else {
                        buffer.text().into_string()
                    }
                };
                self.vim.set_yank_buffer(text);
                // Stay in current mode (yank is non-destructive).
            }
            TuiVimAction::PasteAfter(text) => {
                // `p` — paste after cursor.
                // `move_right` is a no-op when the cursor is already on the
                // very last character, so at end-of-line this effectively
                // inserts before the last character rather than after. This
                // edge case is a known limitation of the current editor model
                // API and is acceptable for the TUI prompt.
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::PasteBefore(text) => {
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::Undo => {
                self.model.update(ctx, |m, ctx| m.undo(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::ChangeModeToInsert(position) => {
                // Apply the cursor movement implied by the entry command
                // before handing off to Insert mode.
                match position {
                    InsertPosition::AtCursor => {}
                    InsertPosition::AfterCursor => {
                        self.model.update(ctx, |m, ctx| m.move_right(ctx));
                    }
                    InsertPosition::LineEnd => {
                        self.model.update(ctx, |m, ctx| m.move_to_line_end(ctx));
                    }
                    InsertPosition::LineFirstNonWhitespace => {
                        self.model.update(ctx, |m, ctx| {
                            m.vim_move_to_first_nonwhitespace(false, ctx);
                        });
                    }
                    // `o` / `O` (insert newline below/above) are not meaningful
                    // for TUI single-line prompts; treat as a plain mode switch.
                    InsertPosition::LineAbove | InsertPosition::LineBelow => {}
                }
                // Entering insert mode clears any visual selection anchor.
                self.visual_selection_anchor = None;
                self.follow_cursor(ctx);
            }
            TuiVimAction::ModeTransition => {
                // When entering visual mode, record the cursor position as the
                // visual selection anchor. On any other transition (Escape
                // Visual→Normal, Escape Normal→Normal, etc.), clear the anchor.
                match self.vim.mode() {
                    VimMode::Visual(_) => {
                        self.visual_selection_anchor = Some(self.cursor_offset(ctx));
                    }
                    _ => {
                        self.visual_selection_anchor = None;
                    }
                }
            }
            TuiVimAction::DeleteVisualSelection => {
                // `d`/`c` in visual mode: delete from anchor to current cursor.
                // Vim charwise visual selection is inclusive on both ends, so
                // the character under the cursor is included in the deletion.
                if let Some(anchor) = self.visual_selection_anchor.take() {
                    let cursor = self.cursor_offset(ctx);
                    let (sel_start, sel_end) = if anchor <= cursor {
                        (anchor, cursor)
                    } else {
                        (cursor, anchor)
                    };
                    // Yank [sel_start, sel_end] inclusive — convert 1-based gap
                    // offsets to 0-based char indices and include the cursor char.
                    let yank_text = {
                        let inner = self.model.as_ref(ctx);
                        let buffer = inner.content().as_ref(ctx);
                        let buffer_text = buffer.text().into_string();
                        let start_char = sel_start.as_usize().saturating_sub(1);
                        let end_char = sel_end.as_usize().saturating_sub(1);
                        // Include the character at end_char (inclusive range).
                        buffer_text
                            .chars()
                            .skip(start_char)
                            .take(end_char - start_char + 1)
                            .collect::<String>()
                    };
                    // Establish the inclusive selection in the model: sel_end + 1
                    // because the model selection head is exclusive.
                    self.model.update(ctx, |m, ctx| {
                        m.select_at(sel_start, false, ctx);
                        m.set_last_selection_head(sel_end + 1usize, ctx);
                    });
                    // Delete the selection.
                    self.model.update(ctx, |m, ctx| m.backspace(ctx));
                    if !yank_text.is_empty() {
                        self.vim.set_yank_buffer(yank_text);
                    }
                    self.follow_cursor(ctx);
                }
            }
            TuiVimAction::YankVisualSelection => {
                // `y` in visual mode: yank from anchor to cursor, non-destructively.
                // Vim charwise visual selection is inclusive on both ends.
                if let Some(anchor) = self.visual_selection_anchor.take() {
                    let cursor = self.cursor_offset(ctx);
                    let (sel_start, sel_end) = if anchor <= cursor {
                        (anchor, cursor)
                    } else {
                        (cursor, anchor)
                    };
                    // Extract the selected text from the buffer directly.
                    let buffer_text = {
                        let inner = self.model.as_ref(ctx);
                        let buffer = inner.content().as_ref(ctx);
                        buffer.text().into_string()
                    };
                    let start_char = sel_start.as_usize().saturating_sub(1);
                    let end_char = sel_end.as_usize().saturating_sub(1);
                    // Include the character at end_char (inclusive range).
                    let yank_text: String = buffer_text
                        .chars()
                        .skip(start_char)
                        .take(end_char - start_char + 1)
                        .collect();
                    if !yank_text.is_empty() {
                        self.vim.set_yank_buffer(yank_text);
                    }
                    // Non-destructive: no buffer mutation.
                }
            }
            TuiVimAction::RepeatCount { inner, count } => {
                // Execute the inner action `count` times, passing the same
                // `prev_vim_mode` so that a mode-changing inner action (e.g. a
                // count-prefixed `v` entering visual mode) is detected by the
                // shared emit check at the bottom of this function.
                for _ in 0..count {
                    self.apply_vim_action(*inner.clone(), prev_vim_mode, ctx);
                }
                // Fall through to the shared mode-change emit and ctx.notify()
                // so a mode transition inside a count-prefixed command is never
                // silently skipped.
            }
            // Pending / unhandled — no buffer edit needed.
            TuiVimAction::Pending | TuiVimAction::Unhandled => {}
        }
        // Emit a mode-change notification whenever the vim FSA transitions to a
        // different mode. This lets TuiTerminalSessionView re-render its footer
        // vim-mode indicator (NOR/VIS/REP) without the indicator living in this
        // view's own render tree.
        if self.vim.mode() != prev_vim_mode {
            ctx.emit(TuiInputViewEvent::VimModeChanged);
        }
        ctx.notify();
    }

    fn close_shortcuts(&self, ctx: &mut ViewContext<Self>) -> bool {
        let is_open = matches!(
            self.suggestions_mode.as_ref(ctx).mode(),
            TuiInputSuggestionsMode::Shortcuts
        );
        if is_open {
            self.suggestions_mode.update(ctx, |mode, ctx| {
                mode.close_if_active(TuiInputSuggestionsMode::Shortcuts, ctx);
            });
        }
        is_open
    }
    fn active_inline_menu(&self, ctx: &AppContext) -> Option<TuiInlineMenu> {
        active_inline_menu(
            &self.inline_menus,
            self.suggestions_mode.as_ref(ctx).mode(),
            ctx,
        )
    }
}

/// Compute the text that vim's `yw` (word-forward yank) would capture,
/// starting at character index `char_idx` (0-based) in `text`.
///
/// Matches vim's `w`-motion word definition:
/// - From a word character (alphanumeric/underscore): skip word chars, then
///   include any trailing whitespace.
/// - From punctuation: skip non-word/non-whitespace chars, then include
///   any trailing whitespace.
/// - From whitespace: skip all whitespace.
///
/// The returned string is a non-destructive yank that leaves the buffer
/// untouched, so `u` after `yw` does not delete the yanked text.
fn yank_word_from_offset(text: &str, char_idx: usize) -> String {
    let chars: Vec<char> = text.chars().skip(char_idx).collect();
    if chars.is_empty() {
        return String::new();
    }
    let mut end = 0;
    let first = chars[0];
    if first.is_whitespace() {
        // Starting on whitespace: skip all whitespace.
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    } else if first.is_alphanumeric() || first == '_' {
        // Starting on a word character: skip word chars, then whitespace.
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    } else {
        // Starting on punctuation: skip non-word, non-whitespace chars, then whitespace.
        while end < chars.len()
            && !chars[end].is_alphanumeric()
            && chars[end] != '_'
            && !chars[end].is_whitespace()
        {
            end += 1;
        }
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    }
    chars.into_iter().take(end).collect()
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
