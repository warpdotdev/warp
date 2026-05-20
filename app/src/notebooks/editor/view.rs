use std::{
    collections::HashSet,
    ops::Range,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use markdown_parser::{parse_html, parse_markdown, FormattedText};
use pathfinder_geometry::vector::vec2f;
use string_offset::CharOffset;
use warp_editor::{
    content::{
        anchor::Anchor,
        text::{BufferTextStyle, CodeBlockType, TextStyles},
        version::BufferVersion,
    },
    editor::{EmbeddedItemModel, NavigationKey, RunnableCommandModel, TextDecoration},
    model::{CoreEditorModel, RichTextEditorModel},
    render::{
        element::{
            DisplayOptions, DisplayStateHandle, RichTextAction, RichTextElement,
            VerticalExpansionBehavior,
        },
        model::{BlockItem, HitTestBlockType, Location, RenderState},
    },
    selection::{TextDirection, TextUnit},
};

use warp_util::{path::LineAndColumnArg, user_input::UserInput};
use warpui::{
    accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole},
    assets::asset_cache::{AssetCache, AssetHandle, AssetState},
    clipboard::ClipboardContent,
    elements::{
        AnchorPair, Axis, Border, ChildAnchor, Clipped, ConstrainedBox, Container, CornerRadius,
        Dismiss, Fill, Flex, Icon, MouseStateHandle, OffsetPositioning, OffsetType, ParentAnchor,
        ParentElement, PositionedElementOffsetBounds, PositioningAxis, Radius, ScrollStateHandle,
        Scrollable, ScrollableElement, ScrollbarWidth, Stack, XAxisAnchor, YAxisAnchor,
    },
    event::ModifiersState,
    fonts::{FallbackFontEvent, FallbackFontModel},
    image_cache::ImageType,
    keymap::{EditableBinding, FixedBinding},
    platform::{Cursor, OperatingSystem},
    presenter::ChildView,
    r#async::SpawnedFutureHandle,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    units::Pixels,
    windowing, AppContext, BlurContext, CursorInfo, Element, Entity, FocusContext, ModelHandle,
    SingletonEntity, TypedActionView, View, ViewContext, ViewHandle, WeakViewHandle,
};
use warpui::{actions::StandardAction, elements::Hoverable};
use warpui::{keymap::PerPlatformKeystroke, windowing::WindowManager};

use crate::{
    appearance::Appearance,
    cmd_or_ctrl_shift,
    editor::InteractionState,
    features::FeatureFlag,
    notebooks::{
        editor::{find_bar::FindBarAction, model::word_unit},
        file::MarkdownDisplayMode,
        link::{LinkTarget, NotebookLinks, ResolveError},
        telemetry::{ActionEntrypoint, BlockInfo, EmbeddedObjectInfo, SelectionMode},
    },
    server::ids::SyncId,
    settings::{AppEditorSettings, FontSettings, SelectionSettings},
    terminal::{grid_renderer::URL_COLOR, links::directly_open_link_keybinding_string},
    ui_components::icons::ICON_DIMENSIONS,
    util::{
        bindings::CustomAction,
        tooltips::{render_tooltip, should_show_open_in_warp_link, TooltipLink, TooltipRedaction},
    },
    view_components::DismissibleToast,
};

#[cfg(feature = "local_fs")]
use crate::util::link_detection::{detect_file_paths, get_word_range_at_offset, DetectedLinkType};

#[cfg(feature = "local_fs")]
use warpui::text::word_boundaries::WordBoundariesPolicy;

use super::{
    block_insertion_menu::{BlockInsertionMenuState, BlockInsertionSource},
    find_bar::{FindBar, FindBarEvent, FindBarState},
    keys::NotebookKeybindings,
    link_editor::{LinkEditor, LinkEditorEvent},
    model::{NotebooksEditorModel, RichTextEditorModelEvent},
    omnibar::{Omnibar, OmnibarEvent},
    rich_text_styles, BlockType, NotebookWorkflow,
};

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const MAX_EDITOR_TIP_WIDTH: f32 = 300.;

