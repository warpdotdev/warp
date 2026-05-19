#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]
// Adding this file level gate as some of the code around editability is not used in WASM yet.

use crate::code::editor::{
    line::EditorLineLocation,
    model::CodeEditorModel,
    view::{CodeEditorEvent, CodeEditorView, VimMode},
};
use crate::{
    cmd_or_ctrl_shift, code_review::comments::CommentId,
    code_review::telemetry_event::CodeReviewTelemetryEvent, editor::InteractionState,
    features::FeatureFlag, notebooks::editor::model::word_unit, send_telemetry_from_ctx,
    util::bindings::CustomAction,
};
use lazy_static::lazy_static;
use rangemap::RangeSet;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::ops::Range;
use string_offset::CharOffset;
use warp_editor::{
    content::version::BufferVersion,
    editor::{EmbeddedItemModel, RunnableCommandModel, TextDecoration},
    model::{CoreEditorModel, PlainTextEditorModel},
    render::{
        element::RichTextAction,
        model::{ExpansionType, LineCount, Location},
    },
    selection::{TextDirection, TextUnit},
};
use warp_util::user_input::UserInput;
use warpui::{
    actions::StandardAction,
    elements::Axis,
    event::ModifiersState,
    keymap::{EditableBinding, FixedBinding, Keystroke, PerPlatformKeystroke},
    units::Pixels,
    AppContext, TypedActionView, ViewContext, WeakViewHandle,
};

/// Limit the keybindings that conflict with the Agent Mode embedded editor.
const NON_EDITABLE_KEYMAP_CONTEXT: &str = "NonEditableKeymapContext";

