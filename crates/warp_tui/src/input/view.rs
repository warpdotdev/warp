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
use vim::vim::{MotionType, VimMode, VimModel, VimSubscriber as _};
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::AppEditorSettings;
#[cfg(feature = "voice_input")]
use warp::tui_export::UserWorkspaces;
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, BlocklistAIInputModel, InputType,
    InputTypeAutoDetectionSource, LLMId, TuiMcpAction, TuiUpArrowHistoryItemKind,
};
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::elements::MouseStateHandle;
#[cfg(feature = "voice_input")]
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiText};
#[cfg(feature = "voice_input")]
use warpui_core::event::KeyState;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding, FixedBinding, Keystroke};
#[cfg(feature = "voice_input")]
use warpui_core::platform::keyboard::KeyCode;
use warpui_core::text::{byte_offset_for_char_offset, count_chars_up_to_byte};
use warpui_core::{
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, TuiView, TypedActionView,
    ViewContext,
};

use crate::completion_menu::TuiCompletionAcceptance;
use crate::editor_element::{TuiEditorAction, TuiEditorElement, TuiEditorStyles};
use crate::editor_interaction::{
    TuiEditorBehavior, TuiEditorCommand, TuiEditorInteractionOutcome, TuiEditorState,
    apply_editor_action, apply_editor_clipboard_action, apply_editor_paste, follow_editor_cursor,
};
use crate::inline_menu::{
    TuiInlineMenu, TuiInlineMenuAccepted, TuiInlineMenuInputOwnership, active_inline_menu,
};
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::{
    KEYBOARD_ENHANCEMENT_AVAILABLE_FLAG, PLAN_TOGGLE_AVAILABLE_FLAG, TUI_BINDING_GROUP,
};
use crate::mcp_install_flow::TuiMcpInstallFlowAction;
use crate::read_only_menu::TuiReadOnlyMenuKind;
use crate::terminal_session_view::state::TuiTerminalSessionStateModel;
use crate::tui_builder::TuiUiBuilder;
#[cfg(feature = "voice_input")]
use crate::voice_input::{
    TuiVoiceInputEvent, TuiVoiceInputModel, TuiVoiceInputState, VoiceInputStartSource,
};