/// Width of the left gutter, which holds the block insertion menu.
const GUTTER_WIDTH: f32 = ICON_DIMENSIONS + 4.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    // Context for text entry/navigation/selection:
    // - The editor is focused
    // - The input method editor is not open
    // - There is no command selection
    let text_entry = id!("RichTextEditorView") & !id!("IMEOpen") & !id!("HasCommandSelection");

    /// Enabled predicate that is true if running in debug mode AND notebooks are enabled.
    fn debug_notebooks_enabled() -> bool {
        FeatureFlag::DebugMode.is_enabled()
    }

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            EditorViewAction::Enter,
            // The BlockInsertionMenu guard is needed because menus don't handle Enter via keybindings.
            // Without this, Enter is processed by both the menu and the editor view.
            id!("RichTextEditorView") & !id!("IMEOpen") & !id!("BlockInsertionMenu"),
        ),
        FixedBinding::new(
            "numpadenter",
            EditorViewAction::Enter,
            id!("RichTextEditorView") & !id!("IMEOpen") & !id!("BlockInsertionMenu"),
        ),
        FixedBinding::new(
            "shift-enter",
            EditorViewAction::ShiftEnter,
            id!("RichTextEditorView") & !id!("IMEOpen") & !id!("BlockInsertionMenu"),
        ),
        FixedBinding::new(
            "backspace",
            EditorViewAction::Backspace,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-backspace",
            EditorViewAction::Backspace,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new("delete", EditorViewAction::Delete, text_entry.clone()),
        FixedBinding::new(
            "shift-up",
            EditorViewAction::SelectUp,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-down",
            EditorViewAction::SelectDown,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-left",
            EditorViewAction::SelectLeft,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-right",
            EditorViewAction::SelectRight,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "up",
            EditorViewAction::MoveUp,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "down",
            EditorViewAction::MoveDown,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "left",
            EditorViewAction::MoveLeft,
            text_entry.clone() & !id!("BlockInsertionMenu"),
        ),
        FixedBinding::new(
            "right",
            EditorViewAction::MoveRight,
            text_entry.clone() & !id!("BlockInsertionMenu"),
        ),
        FixedBinding::new(
            "home",
            EditorViewAction::MoveToLineStart,
            text_entry.clone(),
        ),
        FixedBinding::new("end", EditorViewAction::MoveToLineEnd, text_entry.clone()),
        FixedBinding::new("cmdorctrl-]", EditorViewAction::Indent, text_entry.clone()),
        FixedBinding::new(
            "cmdorctrl-[",
            EditorViewAction::Unindent,
            text_entry.clone(),
        ),
        FixedBinding::new("tab", EditorViewAction::Tab, text_entry.clone()),
        FixedBinding::new("shift-tab", EditorViewAction::ShiftTab, text_entry.clone()),
        // Also create the word movement shortcuts with `meta` in place of `alt`, to accommodate
        // the "Left Option is Meta" and "Right Option is Meta" settings.
        FixedBinding::new(
            "meta-left",
            EditorViewAction::MoveBackwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "meta-right",
            EditorViewAction::MoveForwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "shift-alt-left",
                linux_and_windows: "shift-ctrl-left",
            },
            EditorViewAction::SelectBackwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-meta-left",
            EditorViewAction::SelectBackwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "shift-alt-right",
                linux_and_windows: "shift-ctrl-right",
            },
            EditorViewAction::SelectForwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-meta-right",
            EditorViewAction::SelectForwardsByWord,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-home",
            EditorViewAction::SelectToLineStart,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "shift-end",
            EditorViewAction::SelectToLineEnd,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "cmdorctrl-a",
            EditorViewAction::SelectAll,
            text_entry.clone(),
        ),
        FixedBinding::new(
            "cmdorctrl-b",
            EditorViewAction::Bold,
            text_entry.clone() & id!("EditorIsEditable"),
        ),
        FixedBinding::new("cmdorctrl-i", EditorViewAction::Italic, text_entry.clone()),
        FixedBinding::custom(
            CustomAction::Copy,
            EditorViewAction::Copy,
            "Copy",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        // Bindings for paste require the StandardAction and CustomAction binding to work on all platforms.
        FixedBinding::custom(
            CustomAction::Paste,
            EditorViewAction::Paste,
            "Paste",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::standard(
            StandardAction::Paste,
            EditorViewAction::Paste,
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        #[cfg(windows)]
        FixedBinding::custom(
            CustomAction::WindowsPaste,
            EditorViewAction::Paste,
            "Paste",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        #[cfg(windows)]
        FixedBinding::custom(
            CustomAction::WindowsCopy,
            EditorViewAction::Copy,
            "Copy",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Cut,
            EditorViewAction::Cut,
            "Cut",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Undo,
            EditorViewAction::Undo,
            "Undo",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Redo,
            EditorViewAction::Redo,
            "Redo",
            id!("RichTextEditorView") & !id!("IMEOpen"),
        ),
    ]);

    // Editable command-selection bindings.
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:deselect_command",
            "De-select shell commands",
            EditorViewAction::ExitCommandSelection,
        )
        .with_context_predicate(id!("RichTextEditorView") & id!("HasCommandSelection"))
        .with_key_binding("escape"),
        EditableBinding::new(
            "editor_view:select_command",
            "Select shell command at cursor",
            EditorViewAction::SelectCommandAtCursor,
        )
        .with_context_predicate(
            id!("RichTextEditorView")
                & !id!("HasCommandSelection")
                & id!("CanExecuteShellCommands"),
        )
        .with_key_binding("escape"),
        EditableBinding::new(
            "editor_view:select_previous_command",
            "Select previous command",
            EditorViewAction::CommandUp,
        )
        .with_context_predicate(id!("RichTextEditorView"))
        .with_key_binding("cmdorctrl-up"),
        EditableBinding::new(
            "editor_view:select_next_command",
            "Select next command",
            EditorViewAction::CommandDown,
        )
        .with_context_predicate(id!("RichTextEditorView"))
        .with_key_binding("cmdorctrl-down"),
        EditableBinding::new(
            "editor_view:run_commands",
            "Run selected commands",
            EditorViewAction::RunSelectedCommands,
        )
        .with_context_predicate(id!("RichTextEditorView") & id!("CanExecuteShellCommands"))
        .with_key_binding("cmdorctrl-enter"),
    ]);

    // When shell command execution is disabled (e.g., comment editors),
    // Cmd/Ctrl+Enter emits CmdEnter instead of running commands.
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-enter",
        EditorViewAction::CmdEnter,
        id!("RichTextEditorView") & !id!("CanExecuteShellCommands"),
    )]);

    // When shell command execution is disabled (e.g., comment editors),
    // ExitCommandSelection emits EscapePressed so parent views can dismiss.
    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        EditorViewAction::ExitCommandSelection,
        id!("RichTextEditorView") & !id!("CanExecuteShellCommands") & !id!("HasCommandSelection"),
    )]);

    app.register_fixed_bindings([FixedBinding::new(
        "alt-cmdorctrl-g",
        EditorViewAction::InsertPlaceholder,
        id!("RichTextEditorView"),
    )
    .with_enabled(debug_notebooks_enabled)]);

    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:toggle_debug_mode",
            "Toggle rich-text debug mode",
            EditorViewAction::ToggleDebugMode,
        )
        .with_context_predicate(id!("RichTextEditorView"))
        .with_enabled(debug_notebooks_enabled),
        EditableBinding::new(
            "editor_view:debug_copy_buffer",
            "Copy rich-text buffer",
            EditorViewAction::DebugCopyBuffer,
        )
        .with_context_predicate(id!("RichTextEditorView"))
        .with_enabled(debug_notebooks_enabled),
        EditableBinding::new(
            "editor_view:debug_copy_selection",
            "Copy rich-text selection",
            EditorViewAction::DebugCopySelection,
        )
        .with_context_predicate(id!("RichTextEditorView"))
        .with_enabled(debug_notebooks_enabled),
        EditableBinding::new(
            "editor_view:log_state",
            "Log editor state",
            EditorViewAction::DebugLogState,
        )
        .with_context_predicate(id!("RichTextEditorView"))
        .with_enabled(debug_notebooks_enabled),
    ]);

    // Editable navigation keybindings:
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:move_backward_one_word",
            "Move Backward One Word",
            EditorViewAction::MoveBackwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-left")
        .with_linux_or_windows_key_binding("ctrl-left"),
        EditableBinding::new(
            "editor_view:move_forward_one_word",
            "Move Forward One Word",
            EditorViewAction::MoveForwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-right")
        .with_linux_or_windows_key_binding("ctrl-right"),
        EditableBinding::new(
            "editor_view:move_forward_one_word",
            "Move forward one word",
            EditorViewAction::MoveForwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("meta-f"),
        EditableBinding::new(
            "editor_view:move_backward_one_word",
            "Move backward one word",
            EditorViewAction::MoveBackwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("meta-b"),
        EditableBinding::new("editor_view:up", "Move cursor up", EditorViewAction::MoveUp)
            .with_context_predicate(text_entry.clone())
            .with_key_binding("ctrl-p"),
        EditableBinding::new(
            "editor_view:down",
            "Move cursor down",
            EditorViewAction::MoveDown,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-n"),
        EditableBinding::new(
            "editor_view:left",
            "Move cursor left",
            EditorViewAction::MoveLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-b"),
        EditableBinding::new(
            "editor_view:right",
            "Move cursor right",
            EditorViewAction::MoveRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-f"),
        EditableBinding::new(
            // This doesn't reuse the move_to_line_start naming from the terminal input editor to
            // distinguish between soft-wrapped line and hard-wrapped line (paragraph) movement.
            "editor_view:move_to_paragraph_start",
            "Move to start of paragraph",
            EditorViewAction::MoveToParagraphStart,
        )
        .with_context_predicate(text_entry.clone())
        // Mac-only to not conflict with SelectAll on Linux and Windows.
        .with_mac_key_binding("ctrl-a"),
        EditableBinding::new(
            "editor_view:home",
            "Home",
            EditorViewAction::MoveToLineStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-left")
        .with_linux_or_windows_key_binding("home"),
        EditableBinding::new(
            "editor_view:move_to_paragraph_end",
            "Move to end of paragraph",
            EditorViewAction::MoveToParagraphEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("ctrl-e"),
        EditableBinding::new("editor_view:end", "End", EditorViewAction::MoveToLineEnd)
            .with_context_predicate(text_entry.clone())
            .with_mac_key_binding("cmd-right")
            .with_linux_or_windows_key_binding("end"),
    ]);

    // Editable selection keybindings:
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:select_left_by_word",
            "Select one word to the left",
            EditorViewAction::SelectBackwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-meta-B"),
        EditableBinding::new(
            "editor_view:select_right_by_word",
            "Select one word to the right",
            EditorViewAction::SelectForwardsByWord,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-meta-F"),
        EditableBinding::new(
            "editor_view:select_left",
            "Select one character to the left",
            EditorViewAction::SelectLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-ctrl-B"),
        EditableBinding::new(
            "editor_view:select_right",
            "Select one character to the right",
            EditorViewAction::SelectRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("shift-ctrl-F"),
        EditableBinding::new(
            "editor_view:select_up",
            "Select up",
            EditorViewAction::SelectUp,
        )
        .with_context_predicate(text_entry.clone())
        // Set this to Mac only since otherwise it could conflict with opening the command
        // palette. NOTE `shift-up` still exists as a cross platform keybinding for this action.
        .with_mac_key_binding("shift-ctrl-P"),
        EditableBinding::new(
            "editor_view:select_down",
            "Select down",
            EditorViewAction::SelectDown,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("shift-ctrl-N"),
        EditableBinding::new(
            "editor_view:select_all",
            "Select all",
            EditorViewAction::SelectAll,
        )
        .with_context_predicate(text_entry.clone())
        .with_custom_action(CustomAction::SelectAll),
        EditableBinding::new(
            "editor:select_to_paragraph_start",
            "Select to start of paragraph",
            EditorViewAction::SelectToParagraphStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("shift-ctrl-A"),
        EditableBinding::new(
            "editor:select_to_paragraph_end",
            "Select to end of paragraph",
            EditorViewAction::SelectToParagraphEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("shift-ctrl-E"),
        // `shift-end` is registered on all platforms for this action.
        EditableBinding::new(
            "editor_view:select_to_line_end",
            "Select To Line End",
            EditorViewAction::SelectToLineEnd,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-shift-right"),
        // `end` is registered on all platforms for this action.
        EditableBinding::new(
            "editor_view:select_to_line_start",
            "Select To Line Start",
            EditorViewAction::SelectToLineStart,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("cmd-shift-left"),
    ]);

    // Register mac-only `FixedBinding`s.
    if OperatingSystem::get().is_mac() {
        app.register_fixed_bindings([
            // A native character palette isn't supported on all platforms. The `ctrl-cmd-space`
            // binding is unique to Mac.
            FixedBinding::new(
                "ctrl-cmd-space",
                EditorViewAction::ShowCharacterPalette,
                text_entry.clone(),
            ),
        ]);
    }

    // Editable text-manipulation bindings
    app.register_editable_bindings([
        EditableBinding::new(
            "editor_view:backspace",
            "Remove the previous character",
            EditorViewAction::Backspace,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-h"),
        EditableBinding::new("editor_view:delete", "Delete", EditorViewAction::Delete)
            .with_context_predicate(text_entry.clone())
            .with_key_binding("ctrl-d"),
        EditableBinding::new(
            "editor_view:cut_word_left",
            "Cut word left",
            EditorViewAction::CutWordLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-w"),
        EditableBinding::new(
            "editor:delete_word_left",
            "Delete word left",
            EditorViewAction::DeleteWordLeft,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-backspace")
        .with_linux_or_windows_key_binding("ctrl-backspace"),
        EditableBinding::new(
            "editor_view:cut_word_right",
            "Cut word right",
            EditorViewAction::CutWordRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("alt-d"),
        EditableBinding::new(
            "editor:delete_word_right",
            "Delete word right",
            EditorViewAction::DeleteWordRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_mac_key_binding("alt-delete")
        .with_linux_or_windows_key_binding("ctrl-delete"),
        EditableBinding::new(
            "editor_view:cut_all_left",
            "Cut all left",
            EditorViewAction::CutLineLeft,
        )
        .with_context_predicate(text_entry.clone()),
        EditableBinding::new(
            "editor_view:delete_all_left",
            "Delete all left",
            EditorViewAction::DeleteLineLeft,
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
            EditorViewAction::CutLineRight,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("ctrl-k"),
        EditableBinding::new(
            "editor_view:delete_all_right",
            "Delete all right",
            EditorViewAction::DeleteLineRight,
        )
        .with_context_predicate(text_entry.clone())
        // VSCode only binds a default binding on Mac.
        .with_mac_key_binding("cmd-delete"),
    ]);

    // Rich-text editable keybindings
    app.register_editable_bindings([
        // Most apps use cmd-k / ctrl-k to edit links, but not all (notably, Slack). The binding is
        // editable for users who are used to something else.
        EditableBinding::new(
            "editor:edit_link",
            "Create or edit link",
            EditorViewAction::CreateOrEditLink,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-k"),
        EditableBinding::new(
            "editor_view:inline_code",
            "Toggle inline code styling",
            EditorViewAction::InlineCode,
        )
        .with_context_predicate(text_entry.clone())
        // Slack and other apps use cmd-shift-C on Mac and ctrl-shift-C on Linux/Windows.
        // However, we use ctrl-shift-C for copying, to not conflict with ctrl-c in the
        // terminal. For consistency with the rest of the app, ctrl-shift-C still copies in a
        // notebook, and we leave code styling unbound.
        .with_mac_key_binding("cmd-shift-C"),
        EditableBinding::new(
            "editor_view:strikethrough",
            "Toggle strikethrough styling",
            EditorViewAction::StrikeThrough,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-shift-X"),
        EditableBinding::new(
            "editor_view:underline",
            "Toggle underline styling",
            EditorViewAction::Underline,
        )
        .with_context_predicate(text_entry.clone())
        .with_key_binding("cmdorctrl-u"),
    ]);

    // Bindings for the find bar
    app.register_editable_bindings([
        EditableBinding::new(
            "editor:find",
            "Find in Notebook",
            EditorViewAction::ShowFindBar,
        )
        .with_key_binding(cmd_or_ctrl_shift("f"))
        .with_custom_action(CustomAction::Find)
        .with_context_predicate(id!("RichTextEditorView")),
        EditableBinding::new(
            "editor:next_find_match",
            "Focus next match",
            FindBarAction::FocusNextMatch,
        )
        .with_context_predicate(id!("FindBar")),
        EditableBinding::new(
            "editor:previous_find_match",
            "Focus previous match",
            FindBarAction::FocusPreviousMatch,
        )
        .with_context_predicate(id!("FindBar")),
        EditableBinding::new(
            "editor:toggle_regex_find",
            "Toggle regular expression search",
            FindBarAction::ToggleRegex,
        )
        .with_context_predicate(id!("FindBar")),
        EditableBinding::new(
            "editor:toggle_case_sensitive_find",
            "Toggle case-sensitive search",
            FindBarAction::ToggleCaseSensitive,
        )
        .with_context_predicate(id!("FindBar")),
    ])
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditorViewAction {
    UserTyped(UserInput<String>),
    VimUserTyped(UserInput<String>),
    Enter,
    ShiftEnter,
    /// Cmd/Ctrl+Enter pressed (used by comment editor mode to submit comments).
    CmdEnter,
    Delete,
    Backspace,
    Scroll(Pixels),
    MaybeOpenFileOrUrl {
        offset: CharOffset,
        link_in_text: Option<UserInput<String>>,
        cmd: bool,
    },
    ToggleTaskList(CharOffset),
    DismissOpenLink,
    CopyLink,
    /// Edit the existing link that the cursor is on (used for the link tooltip).
    EditLink,
    /// A link tooltip was clicked.
    OpenTooltipLink(UserInput<LinkTarget>),
    /// Perform a link's secondary action.
    SecondaryLinkAction(UserInput<LinkTarget>),
    /// Open the link editor to modify an existing link or insert a new one.
    CreateOrEditLink,
    SelectUp,
    SelectDown,
    SelectLeft,
    SelectRight,
    SelectBackwardsByWord,
    SelectForwardsByWord,
    SelectToLineStart,
    SelectToLineEnd,
    SelectToParagraphStart,
    SelectToParagraphEnd,
    SelectAll,
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
    MoveToParagraphStart,
    MoveToParagraphEnd,
    Bold,
    Italic,
    Underline,
    InlineCode,
    StrikeThrough,
    SelectWord {
        offset: CharOffset,
        multiselect: bool,
    },
    SelectLine {
        offset: CharOffset,
        multiselect: bool,
    },
    SelectionStart {
        offset: CharOffset,
        multiselect: bool,
    },
    SelectionUpdate(CharOffset),
    SelectBlock {
        block_start: CharOffset,
    },
    SelectionEnd,
    /// Action to update the currently-hovered block.
    BlockHovered {
        /// Starting offset of the block that was hovered over. If `None`, no block is hovered.
        block_start: Option<CharOffset>,
        /// The character offset that was hovered over. If `None`, no character is hovered.
        char_offset: Option<CharOffset>,
    },
    ShowCharacterPalette,
    ShowFindBar,
    /// Action that signals the editor to re-focus itself. This is sent by child views to switch focus from dropdown/context
    /// menus back to the editor.
    Focus,
    /// Debug-only action that copies a debug print of the full buffer to the clipboard.
    DebugCopyBuffer,
    /// Debug-only action that copies a debug print of the current selection to the clipboard.
    DebugCopySelection,
    DebugLogState,
    ToggleDebugMode,
    Paste,
    Cut,
    InsertPlaceholder,
    Copy,
    Undo,
    Redo,
    OpenBlockInsertionMenu,
    /// Insert a block of the given type after the hovered location.
    InsertBlock(warp_editor::content::text::BlockType),
    Indent,
    Unindent,
    Tab,
    ShiftTab,
    CommandUp,
    CommandDown,
    /// Run the selected command blocks in the attached terminal session.
    RunSelectedCommands,
    /// Exits command selection and switches back to text selection.
    ExitCommandSelection,
    /// Selects the command at the text cursor.
    SelectCommandAtCursor,
    /// Signal from a child model ([`NotebookCommand`] or [`EmbeddedItemModel`]) to run a
    /// workflow-like command.
    RunWorkflow(NotebookWorkflow),
    /// Signal from [`NotebookCommand`] to open a workflow.
    EditWorkflow(SyncId),
    /// Signal from [`NotebookCommand`] that a new code block type has been selected.
    CodeBlockTypeSelectedAtOffset {
        start_anchor: Anchor,
        code_block_type: CodeBlockType,
    },
    CopyTextToClipboard {
        text: UserInput<String>,
        block: BlockInfo,
        entrypoint: ActionEntrypoint,
    },
    OpenEmbeddedObjectSearch,
    RemoveEmbeddingAt(CharOffset),
    MiddleClickPaste,
    /// Open a file. If open_in_warp is true, open in Warp's code editor; otherwise use external editor.
    OpenFile {
        path: PathBuf,
        line_and_column_num: Option<LineAndColumnArg>,
        force_open_in_warp: bool,
    },
    /// Signal from a Mermaid toggle view that the user changed the display mode for a block.
    MermaidDisplayModeSelected {
        start_anchor: Anchor,
        mode: MarkdownDisplayMode,
    },
}

impl EditorViewAction {
    fn is_new_text_selection(&self) -> bool {
        matches!(
            self,
            EditorViewAction::SelectUp
                | EditorViewAction::SelectDown
                | EditorViewAction::SelectLeft
                | EditorViewAction::SelectRight
                | EditorViewAction::SelectBackwardsByWord
                | EditorViewAction::SelectForwardsByWord
                | EditorViewAction::SelectToLineStart
                | EditorViewAction::SelectToLineEnd
                | EditorViewAction::SelectToParagraphStart
                | EditorViewAction::SelectToParagraphEnd
                | EditorViewAction::SelectAll
                | EditorViewAction::SelectWord { .. }
                | EditorViewAction::SelectLine { .. }
                | EditorViewAction::SelectionStart { .. }
                | EditorViewAction::SelectionUpdate(_)
                | EditorViewAction::SelectBlock { .. }
                | EditorViewAction::SelectionEnd
        )
    }
}

#[derive(Default)]
struct LayoutAffectingAssetLoads {
    loading: HashSet<AssetHandle>,
    loaded_needs_relayout: bool,
}

enum LayoutAffectingAssetLoad {
    Loading(AssetHandle),
    LoadedNeedsRelayout,
}

fn mermaid_diagram_needs_loaded_layout(
    config: &warp_editor::render::model::ImageBlockConfig,
    image: &ImageType,
) -> bool {
    let ImageType::Svg { svg } = image else {
        return false;
    };
    let intrinsic_size = svg.size();
    let intrinsic_width = intrinsic_size.width();
    let intrinsic_height = intrinsic_size.height();
    if intrinsic_width <= 0. || intrinsic_height <= 0. {
        return false;
    }
    let expected_height = config.width.as_f32() * intrinsic_height / intrinsic_width;
    (config.height.as_f32() - expected_height).abs() > 0.5
}

pub enum EditorViewEvent {
    Edited,
    Focused,
    /// Cmd/Ctrl+Enter was pressed (emitted in comment editor mode).
    CmdEnter,
    Navigate(NavigationKey),
    /// Open a file in the preferred editor
    OpenFile {
        path: PathBuf,
        line_and_column_num: Option<LineAndColumnArg>,
        force_open_in_warp: bool,
    },
    /// Emitted when the user runs a notebook workflow. The parent `NotebookView` is responsible
    /// for sending it to the active terminal.
    RunWorkflow(NotebookWorkflow),
    EditWorkflow(SyncId),
    /// The block insertion menu was opened.
    OpenedBlockInsertionMenu(BlockInsertionSource),
    /// The embedded object search menu was opened.
    OpenedEmbeddedObjectSearch,
    /// The find bar was opened.
    OpenedFindBar,
    /// An embedded object was inserted (via the menu - this doesn't account for copy/pasting
    /// embeds).
    InsertedEmbeddedObject(EmbeddedObjectInfo),
    CopiedBlock {
        block: BlockInfo,
        entrypoint: ActionEntrypoint,
    },
    /// One of the command-navigation keyboard shortcuts was used.
    NavigatedCommands,
    /// The editor switched between text selection and command selection. The event contains the
    /// _new_ selection mode.
    ChangedSelectionMode(SelectionMode),
    /// The text selection changed (cursor moved, selection extended, etc.).
    TextSelectionChanged,
    /// Escape was pressed (emitted when shell command execution is disabled,
    /// e.g. in comment editors).
    EscapePressed,
}

#[derive(Default)]
struct MouseStateHandles {
    /// Hover state for the overall link tooltip.
    link_tooltip_mouse_handle: MouseStateHandle,
    open_link_mouse_handle: MouseStateHandle,
    copy_link_mouse_handle: MouseStateHandle,
    edit_link_mouse_handle: MouseStateHandle,
    secondary_link_mouse_handle: MouseStateHandle,
}

// Represents the states of an ongoing mouse event. Note that these states are mutually exclusive:
// If one is selecting, they couldn't be initiating task list toggling at the same time.
enum OngoingMouseEvent {
    Selecting,
    None,
}

/// Configuration for the link tooltip.
struct LinkToolTipConfig {
    url: String,
    editable: bool,
    state: LinkState,
}

enum LinkState {
    Resolving(SpawnedFutureHandle),
    Resolved(LinkTarget),
    Broken(ResolveError),
}

impl Drop for LinkState {
    fn drop(&mut self) {
        // If the link tooltip is dismissed while resolving, skip resolution.
        if let LinkState::Resolving(handle) = self {
            handle.abort();
        }
    }
}

/// Represents a file path that has been hovered or selected by the user.
#[derive(Clone)]
struct SelectedFilePath {
    range: Range<CharOffset>,
    path: PathBuf,
    line_and_column_num: Option<LineAndColumnArg>,
}

#[derive(Default)]
struct FilePathMouseStateHandles {
    open_file_handle: MouseStateHandle,
    open_in_warp_handle: MouseStateHandle,
}

pub struct RichTextEditorView {
    pub(super) model: ModelHandle<NotebooksEditorModel>,
    display_state: DisplayStateHandle,
    scroll_state: ScrollStateHandle,
    self_handle: WeakViewHandle<Self>,
    ongoing_mouse_state: OngoingMouseEvent,
    hovered_block: Option<CharOffset>,
    links: ModelHandle<NotebookLinks>,
    open_link: Option<LinkToolTipConfig>,
    mouse_states: MouseStateHandles,

    debug_mode: bool,

    omnibar: ViewHandle<Omnibar>,
    link_editor: ViewHandle<LinkEditor>,
    requested_link_editor_open: AtomicBool,
    requested_block_insertion_menu_open: bool,
    link_editor_open: bool,
    pub(super) insertion_menu_state: BlockInsertionMenuState,
    pending_layout_affecting_asset_loads: HashSet<AssetHandle>,

    pub(super) find_bar: FindBarState,
    max_width: Option<Pixels>,

    hovered_file_path: Option<SelectedFilePath>,
    open_file_path: Option<SelectedFilePath>,
    file_path_mouse_states: FilePathMouseStateHandles,

    // Whether the editor or it's children are focused.
    has_focus_within: bool,
    gutter_width: f32,
    vertical_expansion_behavior: VerticalExpansionBehavior,

    /// When true, Cmd/Ctrl+Enter runs selected shell commands.
    /// When false, Cmd/Ctrl+Enter emits a CmdEnter event instead (used by comment editors to submit).
    can_execute_shell_commands: bool,

    /// When true, the editor content is not wrapped in a Scrollable, allowing scroll events
    /// to propagate to the parent. Used for embedded editors like comment chips.
    disable_scrolling: bool,

    /// When true, the block insertion menu (slash menu) is disabled.
    disable_block_insertion_menu: bool,
}

#[derive(Default)]
pub struct RichTextEditorConfig {
    pub max_width: Option<Pixels>,
    pub gutter_width: Option<f32>,
    pub vertical_expansion_behavior: Option<VerticalExpansionBehavior>,

    /// Enable or disable embedded objects (notebooks, workflows) in the block insertion menu.
    pub embedded_objects_enabled: Option<bool>,

    /// Configure whether this editor can execute shell commands via Cmd/Ctrl+Enter.
    /// When disabled, Cmd/Ctrl+Enter emits a CmdEnter event instead, allowing parent views
    /// (like comment editors) to handle it for submitting comments.
    pub can_execute_shell_commands: Option<bool>,

    /// When true, the editor content is not wrapped in a Scrollable, allowing scroll events
    /// to propagate to the parent. Used for embedded editors like comment chips.
    pub disable_scrolling: bool,

    /// Enable or disable the block insertion menu (slash menu).
    /// When disabled, typing "/" will not open the menu.
    pub disable_block_insertion_menu: bool,

    /// When true, adjacent plain-text lines are laid out as one text block instead of separate
    /// paragraph blocks. This avoids inter-paragraph spacing for compact editors such as comments.
    pub group_plain_text_lines: bool,
}

impl RichTextEditorView {
    pub fn new(
        parent_position_id: String,
        model: ModelHandle<NotebooksEditorModel>,
        links: ModelHandle<NotebookLinks>,
        config: RichTextEditorConfig,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let appearance_handle = Appearance::handle(ctx);
        let font_settings_handle = FontSettings::handle(ctx);
        ctx.subscribe_to_model(&appearance_handle, |me, _, _, ctx| {
            me.handle_appearance_or_font_change(ctx);
        });

        ctx.subscribe_to_model(&font_settings_handle, |me, _, _, ctx| {
            me.handle_appearance_or_font_change(ctx);
        });

        ctx.subscribe_to_model(
            &FallbackFontModel::handle(ctx),
            Self::handle_fallback_font_event,
        );

        // Re-render tooltips that include notebook-specific keybindings.
        ctx.observe(&NotebookKeybindings::handle(ctx), |_, _, ctx| ctx.notify());

        ctx.subscribe_to_model(
            &WindowManager::handle(ctx),
            Self::handle_windowing_state_event,
        );

        ctx.subscribe_to_model(&model, Self::handle_model_event);

        ctx.observe(&model, |_, _, ctx| ctx.notify());

        // Ensure that we re-render when the rendering model changes.
        ctx.observe(&model.as_ref(ctx).render_state().clone(), |me, _, ctx| {
            me.watch_layout_affecting_asset_loads(ctx);
            ctx.notify();
        });

        let omnibar = ctx.add_typed_action_view(|ctx| Omnibar::new(model.clone(), ctx));
        ctx.subscribe_to_view(&omnibar, Self::handle_omnibar_event);

        let link_editor = ctx.add_typed_action_view(|ctx| LinkEditor::new(model.clone(), ctx));
        ctx.subscribe_to_view(&link_editor, Self::handle_link_editor_event);

        let find_bar = FindBarState::new(parent_position_id, model.clone(), ctx);
        ctx.subscribe_to_view(find_bar.view(), Self::handle_find_bar_event);

        let insertion_menu_state =
            BlockInsertionMenuState::new(ctx, config.embedded_objects_enabled.unwrap_or(true));
        if config.group_plain_text_lines {
            model
                .as_ref(ctx)
                .render_state()
                .clone()
                .update(ctx, |render_state, _| {
                    render_state.set_group_plain_text_lines(true);
                });
        }

        Self {
            omnibar,
            link_editor,
            model,
            display_state: Default::default(),
            scroll_state: Default::default(),
            self_handle: ctx.handle(),
            ongoing_mouse_state: OngoingMouseEvent::None,
            debug_mode: false,
            mouse_states: Default::default(),
            hovered_block: None,
            open_link: None,
            links,
            link_editor_open: false,
            requested_link_editor_open: Default::default(),
            requested_block_insertion_menu_open: Default::default(),
            insertion_menu_state,
            pending_layout_affecting_asset_loads: Default::default(),
            hovered_file_path: None,
            open_file_path: None,
            file_path_mouse_states: Default::default(),
            find_bar,
            max_width: config.max_width,
            has_focus_within: false,
            gutter_width: config.gutter_width.unwrap_or(GUTTER_WIDTH),
            vertical_expansion_behavior: config.vertical_expansion_behavior.unwrap_or_default(),
            can_execute_shell_commands: config.can_execute_shell_commands.unwrap_or(true),
            disable_scrolling: config.disable_scrolling,
            disable_block_insertion_menu: config.disable_block_insertion_menu,
        }
    }

    pub(super) fn disable_block_insertion_menu(&self) -> bool {
        self.disable_block_insertion_menu
    }

    fn handle_omnibar_event(
        &mut self,
        _handle: ViewHandle<Omnibar>,
        event: &OmnibarEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(event, OmnibarEvent::OpenLinkEditor) {
            self.open_link_editor(ctx);
        }
    }

    fn handle_link_editor_event(
        &mut self,
        _handle: ViewHandle<LinkEditor>,
        event: &LinkEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(event, LinkEditorEvent::Close) {
            self.close_link_editor(ctx)
        }
    }

    fn open_link_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.link_editor.update(ctx, |link_editor, ctx| {
            link_editor.focus_url_editor(ctx);
            link_editor.populate(ctx)
        });
        self.link_editor_open = true;
        ctx.notify();
    }

    fn close_link_editor(&mut self, ctx: &mut ViewContext<Self>) {
        if self.link_editor_open {
            self.link_editor_open = false;
            ctx.notify();
            ctx.focus_self();
        }
    }

    /// Handles rich text model changes.
    fn handle_model_event(
        &mut self,
        _handle: ModelHandle<NotebooksEditorModel>,
        event: &RichTextEditorModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            RichTextEditorModelEvent::ContentChanged(origin) => {
                // Do not emit `Edited` events for system edits. System edits are either:
                // - Transient state that should not be persisted (autosuggestions, syntax highlighting, etc.)
                // - Cloud updates, which must not be echoed back
                if origin.from_user() {
                    // Similar to link editor, if the edit just triggered the slash menu
                    // to open, don't close the block insertion menu.
                    if self.requested_block_insertion_menu_open {
                        self.requested_block_insertion_menu_open = false;
                    } else {
                        self.close_block_insertion_menu(ctx);
                    }

                    if self.hovered_file_path.is_some() {
                        self.hovered_file_path = None;
                        ctx.notify();
                    }

                    if self.open_file_path.is_some() {
                        self.open_file_path = None;
                        ctx.notify();
                    }

                    ctx.emit(EditorViewEvent::Edited)
                }
                self.reset_for_editing_change(ctx);
            }
            RichTextEditorModelEvent::ActiveStylesChanged { .. } => {
                self.reset_for_editing_change(ctx);
                ctx.emit(EditorViewEvent::TextSelectionChanged);
            }
            RichTextEditorModelEvent::SwitchedSelectionMode { new_mode } => {
                ctx.emit(EditorViewEvent::ChangedSelectionMode(*new_mode))
            }
        }
    }

    /// Reset UI state that depends on the current content or selection.
    /// * Resets the timer for blinking the cursor
    /// * Closes the link tooltip or editor
    fn reset_for_editing_change(&mut self, ctx: &mut ViewContext<Self>) {
        // If the content or active style has changed, we need to clear the open link tooltip as it might
        // no longer be accurate.
        if self.open_link.take().is_some() {
            ctx.notify();
        }

        // This AtomicBool is needed to properly display the link editor. Normally we would
        // want to hide the link editor on selection / content change because the underlying selection
        // may no longer be the link user is editing. But when the link editor is triggered from the
        // tooltip, we would expand the selection first and attempt to open the link editor. For this
        // case, we set this mutex to ignore the next check on selection / content change.
        let requested_link_editor_open = self.requested_link_editor_open.get_mut();
        if *requested_link_editor_open {
            *requested_link_editor_open = false;
        } else {
            self.close_link_editor(ctx);
        }

        self.display_state.reset_cursor_blink_timer();
    }

    /// Handles [`Appearance`] changes by updating the render model.
    fn handle_appearance_or_font_change(&mut self, ctx: &mut ViewContext<Self>) {
        let font_settings = FontSettings::as_ref(ctx);
        let appearance = Appearance::as_ref(ctx);
        let new_styles = rich_text_styles(appearance, font_settings);
        self.model.update(ctx, move |model, ctx| {
            model.update_rich_text_styles(new_styles, ctx);
        });
    }

    fn handle_fallback_font_event(
        &mut self,
        _: ModelHandle<FallbackFontModel>,
        event: &FallbackFontEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            FallbackFontEvent::Loaded => {
                // TODO(PLAT-748): We could potentially check if the notebook needs to
                // be rebuilt, by checking if the TextFrames have missing chars.
                self.model.update(ctx, |model, ctx| {
                    model.rebuild_layout(ctx);
                });
            }
        }
    }

    fn layout_affecting_asset_loads(&self, ctx: &ViewContext<Self>) -> LayoutAffectingAssetLoads {
        let render_state = self.model.as_ref(ctx).render_state().clone();
        let render_state = render_state.as_ref(ctx);
        let asset_cache = AssetCache::as_ref(ctx);
        let mut loads = LayoutAffectingAssetLoads::default();

        render_state
            .content()
            .block_items()
            .filter_map(|block| Self::layout_affecting_asset_load(block, asset_cache))
            .for_each(|load| match load {
                LayoutAffectingAssetLoad::Loading(handle) => {
                    loads.loading.insert(handle);
                }
                LayoutAffectingAssetLoad::LoadedNeedsRelayout => {
                    loads.loaded_needs_relayout = true;
                }
            });
        loads
    }

    fn layout_affecting_asset_load(
        block: &BlockItem,
        asset_cache: &AssetCache,
    ) -> Option<LayoutAffectingAssetLoad> {
        match block {
            BlockItem::MermaidDiagram {
                asset_source,
                config,
                ..
            } => match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loading { handle } => Some(LayoutAffectingAssetLoad::Loading(handle)),
                AssetState::Loaded { data } => {
                    mermaid_diagram_needs_loaded_layout(config, data.as_ref())
                        .then_some(LayoutAffectingAssetLoad::LoadedNeedsRelayout)
                }
                AssetState::Evicted | AssetState::FailedToLoad(_) => None,
            },
            // Mermaid-labeled code blocks that are currently rendered as code (because the
            // Mermaid source has not yet been verified as parseable) also need to be watched
            // so we can re-run layout when the asset load resolves.
            BlockItem::RunnableCodeBlock {
                pending_mermaid_asset: Some(asset_source),
                ..
            } => match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loading { handle } => Some(LayoutAffectingAssetLoad::Loading(handle)),
                AssetState::Loaded { .. } | AssetState::Evicted | AssetState::FailedToLoad(_) => {
                    None
                }
            },
            _ => None,
        }
    }

    fn watch_layout_affecting_asset_loads(&mut self, ctx: &mut ViewContext<Self>) {
        let loads = self.layout_affecting_asset_loads(ctx);
        if loads.loaded_needs_relayout {
            self.model.update(ctx, |model, ctx| {
                model.rebuild_layout(ctx);
            });
        }

        for handle in loads.loading {
            if self
                .pending_layout_affecting_asset_loads
                .insert(handle.clone())
            {
                let asset_cache = AssetCache::as_ref(ctx);
                if let Some(future) = handle.when_loaded(asset_cache) {
                    ctx.spawn(future, move |me, (), ctx| {
                        me.pending_layout_affecting_asset_loads.remove(&handle);
                        me.model.update(ctx, |model, ctx| {
                            if Self::should_rebuild_layout_after_layout_affecting_asset_load(
                                model.interaction_state(ctx),
                            ) {
                                model.rebuild_layout(ctx);
                            }
                        });
                        ctx.notify();
                    });
                } else {
                    self.pending_layout_affecting_asset_loads.remove(&handle);
                }
            }
        }
    }

    fn should_rebuild_layout_after_layout_affecting_asset_load(state: InteractionState) -> bool {
        matches!(
            state,
            InteractionState::Selectable
                | InteractionState::Editable
                | InteractionState::EditableWithInvalidSelection
        )
    }

    /// Handles changes to the lifecycle state.
    fn handle_windowing_state_event(
        &mut self,
        _handle: ModelHandle<WindowManager>,
        event: &windowing::StateEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let windowing::StateEvent::ValueChanged { current, previous } = event;
        let focused = ctx.is_self_or_child_focused();
        let previously_focused = focused && previous.active_window == Some(ctx.window_id());
        let currently_focused = focused && current.active_window == Some(ctx.window_id());

        if !previously_focused && currently_focused {
            // Re-render to show cursors.
            self.display_state.reset_cursor_blink_timer();
            ctx.notify();
        } else if previously_focused && !currently_focused {
            // Re-render to hide cursors.
            ctx.notify();
        }
    }

    /// The editor model backing this view.
    pub fn model(&self) -> &ModelHandle<NotebooksEditorModel> {
        &self.model
    }

    pub fn markdown(&self, ctx: &AppContext) -> String {
        self.model.as_ref(ctx).markdown(ctx)
    }

    pub fn markdown_unescaped(&self, ctx: &AppContext) -> String {
        self.model.as_ref(ctx).markdown_unescaped(ctx)
    }

    pub fn interaction_state(&self, app: &AppContext) -> InteractionState {
        self.model.as_ref(app).interaction_state(app)
    }

    pub fn set_interaction_state(&mut self, state: InteractionState, ctx: &mut ViewContext<Self>) {
        if state != self.interaction_state(ctx) {
            // Force a re-render since the cursor and selection depend on interaction state.
            ctx.notify();
        }
        self.model
            .update(ctx, |model, ctx| model.set_interaction_state(state, ctx));
    }

    /// Whether an edit operation (insert, backspace, change style, etc.) should be allowed. Edits are allowed if:
    /// * The view's [`InteractionState`] is `Editable`
    /// * The view is focused
    pub(super) fn can_edit(&self, ctx: &mut ViewContext<Self>) -> bool {
        self.is_editable(ctx) && ctx.is_self_or_child_focused()
    }

    /// Whether an edit operation should be allowed, when only an [`AppContext`] is available.
    /// Where possible, prefer [`Self::can_edit`].
    pub(super) fn can_edit_app(&self, app: &AppContext) -> bool {
        self.is_editable(app) && self.is_focused(app)
    }

    /// Whether or not the editor or a child is focused.
    /// The focus state is cached in the `on_focus` and `on_blur` handlers.
    /// The editor is considered focused if it or any of its children are focused and the
    /// window is active.
    fn is_focused(&self, app: &AppContext) -> bool {
        let Some(handle) = self.self_handle.upgrade(app) else {
            return false;
        };

        // If our window is not active, we don't have user focus, even if we're focused within the app.
        if app.windows().state().active_window != Some(handle.window_id(app)) {
            return false;
        }

        self.has_focus_within
    }

    /// Whether or not the editor should accept user-typed characters (assuming it's focused and
    /// editable).
    ///
    /// This is normally true, unless an overlay editor like the link editor or find bar is
    /// focused, or if there's a command selection.
    fn should_handle_user_input(&self, app: &AppContext) -> bool {
        !(self.link_editor.as_ref(app).editors_focused(app)
            || self.find_bar.is_focused(app)
            || self.model.as_ref(app).has_command_selection(app)
            || self.insertion_menu_state.embedded_object_search_open)
    }

    /// Whether or not the view is currently editable.
    pub fn is_editable(&self, app: &AppContext) -> bool {
        matches!(self.interaction_state(app), InteractionState::Editable)
    }

    /// Update the editor model with user typed content.
    pub fn user_typed(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        if self.is_editable(ctx) && self.should_handle_user_input(ctx) {
            if !self.disable_block_insertion_menu
                && content == "/"
                && self.selection_is_single_cursor(ctx)
            {
                // Check if previous character is not a digit (i.e. writing dates)
                let prev_char = self
                    .model
                    .read(ctx, |model, ctx| model.prev_char_in_non_code_block(ctx));
                if prev_char.is_some_and(|c| !c.is_ascii_digit()) {
                    self.requested_block_insertion_menu_open = true;
                    self.open_block_insertion_menu(BlockInsertionSource::AtCursor, ctx);
                }
            }

            self.model.update(ctx, |model, ctx| {
                model.user_insert(content, ctx);
            });
        }
    }

    pub fn insert_formatted_from_paste(
        &mut self,
        formatted_text: FormattedText,
        plain_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_editable(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.insert_formatted_from_paste(formatted_text, plain_text, ctx);
            });
        }
    }

    /// Return whether there is currently one selection, either a single cursor or a single range.
    pub fn has_single_selection(&self, ctx: &AppContext) -> bool {
        self.model.as_ref(ctx).has_single_selection(ctx)
    }

    /// Whether or not the current text selection is a single cursor.
    pub fn selection_is_single_cursor(&self, ctx: &AppContext) -> bool {
        self.model.as_ref(ctx).selection_is_single_cursor(ctx)
    }

    fn selection_head_in_render_coordinates(&self, ctx: &AppContext) -> CharOffset {
        self.model.as_ref(ctx).selection_head(ctx) + 1
    }

    fn selection_update_offset_for_rendered_mermaid_block(
        &self,
        block_range: Range<CharOffset>,
        ctx: &AppContext,
    ) -> Option<CharOffset> {
        if !FeatureFlag::EditableMarkdownMermaid.is_enabled() {
            return None;
        }
        let selection_head_in_render_coordinates = self.selection_head_in_render_coordinates(ctx);
        if selection_head_in_render_coordinates <= block_range.start {
            Some(block_range.end)
        } else if selection_head_in_render_coordinates >= block_range.end {
            Some(block_range.start)
        } else {
            Some(block_range.end)
        }
    }

    /// Whether or not any command blocks are selected.
    pub fn has_command_selection(&self, ctx: &AppContext) -> bool {
        self.model.as_ref(ctx).has_command_selection(ctx)
    }

    pub fn enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.model.as_ref(ctx).has_command_selection(ctx) {
            self.model
                .update(ctx, |model, ctx| model.exit_command_selection(ctx));
        } else if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| model.enter(ctx));
        }
    }

    pub fn shift_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| model.newline(ctx));
        }
    }

    pub fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_editable(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.backspace(ctx);
            });
        }
    }

    /// Generic delete action. This wraps the model-level `delete` logic to check that the
    /// view is editable.
    fn delete(
        &mut self,
        direction: TextDirection,
        unit: TextUnit,
        cut: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.delete(direction, unit, cut, ctx);
            });
        }
    }

    pub fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.undo(ctx);
            });
        }
    }

    pub fn redo(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.redo(ctx);
            });
        }
    }

    pub fn system_clear_buffer(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.clear_buffer(ctx);
        });
    }

    pub fn reset_with_markdown(&mut self, markdown: &str, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.reset_with_markdown(markdown, ctx);
        });
    }

    /// Open the link editor modal. If `require_existing` is `true`, there *must* be an existing
    /// link to edit at the cursor (for example, we're opening a link from the tooltip). Otherwise,
    /// the link editor is always opened (for example, to insert a new link).
    fn edit_link(&mut self, require_existing: bool, ctx: &mut ViewContext<Self>) {
        // If there is more than one selection, don't show this link editor.
        if !self.has_single_selection(ctx) {
            return;
        }

        self.open_link = None;

        let updated_selection = self
            .model
            .update(ctx, |model, ctx| model.try_select_active_link(ctx));

        if updated_selection || !require_existing {
            self.requested_link_editor_open
                .store(true, Ordering::Relaxed);
            self.open_link_editor(ctx);
        } else {
            ctx.notify();
        }
    }

    /// Close any UI elements that overlay the editor. This does _not_ close the omnibar, since
    /// it's toggled by selection changes and can't be explicitly opened/closed.
    fn close_overlays(&mut self, ctx: &mut ViewContext<Self>) {
        if self.open_link.take().is_some() {
            ctx.notify();
        }

        self.close_link_editor(ctx);
        self.close_block_insertion_menu(ctx);
        ctx.focus_self();
    }

    /// Scroll by `delta` pixels.
    fn scroll(&mut self, delta: Pixels, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.render_state().update(ctx, |render_state, ctx| {
                render_state.scroll(delta, ctx);
            })
        })
    }

    /// Move the cursor to the start of the buffer.
    pub fn cursor_start(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.cursor_at(CharOffset::zero(), ctx);
        });
    }

    pub fn select_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.select_up(ctx);
        });
    }

    pub fn select_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.select_down(ctx);
        });
    }

    pub fn select_left(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.select_left(ctx);
        });
    }

    pub fn select_right(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.select_right(ctx);
        });
    }

    /// Begin word semantic selection.
    pub fn select_word(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ongoing_mouse_state = OngoingMouseEvent::Selecting;
        self.model.update(ctx, |model, ctx| {
            model.select_word_at(offset, multiselect, ctx);
        });
        ctx.notify();
    }

    /// Begin line semantic selection.
    pub fn select_line(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ongoing_mouse_state = OngoingMouseEvent::Selecting;
        self.model.update(ctx, |model, ctx| {
            model.select_line_at(offset, multiselect, ctx);
        });
        ctx.notify();
    }

    /// Select the command above the current selection.
    fn command_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.model
            .update(ctx, |model, ctx| model.select_command_up(ctx));
        ctx.emit(EditorViewEvent::NavigatedCommands);
    }

    /// Select the command below the current selection.
    fn command_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.model
            .update(ctx, |model, ctx| model.select_command_down(ctx));
        ctx.emit(EditorViewEvent::NavigatedCommands);
    }

    pub fn move_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.model.as_ref(ctx).has_command_selection(ctx) {
            self.command_up(ctx);
        } else {
            self.model.update(ctx, |model, ctx| model.move_up(ctx));
        }
    }

    pub fn move_down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.model.as_ref(ctx).has_command_selection(ctx) {
            self.command_down(ctx);
        } else {
            self.model.update(ctx, |model, ctx| model.move_down(ctx));
        }
    }

    pub fn move_left(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.move_left(ctx);
        });
    }

    pub fn move_right(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.move_right(ctx);
        });
    }

    /// Handler for a Tab keypress.
    pub fn tab(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.indent(false, ctx);
        } else {
            ctx.emit(EditorViewEvent::Navigate(NavigationKey::Tab));
        }
    }

    fn indent(&mut self, shift: bool, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.indent(shift, ctx);
            });
        }
    }

    /// Handler for a Tab keypress with Shift held down.
    pub fn shift_tab(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.indent(true, ctx);
        } else {
            ctx.emit(EditorViewEvent::Navigate(NavigationKey::ShiftTab));
        }
    }

    pub fn toggle_style(&mut self, text_style: TextStyles, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.toggle_style(text_style, ctx);
            });
        }
    }

    /// Updates the model to move the cursor to an exact content location.
    fn selection_start(
        &mut self,
        offset: CharOffset,
        multiselect: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Clicking into the editor should restore focus.
        self.focus(ctx);

        self.ongoing_mouse_state = OngoingMouseEvent::Selecting;
        let had_command_selection = self
            .model
            .update(ctx, |model, ctx| model.select_at(offset, multiselect, ctx));
        if had_command_selection {
            ctx.emit(EditorViewEvent::ChangedSelectionMode(SelectionMode::Text));
        }
    }

    /// Updates the current selection that is being dragged.  This should be called after
    /// `selection_start` and before `selection_end`.
    fn selection_update(&mut self, offset: CharOffset, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.update_pending_selection(offset, ctx);
        });
    }

    fn selection_end(&mut self, ctx: &mut ViewContext<Self>) {
        self.ongoing_mouse_state = OngoingMouseEvent::None;
        self.model
            .update(ctx, |model, ctx| model.end_selection(ctx));
        ctx.notify();
    }

    /// Updates the head of the most recent selection.  This is used for shift-clicking.
    fn selection_extend(&mut self, offset: CharOffset, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.set_last_selection_head(offset, ctx);
        });
    }

    /// Select a block by its starting offset. Currently, block-level selection only applies to
    /// workflow/command blocks.
    fn select_block(&mut self, block_start: CharOffset, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
        self.model
            .update(ctx, |model, ctx| model.select_command_at(block_start, ctx))
    }

    fn maybe_open_file_or_url(
        &mut self,
        offset: CharOffset,
        cmd: bool,
        link_in_text: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Don't open links when the user dragged to make a text selection.
        if !self.selection_is_single_cursor(ctx) {
            return;
        }
        if self.maybe_open_file(offset, cmd, ctx) {
            return;
        }
        // If no file was hovered, check if the user is hovering over a URL.
        self.maybe_open_url(offset, cmd, link_in_text, ctx);
    }

    /// If the user is hovering over a file path, select it. Otherwise, do nothing.
    /// Returns whether a file was hovered at the given offset.
    fn maybe_open_file(
        &mut self,
        offset: CharOffset,
        cmd: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let Some(hovered_file_path) = &self.hovered_file_path {
            if hovered_file_path.range.start <= offset && offset <= hovered_file_path.range.end {
                // In read-only comment chips (Selectable), open the file directly
                // on click instead of showing a tooltip.
                if cmd || matches!(self.interaction_state(ctx), InteractionState::Selectable) {
                    ctx.emit(EditorViewEvent::OpenFile {
                        path: hovered_file_path.path.clone(),
                        line_and_column_num: hovered_file_path.line_and_column_num,
                        force_open_in_warp: false,
                    });
                } else {
                    self.open_file_path = Some(hovered_file_path.clone());
                    ctx.notify();
                }
                return true;
            }
        }
        false
    }

    // If there is a link at the given offset, open the link. Otherwise do nothing.
    fn maybe_open_url(
        &mut self,
        offset: CharOffset,
        cmd: bool,
        link_in_text: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let link_at_offset = self.model.as_ref(ctx).link_url_at(offset, ctx);

        // The link should not be editable if it is auto-detected.
        let (url, editable) = match (link_at_offset, link_in_text) {
            (Some(url), _) => (Some(url), true),
            (None, Some(url)) => (Some(url), false),
            (None, None) => (None, false),
        };

        if let Some(url) = url {
            if url.starts_with('#')
                && (cmd || matches!(self.interaction_state(ctx), InteractionState::Selectable))
            {
                let scrolled = self
                    .model
                    .update(ctx, |model, ctx| model.scroll_to_matching_header(&url, ctx));
                if scrolled {
                    self.open_link = None;
                    ctx.notify();
                    return;
                }
            }
            // In read-only comment chips (Selectable), open the link directly on
            // click instead of showing a tooltip.
            if cmd || matches!(self.interaction_state(ctx), InteractionState::Selectable) {
                self.links
                    .update(ctx, |links, ctx| links.resolve_and_open(&url, ctx));
            } else {
                let resolve_future = ctx.spawn(
                    self.links.as_ref(ctx).resolve(&url, ctx),
                    |me, resolved, ctx| {
                        let new_state = match resolved {
                            Ok(target) => LinkState::Resolved(target),
                            Err(err) => LinkState::Broken(err),
                        };
                        if let Some(config) = &mut me.open_link {
                            config.state = new_state;
                            ctx.notify();
                        }
                    },
                );
                self.open_link = Some(LinkToolTipConfig {
                    url,
                    editable,
                    state: LinkState::Resolving(resolve_future),
                });
                ctx.notify();
            }
        }
    }

    /// Make this editor the focused view.
    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        // On focus, notify parent views. This lets us keep the pane tree up to date.
        ctx.emit(EditorViewEvent::Focused);
    }

    /// Returns the currently selected text, or `None` if there is no selection.
    pub fn selected_text(&self, app: &AppContext) -> Option<String> {
        let text = self.model.as_ref(app).selected_text(app);
        if text.is_empty() {
            return None;
        }
        Some(text)
    }

    /// Clears any active text selection by collapsing all selections to cursors.
    pub fn clear_text_selection(&self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.collapse_selections_to_cursors(ctx);
        });
    }

    /// Copy the current selection.
    pub fn copy(&self, entrypoint: ActionEntrypoint, ctx: &mut ViewContext<Self>) {
        if let Some(block) = self.model.update(ctx, |model, ctx| model.copy(ctx)) {
            ctx.emit(EditorViewEvent::CopiedBlock { block, entrypoint });
        }
    }

    /// Cuts the current selection.
    pub fn cut(&mut self, entrypoint: ActionEntrypoint, ctx: &mut ViewContext<Self>) {
        if self.is_editable(ctx) {
            if let Some(block) = self.model.update(ctx, |model, ctx| model.cut(ctx)) {
                ctx.emit(EditorViewEvent::CopiedBlock { block, entrypoint });
            }
        }
    }

    /// Paste from the clipboard.
    pub fn paste(&mut self, ctx: &mut ViewContext<Self>) {
        let content = ctx.clipboard().read();
        self.paste_content(content, ctx);
    }

    fn middle_click_paste(&mut self, ctx: &mut ViewContext<Self>) {
        let content = SelectionSettings::handle(ctx).update(ctx, |selection, ctx| {
            selection.read_for_middle_click_paste(ctx)
        });
        if let Some(content) = content {
            self.paste_content(content, ctx);
        }
    }

    fn paste_content(&mut self, content: ClipboardContent, ctx: &mut ViewContext<Self>) {
        let parsed_html = content.html.and_then(|text| parse_html(text.as_str()).ok());

        // If we failed to get the html string, try parsing plain text into markdown first.
        // If that failed as well, fall back to pasting plain text string.
        match parsed_html.or(parse_markdown(content.plain_text.as_str()).ok()) {
            Some(text) => self.insert_formatted_from_paste(text, content.plain_text.as_str(), ctx),
            None => self.user_typed(content.plain_text.as_str(), ctx),
        }
    }

    /// Inserts a new `block_type` block after the hovered block.
    pub(super) fn insert_block(
        &mut self,
        block_type: warp_editor::content::text::BlockType,
        ctx: &mut ViewContext<Self>,
    ) {
        enum InsertionMode {
            DeleteSlashAndRestyleLine(CharOffset),
            InsertAfter(CharOffset),
            DeleteSlashAndInsertAfter(CharOffset),
        }
        if self.can_edit(ctx) {
            let insertion_mode = match self.insertion_menu_state.open_at_source {
                Some(BlockInsertionSource::AtCursor) if self.selection_is_single_cursor(ctx) => {
                    let cursor_position = self.model.as_ref(ctx).selection_head(ctx);
                    let logical_line_start = self
                        .model
                        .as_ref(ctx)
                        .logical_line_start(cursor_position, ctx);
                    let logical_line_end = self
                        .model
                        .as_ref(ctx)
                        .logical_line_end(cursor_position, ctx);

                    // Check if "/" is the only character in the line.
                    if logical_line_start == cursor_position - 1
                        && logical_line_end == cursor_position + 1
                    {
                        InsertionMode::DeleteSlashAndRestyleLine(cursor_position)
                    } else {
                        // Note that we need to minus one here as the "/" is going to be deleted if user
                        // is inserting a block from the slash menu.
                        InsertionMode::DeleteSlashAndInsertAfter(cursor_position - 1)
                    }
                }
                Some(BlockInsertionSource::BlockInsertionButton)
                    if self.hovered_block.is_some() =>
                {
                    InsertionMode::InsertAfter(self.hovered_block.expect("Just checked above"))
                }
                _ => return,
            };

            self.close_block_insertion_menu(ctx);
            self.model.update(ctx, |model, ctx| {
                match insertion_mode {
                    InsertionMode::InsertAfter(insertion_offset) => {
                        // TODO(CLD-557)
                        model.insert_block_after(insertion_offset + 1, block_type, ctx);
                    }
                    InsertionMode::DeleteSlashAndInsertAfter(insertion_offset) => {
                        model.backspace(ctx);
                        // TODO(CLD-557)
                        model.insert_block_after(insertion_offset + 1, block_type, ctx);
                    }
                    InsertionMode::DeleteSlashAndRestyleLine(cursor_position) => match block_type {
                        warp_editor::content::text::BlockType::Item(item) => {
                            // Set one more offset position to the left to avoid additional linebreaks.
                            // Note: We can use `set_last_selection_head` because the menu cannot
                            // be opened when there are multiple selections.
                            model.set_last_selection_head(cursor_position - 1, ctx);
                            model.insert_block_item(item, ctx);
                            model.cursor_at(cursor_position + 1, ctx);
                        }
                        warp_editor::content::text::BlockType::Text(style) => {
                            // Note: We can use `set_last_selection_head` because the menu cannot
                            // be opened when there are multiple selections.
                            model.set_last_selection_head(cursor_position, ctx);
                            model.set_block_style(style, ctx);
                            model.backspace(ctx);
                        }
                    },
                }
            });
        }
    }

    fn block_hovered(
        &mut self,
        block_start: Option<CharOffset>,
        char_offset: Option<CharOffset>,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(
            self.insertion_menu_state.open_at_source,
            Some(BlockInsertionSource::BlockInsertionButton)
        ) {
            // While the block insertion menu is open, we keep the hovered block fixed to its last
            // location. This ensures that moving the mouse to use the menu won't affect where
            // content is added.
            return;
        }

        if block_start != self.hovered_block {
            self.hovered_block = block_start;
            ctx.notify();
        }

        let Some(char_offset) = char_offset else {
            self.hovered_file_path = None;
            return;
        };

        // Early return if char_offset is already on the hovered file path
        if let Some(hovered) = &self.hovered_file_path {
            if hovered.range.start <= char_offset && char_offset <= hovered.range.end {
                return;
            }
        }

        self.hovered_file_path = None;

        #[cfg(feature = "local_fs")]
        {
            // Check for file paths at the hovered word, expanding to include the previous word
            // to detect line ranges like "file.rs (16-30)" when hovering over "(16-30)"
            let buffer = self.model.as_ref(ctx).content().as_ref(ctx);
            let Some(current_word_range) = get_word_range_at_offset(
                buffer,
                char_offset,
                Some(WordBoundariesPolicy::OnlyWhitespace),
            ) else {
                return;
            };
            let Some(file_link_resolution_context) =
                self.model.as_ref(ctx).file_link_resolution_context()
            else {
                return;
            };

            // Get the previous word to include in our search context
            // This allows us to detect "file.rs (16-30)" when hovering over "(16-30)"
            let search_start = if current_word_range.start > CharOffset::from(1) {
                // Try to get the word before the current word. Skip over expected space
                let before_current = current_word_range.start - CharOffset::from(2);
                get_word_range_at_offset(
                    buffer,
                    before_current,
                    Some(WordBoundariesPolicy::OnlyWhitespace),
                )
                .map(|range| range.start)
                .unwrap_or(current_word_range.start)
            } else {
                current_word_range.start
            };

            // Create expanded context range from previous word start to current word end
            let context_range = search_start..current_word_range.end;

            // Detect ALL file paths in the context (including line ranges)
            let detected_links = detect_file_paths(
                &file_link_resolution_context.working_directory,
                buffer.text_in_range(context_range.clone()).as_str(),
                file_link_resolution_context.shell_launch_data.as_ref(),
            );

            // Find which detected link (if any) contains the hovered char_offset
            for (link_range, link_type) in detected_links {
                // Adjust link_range which is relative to context_range to absolute buffer offsets
                let absolute_range = search_start + CharOffset::from(link_range.start)
                    ..search_start + CharOffset::from(link_range.end);
                if absolute_range.contains(&char_offset) {
                    if let DetectedLinkType::FilePath {
                        absolute_path,
                        line_and_column_num,
                    } = link_type
                    {
                        self.hovered_file_path = Some(SelectedFilePath {
                            range: absolute_range,
                            path: absolute_path,
                            line_and_column_num,
                        });
                        break;
                    }
                }
            }
        }

        ctx.notify();
    }

    /// Run the currently-selected command blocks.
    ///
    /// If there is no selected command, but the text cursor is inside a command block, that
    /// command will be selected first.
    fn run_selected_commands(&self, ctx: &mut ViewContext<Self>) {
        let workflow = self.model.update(ctx, |model, ctx| {
            if !model.has_command_selection(ctx) {
                model.select_command_at_cursor(ctx);
            }
            model.selected_command_workflow(ctx)
        });

        if let Some(workflow) = workflow {
            ctx.emit(EditorViewEvent::RunWorkflow(workflow))
        }
    }

    /// Render tooltip shown when user clicks on a link in the notebook.
    fn render_link_tooltip(
        &self,
        link_url: &LinkToolTipConfig,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let background = appearance.theme().tooltip_background();

        let url = link_url.url.to_string();
        let description = url.clone();
        let mut tool_tip = Flex::row();

        // Set the text color for all elements in the tooltip
        let text_color = appearance.theme().background().into_solid();

        let link = match &link_url.state {
            LinkState::Resolved(target) => {
                let target = target.clone();
                appearance
                    .ui_builder()
                    .tooltip_link(
                        description,
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(EditorViewAction::OpenTooltipLink(
                                UserInput::new(target.clone()),
                            ));
                        })),
                        self.mouse_states.open_link_mouse_handle.clone(),
                    )
                    .soft_wrap(false)
                    .build()
                    .finish()
            }
            LinkState::Resolving(_) => {
                let text_styles = UiComponentStyles {
                    font_color: Some(text_color),
                    ..Default::default()
                };
                appearance
                    .ui_builder()
                    .span(description)
                    .with_style(text_styles)
                    .build()
                    .finish()
            }
            LinkState::Broken(error) => {
                let text_styles = UiComponentStyles {
                    font_color: Some(text_color),
                    ..Default::default()
                };
                let label = appearance
                    .ui_builder()
                    .span(description)
                    .with_style(text_styles)
                    .build()
                    .finish();

                let icon = Container::new(
                    ConstrainedBox::new(
                        Icon::new(
                            "bundled/svg/link-broken-02.svg",
                            appearance.theme().terminal_colors().normal.red,
                        )
                        .finish(),
                    )
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
                )
                .with_padding_right(4.)
                .finish();

                let icon_and_label = Flex::row().with_children([icon, label]).finish();

                let detail_tooltip_styles = UiComponentStyles {
                    background: Some(background.into()),
                    border_color: Some(appearance.theme().outline().into()),
                    border_width: Some(1.),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                    padding: Some(Coords {
                        top: 8.,
                        bottom: 8.,
                        left: 12.,
                        right: 12.,
                    }),
                    font_color: Some(text_color),
                    ..Default::default()
                };

                // We can reuse the open_link_mouse_handle state here, since the link isn't
                // rendered.
                appearance.ui_builder().styled_tool_tip_on_element(
                    error.to_string(),
                    Some(detail_tooltip_styles),
                    self.mouse_states.open_link_mouse_handle.clone(),
                    icon_and_label,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                    vec2f(0., -28.),
                )
            }
        };

        tool_tip.add_child(
            Container::new(
                ConstrainedBox::new(link)
                    .with_max_width(MAX_EDITOR_TIP_WIDTH)
                    .finish(),
            )
            .with_padding_right(12.)
            .finish(),
        );

        // Common secondary link actions:
        let ui_builder = appearance.ui_builder().clone();
        tool_tip.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .copy_button(12., self.mouse_states.copy_link_mouse_handle.clone())
                    .with_tooltip(move || {
                        ui_builder.tool_tip("Copy link".to_owned()).build().finish()
                    })
                    .build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(EditorViewAction::CopyLink))
                    .finish(),
            )
            .finish(),
        );

        // Link-specific secondary action:
        if let LinkState::Resolved(target) = &link_url.state {
            let target = target.clone();
            if let Some(secondary_action) = target.secondary_action() {
                let mut button = appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Text,
                        self.mouse_states.secondary_link_mouse_handle.clone(),
                    )
                    .with_text_label(secondary_action.label.into_owned());
                if let Some(tooltip) = secondary_action.tooltip {
                    let ui_builder = appearance.ui_builder().clone();
                    button = button.with_tooltip(move || {
                        ui_builder.tool_tip(tooltip.into_owned()).build().finish()
                    });
                }
                tool_tip.add_child(
                    Container::new(
                        button
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(EditorViewAction::SecondaryLinkAction(
                                    UserInput::new(target.clone()),
                                ))
                            })
                            .finish(),
                    )
                    .with_padding_left(12.)
                    .finish(),
                );
            }
        }

        if link_url.editable && self.is_editable(ctx) {
            tool_tip.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Text,
                            self.mouse_states.edit_link_mouse_handle.clone(),
                        )
                        .with_text_label("Edit".to_string())
                        .build()
                        .on_click(|ctx, _, _| ctx.dispatch_typed_action(EditorViewAction::EditLink))
                        .finish(),
                )
                .with_padding_left(12.)
                .finish(),
            );
        }

        let tooltip_hoverable =
            Hoverable::new(self.mouse_states.link_tooltip_mouse_handle.clone(), |_| {
                Container::new(
                    ConstrainedBox::new(
                        Container::new(tool_tip.finish())
                            .with_vertical_padding(8.)
                            .with_horizontal_padding(12.)
                            .finish(),
                    )
                    .finish(),
                )
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        Dismiss::new(tooltip_hoverable)
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(EditorViewAction::DismissOpenLink))
            .finish()
    }

    fn link_tooltip_positioning(render_state: &RenderState) -> OffsetPositioning {
        let selection_position = render_state.saved_positions().cursor_id();

        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Middle, XAxisAnchor::Middle),
            )
            .with_conditional_anchor(),
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(-4.),
                AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
            )
            .with_conditional_anchor(),
        )
    }

    /// Render tooltip shown when user hovers over a file path in the notebook.
    fn render_file_path_tooltip(
        &self,
        selected_file_path: &SelectedFilePath,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        use warpui::EventContext;
        type FilePathTooltipLinks = Vec<TooltipLink<Box<dyn Fn(&mut EventContext)>>>;

        let path = selected_file_path.path.clone();
        let line_and_column_num = selected_file_path.line_and_column_num;
        let primary_text = if path.is_dir() {
            "Open folder"
        } else {
            "Open file"
        }
        .to_string();
        let show_open_in_warp = should_show_open_in_warp_link(&path, ctx);
        let path_for_primary = path.clone();
        let modifier = directly_open_link_keybinding_string();

        let mut links: FilePathTooltipLinks = vec![TooltipLink {
            text: primary_text,
            on_click: Box::new(move |ctx: &mut EventContext| {
                ctx.dispatch_typed_action(EditorViewAction::OpenFile {
                    path: path_for_primary.clone(),
                    line_and_column_num,
                    force_open_in_warp: false,
                });
            }),
            detail: Some(format!("[{modifier} Click]")),
            mouse_state: self.file_path_mouse_states.open_file_handle.clone(),
        }];

        if show_open_in_warp {
            let path_for_warp = path.clone();
            links.push(TooltipLink {
                text: "Open in Warp".to_string(),
                on_click: Box::new(move |ctx: &mut EventContext| {
                    ctx.dispatch_typed_action(EditorViewAction::OpenFile {
                        path: path_for_warp.clone(),
                        line_and_column_num,
                        force_open_in_warp: true,
                    });
                }),
                detail: None,
                mouse_state: self.file_path_mouse_states.open_in_warp_handle.clone(),
            });
        }

        let tooltip_content = render_tooltip(links, TooltipRedaction::NoRedaction, appearance, ctx);

        let hoverable = Hoverable::new(Default::default(), move |_| tooltip_content)
            .with_cursor(Cursor::PointingHand)
            .finish();

        Dismiss::new(hoverable)
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(EditorViewAction::DismissOpenLink);
            })
            .finish()
    }

    /// Refresh all render-time decorations.
    ///
    /// Currently, we only decorate search results, but there may be more in the future (like
    /// highlights from command palette search).
    fn update_decorations(&self, ctx: &mut ViewContext<Self>) {
        let search_decorations = self.find_bar.decorations(ctx);
        self.model.update(ctx, |model, ctx| {
            model.render_state().update(ctx, |render, ctx| {
                render.set_text_decorations(search_decorations, ctx);
            });
        });
    }

    fn handle_find_bar_event(
        &mut self,
        _view: ViewHandle<FindBar>,
        event: &FindBarEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            FindBarEvent::Close => self.find_bar.hide(ctx),
            FindBarEvent::SearchDecorationsChanged => (),
        }
        self.update_decorations(ctx);
    }

    fn is_selecting(&self) -> bool {
        matches!(self.ongoing_mouse_state, OngoingMouseEvent::Selecting)
    }

    fn should_show_omnibar(&self, ctx: &AppContext) -> bool {
        let render_state = self.model.as_ref(ctx).render_state().clone();
        let selections = render_state.as_ref(ctx).selections();

        selections.len() == 1
            && !selections.first().is_cursor()
            && self.is_editable(ctx)
            && !self
                .model
                .as_ref(ctx)
                .has_single_exact_rendered_mermaid_selection(ctx)
            && !matches!(self.ongoing_mouse_state, OngoingMouseEvent::Selecting)
            && !self.is_block_insertion_menu_open()
    }

    /// Insert an embedded notebook inline link at the current insertion menu source.
    /// For now, this looks like a regular hyperlink that opens the notebook in a new tab.
    pub(super) fn insert_embedded_notebook_view(
        &mut self,
        title: String,
        link: String,
        ctx: &mut ViewContext<Self>,
    ) {
        match self.insertion_menu_state.open_at_source {
            Some(BlockInsertionSource::AtCursor) if self.selection_is_single_cursor(ctx) => {
                self.model.update(ctx, |model, ctx| {
                    // Remove slash
                    model.backspace(ctx);
                });
            }
            Some(BlockInsertionSource::BlockInsertionButton) if self.hovered_block.is_some() => {
                self.model.update(ctx, |model, ctx| {
                    model.newline(ctx);
                });
            }
            _ => return,
        };

        self.model.update(ctx, |model, ctx| {
            model.set_link(title, link, ctx);
        });
    }
}

