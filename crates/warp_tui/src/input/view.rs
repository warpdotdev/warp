//! [`TuiInputView`] — ratatui-rendered TUI prompt input.
//!
//! Implements [`TuiView`] + [`TypedActionView`]. The view:
//!
//! - Holds a [`ModelHandle<CodeEditorModel>`] constructed in `LayoutMode::CharCell`.
//! - Owns all TUI-specific session state: kill buffer, scroll offset, terminal width.
//! - Dispatches keystrokes as [`TuiInputAction`] typed actions.
//! - Emits [`TuiInputViewEvent::Submitted`] when the user presses Enter.
//!
//! # Architecture
//!
//! The view works directly with [`CodeEditorModel`] (char-cell mode) so that future
//! TUI features — vim, syntax highlighting, diff, hidden lines — come for free from
//! the shared editor infrastructure.  TUI-specific concepts (kill ring, terminal
//! viewport scroll, readline keybindings) belong to this view layer, not to the
//! underlying editor model.
//!
//! See `specs/tui-input-view/TECH.md` for the full keybinding table.

use std::cmp;
use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{BlocklistAIInputModel, InputTypeAutoDetectionSource};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::render::model::{
    char_cell_display_width, char_cell_line_gap_position, char_cell_line_row_starts, ColumnUnit,
    SoftWrapPoint,
};
use warp_editor::selection::TextUnit;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiConstraint, TuiContainer, TuiElement, TuiEvent, TuiEventContext,
    TuiFlex, TuiHoverable, TuiLayoutContext, TuiPaintContext, TuiParentElement, TuiPoint, TuiRect,
    TuiRectExt, TuiSize, TuiStyle, TuiText,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{Context, EditableBinding};
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewContext};

use super::kill_buffer::KillBuffer;
use crate::input_mode_policy::{self, AI_LOCKED_CONFIG, SHELL_LOCKED_CONFIG};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::tui_builder::TuiUiBuilder;

/// Logical rows scrolled per mouse-wheel notch (matches `TuiScrollable`).
const WHEEL_STEP: isize = 2;

/// Keymap-context flag set by [`TuiInputView::keymap_context`] while the input
/// is in `!` shell mode; gates the `tui:input:exit_shell_mode` binding so Esc
/// stays available to ancestors otherwise.
const SHELL_MODE_INPUT_FLAG: &str = "ShellModeInput";

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
/// `TuiInputElement::dispatch_event`, matching the GUI.
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
        // ── Shell mode ──────────────────────────────────────────────────
        EditableBinding::new(
            "tui:input:exit_shell_mode",
            "Exit shell mode",
            TuiInputAction::ExitShellMode,
        )
        .with_context_predicate(id!("TuiInputView") & id!(SHELL_MODE_INPUT_FLAG))
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
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed action enum
// ─────────────────────────────────────────────────────────────────────────────

/// All editing operations dispatched from `TuiInputElement::dispatch_event`.
///
/// Each variant corresponds to one or more keybindings from the spec keybinding table.
#[derive(Debug, Clone)]
pub enum TuiInputAction {
    /// Insert a character (`Char(c)` key events).
    InsertChar(char),
    /// Insert a hard newline (`Shift+Enter`, `Ctrl+J`, `Alt+Enter`).
    InsertNewline,
    /// Submit the current input (`Enter`).
    Submit,
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
    /// Exit shell mode, keeping any typed text (`Esc`, only while in shell mode).
    ExitShellMode,
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
    /// The backing code editor in char-cell (terminal) mode.
    model: ModelHandle<CodeEditorModel>,
    /// Shared input-mode state driving `!` shell-mode handling.
    input_mode: ModelHandle<BlocklistAIInputModel>,
    /// Single-entry kill buffer for `Ctrl+K` / `Ctrl+U` / `Ctrl+Y`.
    kill_buffer: KillBuffer,
    /// First visible visual row (0-indexed).
    scroll_offset: u32,
    /// Maximum number of visible rows before the input scrolls.
    max_visible_rows: u32,
    /// Whether a mouse drag-selection is in progress (set on mouse-down, cleared
    /// on mouse-up). Mirrors the GUI editor's `is_selecting`.
    is_selecting: bool,
    /// Mouse state for the shell-mode `!` gutter; created once here (not inline
    /// during render) so mouse tracking survives per-frame element rebuilds.
    prefix_mouse_state: MouseStateHandle,
}

impl Entity for TuiInputView {
    type Event = TuiInputViewEvent;
}