/// Keymap-context flag set while the input has contextual Escape behavior.
///
/// The input owns a single Escape binding so modes can arbitrate explicitly in
/// [`TuiInputView::handle_escape`] instead of relying on keymap registration
/// order. Inline menus take priority; later input modes should be handled only
/// after the menu branch.
const INPUT_HANDLES_ESCAPE_FLAG: &str = "TuiInputHandlesEscape";
const SHELL_COMPLETION_AVAILABLE_FLAG: &str = "TuiShellCompletionAvailable";
pub(crate) const MCP_MENU_ACTIVE_FLAG: &str = "TuiMcpMenuActive";
pub(crate) const MCP_LOGOUT_BINDING_NAME: &str = "tui:input:mcp_logout";
pub(crate) const INLINE_MENU_CAN_CLEAR_SELECTED_FLAG: &str = "TuiInlineMenuCanClearSelected";
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
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-x",
        TuiInputAction::ClearActiveInlineMenuItem,
        id!("TuiInputView") & id!(INLINE_MENU_CAN_CLEAR_SELECTED_FLAG),
    )
    .with_group(TUI_BINDING_GROUP)]);
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
            MCP_LOGOUT_BINDING_NAME,
            "Log out of the selected MCP server and remove its credentials",
            TuiInputAction::LogOutSelectedMcp,
        )
        .with_context_predicate(id!("TuiInputView") & id!(MCP_MENU_ACTIVE_FLAG))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-r"),
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
    /// Pointer interaction requested focus for the session's current input owner.
    FocusRequested,
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
    AcceptedTeam(warp::tui_export::ServerId),
    /// The user selected an action from the MCP menu.
    AcceptedMcp(TuiMcpAction),
    /// The user advanced the explicit MCP installation flow.
    AcceptedMcpInstall(TuiMcpInstallFlowAction),
    /// Shift+Up should move focus from the first visual row to the region above.
    MoveFocusUp,
    /// The user accepted an item from the up-arrow prompt-and-command history menu.
    AcceptedPromptAndCommandHistory {
        text: String,
        kind: TuiUpArrowHistoryItemKind,
    },
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
    /// Log out of the selected MCP server and remove its stored credentials.
    LogOutSelectedMcp,
    /// Request or advance shell-command completion.
    Complete,
    /// Clear the selected row in an inline menu that supports this action.
    ClearActiveInlineMenuItem,
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
    #[cfg(feature = "voice_input")]
    voice_input: ModelHandle<TuiVoiceInputModel>,
    /// Vim model (shared FSA + event dispatch layer). Always present but only
    /// active when `AppEditorSettings::vim_mode_enabled()` returns `true`.
    /// Wired via `VimSubscriber` to dispatch `VimEvent`s to `VimHandler` impls
    /// on `TuiInputView` (see `input/vim.rs`).
    vim_model: ModelHandle<VimModel>,
    /// Internal yank / delete clipboard for vim operations. Separate from the
    /// OS clipboard so that `p`/`P` work even without clipboard access.
    yank_buffer: String,
    /// Selection shape stored in `yank_buffer`, used to preserve linewise
    /// `dd`/`yy` and Visual-line paste semantics.
    yank_motion_type: MotionType,
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
        #[cfg(feature = "voice_input")]
        let voice_input = {
            let team_context_resolver = UserWorkspaces::team_context_resolver(ctx.handle());
            ctx.add_model(|ctx| {
                TuiVoiceInputModel::new(input_mode.clone(), team_context_resolver, ctx)
            })
        };
        let vim_model = ctx.add_model(|_| VimModel::new());
        // Subscribe to vim events: VimSubscriber blanket impl (TuiInputView: VimHandler)
        // dispatches each VimEvent to the appropriate VimHandler method.
        ctx.subscribe_to_model(&vim_model, Self::handle_vim_event);
        ctx.subscribe_to_model(&model, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                ctx.notify();
            }
        });
        // The model only emits on real config changes, and rendering branches
        // on the config (shell-mode gutter/border), so every event re-renders.
        ctx.subscribe_to_model(&input_mode, |_, _, _, ctx| ctx.notify());
        ctx.subscribe_to_model(&suggestions_mode, |_, _, _, ctx| ctx.notify());
        #[cfg(feature = "voice_input")]
        {
            // Only the voice lifecycle state reaches this view's render (the
            // suppressed shell gutter and the Escape keymap flag). Transcribed text
            // arrives through `insert_text`, and failure or cancellation notices
            // render in the session footer, so neither repaints the input.
            ctx.subscribe_to_model(&voice_input, |_, _, event, ctx| {
                if matches!(event, TuiVoiceInputEvent::StateChanged(_)) {
                    ctx.notify();
                }
            });
        }

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
            #[cfg(feature = "voice_input")]
            voice_input,
            vim_model,
            yank_buffer: String::new(),
            yank_motion_type: MotionType::Charwise,
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
    pub(crate) fn reset_vim_to_insert(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_model
            .update(ctx, |vim, ctx| vim.force_insert_mode(ctx));
        // force_insert_mode bypasses the normal ChangeMode event path; notify
        // the footer indicator manually so it reflects the new Insert state.
        ctx.emit(TuiInputViewEvent::VimModeChanged);
    }

    /// The current vim mode, or `None` when vim mode is disabled.
    pub(crate) fn vim_mode(&self, ctx: &AppContext) -> Option<VimMode> {
        if self.vim_mode_enabled(ctx) {
            Some(self.vim_model.as_ref(ctx).state().mode)
        } else {
            None
        }
    }

    /// Whether the input is in detected or explicitly locked shell mode.
    pub(crate) fn is_shell_mode(&self, ctx: &AppContext) -> bool {
        input_mode_policy::is_shell_mode(self.input_mode.as_ref(ctx))
    }

    #[cfg(feature = "voice_input")]
    pub(crate) fn voice_is_active(&self, ctx: &AppContext) -> bool {
        self.voice_input.as_ref(ctx).is_active()
    }
    #[cfg(feature = "voice_input")]
    pub(crate) fn voice_input_model(&self) -> &ModelHandle<TuiVoiceInputModel> {
        &self.voice_input
    }
    #[cfg(feature = "voice_input")]
    pub(crate) fn voice_state(&self, ctx: &AppContext) -> TuiVoiceInputState {
        self.voice_input.as_ref(ctx).state()
    }

    #[cfg(feature = "voice_input")]
    pub(crate) fn voice_animation_clock(&self, ctx: &AppContext) -> AnimationClock {
        self.voice_input.as_ref(ctx).animation_clock()
    }

    /// The physical modifier holding the current recording open, set only while
    /// a hold-to-talk press started it.
    #[cfg(feature = "voice_input")]
    pub(crate) fn voice_hold_key(&self, ctx: &AppContext) -> Option<KeyCode> {
        self.voice_input.as_ref(ctx).hold_key()
    }

    #[cfg(feature = "voice_input")]
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

    #[cfg(feature = "voice_input")]
    pub(crate) fn stop_voice_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.voice_input
            .update(ctx, |voice_input, ctx| voice_input.stop(ctx));
    }

    #[cfg(feature = "voice_input")]
    pub(crate) fn stop_active_voice_hold(&mut self, ctx: &mut ViewContext<Self>) {
        self.voice_input
            .update(ctx, |voice_input, ctx| voice_input.stop_hold(ctx));
    }

    #[cfg(feature = "voice_input")]
    pub(crate) fn handle_voice_hold_key(
        &mut self,
        key: KeyCode,
        state: KeyState,
        available: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.voice_input.update(ctx, |voice_input, ctx| {
            voice_input.handle_hold_key(key, state, available, ctx);
        });
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
        if self.vim_mode_enabled(ctx) {
            self.reset_vim_to_insert(ctx);
        }
        // The cursor is back at the buffer start, so following it scrolls the
        // viewport back to the top.
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Inserts normalized text at the current cursor without submitting it.
    #[cfg(feature = "voice_input")]
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
        let input_ownership = self.active_inline_menu_input_ownership(ctx);
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
        if let VimMode::Visual(motion_type) = self.vim_model.as_ref(ctx).state().mode {
            let ranges = self
                .model
                .as_ref(ctx)
                .vim_visual_selection_ranges(motion_type, ctx);
            element = element.with_selection_ranges(ranges);
        }
        if input_ownership.is_masked() {
            element = element.masked();
        }
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
        if !input_ownership.inline_menu_owns_input() {
            let session_state = self.session_state.clone();
            element.with_placeholder_ghost_text(move |app| {
                session_state
                    .as_ref(app)
                    .resolve(app)
                    .ok()
                    .and_then(|state| state.hint_text())
                    .map(|hint| (hint, TuiUiBuilder::from_app(app).muted_text_style()))
            })
        } else {
            element
        }
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
        let inline_menu_owns_input = self
            .active_inline_menu_input_ownership(ctx)
            .inline_menu_owns_input();
        #[cfg(feature = "voice_input")]
        if self.voice_is_active(ctx) && !inline_menu_owns_input {
            return self.render_input(ctx);
        }
        let (prefix, prefix_style) = if self.is_shell_mode(ctx) && !inline_menu_owns_input {
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
        let inline_menu_owns_input = self
            .active_inline_menu_input_ownership(ctx)
            .inline_menu_owns_input();
        // In vim mode, escape is handled only when vim actually needs it:
        // - Non-Normal modes (Insert→Normal, Visual→Normal, Replace→Normal)
        // - Normal mode with pending input (clear the partial command)
        // In Normal mode with no pending input, escape is a no-op for vim;
        // passing it through allows session-level bindings (e.g.
        // orchestration focus-main, cancel-restore) to fire instead.
        let vim_mode_enabled = self.vim_mode_enabled(ctx);
        let vim_state = self.vim_model.as_ref(ctx).state();
        input_keymap_context(InputKeymapContextConfig {
            input_handles_escape: self.active_inline_menu(ctx).is_some()
                || suggestions_mode.read_only_menu().is_some()
                || self.is_shell_mode(ctx)
                || {
                    #[cfg(feature = "voice_input")]
                    {
                        self.voice_is_active(ctx)
                    }
                    #[cfg(not(feature = "voice_input"))]
                    {
                        false
                    }
                }
                || (vim_mode_enabled
                    && (!matches!(vim_state.mode, VimMode::Normal)
                        || !vim_state.showcmd.is_empty())),
            mcp_menu_active: suggestions_mode == TuiInputSuggestionsMode::Mcp,
            plan_toggle_available: self.plan_toggle_available(ctx),
            keyboard_enhancement_supported: self.keyboard_enhancement_supported,
            shell_completion_available: self.is_shell_mode(ctx) && !inline_menu_owns_input,
            inline_menu_can_clear_selected: self
                .active_inline_menu(ctx)
                .is_some_and(|menu| menu.can_clear_selected(ctx)),
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
            if let Some(inline_menu) = self.active_inline_menu(ctx) {
                inline_menu.dismiss(ctx);
            }
            self.focused = false;
            ctx.notify();
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InputKeymapContextConfig {
    input_handles_escape: bool,
    mcp_menu_active: bool,
    plan_toggle_available: bool,
    keyboard_enhancement_supported: bool,
    shell_completion_available: bool,
    inline_menu_can_clear_selected: bool,
}

fn input_keymap_context(config: InputKeymapContextConfig) -> keymap::Context {
    let mut context = keymap::Context::default();
    context.set.insert(TuiInputView::ui_name());
    if config.input_handles_escape {
        context.set.insert(INPUT_HANDLES_ESCAPE_FLAG);
    }
    if config.mcp_menu_active {
        context.set.insert(MCP_MENU_ACTIVE_FLAG);
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
    if config.inline_menu_can_clear_selected {
        context.set.insert(INLINE_MENU_CAN_CLEAR_SELECTED_FLAG);
    }
    context
}
impl TypedActionView for TuiInputView {
    type Action = TuiInputAction;

    fn handle_action(&mut self, action: &TuiInputAction, ctx: &mut ViewContext<Self>) {
        if matches!(
            action,
            TuiInputAction::Editor(
                TuiEditorAction::SelectionStartAt { .. }
                    | TuiEditorAction::SelectionExtendTo { .. }
                    | TuiEditorAction::SelectWordAt { .. }
                    | TuiEditorAction::SelectLineAt { .. }
            ) | TuiInputAction::SetCursor { .. }
        ) {
            ctx.emit(TuiInputViewEvent::FocusRequested);
        }
        if self.handle_inline_menu_action(action, ctx) {
            return;
        }
        let input_ownership = self.active_inline_menu_input_ownership(ctx);
        if input_ownership.inline_menu_owns_input() {
            self.handle_inline_menu_owned_input_action(action, input_ownership, ctx);
            return;
        }
        let outcome = match action {
            TuiInputAction::Editor(editor_action) => {
                if let TuiEditorAction::PasteText(text) = editor_action {
                    self.close_read_only_menu(ctx);
                    ctx.emit(TuiInputViewEvent::Pasted(text.clone()));
                    return;
                }
                let closed_menu = self.close_read_only_menu(ctx);
                if closed_menu == Some(TuiReadOnlyMenuKind::Shortcuts) {
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
                        mode.set_mode(
                            TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts),
                            ctx,
                        );
                    });
                    return;
                }
                // Route every typed character through the shared Vim FSA when
                // enabled. Insert-mode routing is required for insert counts
                // and dot-repeat; prompt-specific insertion policy lives in
                // `VimHandler::insert_char`.
                if let TuiEditorAction::InsertChar(c) = *editor_action
                    && self.vim_mode_enabled(ctx)
                {
                    let old_mode = self.vim_model.as_ref(ctx).state().mode;
                    self.vim_model
                        .update(ctx, |vim, ctx| vim.typed_character(c, ctx));
                    if self.vim_model.as_ref(ctx).state().mode != old_mode {
                        ctx.emit(TuiInputViewEvent::VimModeChanged);
                    }
                    return;
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
                self.close_read_only_menu(ctx);
                // In vim normal/visual/replace mode, Enter still submits so the
                // prompt behaves like a command line (same as bash/zsh vi-mode).
                #[cfg(feature = "voice_input")]
                if self.handle_voice_submit(ctx) {
                    return;
                }
                {
                    self.submit(ctx);
                }
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiInputAction::HandleEscape => {
                self.handle_escape(ctx);
                TuiEditorInteractionOutcome::FollowCursor
            }
            TuiInputAction::LogOutSelectedMcp => TuiEditorInteractionOutcome::PreserveViewport,
            TuiInputAction::Complete => {
                if self.is_shell_mode(ctx) {
                    ctx.emit(TuiInputViewEvent::RequestShellCompletion);
                }
                TuiEditorInteractionOutcome::PreserveViewport
            }
            TuiInputAction::ClearActiveInlineMenuItem => {
                TuiEditorInteractionOutcome::PreserveViewport
            }
            TuiInputAction::EditorCommand(command) => {
                self.close_read_only_menu(ctx);
                if matches!(*command, TuiEditorCommand::SelectUp) && self.can_focus_above(ctx) {
                    ctx.emit(TuiInputViewEvent::MoveFocusUp);
                    return;
                }
                let vim_keystroke = match command {
                    TuiEditorCommand::Backspace
                        if !self.is_cursor_at_start(ctx)
                            || (!self.is_shell_mode(ctx) && !self.plain_text(ctx).is_empty()) =>
                    {
                        Some("backspace")
                    }
                    TuiEditorCommand::DeleteForward => Some("delete"),
                    TuiEditorCommand::InsertNewline => Some("shift-enter"),
                    TuiEditorCommand::Backspace
                    | TuiEditorCommand::DeleteWordBackward
                    | TuiEditorCommand::DeleteWordForward
                    | TuiEditorCommand::MoveLeft
                    | TuiEditorCommand::MoveRight
                    | TuiEditorCommand::MoveUp
                    | TuiEditorCommand::MoveDown
                    | TuiEditorCommand::MoveWordLeft
                    | TuiEditorCommand::MoveWordRight
                    | TuiEditorCommand::MoveToLineStart
                    | TuiEditorCommand::MoveToLineEnd
                    | TuiEditorCommand::SelectLeft
                    | TuiEditorCommand::SelectRight
                    | TuiEditorCommand::SelectUp
                    | TuiEditorCommand::SelectDown
                    | TuiEditorCommand::SelectWordLeft
                    | TuiEditorCommand::SelectWordRight
                    | TuiEditorCommand::SelectToLineStart
                    | TuiEditorCommand::SelectToLineEnd
                    | TuiEditorCommand::SelectAll
                    | TuiEditorCommand::Copy
                    | TuiEditorCommand::Cut
                    | TuiEditorCommand::Paste
                    | TuiEditorCommand::KillToLineEnd
                    | TuiEditorCommand::KillToLineStart
                    | TuiEditorCommand::Yank
                    | TuiEditorCommand::Undo
                    | TuiEditorCommand::Redo => None,
                };
                if self.vim_mode_enabled(ctx)
                    && let Some(keystroke) = vim_keystroke
                {
                    let old_mode = self.vim_model.as_ref(ctx).state().mode;
                    let keystroke =
                        Keystroke::parse(keystroke).expect("static Vim keystroke is valid");
                    self.vim_model
                        .update(ctx, |vim, ctx| vim.keypress(&keystroke, ctx));
                    if self.vim_model.as_ref(ctx).state().mode != old_mode {
                        ctx.emit(TuiInputViewEvent::VimModeChanged);
                    }
                    return;
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
                    && self.single_cursor_on_first_row(ctx)
                {
                    self.open_inline_menu(TuiInputSuggestionsMode::PromptAndCommandHistory, ctx);
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
                if self.vim_mode_enabled(ctx)
                    && !matches!(self.vim_model.as_ref(ctx).state().mode, VimMode::Insert)
                {
                    self.vim_model
                        .update(ctx, |vim, ctx| vim.force_insert_mode(ctx));
                    ctx.emit(TuiInputViewEvent::VimModeChanged);
                }
                self.close_read_only_menu(ctx);
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
            TuiEditorInteractionOutcome::Paste => {
                if let Err(error) = apply_editor_paste(&self.model, self.editor_behavior, ctx) {
                    log::error!("Failed to paste into TUI input: {error}");
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
    fn handle_inline_menu_owned_input_action(
        &mut self,
        action: &TuiInputAction,
        input_ownership: TuiInlineMenuInputOwnership,
        ctx: &mut ViewContext<Self>,
    ) {
        let outcome = match action {
            TuiInputAction::Editor(action) => {
                apply_editor_action(&self.model, action, self.editor_behavior, ctx)
            }
            TuiInputAction::EditorCommand(command) => {
                self.editor_state
                    .apply_command(&self.model, *command, self.editor_behavior, ctx)
            }
            TuiInputAction::SetCursor { offset } => {
                self.model.update(ctx, |model, ctx| {
                    model.select_at(*offset, false, ctx);
                    model.end_selection(ctx);
                });
                TuiEditorInteractionOutcome::FollowCursor
            }
            // These menu-policy actions were handled by
            // `handle_inline_menu_action` before input ownership was resolved.
            TuiInputAction::Submit
            | TuiInputAction::HandleEscape
            | TuiInputAction::LogOutSelectedMcp
            | TuiInputAction::Complete
            | TuiInputAction::ClearActiveInlineMenuItem => {
                TuiEditorInteractionOutcome::PreserveViewport
            }
        };
        let outcome = match outcome {
            TuiEditorInteractionOutcome::Clipboard(_) if input_ownership.is_masked() => {
                TuiEditorInteractionOutcome::FollowCursor
            }
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
    pub(crate) fn enter_shell_mode(&mut self, ctx: &mut ViewContext<Self>) {
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

    /// Emits the submission event without clearing the buffer; the owner
    /// decides whether the submission is accepted.
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.plain_text(ctx);
        ctx.emit(TuiInputViewEvent::Submitted(text));
    }

    #[cfg(feature = "voice_input")]
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

    pub(crate) fn route_inline_menu_acceptance(
        &mut self,
        accepted: TuiInlineMenuAccepted,
        ctx: &mut ViewContext<Self>,
    ) {
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
            TuiInlineMenuAccepted::Team(team_uid) => {
                ctx.emit(TuiInputViewEvent::AcceptedTeam(team_uid));
            }
            TuiInlineMenuAccepted::Mcp(action) => {
                ctx.emit(TuiInputViewEvent::AcceptedMcp(action));
            }
            TuiInlineMenuAccepted::McpInstall(action) => {
                ctx.emit(TuiInputViewEvent::AcceptedMcpInstall(action));
            }
            TuiInlineMenuAccepted::PromptAndCommandHistory { text, kind } => {
                ctx.emit(TuiInputViewEvent::AcceptedPromptAndCommandHistory { text, kind });
            }
            TuiInlineMenuAccepted::Completion(acceptance) => {
                self.apply_shell_completion(acceptance, ctx);
            }
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
                | TuiInputAction::LogOutSelectedMcp
                | TuiInputAction::Complete
                | TuiInputAction::ClearActiveInlineMenuItem
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
                    self.route_inline_menu_acceptance(accepted, ctx);
                }
            }
            TuiInputAction::LogOutSelectedMcp => {
                if let Some(TuiInlineMenuAccepted::Mcp(action)) = inline_menu.accept_secondary(ctx)
                {
                    ctx.emit(TuiInputViewEvent::AcceptedMcp(action));
                }
            }
            TuiInputAction::HandleEscape => return self.handle_escape(ctx),
            TuiInputAction::ClearActiveInlineMenuItem => {
                inline_menu.clear_selected(ctx);
            }
            _ => return false,
        }
        ctx.notify();
        true
    }

    /// Handles the input's contextual Escape behavior in explicit priority order.
    fn handle_escape(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if self.close_read_only_menu(ctx).is_some() {
            return true;
        }
        if let Some(inline_menu) = self.active_inline_menu(ctx) {
            inline_menu.dismiss(ctx);
            ctx.notify();
            return true;
        }

        #[cfg(feature = "voice_input")]
        {
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
        }

        // In vim mode, Escape transitions between modes (Insert→Normal,
        // Visual/Replace→Normal, Normal→clear pending). This takes priority
        // over shell-mode exit so that `<Esc>` is always a vim command first.
        // Exception: when the FSA is already in Normal mode with no pending
        // input, a second Escape should exit shell mode if active (matching
        // bash/zsh vi-mode behaviour where `<Esc><Esc>` exits shell mode).
        if self.vim_mode_enabled(ctx) {
            let vim_state = self.vim_model.as_ref(ctx).state();
            if matches!(vim_state.mode, VimMode::Normal)
                && vim_state.showcmd.is_empty()
                && self.is_shell_mode(ctx)
            {
                self.exit_shell_mode(ctx);
                return true;
            }
            // Drive the shared FSA via VimModel. VimSubscriber dispatches the
            // resulting VimEvent (Escape or ChangeMode) to our VimHandler impl.
            let escape = Keystroke::parse("escape").expect("escape key is valid");
            let old_mode = vim_state.mode;
            self.vim_model
                .update(ctx, |vim, ctx| vim.keypress(&escape, ctx));
            if self.vim_model.as_ref(ctx).state().mode != old_mode {
                ctx.emit(TuiInputViewEvent::VimModeChanged);
            }
            ctx.notify();
            return true;
        }

        if self.is_shell_mode(ctx) {
            self.exit_shell_mode(ctx);
            return true;
        }
        false
    }

    fn close_read_only_menu(&self, ctx: &mut ViewContext<Self>) -> Option<TuiReadOnlyMenuKind> {
        let mode = self.suggestions_mode.as_ref(ctx).mode();
        let kind = mode.read_only_menu()?;
        self.suggestions_mode.update(ctx, |model, ctx| {
            model.close_if_active(mode, ctx);
        });
        Some(kind)
    }

    fn active_inline_menu(&self, ctx: &AppContext) -> Option<TuiInlineMenu> {
        active_inline_menu(
            &self.inline_menus,
            self.suggestions_mode.as_ref(ctx).mode(),
            ctx,
        )
    }

    /// Resolves the shared editor owner from the one active inline menu.
    fn active_inline_menu_input_ownership(&self, ctx: &AppContext) -> TuiInlineMenuInputOwnership {
        self.active_inline_menu(ctx)
            .map_or(TuiInlineMenuInputOwnership::Composer, |menu| {
                menu.input_ownership(ctx)
            })
    }
}

// VimHandler implementation for TuiInputView. Declared as a submodule so it
// can access the private fields of TuiInputView while keeping the main view
// file focused on prompt policy.
#[path = "vim.rs"]
mod vim_impl;

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