impl Entity for RichTextEditorView {
    type Event = EditorViewEvent;
}

impl View for RichTextEditorView {
    fn ui_name() -> &'static str {
        "RichTextEditorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let editable = self.is_editable(app);
        let focused = self.is_focused(app);
        let blink_cursors = AppEditorSettings::as_ref(app).cursor_blink_enabled();

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let render_state = self.model.as_ref(app).render_state();

        let display_options = DisplayOptions {
            // If there's a command selection, show that instead of the text cursor.
            // Likewise, if the link editor is open, don't also show a cursor.
            editable: editable
                && !self.model.as_ref(app).has_command_selection(app)
                && !self.link_editor_open,
            focused,
            blink_cursors,
            left_gutter: self.gutter_width,
            debug_bounds: self.debug_mode,
            hovered_block_start: self.hovered_block,
            vertical_expansion_behavior: self.vertical_expansion_behavior,
            ..Default::default()
        };

        let rich_text = RichTextElement::<Self>::new(
            render_state.clone(),
            self.self_handle.clone(),
            display_options,
            self.display_state.clone(),
            None,       // Not currently supporting vim in notebooks
            Vec::new(), // Not currently supporting vim in notebooks
        )
        .with_max_width(self.max_width)
        .finish_scrollable();

        // When disable_scrolling is true, don't wrap in Scrollable to allow scroll events
        // to propagate to the parent (used for embedded editors like comment chips).
        let main_content: Box<dyn Element> = if self.disable_scrolling {
            rich_text
        } else {
            Scrollable::vertical(
                self.scroll_state.clone(),
                rich_text,
                SCROLLBAR_WIDTH,
                theme.disabled_text_color(theme.background()).into(),
                theme.main_text_color(theme.background()).into(),
                Fill::None,
            )
            .finish()
        };