lazy_static! {
    static ref AUTOCOMPLETE_SYMBOLS: HashMap<char, char> =
        HashMap::from([('(', ')'), ('[', ']'), ('{', '}'), ('\'', '\''), ('"', '"'),]);
    static ref CLOSING_SYMBOLS: HashSet<char> = AUTOCOMPLETE_SYMBOLS.values().cloned().collect();
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    let text_entry = id!("CodeEditorView") & !id!("IMEOpen");
    // We use this to disable some keybindings that would conflict with the Agent Mode embedded editor.
    let editable_state = text_entry.clone() & !id!(NON_EDITABLE_KEYMAP_CONTEXT);
    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            CodeEditorViewAction::Enter,
            editable_state.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "enter",
            CodeEditorViewAction::VimEnter,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "numpadenter",
            CodeEditorViewAction::Enter,
            text_entry.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "numpadenter",
            CodeEditorViewAction::VimEnter,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "shift-enter",
            CodeEditorViewAction::Enter,
            text_entry.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "shift-enter",
            CodeEditorViewAction::VimShiftEnter,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "backspace",
            CodeEditorViewAction::Backspace,
            text_entry.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "backspace",
            CodeEditorViewAction::VimBackspace,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "shift-backspace",
            CodeEditorViewAction::Backspace,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-backspace",
            CodeEditorViewAction::VimBackspace,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "delete",
            CodeEditorViewAction::Delete,
            text_entry.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "delete",
            CodeEditorViewAction::VimDelete,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "shift-up",
            CodeEditorViewAction::SelectUp,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-down",
            CodeEditorViewAction::SelectDown,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-left",
            CodeEditorViewAction::SelectLeft,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-right",
            CodeEditorViewAction::SelectRight,
            text_entry.clone(),
        ),
        FixedBinding::new("up", CodeEditorViewAction::MoveUp, editable_state.clone()),
        FixedBinding::new(
            "down",
            CodeEditorViewAction::MoveDown,
            editable_state.clone(),
        ),
        FixedBinding::new(
            "left",
            CodeEditorViewAction::MoveLeft,
            editable_state.clone(),
        ),
        FixedBinding::new(
            "right",
            CodeEditorViewAction::MoveRight,
            editable_state.clone(),
        ),
        FixedBinding::new(
            "home",
            CodeEditorViewAction::MoveToLineStart,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "end",
            CodeEditorViewAction::MoveToLineEnd,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "tab",
            CodeEditorViewAction::Tab,
            text_entry.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "tab",
            CodeEditorViewAction::VimTab,
            text_entry.clone() & id!("Vim"),
        ),
        FixedBinding::new(
            "shift-tab",
            CodeEditorViewAction::ShiftTab,
            text_entry.clone() & !id!("Vim"),
        ),
        FixedBinding::new(
            "shift-tab",
            CodeEditorViewAction::VimShiftTab,
            text_entry.clone() & id!("Vim"),
        ),
        // Also create the word movement shortcuts with `meta` in place of `alt`, to accommodate
        // the "Left Option is Meta" and "Right Option is Meta" settings.
        FixedBinding::new(
            "meta-left",
            CodeEditorViewAction::MoveBackwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "meta-right",
            CodeEditorViewAction::MoveForwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "shift-alt-left",
                linux_and_windows: "shift-ctrl-left",
            },
            CodeEditorViewAction::SelectBackwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-meta-left",
            CodeEditorViewAction::SelectBackwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "shift-alt-right",
                linux_and_windows: "shift-ctrl-right",
            },
            CodeEditorViewAction::SelectForwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-meta-right",
            CodeEditorViewAction::SelectForwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-home",
            CodeEditorViewAction::SelectToLineStart,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-end",
            CodeEditorViewAction::SelectToLineEnd,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "cmdorctrl-a",
            CodeEditorViewAction::SelectAll,
            text_entry.clone(),
        ),
        // TODO(kevin): Only for testing purposes.
        FixedBinding::new(
            "cmdorctrl-shift-X",
            CodeEditorViewAction::ToggleDiffNav(None),
            text_entry.clone(),
        ),
        FixedBinding::custom(
            CustomAction::Copy,
            CodeEditorViewAction::Copy,
            "Copy",
            text_entry.clone(),
        ),
        // Bindings for paste require the StandardAction and CustomAction binding to work on all platforms.
        FixedBinding::custom(
            CustomAction::Paste,
            CodeEditorViewAction::Paste,
            "Paste",
            text_entry.clone(),
        ),
        FixedBinding::standard(
            StandardAction::Paste,
            CodeEditorViewAction::Paste,
            text_entry.clone(),
        ),
        #[cfg(windows)]
        FixedBinding::custom(
            CustomAction::WindowsPaste,
            CodeEditorViewAction::Paste,
            "Paste",
            text_entry.clone(),
        ),
        #[cfg(windows)]
        FixedBinding::custom(
            CustomAction::WindowsCopy,
            CodeEditorViewAction::WindowsCtrlC,
            "Copy",
            text_entry.clone(),
        ),
        FixedBinding::custom(
            CustomAction::Cut,
            CodeEditorViewAction::Cut,
            "Cut",
            text_entry.clone(),
        ),
        FixedBinding::custom(
            CustomAction::Undo,
            CodeEditorViewAction::Undo,
            "Undo",
            text_entry.clone(),
        ),
        FixedBinding::custom(
            CustomAction::Redo,
            CodeEditorViewAction::Redo,
            "Redo",
            text_entry.clone(),
        ),
        FixedBinding::new("escape", CodeEditorViewAction::Escape, text_entry.clone()),
    ]);

    // Bind Ctrl-R to Redo when in Vim normal mode.
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-r",
        CodeEditorViewAction::Redo,
        text_entry.clone() & id!("VimNormalMode"),
    )]);

    // Editable navigation keybindings:
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:move_backward_one_word",
            "Move Backward One Word",
            CodeEditorViewAction::MoveBackwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-left")
        .with_linux_or_windows_key_binding("ctrl-left"),
        EditableBinding::new(
            "editor_view:move_forward_one_word",
            "Move Forward One Word",
            CodeEditorViewAction::MoveForwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-right")
        .with_linux_or_windows_key_binding("ctrl-right"),
        EditableBinding::new(
            "editor_view:move_forward_one_word",
            "Move forward one word",
            CodeEditorViewAction::MoveForwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("meta-f"),
        EditableBinding::new(
            "editor_view:move_backward_one_word",
            "Move backward one word",
            CodeEditorViewAction::MoveBackwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("meta-b"),
        EditableBinding::new(
            "editor_view:up",
            "Move cursor up",
            CodeEditorViewAction::MoveUp,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-p"),
        EditableBinding::new(
            "editor_view:down",
            "Move cursor down",
            CodeEditorViewAction::MoveDown,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-n"),
        EditableBinding::new(
            "editor_view:left",
            "Move cursor left",
            CodeEditorViewAction::MoveLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-b"),
        EditableBinding::new(
            "editor_view:right",
            "Move cursor right",
            CodeEditorViewAction::MoveRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-f"),
        EditableBinding::new(
            "editor_view:move_to_line_start",
            "Move to line start",
            CodeEditorViewAction::MoveToLineStart,
        )
        .with_context_predicate(text_entry.clone())
        // Mac-only to not conflict with SelectAll on Linux and Windows.
        .with_mac_key_binding("ctrl-a"),
        EditableBinding::new(
            "editor_view:home",
            "Home",
            CodeEditorViewAction::MoveToLineStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-left")
        .with_linux_or_windows_key_binding("home"),
        EditableBinding::new(
            "editor_view:move_to_line_end",
            "Move to line end",
            CodeEditorViewAction::MoveToLineEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("ctrl-e"),
        EditableBinding::new(
            "editor_view:end",
            "End",
            CodeEditorViewAction::MoveToLineEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-right")
        .with_linux_or_windows_key_binding("end"),
        EditableBinding::new(
            "editor_view:cursor_at_buffer_start",
            "Cursor at buffer start",
            CodeEditorViewAction::CursorAtBufferStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-up"),
        EditableBinding::new(
            "editor_view:cursor_at_buffer_end",
            "Cursor at buffer end",
            CodeEditorViewAction::CursorAtBufferEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-down"),
    ]);

    // Editable selection keybindings:
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:select_left_by_word",
            "Select one word to the left",
            CodeEditorViewAction::SelectBackwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-meta-B"),
        EditableBinding::new(
            "editor_view:select_right_by_word",
            "Select one word to the right",
            CodeEditorViewAction::SelectForwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-meta-F"),
        EditableBinding::new(
            "editor_view:select_left",
            "Select one character to the left",
            CodeEditorViewAction::SelectLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-ctrl-B"),
        EditableBinding::new(
            "editor_view:select_right",
            "Select one character to the right",
            CodeEditorViewAction::SelectRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-ctrl-F"),
        EditableBinding::new(
            "editor_view:select_up",
            "Select up",
            CodeEditorViewAction::SelectUp,
        )
        .with_context_predicate(text_entry.clone())
        // Set this to Mac only since otherwise it could conflict with opening the command
        // palette. NOTE `shift-up` still exists as a cross platform keybinding for this action.
        .with_mac_key_binding("shift-ctrl-P"),
        EditableBinding::new(
            "editor_view:select_down",
            "Select down",
            CodeEditorViewAction::SelectDown,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("shift-ctrl-N"),
        EditableBinding::new(
            "editor_view:select_all",
            "Select all",
            CodeEditorViewAction::SelectAll,
        )
        .with_context_predicate(text_entry.clone())
        .with_custom_action(CustomAction::SelectAll),
        EditableBinding::new(
            "editor:select_to_line_start",
            "Select to start of line",
            CodeEditorViewAction::SelectToLineStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("shift-ctrl-A"),
        EditableBinding::new(
            "editor:select_to_line_end",
            "Select to end of line",
            CodeEditorViewAction::SelectToLineEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("shift-ctrl-E"),
        // `shift-end` is registered on all platforms for this action.
        EditableBinding::new(
            "editor_view:select_to_line_end",
            "Select To Line End",
            CodeEditorViewAction::SelectToLineEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-shift-right"),
        // `end` is registered on all platforms for this action.
        EditableBinding::new(
            "editor_view:select_to_line_start",
            "Select To Line Start",
            CodeEditorViewAction::SelectToLineStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-shift-left"),
    ]);

    // Editable text-manipulation bindings
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:backspace",
            "Remove the previous character",
            CodeEditorViewAction::Backspace,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-h"),
        EditableBinding::new(
            "editor_view:toggle_comment",
            "Toggle comment",
            CodeEditorViewAction::ToggleComment,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-/"),
        EditableBinding::new("editor_view:delete", "Delete", CodeEditorViewAction::Delete)
            .with_context_predicate(
                text_entry.clone() & !id!("VimNormalMode") & !id!("VimVisualMode"),
            )
            .with_key_binding("ctrl-d"),
        EditableBinding::new(
            "editor_view:vim_scroll_half_page_down",
            "Scroll down half a page (vim)",
            CodeEditorViewAction::ScrollHalfPageDown,
        )
        .with_context_predicate(text_entry.clone() & (id!("VimNormalMode") | id!("VimVisualMode")))
        .with_key_binding("ctrl-d"),
        EditableBinding::new(
            "editor_view:vim_scroll_half_page_up",
            "Scroll up half a page (vim)",
            CodeEditorViewAction::ScrollHalfPageUp,
        )
        .with_context_predicate(text_entry.clone() & (id!("VimNormalMode") | id!("VimVisualMode")))
        .with_key_binding("ctrl-u"),
        EditableBinding::new(
            "editor_view:cut_word_left",
            "Cut word left",
            CodeEditorViewAction::CutWordLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-w"),
        EditableBinding::new(
            "editor:delete_word_left",
            "Delete word left",
            CodeEditorViewAction::DeleteWordLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-backspace")
        .with_linux_or_windows_key_binding("ctrl-backspace"),
        EditableBinding::new(
            "editor_view:cut_word_right",
            "Cut word right",
            CodeEditorViewAction::CutWordRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("alt-d"),
        EditableBinding::new(
            "editor:delete_word_right",
            "Delete word right",
            CodeEditorViewAction::DeleteWordRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-delete")
        .with_linux_or_windows_key_binding("ctrl-delete"),
        EditableBinding::new(
            "editor_view:cut_all_left",
            "Cut all left",
            CodeEditorViewAction::CutLineLeft,
        )
        .with_context_predicate(text_entry.clone()),
        EditableBinding::new(
            "editor_view:delete_all_left",
            "Delete all left",
            CodeEditorViewAction::DeleteLineLeft,
        )
        .with_context_predicate(text_entry.clone())
        // Intellij uses `ctrl-Y` to delete a line on Windows/Linux whereas VSCode uses
        // `ctrl-shift-k`. We use the former because `ctrl-shift-k` would interfere with the binding
        // to clear all blocks within the blocklist.
        .with_mac_key_binding("cmd-backspace")
        .with_linux_or_windows_key_binding("ctrl-y"),
        EditableBinding::new(
            "editor_view:cut_all_right",
            "Cut all right",
            CodeEditorViewAction::CutLineRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-k"),
        EditableBinding::new(
            "editor_view:delete_all_right",
            "Delete all right",
            CodeEditorViewAction::DeleteLineRight,
        )
        .with_context_predicate(text_entry.clone())
        // VSCode only binds a default binding on Mac.
        .with_mac_key_binding("cmd-delete"),
    ]);

    // Editable Vim keybindings
    app.register_editable_bindings([EditableBinding::new(
        "editor_view:vim_exit_insert_mode",
        "Exit Vim insert mode",
        CodeEditorViewAction::VimEscape,
    )
    .with_context_predicate(text_entry.clone() & id!("Vim"))
    .with_key_binding("ctrl-[")]);

    // Editable Find Bar keybindings
    app.register_editable_bindings([EditableBinding::new(
        "code_editor:find",
        "Find in code editor",
        CodeEditorViewAction::ShowFindBar,
    )
    .with_key_binding(cmd_or_ctrl_shift("f"))
    .with_custom_action(CustomAction::Find)
    .with_context_predicate(text_entry.clone() & id!("FindBarAvailable"))
    .with_enabled(|| FeatureFlag::CodeFindReplace.is_enabled())]);

    // Editable Go to Line keybinding
    app.register_editable_bindings([EditableBinding::new(
        "editor_view:go_to_line",
        "Go to line",
        CodeEditorViewAction::ShowGoToLine,
    )
    .with_key_binding("ctrl-g") // Matches VSCode; editor-scoped via text_entry predicate
    .with_custom_action(CustomAction::GoToLine)
    .with_context_predicate(text_entry.clone())]);
}

#[derive(Debug, Clone)]
pub enum CodeEditorViewAction {
    UserTyped(UserInput<String>),
    VimUserTyped(UserInput<String>),
    Enter,
    Delete,
    Backspace,
    VimBackspace,
    ToggleComment,
    ScrollVertical(Pixels),
    ScrollHorizontal(Pixels),
    ScrollHalfPageDown,
    ScrollHalfPageUp,
    SelectUp,
    SelectDown,
    SelectLeft,
    SelectRight,
    SelectBackwardsByWord,
    SelectForwardsByWord,
    SelectToLineStart,
    SelectToLineEnd,
    SelectAll,
    ToggleDiffNav(Option<Range<LineCount>>),
    HiddenSectionExpansion {
        line_range: Range<LineCount>,
        expansion_type: ExpansionType,
    },
    /// Add diff hunk content as context (when clicking plus icon)
    AddDiffHunkContext {
        line_range: Range<LineCount>,
    },
    /// Revert diff hunk changes (when clicking revert icon)
    RevertDiffHunk {
        line_range: Range<LineCount>,
    },
    /// Open comment line (when opening a comment on a specific line)
    NewCommentOnLine {
        line: EditorLineLocation,
    },
    RequestOpenSavedComment {
        uuid: CommentId,
    },
    DeleteLineLeft,
    DeleteLineRight,
    DeleteWordLeft,
    DeleteWordRight,
    CutLineLeft,
    CutLineRight,
    CutWordLeft,
    CutWordRight,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    MoveBackwardsByWord,
    MoveForwardsByWord,
    MoveToLineStart,
    MoveToLineEnd,
    CursorAtBufferStart,
    CursorAtBufferEnd,
    SelectWord {
        offset: CharOffset,
        modifiers: ModifiersState,
    },
    SelectLine {
        offset: CharOffset,
        modifiers: ModifiersState,
    },
    SelectionStart {
        offset: CharOffset,
        modifiers: ModifiersState,
    },
    SelectionUpdate(CharOffset),
    SelectionEnd,
    MaybeClickOnHoveredLink(CharOffset),
    MouseHovered {
        offset: CharOffset,
        cmd: bool,
        clamped: bool,
        /// Whether the mouse move event was covered by an element above the editor.
        is_covered: bool,
    },
    RightMouseDown {
        offset: CharOffset,
    },
    Paste,
    Cut,
    Copy,
    #[cfg(windows)]
    WindowsCtrlC,
    Undo,
    Redo,
    Tab,
    ShiftTab,
    ShowFindBar,
    ShowGoToLine,
    Escape,
    VimEnter,
    VimTab,
    VimDelete,
    VimShiftTab,
    VimShiftEnter,
    VimEscape,
}

impl CodeEditorViewAction {
    pub fn allowed_in_interaction_state(&self, state: InteractionState) -> bool {
        match state {
            InteractionState::Editable => true,
            InteractionState::Selectable | InteractionState::EditableWithInvalidSelection => {
                self.allowed_in_selectable_state()
            }
            InteractionState::Disabled => false,
        }
    }

    fn allowed_in_selectable_state(&self) -> bool {
        match self {
            Self::UserTyped(_)
            | Self::VimUserTyped(_)
            | Self::Enter
            | Self::Delete
            | Self::Backspace
            | Self::VimBackspace
            | Self::ToggleComment
            | Self::DeleteLineLeft
            | Self::DeleteLineRight
            | Self::DeleteWordLeft
            | Self::DeleteWordRight
            | Self::CutLineLeft
            | Self::CutLineRight
            | Self::CutWordLeft
            | Self::CutWordRight
            | Self::Paste
            | Self::Cut
            | Self::Undo
            | Self::Redo
            | Self::Tab
            | Self::ShiftTab
            | Self::VimEnter
            | Self::VimTab
            | Self::VimDelete
            | Self::VimShiftTab
            | Self::VimShiftEnter
            | Self::VimEscape => false,

            #[cfg(windows)]
            Self::WindowsCtrlC => true,
            Self::ScrollVertical(_)
            | Self::ScrollHorizontal(_)
            | Self::ScrollHalfPageDown
            | Self::ScrollHalfPageUp
            | Self::SelectUp
            | Self::SelectDown
            | Self::SelectLeft
            | Self::SelectRight
            | Self::SelectBackwardsByWord
            | Self::SelectForwardsByWord
            | Self::SelectToLineStart
            | Self::SelectToLineEnd
            | Self::SelectAll
            | Self::ToggleDiffNav(_)
            | Self::MoveUp
            | Self::MoveDown
            | Self::MoveLeft
            | Self::MoveRight
            | Self::MoveBackwardsByWord
            | Self::MoveForwardsByWord
            | Self::MoveToLineStart
            | Self::MoveToLineEnd
            | Self::SelectWord { .. }
            | Self::SelectLine { .. }
            | Self::SelectionStart { .. }
            | Self::SelectionUpdate(_)
            | Self::SelectionEnd
            | Self::CursorAtBufferStart
            | Self::CursorAtBufferEnd
            | Self::Copy
            | Self::ShowFindBar
            | Self::ShowGoToLine
            | Self::Escape
            | Self::HiddenSectionExpansion { .. }
            | Self::AddDiffHunkContext { .. }
            | Self::RevertDiffHunk { .. }
            | Self::NewCommentOnLine { .. }
            | Self::RequestOpenSavedComment { .. }
            | Self::MouseHovered { .. }
            | Self::MaybeClickOnHoveredLink(_)
            | Self::RightMouseDown { .. } => true,
        }
    }
}

impl TypedActionView for CodeEditorView {
    type Action = CodeEditorViewAction;

    fn handle_action(&mut self, action: &CodeEditorViewAction, ctx: &mut ViewContext<Self>) {
        use CodeEditorViewAction::*;

        // Some actions are not allowed if editing or selection is disabled.
        if !action.allowed_in_interaction_state(self.interaction_state(ctx)) {
            return;
        }

        match action {
            UserTyped(content) => self.user_insert(content, ctx),
            VimUserTyped(content) => {
                // Don't handle vim input when the find input is active
                let allow_vim = match &self.find_bar {
                    Some(find_bar) => {
                        let open = find_bar.as_ref(ctx).is_open();
                        let editable = find_bar.as_ref(ctx).is_find_input_editable(ctx);
                        !(open && editable)
                    }
                    None => true,
                };
                if allow_vim {
                    self.vim_user_insert(content, ctx);
                }
            }
            ToggleDiffNav(line_range) => self.toggle_diff_nav(line_range.clone(), ctx),
            Enter => self.model.update(ctx, |model, ctx| model.enter(ctx)),
            VimEnter => self.vim_keystroke(&Keystroke::parse("enter").expect("enter parses"), ctx),
            Delete => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
            }),
            VimDelete => {
                self.vim_keystroke(&Keystroke::parse("delete").expect("delete parses"), ctx)
            }
            Backspace => {
                self.model.update(ctx, |model, ctx| {
                    model.backspace(ctx);
                });
            }
            VimBackspace => {
                self.vim_keystroke(
                    &Keystroke::parse("backspace").expect("backspace parses"),
                    ctx,
                );
            }
            ToggleComment => {
                match self.vim_mode(ctx) {
                    Some(VimMode::Visual(_)) => {
                        // In Vim Visual mode, if we get a ToggleComment request via the keyboard
                        // shortcut (cmd+/), simulate `gc` to the VimModel so that we correctly
                        // calculate the current visual selections, apply the toggle, and exit to
                        // normal mode.
                        self.vim_user_insert("gc", ctx);
                    }
                    _ => {
                        self.model.update(ctx, |model, ctx| {
                            model.toggle_comments(ctx);
                        });
                    }
                }
            }
            ScrollVertical(delta) => self.model.update(ctx, |model, ctx| {
                model.render_state().update(ctx, |render_state, ctx| {
                    render_state.scroll(*delta, ctx);
                })
            }),
            ScrollHorizontal(delta) => self.model.update(ctx, |model, ctx| {
                model.render_state().update(ctx, |render_state, ctx| {
                    render_state.scroll_horizontal(*delta, ctx);
                })
            }),
            ScrollHalfPageDown => {
                self.vim_keystroke(&Keystroke::parse("ctrl-d").expect("ctrl-d parses"), ctx)
            }
            ScrollHalfPageUp => {
                self.vim_keystroke(&Keystroke::parse("ctrl-u").expect("ctrl-u parses"), ctx)
            }
            SelectUp => self.model.update(ctx, |model, ctx| {
                model.select_up(ctx);
            }),
            SelectDown => self.model.update(ctx, |model, ctx| {
                model.select_down(ctx);
            }),
            SelectLeft => self.model.update(ctx, |model, ctx| {
                model.select_left(ctx);
            }),
            SelectRight => self.model.update(ctx, |model, ctx| {
                model.select_right(ctx);
            }),
            SelectBackwardsByWord => self.model.update(ctx, |model, ctx| {
                model.backward_word(true, ctx);
            }),
            SelectForwardsByWord => self.model.update(ctx, |model, ctx| {
                model.forward_word(true, ctx);
            }),
            SelectToLineStart => self.model.update(ctx, |model, ctx| {
                model.select_to_line_start(ctx);
            }),
            SelectToLineEnd => self.model.update(ctx, |model, ctx| {
                model.select_to_paragraph_end(ctx);
            }),
            SelectAll => self.model.update(ctx, |model, ctx| {
                model.select_all(ctx);
            }),
            CursorAtBufferStart => self.model.update(ctx, |model, ctx| {
                model.cursor_at(CharOffset::from(1), ctx);
            }),
            CursorAtBufferEnd => self.model.update(ctx, |model, ctx| {
                model.cursor_at(model.max_character_offset(ctx), ctx);
            }),
            DeleteLineLeft => self.delete_line_left(ctx),
            DeleteLineRight => self.model.update(ctx, |model, ctx| {
                model.delete(
                    TextDirection::Forwards,
                    TextUnit::ParagraphBoundary,
                    false,
                    ctx,
                );
            }),
            DeleteWordLeft => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Backwards, word_unit(ctx), false, ctx);
            }),
            DeleteWordRight => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Forwards, word_unit(ctx), false, ctx);
            }),
            CutLineLeft => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Backwards, TextUnit::LineBoundary, true, ctx);
            }),
            CutLineRight => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Forwards, TextUnit::LineBoundary, true, ctx);
            }),
            CutWordLeft => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Backwards, word_unit(ctx), true, ctx);
            }),
            CutWordRight => self.model.update(ctx, |model, ctx| {
                model.delete(TextDirection::Forwards, word_unit(ctx), true, ctx);
            }),
            MoveUp => self.model.update(ctx, |model, ctx| model.move_up(ctx)),
            MoveDown => self.model.update(ctx, |model, ctx| model.move_down(ctx)),
            MoveLeft => self.model.update(ctx, |model, ctx| {
                model.move_left(ctx);
            }),
            MoveRight => self.model.update(ctx, |model, ctx| {
                model.move_right(ctx);
            }),
            MoveBackwardsByWord => self.model.update(ctx, |model, ctx| {
                model.backward_word(false, ctx);
            }),
            MoveForwardsByWord => self.model.update(ctx, |model, ctx| {
                model.forward_word(false, ctx);
            }),
            MoveToLineStart => self.model.update(ctx, |model, ctx| {
                model.move_to_line_start(ctx);
            }),
            MoveToLineEnd => self.model.update(ctx, |model, ctx| {
                model.move_to_paragraph_end(ctx);
            }),
            SelectWord { offset, modifiers } => {
                self.is_selecting = true;
                let multiselect = modifiers.alt && FeatureFlag::RichTextMultiselect.is_enabled();
                self.model.update(ctx, |model, ctx| {
                    model.select_word_at(*offset, multiselect, ctx);
                });
            }
            SelectLine { offset, modifiers } => {
                self.is_selecting = true;
                let multiselect = modifiers.alt && FeatureFlag::RichTextMultiselect.is_enabled();
                self.model.update(ctx, |model, ctx| {
                    model.select_line_at(*offset, multiselect, ctx);
                });
            }
            SelectionStart { offset, modifiers } => self.selection_start(*offset, *modifiers, ctx),
            SelectionUpdate(offset) => {
                if self.is_selecting {
                    self.selection_update(*offset, ctx);
                } else {
                    self.selection_extend(*offset, ctx);
                }
            }
            SelectionEnd => self.selection_end(ctx),
            Paste => self.model.update(ctx, |model, ctx| {
                model.paste(ctx);
            }),
            Cut => self.model.update(ctx, |model, ctx| {
                model.cut(ctx);
            }),
            // Note that this is _not_ the only code path that could copy selected text to the clipboard.
            // This is only for the case when the editor is focused and the copy action gets dispatched directly.
            // The owner of the editor can also perform a copy by accessing the selected text and copying it to the clipboard.
            // This is the case when the code block is owned by an AIBlock and unfocused.
            Copy => {
                self.model.update(ctx, |model, ctx| {
                    model.copy(ctx);
                });
                // It's possible that the copy action was dispatched to the focused editor even when
                // the user intended to copy selected text from a parent view (i.e. an `AIBlock`).
                // The `CopiedEmptyText` event gives the parent view a signal to attempt a copy action.
                if self.selected_text(ctx).is_none() {
                    ctx.emit(CodeEditorEvent::CopiedEmptyText);
                }
            }
            #[cfg(windows)]
            WindowsCtrlC => self.model.update(ctx, |model, ctx| {
                model.handle_windows_ctrl_c(ctx);
            }),
            Undo => self.model.update(ctx, |model, ctx| {
                model.undo(ctx);
            }),
            Redo => self.model.update(ctx, |model, ctx| {
                model.redo(ctx);
            }),
            Tab => self.model.update(ctx, |model, ctx| {
                model.indent(false, ctx);
            }),
            VimTab => self.vim_keystroke(&Keystroke::parse("tab").expect("tab parses"), ctx),
            ShiftTab => self.model.update(ctx, |model, ctx| {
                model.indent(true, ctx);
            }),
            VimShiftTab => self.vim_keystroke(
                &Keystroke::parse("shift-tab").expect("shift-tab parses"),
                ctx,
            ),
            VimShiftEnter => self.vim_keystroke(
                &Keystroke::parse("shift-enter").expect("shift-enter parses"),
                ctx,
            ),
            VimEscape => {
                self.vim_keystroke(&Keystroke::parse("escape").expect("escape parses"), ctx)
            }

            ShowFindBar => self.show_find_bar(ctx),
            ShowGoToLine => self.show_goto_line(ctx),
            Escape => self.escape(ctx),
            HiddenSectionExpansion {
                line_range,
                expansion_type,
            } => {
                self.expand_hidden_section(line_range.clone(), expansion_type, ctx);
            }
            AddDiffHunkContext { line_range } => {
                // Record this range as clicked so the button disappears
                self.display_states
                    .wrapper_state_handle
                    .record_clicked_range(line_range.clone());

                // Emit event for parent to handle adding context
                ctx.emit(CodeEditorEvent::DiffHunkContextAdded {
                    line_range: line_range.clone(),
                });

                // Notify to re-render and hide the button
                ctx.notify();
            }
            RevertDiffHunk { line_range } => {
                if FeatureFlag::RevertDiffHunk.is_enabled() {
                    send_telemetry_from_ctx!(CodeReviewTelemetryEvent::RevertHunkClicked, ctx);

                    // Convert line range to diff hunk index and revert it
                    let hunk_index = self
                        .model
                        .as_ref(ctx)
                        .diff()
                        .as_ref(ctx)
                        .diff_hunk_count_before_line(line_range.start.as_usize());

                    self.model.update(ctx, |model, ctx| {
                        model.reverse_diff_by_index(hunk_index, ctx);
                    });

                    // Emit event for parent to handle
                    ctx.emit(CodeEditorEvent::DiffReverted);

                    // Notify to re-render
                    ctx.notify();
                }
            }
            NewCommentOnLine { line: line_info } => {
                if FeatureFlag::InlineCodeReview.is_enabled() {
                    self.model.update(ctx, |model: &mut CodeEditorModel, ctx| {
                        model.open_comment_line(line_info, ctx);
                    });

                    ctx.focus(&self.active_comment_editor);
                    ctx.notify();
                }
            }
            RequestOpenSavedComment { uuid } => {
                if FeatureFlag::InlineCodeReview.is_enabled() {
                    ctx.emit(CodeEditorEvent::RequestOpenComment(*uuid))
                }
            }
            MouseHovered {
                offset,
                cmd,
                clamped,
                is_covered,
            } => {
                ctx.emit(CodeEditorEvent::MouseHovered {
                    offset: *offset,
                    cmd: *cmd,
                    clamped: *clamped,
                    is_covered: *is_covered,
                });
            }
            RightMouseDown { offset } => {
                // Right mouse down should set the cursor at the offset location. This matches the behavior with other editors.
                self.model.update(ctx, |model, ctx| {
                    model.cursor_at(*offset, ctx);
                });
            }
            MaybeClickOnHoveredLink(offset) => {
                self.model.update(ctx, |model, ctx| {
                    model.maybe_click_on_hovered_link(offset, ctx)
                });
            }
        }
    }
}

impl warp_editor::editor::EditorView for CodeEditorView {
    type RichTextAction = CodeEditorViewAction;

    fn runnable_command_at<'a>(
        &self,
        _block_offset: CharOffset,
        _ctx: &'a AppContext,
    ) -> Option<&'a dyn RunnableCommandModel> {
        None
    }

    fn embedded_item_at<'a>(
        &self,
        _block_offset: CharOffset,
        _ctx: &'a AppContext,
    ) -> Option<&'a dyn EmbeddedItemModel> {
        None
    }

    fn text_decorations<'a>(
        &'a self,
        viewport_ranges: RangeSet<CharOffset>,
        buffer_version: Option<BufferVersion>,
        ctx: &'a AppContext,
    ) -> TextDecoration<'a> {
        self.model
            .as_ref(ctx)
            .text_decoration_for_ranges(viewport_ranges, buffer_version, ctx)
    }
}