impl TuiInputView {
    /// Construct a new `TuiInputView` backed by `model` (must be in char-cell
    /// mode). The model carries the terminal width (set via
    /// [`CodeEditorModel::new_tui`]); the view does not keep its own copy.
    ///
    /// `input_mode` is the shared input-mode model backing `!` shell-mode
    /// handling; the view re-renders whenever the mode changes.
    ///
    /// Subscribes to [`CodeEditorModelEvent::ContentChanged`] to trigger re-renders
    /// whenever the buffer changes from outside `handle_action`.
    pub fn new(
        model: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
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
        Self {
            model,
            input_mode,
            kill_buffer: KillBuffer::default(),
            scroll_offset: 0,
            max_visible_rows: 6,
            is_selecting: false,
            prefix_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Whether the input is in `!` shell mode (locked shell input).
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

    /// Clears the input buffer and resets the viewport scroll.
    pub fn clear(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |m, ctx| m.clear_buffer(ctx));
        self.scroll_offset = 0;
        ctx.notify();
    }

    /// The editor element for this frame, boxed for the render tree.
    fn render_input(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        TuiInputElement::new(self, ctx).finish()
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

    fn keymap_context(&self, app: &AppContext) -> Context {
        let mut ctx = Self::default_keymap_context();
        if self.is_shell_mode(app) {
            ctx.set.insert(SHELL_MODE_INPUT_FLAG);
        }
        ctx
    }
}

impl TypedActionView for TuiInputView {
    type Action = TuiInputAction;

    fn handle_action(&mut self, action: &TuiInputAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiInputAction::InsertChar(c) => {
                // A `!` typed at the very start of the input enters shell mode
                // instead of inserting (matching the GUI's typed-only trigger).
                if *c == '!' && !self.is_shell_mode(ctx) && self.is_cursor_at_start(ctx) {
                    self.enter_shell_mode(ctx);
                } else {
                    let s = c.to_string();
                    self.model.update(ctx, |m, ctx| m.user_insert(&s, ctx));
                }
            }
            TuiInputAction::InsertNewline => {
                self.model.update(ctx, |m, ctx| m.user_insert("\n", ctx));
            }
            TuiInputAction::Submit => self.submit(ctx),
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
                self.model.update(ctx, |m, ctx| m.move_left(ctx));
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
            TuiInputAction::KillToLineEnd => self.kill_to_line_end(ctx),
            TuiInputAction::KillToLineStart => self.kill_to_line_start(ctx),
            TuiInputAction::Yank => self.yank(ctx),
            TuiInputAction::Undo => {
                self.model.update(ctx, |m, ctx| m.undo(ctx));
            }
            TuiInputAction::Redo => {
                self.model.update(ctx, |m, ctx| m.redo(ctx));
            }
            TuiInputAction::ExitShellMode => {
                if self.is_shell_mode(ctx) {
                    self.exit_shell_mode(ctx);
                }
            }
            TuiInputAction::SelectionStartAt { offset } => {
                self.is_selecting = true;
                self.model
                    .update(ctx, |m, ctx| m.select_at(*offset, false, ctx));
            }
            TuiInputAction::SelectionExtendTo { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.set_last_selection_head(*offset, ctx));
            }
            TuiInputAction::SelectWordAt { offset } => {
                self.is_selecting = true;
                self.model
                    .update(ctx, |m, ctx| m.select_word_at(*offset, false, ctx));
            }
            TuiInputAction::SelectLineAt { offset } => {
                self.is_selecting = true;
                self.model
                    .update(ctx, |m, ctx| m.select_line_at(*offset, false, ctx));
            }
            TuiInputAction::SelectionUpdateTo { offset } => {
                if self.is_selecting {
                    self.model
                        .update(ctx, |m, ctx| m.update_pending_selection(*offset, ctx));
                }
            }
            TuiInputAction::SelectionEnd => {
                if self.is_selecting {
                    self.is_selecting = false;
                    self.model.update(ctx, |m, ctx| m.end_selection(ctx));
                }
            }
            TuiInputAction::SetCursor { offset } => {
                self.model.update(ctx, |m, ctx| {
                    m.select_at(*offset, false, ctx);
                    m.end_selection(ctx);
                });
            }
            TuiInputAction::Scroll { rows } => {
                // Wheel scrolling moves the viewport only; it must NOT snap back
                // to the cursor, so it returns early (skipping `scroll_to_cursor`).
                self.scroll_by(*rows, ctx);
                ctx.notify();
                return;
            }
        }

        let total_rows = self.total_visual_rows(ctx);
        let visible_rows = cmp::min(total_rows, self.max_visible_rows);
        self.scroll_to_cursor(total_rows, visible_rows.max(1), ctx);
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

    /// The terminal width (in cells) for char-cell layout, read from the backing
    /// model. The model is the single source of truth — it must hold the width
    /// for event-time navigation/scroll, so the view keeps no separate copy.
    fn terminal_width(&self, ctx: &AppContext) -> u16 {
        self.model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .char_cell()
            .map(|cc| cc.terminal_width())
            .unwrap_or(0)
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

    pub fn visual_line_count(&self, ctx: &AppContext) -> u32 {
        self.model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .max_line()
            .as_u32()
            .max(1)
    }

    /// The cursor's visual row in the char-cell soft-wrap map (0-based).
    fn cursor_visual_row(&self, ctx: &AppContext) -> u32 {
        let cursor_offset = self.cursor_offset(ctx);
        let inner = self.model.as_ref(ctx);
        let render = inner.render_state().as_ref(ctx);
        // `offset_to_softwrap_point` is 0-based (see `char_cell_offset_to_softwrap_point`),
        // while the cursor is a 1-based `CharOffset`, so convert by subtracting 1.
        let cursor_char_index = CharOffset::from(cursor_offset.as_usize().saturating_sub(1));
        render.offset_to_softwrap_point(cursor_char_index).row()
    }

    /// Total visual rows the input occupies, including the "phantom" row the
    /// cursor wraps onto when a logical line exactly fills the terminal width
    /// (deferred wrap). [`Self::visual_line_count`] never counts that row — the
    /// soft-wrap map only adds a row once a character actually overflows — but
    /// the cursor legitimately sits there, so sizing and scrolling must include
    /// it or the viewport scrolls to a row that is never rendered.
    fn total_visual_rows(&self, ctx: &AppContext) -> u32 {
        self.visual_line_count(ctx)
            .max(self.cursor_visual_row(ctx) + 1)
    }

    // ── Scroll ────────────────────────────────────────────────────────────────────

    fn scroll_to_cursor(&mut self, total_rows: u32, visible_rows: u32, ctx: &AppContext) {
        let cursor_row = self.cursor_visual_row(ctx);

        // A stale offset can point past the last remaining row (e.g. after a
        // deletion shrank the content); clamp it so the visible window always
        // overlaps real rows before following the cursor.
        self.scroll_offset = self
            .scroll_offset
            .min(total_rows.saturating_sub(visible_rows));

        if cursor_row < self.scroll_offset {
            self.scroll_offset = cursor_row;
        } else if cursor_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = cursor_row.saturating_sub(visible_rows - 1);
        }
    }

    /// Scrolls the viewport by `rows` visual rows (negative scrolls toward the
    /// top), clamped to `[0, max_scroll]`. Independent of the cursor, so callers
    /// must not follow it with `scroll_to_cursor`.
    fn scroll_by(&mut self, rows: isize, ctx: &AppContext) {
        let lines = self.total_visual_rows(ctx);
        let visible_rows = cmp::min(lines, self.max_visible_rows).max(1);
        let max_scroll = lines.saturating_sub(visible_rows) as isize;
        let new_offset = (self.scroll_offset as isize + rows).clamp(0, max_scroll);
        self.scroll_offset = new_offset as u32;
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

    /// Restores the TUI's default agent input mode; any typed text is
    /// preserved. Also called by the session view after an accepted shell
    /// submission clears the input.
    pub(crate) fn exit_shell_mode(&mut self, ctx: &mut ViewContext<Self>) {
        let is_input_buffer_empty = self.plain_text(ctx).is_empty();
        self.input_mode.clone().update(ctx, |input_mode, ctx| {
            input_mode.set_input_config(AI_LOCKED_CONFIG, is_input_buffer_empty, None, ctx);
        });
    }

    // ── Submit ────────────────────────────────────────────────────────────────

    /// Emits [`TuiInputViewEvent::Submitted`] without clearing the buffer; the
    /// owner decides whether the submission is accepted and calls [`Self::clear`].
    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.plain_text(ctx);
        ctx.emit(TuiInputViewEvent::Submitted(text));
    }

    // ── Kill / yank ───────────────────────────────────────────────────────────

    fn kill_to_line_end(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(range) = self.range_to_visual_line_end(ctx) {
            let killed = self
                .model
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.model.update(ctx, |inner, ctx| {
                use warp_editor::content::buffer::{BufferEditAction, EditOrigin};
                inner.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                            BufferEditAction::Delete(vec1::vec1![range]),
                            EditOrigin::UserInitiated,
                            inner.buffer_selection_model().clone(),
                            ctx,
                        );
                    },
                    ctx,
                );
            });
        }
    }

    fn kill_to_line_start(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(range) = self.range_from_visual_line_start(ctx) {
            let killed = self
                .model
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.model.update(ctx, |inner, ctx| {
                use warp_editor::content::buffer::{BufferEditAction, EditOrigin};
                inner.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                            BufferEditAction::Delete(vec1::vec1![range]),
                            EditOrigin::UserInitiated,
                            inner.buffer_selection_model().clone(),
                            ctx,
                        );
                    },
                    ctx,
                );
            });
        }
    }

    fn yank(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(text) = self.kill_buffer.yank().map(str::to_owned) {
            self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
        }
    }

    // ── Kill range helpers ────────────────────────────────────────────────────
    //
    // These compute ranges purely from the plain-text string using the same
    // char-cell soft-wrap arithmetic as `build_visual_rows`, avoiding any
    // dependency on the render-state softwrap API (which uses 0-indexed offsets
    // that differ from the buffer's 1-indexed `CharOffset` convention).

    fn range_to_visual_line_end(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let text = self.plain_text(ctx);
        // `cursor_offset` is a 1-indexed gap position (gap 1 sits before the
        // first character); the buffer's `text_in_range` / `Delete` use those
        // same coordinates, so the kill range starts exactly at `cursor_gap`.
        let cursor_gap = self.cursor_offset(ctx).as_usize();
        // 0-based index of the character at the cursor, for the pure helpers.
        let cursor_idx = cursor_gap.saturating_sub(1);
        let end_idx = visual_line_end_exclusive(&text, cursor_idx, self.terminal_width(ctx));
        if end_idx <= cursor_idx {
            return None;
        }
        // `text[i]` lives at gap `i + 1`, so the exclusive end gap is `end_idx + 1`.
        Some(CharOffset::from(cursor_gap)..CharOffset::from(end_idx + 1))
    }

    fn range_from_visual_line_start(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let text = self.plain_text(ctx);
        let cursor_gap = self.cursor_offset(ctx).as_usize();
        let cursor_idx = cursor_gap.saturating_sub(1);
        let start_idx = visual_line_start_idx(&text, cursor_idx, self.terminal_width(ctx));
        if start_idx >= cursor_idx {
            return None;
        }
        Some(CharOffset::from(start_idx + 1)..CharOffset::from(cursor_gap))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Kill range pure-text helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The `(logical_line_start, logical_line_end)` char indices bounding the
/// logical line that contains `cursor_idx` (end excludes the trailing `\n`).
fn logical_line_bounds(chars: &[char], cursor_idx: usize) -> (usize, usize) {
    let ci = cursor_idx.min(chars.len());
    let start = chars[..ci]
        .iter()
        .rposition(|&c| c == '\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let end = chars[ci..]
        .iter()
        .position(|&c| c == '\n')
        .map(|p| ci + p)
        .unwrap_or(chars.len());
    (start, end)
}

/// For the logical line containing `cursor_idx`, returns
/// `(logical_line_start, pos_in_line, row_starts, line_len)` using width-aware
/// wrapping. `row_starts` are 0-based char indices within the line where each
/// visual row begins; `line_len` is the line's char count (excluding the
/// trailing `\n`). Shared by the two kill-range helpers below.
fn visual_line_segments(
    text: &str,
    cursor_idx: usize,
    terminal_width: u16,
) -> (usize, usize, Vec<usize>, usize) {
    let chars: Vec<char> = text.chars().collect();
    let (logical_line_start, logical_line_end) = logical_line_bounds(&chars, cursor_idx);
    let line_widths: Vec<u8> = chars[logical_line_start..logical_line_end]
        .iter()
        .map(|&c| char_cell_display_width(c) as u8)
        .collect();
    let row_starts = char_cell_line_row_starts(&line_widths, terminal_width);
    let pos_in_line = cursor_idx
        .min(chars.len())
        .saturating_sub(logical_line_start);
    (
        logical_line_start,
        pos_in_line,
        row_starts,
        line_widths.len(),
    )
}

/// Returns the 0-based exclusive end index of the visual line segment at
/// `cursor_idx`. Does NOT include any trailing `\n`. Width-aware: the segment
/// boundaries follow the same display-width wrapping as the rendered rows.
fn visual_line_end_exclusive(text: &str, cursor_idx: usize, terminal_width: u16) -> usize {
    let (logical_line_start, pos_in_line, row_starts, line_len) =
        visual_line_segments(text, cursor_idx, terminal_width);
    let row = row_starts
        .partition_point(|&s| s <= pos_in_line)
        .saturating_sub(1);
    // Exclusive end = start of the next visual row, or the logical line's end
    // (excluding the '\n') for the final row.
    let seg_end_in_line = row_starts.get(row + 1).copied().unwrap_or(line_len);
    logical_line_start + seg_end_in_line
}

/// Returns the 0-based start index of the visual line segment at `cursor_idx`.
/// Width-aware (see [`visual_line_end_exclusive`]).
fn visual_line_start_idx(text: &str, cursor_idx: usize, terminal_width: u16) -> usize {
    let (logical_line_start, pos_in_line, row_starts, _line_len) =
        visual_line_segments(text, cursor_idx, terminal_width);
    let row = row_starts
        .partition_point(|&s| s <= pos_in_line)
        .saturating_sub(1);
    logical_line_start + row_starts[row]
}

// ─────────────────────────────────────────────────────────────────────────────
// TuiInputElement — element returned from render()
// ─────────────────────────────────────────────────────────────────────────────

/// The element returned from [`TuiInputView::render`].
///
/// `render` captures only width-independent state; the row wrapping, cursor
/// placement, and selection spans are computed in [`TuiElement::layout`], the
/// first point that knows the terminal width (from the layout constraint). This
/// mirrors the GUI, where the element computes geometry in `layout`.
struct TuiInputElement {
    /// Backing model: used during layout to push the terminal width and read the
    /// char-cell visual line count at that width.
    model: ModelHandle<CodeEditorModel>,
    /// Plain-text content captured at render time.
    text: String,
    /// Cursor gap offset (1-based) captured at render time.
    cursor_offset: CharOffset,
    /// Selection as 0-based `(start, end)` char indices, if any.
    sel_char_range: Option<(usize, usize)>,
    /// First visible visual row (0-indexed).
    scroll_offset: u32,
    /// Maximum number of visible rows before the input scrolls.
    max_visible_rows: u32,
    /// Whether a mouse drag-selection is in progress (captured from the view at
    /// render time); gates drag/up handling in `dispatch_event`.
    is_selecting: bool,
    /// Visible rows, built during `layout`.
    column: TuiFlex,
    /// The cursor's 0-based column within the visible area (set during `layout`).
    cursor_col: u16,
    /// The cursor's 0-based row within the visible area (set during `layout`).
    cursor_row_in_view: u16,
    /// Whether the cursor's visual row falls within the scrolled viewport; when
    /// false (e.g. after wheel-scrolling away) no terminal cursor is drawn.
    cursor_visible: bool,
    /// Selected spans `(row_in_view, start_col, exclusive_end_col)` (set during `layout`).
    selected_spans: Vec<(u16, u16, u16)>,
}

impl TuiInputElement {
    /// Captures `view`'s width-independent state for this frame; width-dependent
    /// layout (row wrapping, cursor placement, selection spans) happens later in
    /// [`TuiElement::layout`], the first point that knows the terminal width.
    fn new(view: &TuiInputView, ctx: &AppContext) -> Self {
        let text = view.plain_text(ctx);
        let cursor_offset = view.cursor_offset(ctx);
        let sel_char_range = view.selection_range(ctx).map(|r| {
            let start = r.start.as_usize().saturating_sub(1);
            let end = r.end.as_usize().saturating_sub(1);
            (start, end)
        });

        Self {
            model: view.model.clone(),
            text,
            cursor_offset,
            sel_char_range,
            scroll_offset: view.scroll_offset,
            max_visible_rows: view.max_visible_rows,
            is_selecting: view.is_selecting,
            column: TuiFlex::column(),
            cursor_col: 0,
            cursor_row_in_view: 0,
            cursor_visible: false,
            selected_spans: Vec::new(),
        }
    }

    /// Builds the visible rows, cursor position, and selection spans for
    /// `terminal_width`, storing them for `render`/`cursor_position`.
    fn build(&mut self, terminal_width: u16, visible_rows: u32) {
        let (cursor_visual_row, cursor_col) =
            char_cell_cursor_pos(&self.text, self.cursor_offset, terminal_width);
        let cursor_row_in_view = cursor_visual_row.saturating_sub(self.scroll_offset);
        let cursor_visible = cursor_visual_row >= self.scroll_offset
            && cursor_visual_row < self.scroll_offset + visible_rows;

        let rows_with_offsets = build_visual_rows_with_offsets(&self.text, terminal_width);
        // The cursor sits one row past the last text row when a logical line
        // exactly fills the width (deferred wrap); that phantom row is part of
        // the layout, so include it when windowing the visible rows.
        let total_rows = rows_with_offsets.len().max(cursor_visual_row as usize + 1);
        let visible_start = self.scroll_offset as usize;
        let visible_end = (visible_start + visible_rows as usize).min(total_rows);
        let text_rows_end = visible_end.min(rows_with_offsets.len());
        let mut visible_rows_slice: Vec<(String, usize)> = if visible_start < text_rows_end {
            rows_with_offsets[visible_start..text_rows_end].to_vec()
        } else {
            Vec::new()
        };
        // Pad any phantom rows in the window with empty text at the buffer's
        // end offset, so the cursor's row renders and selection math stays
        // in bounds.
        let end_char_offset = self.text.chars().count();
        while visible_rows_slice.len() < visible_end.saturating_sub(visible_start) {
            visible_rows_slice.push((String::new(), end_char_offset));
        }
        if visible_rows_slice.is_empty() {
            visible_rows_slice.push((String::new(), 0));
        }

        // Selection spans per visible row. Selection offsets are char indices;
        // terminal highlighting works in display columns, so convert via each
        // char's display width (wide chars span two columns).
        let mut selected_spans: Vec<(u16, u16, u16)> = Vec::new();
        if let Some((sel_start, sel_end)) = self.sel_char_range {
            if sel_start < sel_end {
                for (vis_idx, (row_text, row_char_start)) in visible_rows_slice.iter().enumerate() {
                    let row_chars: Vec<char> = row_text.chars().collect();
                    let row_len = row_chars.len();
                    let row_char_end = row_char_start + row_len;
                    if sel_end > *row_char_start && sel_start < row_char_end {
                        let start_char = sel_start.saturating_sub(*row_char_start);
                        let end_char = (sel_end - row_char_start).min(row_len);
                        let disp_start: usize = row_chars[..start_char]
                            .iter()
                            .map(|&c| char_cell_display_width(c))
                            .sum();
                        let disp_end: usize = row_chars[..end_char]
                            .iter()
                            .map(|&c| char_cell_display_width(c))
                            .sum();
                        selected_spans.push((vis_idx as u16, disp_start as u16, disp_end as u16));
                    }
                }
            }
        }

        let mut column = TuiFlex::column();
        for (row_text, _) in &visible_rows_slice {
            // An empty `TuiText` lays out to zero rows, which would collapse the
            // row and clip the cursor (or following rows) off the column. Render
            // a single space so every visual row keeps a height of exactly one.
            let row_display = if row_text.is_empty() {
                " ".to_string()
            } else {
                row_text.clone()
            };
            column = column.with_child(Box::new(TuiText::new(row_display).truncate()));
        }

        self.column = column;
        self.cursor_col = cursor_col as u16;
        self.cursor_row_in_view = cursor_row_in_view as u16;
        self.cursor_visible = cursor_visible;
        self.selected_spans = selected_spans;
    }

    /// Maps a terminal cell `position` to the 1-based buffer [`CharOffset`] under
    /// it (the gap the cursor should move to for a click/drag at that cell).
    ///
    /// The mouse reports an absolute terminal cell, so getting to a buffer offset
    /// crosses three coordinate spaces:
    ///   1. screen cell (`position`) -> row/col relative to the input's `area`,
    ///   2. + `scroll_offset` -> the buffer's *visual* row (undoes scrolling),
    ///   3. (visual row, col) -> char offset via the char-cell soft-wrap map.
    ///
    /// Points outside the input's vertical bounds are intentionally *not* clamped
    /// to the viewport: a point above the input maps toward row 0 and a point
    /// below it maps past the last visible row (bounded by the buffer's last
    /// visual row), so a drag that leaves the input drives auto-scroll.
    fn offset_at(&self, position: TuiPoint, area: TuiRect, app: &AppContext) -> CharOffset {
        let inner = self.model.as_ref(app);
        let render = inner.render_state().as_ref(app);

        // Step 1: row of the pointer within the input, where 0 is the input's top
        // row. Signed so a point *above* the input (`position.y < area.y`) stays
        // negative instead of wrapping around at 0.
        let row_in_view = i64::from(position.y) - i64::from(area.y);
        // Step 2: the input shows buffer visual rows starting at `scroll_offset`,
        // so the buffer row under the pointer is `scroll_offset + row_in_view`
        // (floored at the first row).
        let visual_row = (i64::from(self.scroll_offset) + row_in_view).max(0) as u32;
        // The rendered layout can include a "phantom" row one past the soft-wrap
        // map's last row when the final logical line exactly fills the terminal
        // width (deferred wrap; see `build`). The map has no entry for it —
        // every cell on it is the end-of-buffer gap — so resolve it directly
        // instead of clamping into the preceding real row (which would map the
        // click near that row's start).
        let last_row = render.max_line().as_u32().max(1).saturating_sub(1);
        let end_char_count = self.text.chars().count();
        let end_gap_row = render
            .offset_to_softwrap_point(CharOffset::from(end_char_count))
            .row();
        if visual_row > last_row && end_gap_row > last_row {
            return CharOffset::from(end_char_count + 1);
        }
        // ...otherwise cap at the last real visual row, so a drag below the
        // text resolves to the buffer's end rather than past it.
        let visual_row = visual_row.min(last_row);

        // Column within that row, in display cells (0 is the input's left edge;
        // drags left of it clamp to column 0).
        let col = position.x.saturating_sub(area.x);

        // Step 3: resolve (visual_row, col) to a char offset. The soft-wrap map
        // clamps the column to the row's end and is 0-based, while the buffer
        // uses 1-based gap offsets (see the kill-range helpers), so re-add 1.
        let point = SoftWrapPoint::new(visual_row, ColumnUnit::Chars(col));
        let zero_based = render.softwrap_point_to_offset(point);
        CharOffset::from(zero_based.as_usize() + 1)
    }

    /// Maps a mouse `event` to the [`TuiInputAction`] it should dispatch, or
    /// `None` when the event should be ignored (a press outside the input, a
    /// drag/up with no selection in progress, or a non-mouse event).
    ///
    /// Mirrors the GUI's `left_mouse_down`/`dragged`/`up` mapping: click count 1
    /// starts a selection (shift extends), 2 selects a word, 3 selects a line;
    /// drag updates the pending selection and up ends it.
    fn mouse_action(
        &self,
        event: &TuiEvent,
        area: TuiRect,
        app: &AppContext,
    ) -> Option<TuiInputAction> {
        match event {
            TuiEvent::LeftMouseDown {
                position,
                modifiers,
                click_count,
                is_first_mouse,
            } => {
                // The focus-bringing first click has no matching mouse-up, and a
                // press outside the input must not start a selection.
                if *is_first_mouse || !area.contains_point(*position) {
                    return None;
                }
                let offset = self.offset_at(*position, area, app);
                Some(match *click_count {
                    0 | 1 if modifiers.shift => TuiInputAction::SelectionExtendTo { offset },
                    0 | 1 => TuiInputAction::SelectionStartAt { offset },
                    2 => TuiInputAction::SelectWordAt { offset },
                    _ => TuiInputAction::SelectLineAt { offset },
                })
            }
            // Drags continue even outside the input's bounds (drag-to-scroll),
            // but only while a selection that began inside it is active.
            TuiEvent::LeftMouseDragged { position, .. } if self.is_selecting => {
                Some(TuiInputAction::SelectionUpdateTo {
                    offset: self.offset_at(*position, area, app),
                })
            }
            TuiEvent::LeftMouseUp { .. } if self.is_selecting => Some(TuiInputAction::SelectionEnd),
            // Mouse wheel over the input scrolls the viewport (cursor unmoved).
            TuiEvent::ScrollWheel {
                position, delta, ..
            } if area.contains_point(*position) => Some(TuiInputAction::Scroll {
                // crossterm reports ScrollUp as +1 row / ScrollDown as -1; negate
                // so wheel-up scrolls toward the top (matches `TuiScrollable`).
                rows: -(delta.1 * WHEEL_STEP),
            }),
            _ => None,
        }
    }
}

impl TuiElement for TuiInputElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        // The layout constraint is the first place the real editor width is
        // known (in shell mode the enclosing row has already reserved the `!`
        // gutter, so the constraint is the editable width). Push that width
        // onto the model (interior-mutable) so event-time navigation/scroll
        // read the right width, then build the rows at that width — mirroring
        // how the GUI computes geometry during layout.
        let editor_width = constraint.constrain_width(constraint.max.width);
        let render_state = self.model.as_ref(app).render_state().clone();
        if let Some(cc) = render_state.as_ref(app).char_cell() {
            cc.set_terminal_width(editor_width);
        }
        let visual_line_count = render_state.as_ref(app).max_line().as_u32().max(1);
        // Include the "phantom" row the cursor wraps onto when a logical line
        // exactly fills the width (deferred wrap): `max_line` doesn't count it,
        // but the input must grow so the cursor's row is rendered.
        let (cursor_row, _) = char_cell_cursor_pos(&self.text, self.cursor_offset, editor_width);
        let total_rows = visual_line_count.max(cursor_row + 1);
        let visible_rows = cmp::min(total_rows, self.max_visible_rows);

        self.build(editor_width, visible_rows);
        let content_size = self.column.layout(
            TuiConstraint::loose(TuiSize::new(editor_width, constraint.max.height)),
            ctx,
            app,
        );
        // The editor claims the full width it was offered (its wrap width),
        // not just the longest row's width the content-sized column reports.
        // Both components are already within `constraint`.
        TuiSize::new(editor_width, content_size.height)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        self.column.render(area, buffer, ctx);
        if !self.selected_spans.is_empty() {
            let reversed = TuiStyle::default().add_modifier(Modifier::REVERSED);
            for &(row_in_view, start_col, end_col) in &self.selected_spans {
                let y = area.y.saturating_add(row_in_view);
                let x = area.x.saturating_add(start_col);
                let width = end_col.saturating_sub(start_col);
                if y < area.y + area.height && width > 0 {
                    let sel_rect =
                        TuiRect::new(x, y, width.min(area.width.saturating_sub(start_col)), 1);
                    buffer.set_style(sel_rect, reversed);
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        if !self.cursor_visible
            || self.cursor_col >= area.width
            || self.cursor_row_in_view >= area.height
        {
            return None;
        }
        Some((self.cursor_col, self.cursor_row_in_view))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if self.column.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        if let Some(action) = self.mouse_action(event, area, app) {
            event_ctx.dispatch_typed_action(action);
            return true;
        }

        if let TuiEvent::KeyDown {
            keystroke, chars, ..
        } = event
        {
            // The chorded editing commands (movement, deletion, kill/yank,
            // undo/redo, …) are dispatched by the keymap pass via the
            // `tui:input:*` bindings registered in [`TuiInputView::init`],
            // which runs before the element pass ever sees the key — including
            // the shell-mode Esc, whose binding is gated on the view's
            // shell-mode keymap-context flag. Only printable-character
            // insertion stays element-level — text insertion is not a
            // keybinding, matching the GUI.
            if !keystroke.ctrl && !keystroke.alt && !chars.is_empty() {
                if let Some(char) = chars.chars().next() {
                    event_ctx.dispatch_typed_action(TuiInputAction::InsertChar(char));
                    return true;
                }
            }
        }

        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Char-cell helpers (pure functions, no model dependency)
// ─────────────────────────────────────────────────────────────────────────────

/// Splits `text` into visual rows, returning `(row_text, char_start_offset)` pairs.
///
/// Wrapping is display-width aware (wide CJK/emoji span two columns, combining
/// marks zero), sharing the same wrapping rule as the editor's char-cell layout
/// via [`char_cell_line_row_starts`].
pub fn build_visual_rows_with_offsets(text: &str, terminal_width: u16) -> Vec<(String, usize)> {
    let mut rows: Vec<(String, usize)> = Vec::new();
    let mut global_char_offset: usize = 0;

    for logical_line in text.split('\n') {
        let line_char_start = global_char_offset;
        let chars: Vec<char> = logical_line.chars().collect();
        let line_len = chars.len();
        let widths: Vec<u8> = chars
            .iter()
            .map(|&c| char_cell_display_width(c) as u8)
            .collect();
        let row_starts = char_cell_line_row_starts(&widths, terminal_width);
        for r in 0..row_starts.len() {
            let start = row_starts[r];
            let end = row_starts.get(r + 1).copied().unwrap_or(line_len);
            rows.push((chars[start..end].iter().collect(), line_char_start + start));
        }
        global_char_offset += line_len + 1;
    }

    if rows.is_empty() {
        rows.push((String::new(), 0));
    }
    rows
}

/// Splits `text` into visual rows, display-width aware. Thin wrapper over
/// [`build_visual_rows_with_offsets`] that drops the per-row char offsets.
pub fn build_visual_rows(text: &str, terminal_width: u16) -> Vec<String> {
    build_visual_rows_with_offsets(text, terminal_width)
        .into_iter()
        .map(|(row_text, _)| row_text)
        .collect()
}

/// Returns `(visual_row, visual_col)` for `cursor_offset` in char-cell
/// coordinates, where `visual_col` is a display column (wide chars span two).
pub fn char_cell_cursor_pos(
    text: &str,
    cursor_offset: CharOffset,
    terminal_width: u16,
) -> (u32, u32) {
    let cursor_char_idx = cursor_offset.as_usize().saturating_sub(1);
    let mut visual_row: u32 = 0;
    let mut chars_so_far: usize = 0;

    for logical_line in text.split('\n') {
        let chars: Vec<char> = logical_line.chars().collect();
        let line_len = chars.len();
        let widths: Vec<u8> = chars
            .iter()
            .map(|&c| char_cell_display_width(c) as u8)
            .collect();
        let line_end_exclusive = chars_so_far + line_len;

        if cursor_char_idx <= line_end_exclusive {
            let char_in_line = cursor_char_idx.saturating_sub(chars_so_far);
            let (row_in_line, col) =
                char_cell_line_gap_position(&widths, terminal_width, char_in_line);
            return (visual_row + row_in_line, col as u32);
        }

        visual_row += char_cell_line_row_starts(&widths, terminal_width).len() as u32;
        chars_so_far = line_end_exclusive + 1;
    }

    (visual_row, 0)
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