        let mut main_stack = Stack::new();
        main_stack.add_child(main_content);

        self.render_block_insertion_menu(&mut main_stack, app);

        // Clip main editor view within bounds, and add find bar later
        let main_stack_clipped = Clipped::new(main_stack.finish()).finish();

        let mut stack = Stack::new();
        stack.add_child(main_stack_clipped);

        // The omnibar is only shown if there is only one selection, it's not a single cursor,
        // the editor is in a editable state, user
        // is not actively updating the selection and the block insertion menu isn't already open.
        let show_omnibar = self.should_show_omnibar(app);
        if show_omnibar {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.omnibar).finish(),
                Omnibar::positioning(render_state.as_ref(app)),
            );
        } else if let Some(selected_file_path) = &self.open_file_path {
            stack.add_positioned_overlay_child(
                self.render_file_path_tooltip(selected_file_path, appearance, app),
                RichTextEditorView::link_tooltip_positioning(render_state.as_ref(app)),
            )
        } else if let Some(link) = self.open_link.as_ref() {
            stack.add_positioned_overlay_child(
                self.render_link_tooltip(link, appearance, app),
                RichTextEditorView::link_tooltip_positioning(render_state.as_ref(app)),
            )
        }

        let show_link_editor = editable && self.link_editor_open;
        if show_link_editor {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.link_editor).finish(),
                LinkEditor::positioning(render_state.as_ref(app)),
            )
        }

        if focused {
            self.find_bar.render(&mut stack);
        }

        stack.finish()
    }

    fn active_cursor_position(&self, ctx: &ViewContext<Self>) -> Option<warpui::CursorInfo> {
        let model = self.model.as_ref(ctx);
        let render_state = model.render_state().as_ref(ctx);
        let font_size = model.cursor_font_size(ctx);
        ctx.element_position_by_id(render_state.saved_positions().cursor_id())
            .map(|position| CursorInfo {
                position,
                font_size,
            })
    }

    fn keymap_context(&self, ctx: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        if self.is_editable(ctx) {
            context.set.insert("EditorIsEditable");
        }

        if self.insertion_menu_state.open_at_source.is_some() {
            context.set.insert("BlockInsertionMenu");
        }

        if self.model.as_ref(ctx).has_command_selection(ctx) {
            context.set.insert("HasCommandSelection");
        }

        if self.can_execute_shell_commands {
            context.set.insert("CanExecuteShellCommands");
        }

        context
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // If we're editable but gaining focus, re-render to show the cursor/selection.
        if self.is_editable(ctx) {
            ctx.notify();
        }
        self.has_focus_within = true;
    }

    fn on_blur(&mut self, _blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        // If we're editable but losing focus, re-render to hide the cursor/selection.
        if self.is_editable(ctx) {
            ctx.notify();
        }
        self.has_focus_within = false;
    }
}