impl RichTextAction<CodeEditorView> for CodeEditorViewAction {
    fn scroll(delta: Pixels, axis: Axis) -> Option<Self> {
        Some(match axis {
            Axis::Horizontal => CodeEditorViewAction::ScrollHorizontal(delta),
            Axis::Vertical => CodeEditorViewAction::ScrollVertical(delta),
        })
    }

    fn user_typed(
        chars: String,
        view: &WeakViewHandle<CodeEditorView>,
        ctx: &AppContext,
    ) -> Option<Self> {
        let view = view.upgrade(ctx)?;
        if !view.as_ref(ctx).is_editable(ctx) {
            return None;
        }
        Some(CodeEditorViewAction::UserTyped(UserInput::new(chars)))
    }

    fn vim_user_typed(
        chars: String,
        parent_view: &WeakViewHandle<CodeEditorView>,
        ctx: &AppContext,
    ) -> Option<Self> {
        let view = parent_view.upgrade(ctx)?;
        if !view.as_ref(ctx).is_editable(ctx) {
            return None;
        }
        Some(CodeEditorViewAction::VimUserTyped(UserInput::new(chars)))
    }

    fn left_mouse_down(
        location: Location,
        modifiers: ModifiersState,
        click_count: u32,
        is_first_mouse: bool,
        _view: &WeakViewHandle<CodeEditorView>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        log::debug!(
            "Clicked {click_count} times on {location:?}, cmd = {}, shift = {}",
            modifiers.cmd,
            modifiers.shift
        );

        // The first mouse down to bring focus to a Warp window will not have a corresponding mouse up.
        // We ignore it, and they can click again.
        if is_first_mouse {
            return None;
        }

        match location {
            Location::Text { char_offset, .. } => match click_count {
                // TODO(CLD-558): We need to align render model with the content model offset.
                1 if modifiers.shift => {
                    Some(CodeEditorViewAction::SelectionUpdate(char_offset + 1))
                }
                1 => Some(CodeEditorViewAction::SelectionStart {
                    offset: char_offset + 1,
                    modifiers,
                }),
                2 => Some(CodeEditorViewAction::SelectWord {
                    offset: char_offset + 1,
                    modifiers,
                }),
                3 => Some(CodeEditorViewAction::SelectLine {
                    offset: char_offset + 1,
                    modifiers,
                }),
                _ => None,
            },
            _ => None,
        }
    }

