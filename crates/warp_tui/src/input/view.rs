//! [`TuiInputView`] — ratatui-rendered TUI prompt input.
//!
//! Implements [`TuiView`] + [`TypedActionView`]. The view:
//!
//! - Holds a [`ModelHandle<CodeEditorModel>`] constructed in `LayoutMode::CharCell`.
//! - Renders the core [`TuiEditorElement`] verbatim (editable, scroll-windowed).
//! - Owns the kill buffer and the `!` shell-mode composition.
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
//! the readline keybinding table, the kill buffer, submit, and shell mode.
//!
//! See `specs/tui-input-view/TECH.md` for the full keybinding table.

use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, BlocklistAIInputModel, InputType,
    InputTypeAutoDetectionSource, LLMId, TuiMcpAction,
};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::TextUnit;
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiText};
use warpui_core::elements::MouseStateHandle;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, EditableBinding};
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{
    AppContext, BlurContext, Entity, FocusContext, ModelHandle, TuiView, TypedActionView,
    ViewContext,
};

use super::kill_buffer::KillBuffer;
use crate::editor_element::{TuiEditorAction, TuiEditorElement, TuiEditorStyles};
use crate::inline_menu::{active_inline_menu, TuiInlineMenu, TuiInlineMenuAccepted};
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::TUI_BINDING_GROUP;
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
        // ── Submit / newline ─────────────────────────────────────────
        EditableBinding::new(
            "tui:input:submit",
            "Submit the input",
            TuiInputAction::Submit,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:input:insert_newline",
            "Insert a newline",
            TuiInputAction::InsertNewline,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-enter"),
        EditableBinding::new(
            "tui:input:insert_newline",
            "Insert a newline",
            TuiInputAction::InsertNewline,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-j"),
        EditableBinding::new(
            "tui:input:insert_newline",
            "Insert a newline",
            TuiInputAction::InsertNewline,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-enter"),
        EditableBinding::new(
            "tui:input:handle_escape",
            "Handle contextual input escape",
            TuiInputAction::HandleEscape,
        )
        .with_context_predicate(id!("TuiInputView") & id!(INPUT_HANDLES_ESCAPE_FLAG))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("escape"),
        // ── Deletion ───────────────────────────────────────────────────
        EditableBinding::new(
            "tui:input:backspace",
            "Delete the previous character",
            TuiInputAction::Backspace,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("backspace"),
        EditableBinding::new(
            "tui:input:backspace",
            "Delete the previous character",
            TuiInputAction::Backspace,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-backspace"),
        EditableBinding::new(
            "tui:input:backspace",
            "Delete the previous character",
            TuiInputAction::Backspace,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-h"),
        EditableBinding::new(
            "tui:input:delete_forward",
            "Delete the next character",
            TuiInputAction::DeleteForward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("delete"),
        EditableBinding::new(
            "tui:input:delete_forward",
            "Delete the next character",
            TuiInputAction::DeleteForward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-d"),
        EditableBinding::new(
            "tui:input:delete_word_backward",
            "Delete the previous word",
            TuiInputAction::DeleteWordBackward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-w"),
        EditableBinding::new(
            "tui:input:delete_word_backward",
            "Delete the previous word",
            TuiInputAction::DeleteWordBackward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-backspace"),
        EditableBinding::new(
            "tui:input:delete_word_backward",
            "Delete the previous word",
            TuiInputAction::DeleteWordBackward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-backspace"),
        EditableBinding::new(
            "tui:input:delete_word_forward",
            "Delete the next word",
            TuiInputAction::DeleteWordForward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-d"),
        EditableBinding::new(
            "tui:input:delete_word_forward",
            "Delete the next word",
            TuiInputAction::DeleteWordForward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-delete"),
        EditableBinding::new(
            "tui:input:delete_word_forward",
            "Delete the next word",
            TuiInputAction::DeleteWordForward,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-delete"),
        // ── Cursor movement ─────────────────────────────────────────────
        EditableBinding::new(
            "tui:input:move_left",
            "Move cursor left",
            TuiInputAction::MoveLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("left"),
        EditableBinding::new(
            "tui:input:move_left",
            "Move cursor left",
            TuiInputAction::MoveLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-b"),
        EditableBinding::new(
            "tui:input:move_right",
            "Move cursor right",
            TuiInputAction::MoveRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("right"),
        EditableBinding::new(
            "tui:input:move_right",
            "Move cursor right",
            TuiInputAction::MoveRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-f"),
        EditableBinding::new(
            "tui:input:move_up",
            "Move cursor up",
            TuiInputAction::MoveUp,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("up"),
        EditableBinding::new(
            "tui:input:move_up",
            "Move cursor up",
            TuiInputAction::MoveUp,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-p"),
        EditableBinding::new(
            "tui:input:move_down",
            "Move cursor down",
            TuiInputAction::MoveDown,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("down"),
        EditableBinding::new(
            "tui:input:move_down",
            "Move cursor down",
            TuiInputAction::MoveDown,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-n"),
        EditableBinding::new(
            "tui:input:move_word_left",
            "Move cursor one word left",
            TuiInputAction::MoveWordLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-left"),
        EditableBinding::new(
            "tui:input:move_word_left",
            "Move cursor one word left",
            TuiInputAction::MoveWordLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-b"),
        EditableBinding::new(
            "tui:input:move_word_left",
            "Move cursor one word left",
            TuiInputAction::MoveWordLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-left"),
        EditableBinding::new(
            "tui:input:move_word_right",
            "Move cursor one word right",
            TuiInputAction::MoveWordRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-right"),
        EditableBinding::new(
            "tui:input:move_word_right",
            "Move cursor one word right",
            TuiInputAction::MoveWordRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-f"),
        EditableBinding::new(
            "tui:input:move_word_right",
            "Move cursor one word right",
            TuiInputAction::MoveWordRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-right"),
        EditableBinding::new(
            "tui:input:move_to_line_start",
            "Move cursor to start of line",
            TuiInputAction::MoveToLineStart,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("home"),
        EditableBinding::new(
            "tui:input:move_to_line_start",
            "Move cursor to start of line",
            TuiInputAction::MoveToLineStart,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-a"),
        EditableBinding::new(
            "tui:input:move_to_line_end",
            "Move cursor to end of line",
            TuiInputAction::MoveToLineEnd,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("end"),
        EditableBinding::new(
            "tui:input:move_to_line_end",
            "Move cursor to end of line",
            TuiInputAction::MoveToLineEnd,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-e"),
        // ── Selection ────────────────────────────────────────────────────────────────
        EditableBinding::new(
            "tui:input:select_left",
            "Extend selection left",
            TuiInputAction::SelectLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-left"),
        EditableBinding::new(
            "tui:input:select_right",
            "Extend selection right",
            TuiInputAction::SelectRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-right"),
        EditableBinding::new(
            "tui:input:select_up",
            "Extend selection up",
            TuiInputAction::SelectUp,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-up"),
        EditableBinding::new(
            "tui:input:select_down",
            "Extend selection down",
            TuiInputAction::SelectDown,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("shift-down"),
        EditableBinding::new(
            "tui:input:select_word_left",
            "Extend selection one word left",
            TuiInputAction::SelectWordLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-left"),
        EditableBinding::new(
            "tui:input:select_word_left",
            "Extend selection one word left",
            TuiInputAction::SelectWordLeft,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-shift-left"),
        EditableBinding::new(
            "tui:input:select_word_right",
            "Extend selection one word right",
            TuiInputAction::SelectWordRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-right"),
        EditableBinding::new(
            "tui:input:select_word_right",
            "Extend selection one word right",
            TuiInputAction::SelectWordRight,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("alt-shift-right"),
        EditableBinding::new(
            "tui:input:select_all",
            "Select all text",
            TuiInputAction::SelectAll,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-shift-A"),
        // ── Kill / yank ─────────────────────────────────────────────────
        EditableBinding::new(
            "tui:input:kill_to_line_end",
            "Delete to end of line",
            TuiInputAction::KillToLineEnd,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-k"),
        EditableBinding::new(
            "tui:input:kill_to_line_start",
            "Delete to start of line",
            TuiInputAction::KillToLineStart,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-u"),
        EditableBinding::new(
            "tui:input:yank",
            "Paste the last deleted text",
            TuiInputAction::Yank,
        )
        .with_context_predicate(id!("TuiInputView"))
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-y"),
        // ── Undo / redo ─────────────────────────────────────────────────
        EditableBinding::new("tui:input:undo", "Undo", TuiInputAction::Undo)
            .with_context_predicate(id!("TuiInputView"))
            .with_group(TUI_BINDING_GROUP)
            .with_key_binding("ctrl-z"),
        EditableBinding::new("tui:input:redo", "Redo", TuiInputAction::Redo)
            .with_context_predicate(id!("TuiInputView"))
            .with_group(TUI_BINDING_GROUP)
            .with_key_binding("ctrl-shift-Z"),
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

/// All editing operations dispatched from [`TuiEditorElement`].
///
/// Each variant corresponds to one or more keybindings from the spec keybinding table.
#[derive(Debug, Clone)]
pub enum TuiInputAction {
    /// Insert a character (`Char(c)` key events).
    InsertChar(char),
    /// Insert one complete bracketed-paste payload without submitting it.
    InsertText(String),
    /// Insert a hard newline (`Shift+Enter`, `Ctrl+J`, `Alt+Enter`).
    InsertNewline,
    /// Submit the current input (`Enter`).
    Submit,
    /// Handle contextual input Escape behavior, prioritizing an open inline menu.
    HandleEscape,
    /// Delete the character before the cursor (`Backspace`, `Ctrl+H`).
    Backspace,
    /// Delete the character after the cursor (`Delete`, `Ctrl+D`).
    DeleteForward,
    /// Move cursor left one char (`←`, `Ctrl+B`).
    MoveLeft,
    /// Move cursor right one char (`→`, `Ctrl+F`).
    MoveRight,
    /// Move cursor up one visual row (`↑`, `Ctrl+P`).
    MoveUp,
    /// Move cursor down one visual row (`↓`, `Ctrl+N`).
    MoveDown,
    /// Move cursor one word backward (`Alt+←`, `Alt+B`, `Ctrl+←`).
    MoveWordLeft,
    /// Move cursor one word forward (`Alt+→`, `Alt+F`, `Ctrl+→`).
    MoveWordRight,
    /// Move cursor to start of visual line (`Home`, `Ctrl+A`).
    MoveToLineStart,
    /// Move cursor to end of visual line (`End`, `Ctrl+E`).
    MoveToLineEnd,
    /// Extend selection left (`Shift+←`).
    SelectLeft,
    /// Extend selection right (`Shift+→`).
    SelectRight,
    /// Extend selection up (`Shift+↑`).
    SelectUp,
    /// Extend selection down (`Shift+↓`).
    SelectDown,
    /// Extend selection one word left (`Ctrl+Shift+←`, `Alt+Shift+←`).
    SelectWordLeft,
    /// Extend selection one word right (`Ctrl+Shift+→`, `Alt+Shift+→`).
    SelectWordRight,
    /// Select all text (`Ctrl+Shift+A` / `Meta+A`).
    SelectAll,
    /// Delete word backward (`Ctrl+W`, `Alt+Backspace`, `Ctrl+Backspace`).
    DeleteWordBackward,
    /// Delete word forward (`Alt+D`, `Alt+Delete`, `Ctrl+Delete`).
    DeleteWordForward,
    /// Kill from cursor to end of visual line (`Ctrl+K`).
    KillToLineEnd,
    /// Kill from cursor to start of visual line (`Ctrl+U`).
    KillToLineStart,
    /// Yank last killed text (`Ctrl+Y`).
    Yank,
    /// Undo (`Ctrl+Z`).
    Undo,
    /// Redo (`Ctrl+Shift+Z`).
    Redo,
    /// Place the cursor / begin a character selection at `offset` (single click).
    SelectionStartAt { offset: CharOffset },
    /// Extend the active selection's head to `offset` (shift-click).
    SelectionExtendTo { offset: CharOffset },
    /// Select the word at `offset` (double click).
    SelectWordAt { offset: CharOffset },
    /// Select the line at `offset` (triple click).
    SelectLineAt { offset: CharOffset },
    /// Update the in-progress drag selection to `offset` (mouse drag).
    SelectionUpdateTo { offset: CharOffset },
    /// Finish the in-progress drag selection (mouse up).
    SelectionEnd,
    /// Place the cursor at `offset` without starting a drag selection
    /// (the `!` gutter click).
    SetCursor { offset: CharOffset },
    /// Scroll the viewport by `rows` visual rows without moving the cursor
    /// (negative scrolls toward the top). Driven by the mouse wheel.
    Scroll { rows: isize },
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
    /// Single-entry kill buffer for `Ctrl+K` / `Ctrl+U` / `Ctrl+Y`.
    kill_buffer: KillBuffer,
    /// Maximum number of visible rows before the input scrolls.
    max_visible_rows: u32,
    /// Mouse state for the shell-mode `!` gutter; created once here (not inline
    /// during render) so mouse tracking survives per-frame element rebuilds.
    prefix_mouse_state: MouseStateHandle,
    /// Whether this view is focused, tracked via `on_focus`/`on_blur` like
    /// the GUI's `EditorView::focused`. Snapshotted into the editor element
    /// so it only consumes typed text while the input is focused.
    focused: bool,
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
            kill_buffer: KillBuffer::default(),
            max_visible_rows: 6,
            prefix_mouse_state: MouseStateHandle::default(),
            focused: false,
        }
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
            .with_viewport_rows(self.max_visible_rows)
            .with_styles(styles)
            .on_action(|action, event_ctx| {
                event_ctx.dispatch_typed_action(TuiInputAction::from(action))
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
        input_keymap_context(self.active_inline_menu(ctx).is_some() || self.is_shell_mode(ctx))
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

impl From<TuiEditorAction> for TuiInputAction {
    fn from(action: TuiEditorAction) -> Self {
        match action {
            TuiEditorAction::InsertChar(c) => Self::InsertChar(c),
            TuiEditorAction::InsertText(text) => Self::InsertText(text),
            TuiEditorAction::SelectionStartAt { offset } => Self::SelectionStartAt { offset },
            TuiEditorAction::SelectionExtendTo { offset } => Self::SelectionExtendTo { offset },
            TuiEditorAction::SelectWordAt { offset } => Self::SelectWordAt { offset },
            TuiEditorAction::SelectLineAt { offset } => Self::SelectLineAt { offset },
            TuiEditorAction::SelectionUpdateTo { offset } => Self::SelectionUpdateTo { offset },
            TuiEditorAction::SelectionEnd => Self::SelectionEnd,
            TuiEditorAction::Scroll { rows } => Self::Scroll { rows },
        }
    }
}

fn input_keymap_context(input_handles_escape: bool) -> keymap::Context {
    let mut context = keymap::Context::default();
    context.set.insert(TuiInputView::ui_name());
    if input_handles_escape {
        context.set.insert(INPUT_HANDLES_ESCAPE_FLAG);
    }
    context
}
impl TypedActionView for TuiInputView {
    type Action = TuiInputAction;

    fn handle_action(&mut self, action: &TuiInputAction, ctx: &mut ViewContext<Self>) {
        if self.handle_inline_menu_action(action, ctx) {
            return;
        }
        match action {
            TuiInputAction::InsertChar(c) => {
                // A `!` typed at the very start of the input enters shell mode
                // instead of inserting (matching the GUI's typed-only trigger).
                if *c == '!'
                    && !self.is_shell_mode(ctx)
                    && self.is_cursor_at_start(ctx)
                    && !self
                        .input_mode
                        .as_ref(ctx)
                        .is_terminal_use_active_or_pending()
                {
                    self.enter_shell_mode(ctx);
                } else {
                    let s = c.to_string();
                    self.model.update(ctx, |m, ctx| m.user_insert(&s, ctx));
                }
            }
            TuiInputAction::InsertText(text) => {
                self.model.update(ctx, |m, ctx| m.user_insert(text, ctx));
            }
            TuiInputAction::InsertNewline => {
                self.model.update(ctx, |m, ctx| m.user_insert("\n", ctx));
            }
            TuiInputAction::Submit => self.submit(ctx),
            TuiInputAction::HandleEscape => {
                self.handle_escape(ctx);
            }
            TuiInputAction::Backspace => {
                // With nothing left to delete, backspace removes the `!`
                // affordance instead; typed text is preserved.
                if self.is_shell_mode(ctx) && self.is_cursor_at_start(ctx) {
                    self.exit_shell_mode(ctx);
                } else {
                    self.model.update(ctx, |m, ctx| m.backspace(ctx));
                }
            }
            TuiInputAction::DeleteForward => {
                self.model.update(ctx, |m, ctx| {
                    m.delete(
                        warp_editor::selection::TextDirection::Forwards,
                        TextUnit::Character,
                        false,
                        ctx,
                    )
                });
            }
            TuiInputAction::MoveLeft => {
                // Only open the conversation list from normal agent input; in
                // `!` shell mode the `!` prefix is not part of `plain_text`, so
                // an empty shell command would otherwise trip this branch and
                // open the picker while the input stayed shell-mode.
                if !self.is_shell_mode(ctx)
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
                } else {
                    self.model.update(ctx, |m, ctx| m.move_left(ctx));
                }
            }
            TuiInputAction::MoveRight => {
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
            }
            TuiInputAction::MoveUp => {
                self.model.update(ctx, |m, ctx| m.move_up(ctx));
            }
            TuiInputAction::MoveDown => {
                self.model.update(ctx, |m, ctx| m.move_down(ctx));
            }
            TuiInputAction::MoveWordLeft => {
                self.model.update(ctx, |m, ctx| {
                    m.backward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::MoveWordRight => {
                self.model.update(ctx, |m, ctx| {
                    m.forward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::MoveToLineStart => {
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
            }
            TuiInputAction::MoveToLineEnd => {
                self.model.update(ctx, |m, ctx| m.move_to_line_end(ctx));
            }
            TuiInputAction::SelectLeft => {
                self.model.update(ctx, |m, ctx| m.select_left(ctx));
            }
            TuiInputAction::SelectRight => {
                self.model.update(ctx, |m, ctx| m.select_right(ctx));
            }
            TuiInputAction::SelectUp => {
                self.model.update(ctx, |m, ctx| m.select_up(ctx));
            }
            TuiInputAction::SelectDown => {
                self.model.update(ctx, |m, ctx| m.select_down(ctx));
            }
            TuiInputAction::SelectWordLeft => {
                self.model.update(ctx, |m, ctx| {
                    m.backward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::SelectWordRight => {
                self.model.update(ctx, |m, ctx| {
                    m.forward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::SelectAll => {
                self.model.update(ctx, |m, ctx| m.select_all(ctx));
            }
            TuiInputAction::DeleteWordBackward => {
                self.model.update(ctx, |m, ctx| {
                    m.delete(
                        warp_editor::selection::TextDirection::Backwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    )
                });
            }
            TuiInputAction::DeleteWordForward => {
                self.model.update(ctx, |m, ctx| {
                    m.delete(
                        warp_editor::selection::TextDirection::Forwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    )
                });
            }
            TuiInputAction::KillToLineEnd => {
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                {
                    self.kill_buffer.kill(killed);
                }
            }
            TuiInputAction::KillToLineStart => {
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_start(ctx))
                {
                    self.kill_buffer.kill(killed);
                }
            }
            TuiInputAction::Yank => self.yank(ctx),
            TuiInputAction::Undo => {
                self.model.update(ctx, |m, ctx| m.undo(ctx));
            }
            TuiInputAction::Redo => {
                self.model.update(ctx, |m, ctx| m.redo(ctx));
            }
            TuiInputAction::SelectionStartAt { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.select_at(*offset, false, ctx));
            }
            TuiInputAction::SelectionExtendTo { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.set_last_selection_head(*offset, ctx));
            }
            TuiInputAction::SelectWordAt { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.select_word_at(*offset, false, ctx));
            }
            TuiInputAction::SelectLineAt { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.select_line_at(*offset, false, ctx));
            }
            // Both are model-side no-ops unless a drag selection is pending
            // (begun by a mouse-down on the element), so no gating is needed.
            TuiInputAction::SelectionUpdateTo { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.update_pending_selection(*offset, ctx));
            }
            TuiInputAction::SelectionEnd => {
                self.model.update(ctx, |m, ctx| m.end_selection(ctx));
            }
            TuiInputAction::SetCursor { offset } => {
                self.model.update(ctx, |m, ctx| {
                    m.select_at(*offset, false, ctx);
                    m.end_selection(ctx);
                });
            }
            TuiInputAction::Scroll { rows } => {
                // Wheel scrolling moves the viewport only; it must NOT snap back
                // to the cursor, so it returns early (skipping the follow-cursor
                // tail below).
                self.scroll_viewport_by(*rows, ctx);
                ctx.notify();
                return;
            }
        }

        self.follow_cursor(ctx);
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
    // ranges — and apply the input's viewport policy (`max_visible_rows`).

    /// Scrolls the viewport the minimal amount needed to keep the cursor
    /// visible.
    fn follow_cursor(&self, ctx: &AppContext) {
        let model = self.model.as_ref(ctx);
        let render = model.render_state().as_ref(ctx);
        let Some(char_cell) = render.char_cell() else {
            return;
        };
        let cursor_offset = CharOffset::from(self.cursor_offset(ctx).as_usize().saturating_sub(1));
        let hidden = char_cell.hidden_line_ranges(ctx);
        char_cell.follow_cursor(cursor_offset, self.max_visible_rows, &hidden);
    }

    /// Scrolls the viewport by `rows` display rows (negative scrolls toward
    /// the top) without moving the cursor.
    fn scroll_viewport_by(&self, rows: isize, ctx: &AppContext) {
        let model = self.model.as_ref(ctx);
        let render = model.render_state().as_ref(ctx);
        let Some(char_cell) = render.char_cell() else {
            return;
        };
        let cursor_offset = CharOffset::from(self.cursor_offset(ctx).as_usize().saturating_sub(1));
        let hidden = char_cell.hidden_line_ranges(ctx);
        char_cell.scroll_by(rows, self.max_visible_rows, cursor_offset, &hidden);
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
        ctx.emit(TuiInputViewEvent::Submitted(text));
    }

    fn handle_inline_menu_action(
        &mut self,
        action: &TuiInputAction,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !matches!(
            action,
            TuiInputAction::MoveUp
                | TuiInputAction::MoveDown
                | TuiInputAction::Submit
                | TuiInputAction::HandleEscape
        ) {
            return false;
        }
        let Some(inline_menu) = self.active_inline_menu(ctx) else {
            return false;
        };

        match action {
            TuiInputAction::MoveUp => {
                inline_menu.select_previous(ctx);
            }
            TuiInputAction::MoveDown => {
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

    // ── Kill / yank ───────────────────────────────────────────────────────────
    //
    // The kill *edits* (visual-row range computation and deletion) live on
    // `CodeEditorModel::kill_to_char_cell_visual_row_end` / `_start`; the view
    // owns only the kill buffer the deleted text lands in.

    fn yank(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(text) = self.kill_buffer.yank().map(str::to_owned) {
            self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
        }
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