impl TypedActionView for RichTextEditorView {
    type Action = EditorViewAction;

    fn handle_action(&mut self, action: &EditorViewAction, ctx: &mut ViewContext<Self>) {
        use EditorViewAction::*;

        match action {
            UserTyped(content) => {
                if self.can_edit(ctx) {
                    self.user_typed(content, ctx)
                }
            }
            VimUserTyped(content) => {
                if self.can_edit(ctx) {
                    // Not supporting vim mode for notebooks yet.
                    // This should never be triggered because VimMode should be set to None for
                    // `RichTextEditorView`.
                    //
                    // If we do reach this, log a warning and handle text as if in non-vim mode.
                    log::warn!("vim mode triggered in a notebook, should not be enabled");
                    self.user_typed(content, ctx)
                }
            }
            Enter => self.enter(ctx),
            ShiftEnter => self.shift_enter(ctx),
            CmdEnter => ctx.emit(EditorViewEvent::CmdEnter),
            Backspace => self.backspace(ctx),
            MaybeOpenFileOrUrl {
                offset,
                link_in_text,
                cmd,
            } => self.maybe_open_file_or_url(
                *offset,
                *cmd,
                link_in_text.clone().map(UserInput::into_inner),
                ctx,
            ),
            DismissOpenLink => {
                self.open_link = None;
                self.open_file_path = None;
                ctx.notify();
            }
            CopyLink => {
                if let Some(tooltip) = self.open_link.take() {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(tooltip.url));
                }
                let window_id = ctx.window_id();
                crate::workspace::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::default(String::from("Link copied"));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
            EditLink => self.edit_link(true, ctx),
            OpenTooltipLink(target) => self.links.update(ctx, |links, ctx| {
                links.open(target.clone().into_inner(), ctx)
            }),
            SecondaryLinkAction(target) => self
                .links
                .update(ctx, |links, ctx| links.secondary_action(target, ctx)),
            CreateOrEditLink => self.edit_link(false, ctx),
            Scroll(delta) => self.scroll(*delta, ctx),
            SelectUp => self.select_up(ctx),
            SelectDown => self.select_down(ctx),
            SelectLeft => self.select_left(ctx),
            SelectRight => self.select_right(ctx),
            SelectToLineStart => {
                self.model.update(ctx, |model, ctx| {
                    model.select_to_line_start(ctx);
                });
                ctx.notify();
            }
            SelectToLineEnd => {
                self.model.update(ctx, |model, ctx| {
                    model.select_to_line_end(ctx);
                });
                ctx.notify();
            }
            SelectToParagraphStart => {
                self.model
                    .update(ctx, |model, ctx| model.select_to_paragraph_start(ctx));
                ctx.notify();
            }
            SelectToParagraphEnd => {
                self.model
                    .update(ctx, |model, ctx| model.select_to_paragraph_end(ctx));
                ctx.notify();
            }
            SelectForwardsByWord => {
                self.model.update(ctx, |model, ctx| {
                    model.forward_word(true, ctx);
                });
                ctx.notify();
            }
            SelectBackwardsByWord => {
                self.model.update(ctx, |model, ctx| {
                    model.backward_word(true, ctx);
                });
                ctx.notify();
            }
            SelectWord {
                offset,
                multiselect,
            } => self.select_word(*offset, *multiselect, ctx),
            SelectLine {
                offset,
                multiselect,
            } => self.select_line(*offset, *multiselect, ctx),
            SelectAll => {
                self.model.update(ctx, |model, ctx| {
                    model.select_all(ctx);
                });
                ctx.notify();
            }
            Delete => self.delete(TextDirection::Forwards, TextUnit::Character, false, ctx),
            DeleteLineLeft => {
                self.delete(TextDirection::Backwards, TextUnit::LineBoundary, false, ctx)
            }
            DeleteLineRight => {
                self.delete(TextDirection::Forwards, TextUnit::LineBoundary, false, ctx)
            }
            DeleteWordLeft => self.delete(TextDirection::Backwards, word_unit(ctx), false, ctx),
            DeleteWordRight => self.delete(TextDirection::Forwards, word_unit(ctx), false, ctx),
            CutLineLeft => self.delete(TextDirection::Backwards, TextUnit::LineBoundary, true, ctx),
            CutLineRight => self.delete(TextDirection::Forwards, TextUnit::LineBoundary, true, ctx),
            CutWordLeft => self.delete(TextDirection::Backwards, word_unit(ctx), true, ctx),
            CutWordRight => self.delete(TextDirection::Forwards, word_unit(ctx), true, ctx),
            MoveLeft => self.move_left(ctx),
            MoveRight => self.move_right(ctx),
            MoveToLineStart => {
                self.model.update(ctx, |model, ctx| {
                    model.move_to_line_start(ctx);
                });
                ctx.notify();
            }
            ToggleTaskList(offset) => {
                // This fixes CLD-1037. Users expect task lists to be interactable even when the notebook is not in-focus.
                if self.is_editable(ctx) {
                    self.model.update(ctx, |model, ctx| {
                        model.toggle_task_list(*offset, ctx);
                    });
                    ctx.notify();
                }
                self.ongoing_mouse_state = OngoingMouseEvent::None;
            }
            MoveToLineEnd => {
                self.model.update(ctx, |model, ctx| {
                    model.move_to_line_end(ctx);
                });
                ctx.notify();
            }
            MoveToParagraphStart => {
                self.model
                    .update(ctx, |model, ctx| model.move_to_paragraph_start(ctx));
                ctx.notify();
            }
            MoveToParagraphEnd => {
                self.model
                    .update(ctx, |model, ctx| model.move_to_paragraph_end(ctx));
                ctx.notify();
            }
            MoveForwardsByWord => {
                self.model.update(ctx, |model, ctx| {
                    model.forward_word(false, ctx);
                });
                ctx.notify();
            }
            MoveBackwardsByWord => {
                self.model.update(ctx, |model, ctx| {
                    model.backward_word(false, ctx);
                });
                ctx.notify();
            }
            MoveUp => self.move_up(ctx),
            MoveDown => self.move_down(ctx),
            Tab => self.tab(ctx),
            ShiftTab => self.shift_tab(ctx),
            Indent => self.indent(false, ctx),
            Unindent => self.indent(true, ctx),
            Bold => self.toggle_style(TextStyles::default().bold(), ctx),
            Italic => self.toggle_style(TextStyles::default().italic(), ctx),
            Underline => self.toggle_style(TextStyles::default().underline(), ctx),
            InlineCode => self.toggle_style(TextStyles::default().inline_code(), ctx),
            StrikeThrough => self.toggle_style(TextStyles::default().strikethrough(), ctx),
            SelectionStart {
                offset,
                multiselect,
            } => self.selection_start(*offset, *multiselect, ctx),
            SelectionUpdate(offset) => {
                if self.is_selecting() {
                    self.selection_update(*offset, ctx)
                } else {
                    self.selection_extend(*offset, ctx)
                }
            }
            SelectionEnd => self.selection_end(ctx),
            SelectBlock { block_start } => self.select_block(*block_start, ctx),
            BlockHovered {
                block_start,
                char_offset,
            } => self.block_hovered(*block_start, *char_offset, ctx),
            ShowCharacterPalette => ctx.open_character_palette(),
            ShowFindBar => self.find_bar.show(ctx),
            Focus => self.focus(ctx),
            DebugCopyBuffer => {
                let content = self.model.as_ref(ctx).debug_buffer(ctx);
                ctx.clipboard().write(ClipboardContent::plain_text(content));
            }
            DebugCopySelection => {
                let content = self.model.as_ref(ctx).debug_selection(ctx);
                ctx.clipboard().write(ClipboardContent::plain_text(content));
            }
            ToggleDebugMode => {
                self.debug_mode = !self.debug_mode;
                ctx.notify();
            }
            DebugLogState => {
                let model = self.model.as_ref(ctx);
                log::info!("BUFFER STATE:\n{}", model.debug_buffer(ctx));
                model.render_state.as_ref(ctx).log_state();
            }
            Paste => self.paste(ctx),
            InsertPlaceholder => {
                self.model.update(ctx, |model, ctx| {
                    model.insert_placeholder(ctx);
                });
                ctx.notify();
            }
            Copy => self.copy(ActionEntrypoint::Keyboard, ctx),
            Cut => self.cut(ActionEntrypoint::Keyboard, ctx),
            Undo => self.undo(ctx),
            Redo => self.redo(ctx),
            OpenBlockInsertionMenu => {
                self.open_block_insertion_menu(BlockInsertionSource::BlockInsertionButton, ctx)
            }
            InsertBlock(block_type) => self.insert_block(block_type.clone(), ctx),
            CommandUp => self.command_up(ctx),
            CommandDown => self.command_down(ctx),
            RunSelectedCommands => self.run_selected_commands(ctx),
            ExitCommandSelection => {
                self.close_overlays(ctx);
                if !self.model.as_ref(ctx).has_command_selection(ctx) {
                    // No command selection to exit (e.g. comment editors) —
                    // emit EscapePressed so parent views can handle dismissal.
                    ctx.emit(EditorViewEvent::EscapePressed);
                }
                self.model
                    .update(ctx, |model, ctx| model.exit_command_selection(ctx))
            }
            SelectCommandAtCursor => {
                self.close_overlays(ctx);
                self.model
                    .update(ctx, |model, ctx| model.select_command_at_cursor(ctx))
            }
            EditWorkflow(id) => ctx.emit(EditorViewEvent::EditWorkflow(*id)),
            RunWorkflow(workflow) => ctx.emit(EditorViewEvent::RunWorkflow(workflow.clone())),
            CodeBlockTypeSelectedAtOffset {
                start_anchor,
                code_block_type,
            } => self.model.update(ctx, |model, ctx| {
                model.update_code_block_type_at_offset(code_block_type, start_anchor.clone(), ctx)
            }),
            MermaidDisplayModeSelected { start_anchor, mode } => {
                let offset = self
                    .model
                    .as_ref(ctx)
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .resolve_anchor(start_anchor);
                if let Some(offset) = offset {
                    let mode = *mode;
                    self.model.update(ctx, |model, ctx| {
                        model.set_mermaid_render_mode(offset, mode, ctx);
                    });
                }
            }
            CopyTextToClipboard {
                text,
                block,
                entrypoint,
            } => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone().into_inner()));
                ctx.emit(EditorViewEvent::CopiedBlock {
                    block: *block,
                    entrypoint: *entrypoint,
                });
            }
            OpenEmbeddedObjectSearch => {
                self.open_embedded_object_search(ctx);
                ctx.notify();
            }
            RemoveEmbeddingAt(offset) => self
                .model
                .update(ctx, |model, ctx| model.remove_embedding_at(*offset, ctx)),
            MiddleClickPaste => self.middle_click_paste(ctx),
            OpenFile {
                path,
                line_and_column_num,
                force_open_in_warp,
            } => {
                ctx.emit(EditorViewEvent::OpenFile {
                    path: path.clone(),
                    line_and_column_num: *line_and_column_num,
                    force_open_in_warp: *force_open_in_warp,
                });
            }
        }

        if action.is_new_text_selection() {
            SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
                let clipboard_contents_fn = |ctx: &mut AppContext| {
                    self.model
                        .as_ref(ctx)
                        .read_selected_text_as_clipboard_content(ctx)
                };
                selection_settings
                    .maybe_write_to_linux_selection_clipboard(clipboard_contents_fn, ctx);
            });
        }
    }

    fn action_accessibility_contents(
        &mut self,
        action: &Self::Action,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        match action {
            EditorViewAction::UserTyped(text) => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    text.clone().into_inner(),
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::Paste | EditorViewAction::MiddleClickPaste => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    format!("Pasting: {}", ctx.clipboard().read().plain_text),
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::Enter
            | EditorViewAction::ShiftEnter
            | EditorViewAction::Cut
            | EditorViewAction::Copy
            | EditorViewAction::Undo
            | EditorViewAction::Redo
            | EditorViewAction::Indent
            | EditorViewAction::Unindent
            | EditorViewAction::Tab => ActionAccessibilityContent::from_debug(),
            EditorViewAction::ShiftTab => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Shift-tab", WarpA11yRole::UserAction),
            ),
            EditorViewAction::EditLink | EditorViewAction::CreateOrEditLink => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Edit Link",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::CopyLink => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Copy Link", WarpA11yRole::UserAction),
            ),
            EditorViewAction::OpenTooltipLink(link) => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    format!("Open link: {}", **link),
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::SecondaryLinkAction(link) => {
                let content = link.secondary_action().map_or_else(
                    || format!("Secondary click on {}", **link),
                    |action| action.accessibility_content.into_owned(),
                );
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    content,
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::DeleteLineLeft => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Delete line left",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::DeleteLineRight => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Delete line right",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::DeleteWordLeft => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Delete word left",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::DeleteWordRight => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Delete word right",
                    WarpA11yRole::UserAction,
                ))
            }

            EditorViewAction::CutLineLeft => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Cut line left", WarpA11yRole::UserAction),
            ),
            EditorViewAction::CutLineRight => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Cut line right", WarpA11yRole::UserAction),
            ),
            EditorViewAction::CutWordLeft => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Cut word left", WarpA11yRole::UserAction),
            ),
            EditorViewAction::CutWordRight => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Cut word right", WarpA11yRole::UserAction),
            ),

            EditorViewAction::ShowCharacterPalette => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Show character palette",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::ShowFindBar => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Show find bar", WarpA11yRole::UserAction),
            ),
            EditorViewAction::OpenBlockInsertionMenu => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Open block-insertion menu",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::OpenEmbeddedObjectSearch => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Open embedded object search menu",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::InsertBlock(block_type) => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    format!("Insert {} block", BlockType::from(block_type).label()),
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::Bold => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::bold()),
            EditorViewAction::Italic => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::Italic),
            EditorViewAction::Underline => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::Underline),
            EditorViewAction::InlineCode => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::InlineCode),
            EditorViewAction::StrikeThrough => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::StrikeThrough),
            EditorViewAction::ExitCommandSelection => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new(
                    "De-select command",
                    "Switch from selecting commands to selecting text",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::CodeBlockTypeSelectedAtOffset {
                code_block_type, ..
            } => ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                format!("Change code block language to {code_block_type}"),
                WarpA11yRole::UserAction,
            )),
            EditorViewAction::CopyTextToClipboard { .. } => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Copy code block", WarpA11yRole::UserAction),
            ),
            EditorViewAction::ToggleTaskList(_) => {
                // TODO(ben): Is it useful to include the text and/or on/off state here?
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    "Toggle task list",
                    WarpA11yRole::UserAction,
                ))
            }
            EditorViewAction::Delete
            | EditorViewAction::Backspace
            | EditorViewAction::Scroll(_)
            | EditorViewAction::SelectUp
            | EditorViewAction::SelectDown
            | EditorViewAction::SelectLeft
            | EditorViewAction::SelectRight
            | EditorViewAction::SelectBackwardsByWord
            | EditorViewAction::SelectForwardsByWord
            | EditorViewAction::SelectToLineStart
            | EditorViewAction::SelectToLineEnd
            | EditorViewAction::SelectToParagraphStart
            | EditorViewAction::SelectToParagraphEnd
            | EditorViewAction::SelectAll
            | EditorViewAction::SelectCommandAtCursor
            | EditorViewAction::MoveUp
            | EditorViewAction::MoveDown
            | EditorViewAction::MoveLeft
            | EditorViewAction::MoveRight
            | EditorViewAction::MoveForwardsByWord
            | EditorViewAction::MoveBackwardsByWord
            | EditorViewAction::MoveToLineEnd
            | EditorViewAction::MoveToLineStart
            | EditorViewAction::MoveToParagraphStart
            | EditorViewAction::MoveToParagraphEnd
            | EditorViewAction::CommandUp
            | EditorViewAction::CommandDown
            | EditorViewAction::SelectWord { .. }
            | EditorViewAction::SelectLine { .. }
            | EditorViewAction::SelectionStart { .. }
            | EditorViewAction::SelectionUpdate(_)
            | EditorViewAction::SelectionEnd
            | EditorViewAction::SelectBlock { .. }
            | EditorViewAction::BlockHovered { .. }
            | EditorViewAction::Focus
            | EditorViewAction::DebugCopyBuffer
            | EditorViewAction::DebugCopySelection
            | EditorViewAction::DebugLogState
            | EditorViewAction::ToggleDebugMode
            | EditorViewAction::InsertPlaceholder
            | EditorViewAction::DismissOpenLink
            | EditorViewAction::MaybeOpenFileOrUrl { .. }
            | EditorViewAction::RunSelectedCommands
            | EditorViewAction::CmdEnter
            | EditorViewAction::EditWorkflow(_)
            | EditorViewAction::RunWorkflow(_)
            | EditorViewAction::RemoveEmbeddingAt(_)
            | EditorViewAction::OpenFile { .. }
            | EditorViewAction::MermaidDisplayModeSelected { .. }
            | EditorViewAction::VimUserTyped(_) => ActionAccessibilityContent::Empty,
        }
    }
}