    fn left_mouse_dragged(
        location: Location,
        _cmd: bool,
        _shift: bool,
        view: &WeakViewHandle<CodeEditorView>,
        ctx: &AppContext,
    ) -> Option<Self> {
        let view = view.upgrade(ctx)?;
        match location {
            Location::Text { char_offset, .. } if view.as_ref(ctx).is_selecting => {
                Some(CodeEditorViewAction::SelectionUpdate(char_offset + 1))
            }
            _ => None,
        }
    }

    fn left_mouse_up(
        location: Location,
        cmd: bool,
        _shift: bool,
        view: &WeakViewHandle<CodeEditorView>,
        ctx: &AppContext,
    ) -> Vec<Self> {
        let mut actions_to_dispatch = vec![];
        let Some(view) = view.upgrade(ctx) else {
            return actions_to_dispatch;
        };

        if view.as_ref(ctx).is_selecting {
            actions_to_dispatch.push(CodeEditorViewAction::SelectionEnd);
        } else if cmd {
            if let Location::Text { char_offset, .. } = location {
                actions_to_dispatch
                    .push(CodeEditorViewAction::MaybeClickOnHoveredLink(char_offset));
            }
        }
        actions_to_dispatch
    }

    fn mouse_hovered(
        location: Option<Location>,
        _parent_view: &WeakViewHandle<CodeEditorView>,
        cmd: bool,
        is_covered: bool,
        _ctx: &AppContext,
    ) -> Option<Self> {
        match location {
            Some(Location::Text {
                char_offset,
                clamped,
                ..
            }) => Some(CodeEditorViewAction::MouseHovered {
                offset: char_offset,
                clamped,
                cmd,
                is_covered,
            }),
            // When the mouse moves outside the editor bounds (location is None),
            // emit a clamped hover event to clear any active hover state.
            _ => Some(CodeEditorViewAction::MouseHovered {
                offset: CharOffset::default(),
                clamped: true,
                cmd,
                is_covered,
            }),
        }
    }

    fn task_list_clicked(
        _block_start: CharOffset,
        _parent_view: &WeakViewHandle<CodeEditorView>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        None
    }

    fn middle_mouse_down(_ctx: &AppContext) -> Option<Self> {
        None
    }

    fn right_mouse_down(
        location: Location,
        _parent_view: &WeakViewHandle<CodeEditorView>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        match location {
            Location::Text { char_offset, .. } => Some(CodeEditorViewAction::RightMouseDown {
                offset: char_offset + 1,
            }),
            _ => None,
        }
    }
}