impl warp_editor::editor::EditorView for RichTextEditorView {
    type RichTextAction = EditorViewAction;

    fn runnable_command_at<'a>(
        &self,
        block_offset: CharOffset,
        ctx: &'a AppContext,
    ) -> Option<&'a dyn RunnableCommandModel> {
        self.model
            .as_ref(ctx)
            .notebook_command_for_block(block_offset)
            .map(|model| model.as_ref(ctx) as &'a dyn RunnableCommandModel)
    }

    fn embedded_item_at<'a>(
        &self,
        block_offset: CharOffset,
        ctx: &'a AppContext,
    ) -> Option<&'a dyn EmbeddedItemModel> {
        self.model
            .as_ref(ctx)
            .notebook_embed_for_block(block_offset)
            .map(|model| model.as_ref(ctx) as &'a dyn EmbeddedItemModel)
    }

    fn text_decorations<'a>(
        &'a self,
        _viewport_ranges: rangemap::RangeSet<CharOffset>,
        _buffer_version: Option<BufferVersion>,
        _ctx: &'a AppContext,
    ) -> TextDecoration<'a> {
        use rangemap::RangeMap;

        let mut override_color_map = RangeMap::new();
        let mut underline_map = RangeMap::new();

        if let Some(hovered_file_path) = &self.hovered_file_path {
            // Convert content model offsets to render model offsets (CLD-558)
            let render_range =
                (hovered_file_path.range.start - 1)..(hovered_file_path.range.end - 1);
            override_color_map.insert(render_range.clone(), *URL_COLOR);
            underline_map.insert(render_range, *URL_COLOR);
        }

        TextDecoration {
            base_color_map: None,
            override_color_map: if override_color_map.is_empty() {
                None
            } else {
                Some(override_color_map)
            },
            underline_range: if underline_map.is_empty() {
                None
            } else {
                Some(underline_map)
            },
        }
    }
}

impl RichTextAction<RichTextEditorView> for EditorViewAction {
    fn scroll(delta: Pixels, axis: Axis) -> Option<Self> {
        match axis {
            Axis::Vertical => Some(EditorViewAction::Scroll(delta)),
            Axis::Horizontal => None,
        }
    }

    fn user_typed(
        chars: String,
        _view: &WeakViewHandle<RichTextEditorView>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        // TODO(CORE-346): Ideally, we would only dispatch an action here if the editor or one of
        // its children (like the omnibar) is focused. However, to check if a child view is focused,
        // we need a AppContext. For now, we always dispatch the event and check focus in the
        // event handler.
        Some(EditorViewAction::UserTyped(UserInput::new(chars)))
    }

    fn vim_user_typed(
        chars: String,
        _view: &WeakViewHandle<RichTextEditorView>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        // Vim mode not enabled yet for notebooks; this should not be triggered.
        Some(EditorViewAction::VimUserTyped(UserInput::new(chars)))
    }

    fn left_mouse_down(
        location: Location,
        modifiers: ModifiersState,
        click_count: u32,
        is_first_mouse: bool,
        view: &WeakViewHandle<RichTextEditorView>,
        ctx: &AppContext,
    ) -> Option<Self> {
        log::debug!(
            "Clicked {click_count} times on {location:?}, cmd = {}, shift = {}",
            modifiers.cmd,
            modifiers.shift
        );
        let multiselect = modifiers.alt && FeatureFlag::RichTextMultiselect.is_enabled();

        // The first mouse down to bring focus to a Warp window will not have a corresponding mouse up.
        // We ignore it, and they can click again.
        if is_first_mouse {
            return None;
        }

        match location {
            Location::Text { char_offset, .. } => match click_count {
                // TODO(CLD-558): We need to align render model with the content model offset.
                1 if modifiers.shift => Some(EditorViewAction::SelectionUpdate(char_offset + 1)),
                1 => Some(EditorViewAction::SelectionStart {
                    offset: char_offset + 1,
                    multiselect,
                }),
                2 => Some(EditorViewAction::SelectWord {
                    offset: char_offset + 1,
                    multiselect,
                }),
                3 => Some(EditorViewAction::SelectLine {
                    offset: char_offset + 1,
                    multiselect,
                }),
                _ => None,
            },
            Location::Block {
                start_offset,
                end_offset,
                block_type:
                    HitTestBlockType::Code
                    | HitTestBlockType::MermaidDiagram
                    | HitTestBlockType::Embedding,
                ..
            } => match click_count {
                1 if modifiers.shift => view
                    .upgrade(ctx)
                    .and_then(|view| {
                        view.as_ref(ctx)
                            .selection_update_offset_for_rendered_mermaid_block(
                                start_offset..end_offset,
                                ctx,
                            )
                    })
                    .map(EditorViewAction::SelectionUpdate)
                    .or(Some(EditorViewAction::SelectBlock {
                        block_start: start_offset,
                    })),
                1 => Some(EditorViewAction::SelectBlock {
                    block_start: start_offset,
                }),
                2 => Some(EditorViewAction::ExitCommandSelection),
                _ => None,
            },
        }
    }

    fn left_mouse_dragged(
        location: Location,
        _cmd: bool,
        _shift: bool,
        view: &WeakViewHandle<RichTextEditorView>,
        ctx: &AppContext,
    ) -> Option<Self> {
        let view = view.upgrade(ctx)?;
        let is_selecting = matches!(
            view.as_ref(ctx).ongoing_mouse_state,
            OngoingMouseEvent::Selecting
        );
        match location {
            Location::Text { char_offset, .. } if is_selecting => {
                Some(EditorViewAction::SelectionUpdate(char_offset + 1))
            }
            Location::Block {
                start_offset,
                end_offset,
                block_type: HitTestBlockType::MermaidDiagram,
                ..
            } if is_selecting => view
                .as_ref(ctx)
                .selection_update_offset_for_rendered_mermaid_block(start_offset..end_offset, ctx)
                .map(EditorViewAction::SelectionUpdate),
            _ => None,
        }
    }

    fn left_mouse_up(
        location: Location,
        cmd: bool,
        _shift: bool,
        view: &WeakViewHandle<RichTextEditorView>,
        ctx: &AppContext,
    ) -> Vec<Self> {
        let mut actions_to_dispatch = Vec::new();
        let Some(view) = view.upgrade(ctx) else {
            return actions_to_dispatch;
        };

        if let Location::Text {
            char_offset,
            link,
            clamped,
            ..
        } = location.clone()
        {
            if !clamped {
                actions_to_dispatch.push(EditorViewAction::MaybeOpenFileOrUrl {
                    offset: char_offset + 1,
                    link_in_text: link.map(UserInput::new),
                    cmd,
                });
            }
        }

        match view.as_ref(ctx).ongoing_mouse_state {
            OngoingMouseEvent::Selecting => {
                actions_to_dispatch.push(EditorViewAction::SelectionEnd)
            }
            OngoingMouseEvent::None => (),
        }

        actions_to_dispatch
    }

    fn mouse_hovered(
        location: Option<Location>,
        _parent_view: &WeakViewHandle<RichTextEditorView>,
        _cmd: bool,
        _is_covered: bool,
        _ctx: &AppContext,
    ) -> Option<Self> {
        let char_offset = location.as_ref().and_then(|loc| {
            if let Location::Text {
                char_offset,
                clamped,
                ..
            } = loc
            {
                if !clamped {
                    Some(*char_offset)
                } else {
                    None
                }
            } else {
                None
            }
        });

        let block_start = location.map(|location| location.block_start());

        Some(EditorViewAction::BlockHovered {
            char_offset,
            block_start,
        })
    }

    fn task_list_clicked(
        block_start: CharOffset,
        _parent_view: &WeakViewHandle<RichTextEditorView>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        Some(EditorViewAction::ToggleTaskList(block_start))
    }

    fn middle_mouse_down(_ctx: &AppContext) -> Option<Self> {
        Some(EditorViewAction::MiddleClickPaste)
    }
}
