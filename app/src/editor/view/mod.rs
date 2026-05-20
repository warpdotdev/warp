mod element;
mod figma_utils;
mod model;
mod movement;
mod snapshot;
#[cfg(feature = "voice_input")]
mod voice;

/// The editor interfaces that we publicly expose to consumers.
/// This should be a very limited set; if you need to add something here,
/// you should carefully consider if it leaks the internal details of the editor.
pub use {
    element::{EditorDecoratorElements, EditorElement, TextColors},
    model::{
        Chars, CrdtOperation, DisplayPoint, EditOrigin, EditorSnapshot, InteractionState,
        LocalDrawableSelectionData, PeerSelectionData, RemoteDrawableSelectionData, ReplicaId,
        SelectAction, TextRun, TextStyleOperation,
    },
};

use self::model::{LocalSelections, Selection, UpdateBufferOption};
use super::soft_wrap::{ClampDirection, DisplayPointAndClampDirection};
use super::Point;
#[cfg(feature = "voice_input")]
use crate::view_components::FeaturePopup;
use base64::{engine::general_purpose, Engine as _};
use element::CommandXRayMouseStateHandle;
use figma_utils::is_figma_png;
use itertools::{Either, Itertools};
use mime_guess::from_path;
use model::{
    Anchor, AnchorBias, Bias, DisplayMap, DrawableSelection, LocalPendingSelection, LocalSelection,
    MarkedTextState, MovementResult, SelectionMode, SubwordBoundaries, ToBufferOffset,
    ToCharOffset, ToDisplayPoint, ToPoint,
};
use model::{EditorModel, EditorModelEvent, Edits};
use pathfinder_color::ColorU;
use settings::Setting as _;
use snapshot::{EditorHeightShrinkDelay, ViewSnapshot};
use vec1::{vec1, Vec1};
use warp_core::{safe_error, send_telemetry_from_ctx};
use warp_util::{path::ShellFamily, user_input::UserInput};
use warpui::platform::keyboard::KeyCode;
use warpui::ui_components::button::ButtonTooltipPosition;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{elements, ViewHandle};

use crate::ai::agent::ImageContext;
use crate::ai::blocklist::{BlocklistAIContextModel, PendingAttachment, PendingFile};
use crate::ai::predict::next_command_model::{NextCommandModel, NextCommandSuggestionState};
use crate::appearance::Appearance;
use crate::channel::{Channel, ChannelState};
use crate::editor::accept_autosuggestion_keybinding_view::AcceptAutosuggestionKeybinding;
use crate::editor::autosuggestion_ignore_view::{AutosuggestionIgnore, AutosuggestionIgnoreEvent};
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::view::{
    AIContextMenu, AIContextMenuCategory, AIContextMenuEvent,
};
use crate::server::telemetry::TelemetryEvent;
use crate::settings_view::flags;
use crate::suggestions::ignored_suggestions_model::{IgnoredSuggestionsModel, SuggestionType};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons;
use crate::view_components::DismissibleToast;
use crate::vim_registers::{RegisterContent, VimRegisters};
use crate::workspace::ToastStack;
use crate::{ai::blocklist::InputType, settings::AISettings};

use crate::editor::RangeExt;
use crate::features::FeatureFlag;
#[cfg(feature = "voice_input")]
use crate::settings::AISettingsChangedEvent;
use crate::settings::{AppEditorSettings, CursorBlink};
use crate::settings::{
    AppEditorSettingsChangedEvent, CursorDisplayType, InputSettings, SelectionSettings,
};
use crate::terminal::grid_size_util::grid_cell_dimensions;
use crate::terminal::model::block::BlockId;
use crate::themes::theme::Fill;
use crate::ui_components::avatar::{Avatar, AvatarContent};
use crate::util::bindings::{cmd_or_ctrl_shift, keybinding_name_to_keystroke, CustomAction};
use crate::util::clipboard::clipboard_content_with_escaped_paths;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};
use crate::util::image::{resize_image, MAX_IMAGE_COUNT_FOR_QUERY, MAX_IMAGE_SIZE_BYTES};
use crate::util::merge_ranges;
use crate::{workspace::Workspace, BlocklistAIHistoryModel};
use anyhow::Result;
use core::f32;
use std::path::Path;
use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
    InsertPosition, LineMotion, ModeTransition, MotionType, TextObjectInclusion, TextObjectType,
    VimHandler, VimMode, VimModel, VimMotion, VimOperand, VimOperator, VimState, VimSubscriber,
    VimTextObject, WordBound, WordMotion, WordType,
};
use vim::{
    vim_a_block, vim_a_paragraph, vim_a_quote, vim_a_word, vim_inner_block, vim_inner_paragraph,
    vim_inner_quote, vim_inner_word, vim_word_iterator_from_offset,
};
use warp_core::semantic_selection::SemanticSelection;

use num_traits::SaturatingSub;
use parking_lot::Mutex;
use pathfinder_geometry::vector::Vector2F;

use async_fs;
use std::collections::HashMap;
use std::{borrow::Cow, rc::Rc};
use std::{
    cmp::{self, Ordering},
    fmt,
    ops::Range,
    sync::Arc,
    time::Duration,
};
use string_offset::{ByteOffset, CharOffset};
use warp_completer::completer::Description;
use warp_editor::editor::NavigationKey;
use warpui::actions::StandardAction;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    ChildView, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable, MainAxisSize,
    ParentElement, Shrinkable, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::elements::{MouseStateHandle, Radius};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::keymap::{Keystroke, PerPlatformKeystroke};
use warpui::platform::{Cursor, FilePickerConfiguration, OperatingSystem};
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::text::word_boundaries::WordBoundariesPolicy;
use warpui::text::TextBuffer;
use warpui::text_layout::TextStyle;
use warpui::windowing::WindowManager;
use warpui::{
    accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole},
    fonts::Cache as FontCache,
    keymap::{EditableBinding, FixedBinding},
    AppContext, Element, Entity, ModelAsRef, ModelHandle, View, ViewContext, WindowId,
};
use warpui::{windowing, BlurContext, EntityId, FocusContext};
use warpui::{CursorInfo, ModelContext, SingletonEntity, TypedActionView};

const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_TAB_SIZE: usize = 4;

pub const ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME: &str = "editor_view:insert_autosuggestion";
pub const VOICE_LIMIT_HIT_TOAST_TEXT: &str = "You have hit the limit for Voice requests. Your limit will be refreshed as a part of your next cycle.";
pub const VOICE_ERROR_TOAST_TEXT: &str = "An error occurred while processing your voice input.";

pub const MAX_IMAGES_PER_CONVERSATION: usize = 200;

use warpui::clipboard_utils::CLIPBOARD_IMAGE_MIME_TYPES;

#[derive(Clone, Copy)]
pub enum AutosuggestionLocation {
    EndOfBuffer,
    Inline(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutosuggestionType {
    Command {
        was_intelligent_autosuggestion: bool,
    },
    AgentModeQuery {
        context_block_ids: Vec<BlockId>,
        was_intelligent_autosuggestion: bool,
    },
}

impl AutosuggestionType {
    pub fn matches_input_type(&self, input_type: InputType) -> bool {
        if input_type.is_ai() {
            matches!(self, AutosuggestionType::AgentModeQuery { .. })
        } else {
            matches!(self, AutosuggestionType::Command { .. })
        }
    }
}

impl fmt::Display for AutosuggestionLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AutosuggestionLocation::EndOfBuffer => write!(f, "EndOfBuffer"),
            AutosuggestionLocation::Inline(_) => write!(f, "Inline"),
        }
    }
}

pub const SELECT_UP_ACTION_NAME: &str = "editor_view:select_up";
pub const SELECT_DOWN_ACTION_NAME: &str = "editor_view:select_down";

pub fn init(ctx: &mut AppContext) {
    use warpui::keymap::macros::*;

    ctx.register_fixed_bindings(vec![
        // Below are default bindings that are similar to the behavior in all other text editors.
        // Those are not exposed to change, however, there may be editable bindings with different
        // bindings/triggers defined (so users can still adjust).
        FixedBinding::new(
            "escape",
            EditorAction::Escape,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "backspace",
            EditorAction::Backspace,
            id!("EditorView") & !id!("IMEOpen") & !id!("Vim"),
        ),
        FixedBinding::new(
            "backspace",
            EditorAction::VimBackspace,
            id!("EditorView") & !id!("IMEOpen") & id!("Vim"),
        ),
        FixedBinding::new(
            "enter",
            EditorAction::Enter,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "cmd-enter",
                linux_and_windows: "ctrl-shift-enter",
            },
            EditorAction::CmdEnter,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "numpadenter",
            EditorAction::Enter,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "delete",
            EditorAction::Delete,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-enter",
            EditorAction::ShiftEnter,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Copy,
            EditorAction::Copy,
            "Copy",
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Cut,
            EditorAction::Cut,
            "Cut",
            id!("EditorView") & !id!("IMEOpen"),
        ),
        // Bindings for paste require the StandardAction and CustomAction binding to work on all platforms.
        FixedBinding::custom(
            CustomAction::Paste,
            EditorAction::Paste,
            "Paste",
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::standard(
            StandardAction::Paste,
            EditorAction::Paste,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        #[cfg(windows)]
        FixedBinding::custom(
            CustomAction::WindowsPaste,
            EditorAction::Paste,
            "Paste",
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "ctrl-y",
            EditorAction::Yank,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-tab",
            EditorAction::ShiftTab,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "tab",
            EditorAction::Tab,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new("up", EditorAction::Up, id!("EditorView") & !id!("IMEOpen")),
        FixedBinding::new(
            "down",
            EditorAction::Down,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "left",
            EditorAction::Left,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "right",
            EditorAction::Right,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "home",
            EditorAction::Home,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "end",
            EditorAction::End,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-up",
            EditorAction::SelectUp,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-down",
            EditorAction::SelectDown,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-left",
            EditorAction::SelectLeft,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-right",
            EditorAction::SelectRight,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "shift-alt-left",
                linux_and_windows: "shift-ctrl-left",
            },
            EditorAction::SelectLeftByWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-meta-left",
            EditorAction::SelectLeftByWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new_per_platform(
            PerPlatformKeystroke {
                mac: "shift-alt-right",
                linux_and_windows: "shift-ctrl-right",
            },
            EditorAction::SelectRightByWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-meta-right",
            EditorAction::SelectRightByWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-home",
            EditorAction::SelectToLineStart,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "shift-end",
            EditorAction::SelectToLineEnd,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "pageup",
            EditorAction::PageUp,
            id!("EditorView")
                & !id!("IMEOpen")
                & !id!(flags::TERMINAL_INPUT_PAGE_KEYS_HANDLED_BY_INPUT),
        ),
        FixedBinding::new(
            "pagedown",
            EditorAction::PageDown,
            id!("EditorView")
                & !id!("IMEOpen")
                & !id!(flags::TERMINAL_INPUT_PAGE_KEYS_HANDLED_BY_INPUT),
        ),
        // Some editable bindings currently have more than 1 action.
        // Below's the list of those.
        // TODO better way of exposing multiple keybindings to the user
        FixedBinding::new(
            "shift-backspace",
            EditorAction::Backspace,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "ctrl-enter",
            EditorAction::CtrlEnter,
            id!("EditorView")
                & !id!("IMEOpen")
                & !id!(flags::CTRL_ENTER_ACCEPTS_PROMPT_SUGGESTION)
                & !(id!(flags::AGENT_VIEW_ENABLED) & id!(flags::CTRL_ENTER_ENTERS_AGENT_VIEW)),
        ),
        FixedBinding::new(
            "alt-enter",
            EditorAction::AltEnter,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Undo,
            EditorAction::Undo,
            "Undo",
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::custom(
            CustomAction::Redo,
            EditorAction::Redo,
            "Redo",
            id!("EditorView") & !id!("IMEOpen"),
        ),
        // This might seem like a no-op since `ctrl-right` changes desktops on Mac by default.
        // However, many Mac users coming from fish shell have asked for this binding.
        // They've already disabled the desktop change shortcut, and are expecting that this
        // binding also works in Warp. We should not break their workflow.
        FixedBinding::new(
            "ctrl-right",
            EditorAction::MoveForwardOneWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-f",
            EditorAction::MoveForwardOneWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-right",
            EditorAction::MoveForwardOneWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-b",
            EditorAction::MoveBackwardOneWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-left",
            EditorAction::MoveBackwardOneWord,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-backspace",
            EditorAction::CutWordLeft,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-d",
            EditorAction::CutWordRight,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-shift-{",
            EditorAction::MoveToParagraphStart,
            id!("EditorView") & !id!("IMEOpen"),
        ),
        FixedBinding::new(
            "meta-shift-}",
            EditorAction::MoveToParagraphEnd,
            id!("EditorView") & !id!("IMEOpen"),
        ),
    ]);

    // Register mac-only `FixedBinding`s.
    if OperatingSystem::get().is_mac() {
        ctx.register_fixed_bindings([
            // A native character palette isn't supported on all platforms. The `ctrl-cmd-space`
            // binding is unique to Mac.
            FixedBinding::new(
                "ctrl-cmd-space",
                EditorAction::ShowCharacterPalette,
                id!("EditorView") & !id!("IMEOpen"),
            ),
        ]);
    }

    // Register Linux-specific `FixedBinding`s.
    if OperatingSystem::get().is_linux() {
        // The Emacs banner banner binding is registered as a `FixedBinding` so
        // that it doesn't get displayed under Settings --> Keyboard Shortcuts.
        ctx.register_fixed_bindings([FixedBinding::new(
            "ctrl-e",
            EditorAction::EmacsBinding,
            id!("EditorView") & !id!("IMEOpen"),
        )]);
    }

    if ChannelState::channel() == Channel::Integration {
        ctx.register_fixed_bindings([
            // Hack: Add explicit bindings for the tests, since the tests' injected
            // keypresses won't trigger Mac menu items. Unfortunately we can't use
            // cfg[test] because we are a separate process!
            FixedBinding::new(
                "cmdorctrl-z",
                EditorAction::Undo,
                id!("EditorView") & !id!("IMEOpen"),
            ),
            FixedBinding::new(
                "shift-cmdorctrl-Z",
                EditorAction::Redo,
                id!("EditorView") & !id!("IMEOpen"),
            ),
            FixedBinding::new(
                "ctrl-shift-up",
                EditorAction::AddCursorAbove,
                id!("EditorView") & !id!("IMEOpen"),
            ),
            FixedBinding::new(
                cmd_or_ctrl_shift("a"),
                EditorAction::SelectAll,
                id!("EditorView") & !id!("IMEOpen"),
            ),
            FixedBinding::new(
                "ctrl-g",
                EditorAction::AddNextOccurrence,
                id!("EditorView") & !id!("IMEOpen"),
            ),
        ]);
    }

    ctx.register_editable_bindings([
        // Selections
        EditableBinding::new(
            "editor_view:select_left_by_word",
            "Select one word to the left",
            EditorAction::SelectLeftByWord,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("shift-meta-B"),
        EditableBinding::new(
            "editor_view:select_right_by_word",
            "Select one word to the right",
            EditorAction::SelectRightByWord,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("shift-meta-F"),
        EditableBinding::new(
            "editor_view:select_left",
            "Select one character to the left",
            EditorAction::SelectLeft,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Make this mac only so it is symmetric with the emacs keybinding for `SelectRight`,
        // which is Mac-only because it would otherwise conflict with the find bar.
        .with_mac_key_binding("shift-ctrl-B"),
        // Mac only to prevent conflicts with the opening the find bar.
        // NOTE "shift-right" exists a cross-platform keybinding for this action.
        EditableBinding::new(
            "editor_view:select_right",
            "Select one character to the right",
            EditorAction::SelectRight,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("shift-ctrl-F"),
        EditableBinding::new(SELECT_UP_ACTION_NAME, "Select up", EditorAction::SelectUp)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            // Set this to Mac only since otherwise it could conflict with opening the command
            // palette. NOTE `shift-up` still exists as a cross platform keybinding for this action.
            .with_mac_key_binding("shift-ctrl-P"),
        EditableBinding::new(
            SELECT_DOWN_ACTION_NAME,
            "Select down",
            EditorAction::SelectDown,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("shift-ctrl-N"),
        EditableBinding::new(
            "editor_view:select_all",
            "Select all",
            EditorAction::SelectAll,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_custom_action(CustomAction::SelectAll),
        EditableBinding::new(
            "editor:select_to_line_start",
            "Select to start of line",
            EditorAction::SelectToLineStart,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("shift-ctrl-A"),
        EditableBinding::new(
            "editor:select_to_line_end",
            "Select to end of line",
            EditorAction::SelectToLineEnd,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("shift-ctrl-E"),
        EditableBinding::new(
            "editor_view:clear_and_copy_lines",
            "Copy and clear selected lines",
            EditorAction::ClearAndCopyLines,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("ctrl-u"),
        EditableBinding::new(
            "editor_view:add_next_occurrence",
            "Add selection for next occurrence",
            EditorAction::AddNextOccurrence,
        )
        .with_custom_action(CustomAction::AddNextOccurrence)
        .with_context_predicate(
            id!("EditorView") & !id!("IMEOpen") & !id!(flags::CLI_AGENT_RICH_INPUT_OPEN),
        ),
        // `shift-end` is registered on all platforms for this action.
        EditableBinding::new(
            "editor_view:select_to_line_end",
            "Select To Line End",
            EditorAction::SelectToLineEnd,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("cmd-shift-right"),
        // `end` is registered on all platforms for this action.
        EditableBinding::new(
            "editor_view:select_to_line_start",
            "Select To Line Start",
            EditorAction::SelectToLineStart,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("cmd-shift-left"),
        // Navigation
        EditableBinding::new("editor_view:up", "Move cursor up", EditorAction::Up)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_key_binding("ctrl-p"),
        EditableBinding::new("editor_view:down", "Move cursor down", EditorAction::Down)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_key_binding("ctrl-n"),
        EditableBinding::new("editor_view:left", "Move cursor left", EditorAction::Left)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_key_binding("ctrl-b"),
        EditableBinding::new(
            "editor_view:right",
            "Move cursor right",
            EditorAction::Right,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("ctrl-f"),
        EditableBinding::new(
            "editor_view:move_to_line_start",
            "Move to start of line",
            EditorAction::MoveToLineStart,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Mac only so it doesn't conflict with "SelectAll" on Linux / Windows.
        // VSCode does not have a default binding for this on non-Mac.
        .with_mac_key_binding("ctrl-a"),
        EditableBinding::new(
            "editor_view:move_to_line_end",
            "Move to end of line",
            EditorAction::MoveToLineEnd,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Make this Mac-only so it is symmetric with ctrl-a for `MoveToLineStart`, which is Mac
        // only because it would otherwise conflict with `SelectAll`. VSCode does not have a default
        // binding for this on non-Mac.
        .with_mac_key_binding("ctrl-e"),
        // Match the behavior of both VSCode and Intellij by using `cmd-left/right` on Mac and
        // `home/end` on Windows and Linux. See https://www.jetbrains.com/help/idea/reference-keymap-win-default.html#caret_navigation.
        EditableBinding::new("editor_view:home", "Home", EditorAction::Home)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_mac_key_binding("cmd-left")
            .with_linux_or_windows_key_binding("home"),
        EditableBinding::new("editor_view:end", "End", EditorAction::End)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_mac_key_binding("cmd-right")
            .with_linux_or_windows_key_binding("end"),
        EditableBinding::new(
            "editor_view:cmd_down",
            "Move cursor to the bottom",
            EditorAction::CmdDown,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Match the behavior of VSCode, see https://code.visualstudio.com/docs/getstarted/keybindings#_basic-editing.
        .with_mac_key_binding("cmd-down")
        .with_linux_or_windows_key_binding("ctrl-end"),
        EditableBinding::new(
            "editor_view:cmd_up",
            "Move cursor to the top",
            EditorAction::CmdUp,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Match the behavior of VSCode, see https://code.visualstudio.com/docs/getstarted/keybindings#_basic-editing.
        .with_mac_key_binding("cmd-up")
        .with_linux_or_windows_key_binding("ctrl-home"),
        EditableBinding::new(
            "editor_view:move_to_and_select_buffer_start",
            "Select and move to the top",
            EditorAction::MoveToAndSelectBufferStart,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("cmd-shift-up")
        .with_linux_or_windows_key_binding("ctrl-shift-home"),
        EditableBinding::new(
            "editor_view:move_to_and_select_buffer_end",
            "Select and move to the bottom",
            EditorAction::MoveToAndSelectBufferEnd,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("cmd-shift-down")
        .with_linux_or_windows_key_binding("ctrl-shift-end"),
        EditableBinding::new(
            "editor_view:move_forward_one_word",
            "Move forward one word",
            EditorAction::MoveForwardOneWord,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-f"),
        EditableBinding::new(
            "editor_view:move_backward_one_word",
            "Move backward one word",
            EditorAction::MoveBackwardOneWord,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-b"),
        EditableBinding::new(
            "editor_view:move_to_paragraph_start",
            "Move to the start of the paragraph",
            EditorAction::MoveToParagraphStart,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-a"),
        EditableBinding::new(
            "editor_view:move_to_paragraph_end",
            "Move to the end of the paragraph",
            EditorAction::MoveToParagraphEnd,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-e"),
        EditableBinding::new(
            "editor_view:move_to_buffer_start",
            "Move to the start of the buffer",
            EditorAction::MoveToBufferStart,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-shift-<"),
        EditableBinding::new(
            "editor_view:move_to_buffer_end",
            "Move to the end of the buffer",
            EditorAction::MoveToBufferEnd,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-shift->"),
        // Buffer modifications
        EditableBinding::new(
            "editor_view:backspace",
            "Remove the previous character",
            EditorAction::Backspace,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("ctrl-h"),
        EditableBinding::new(
            "editor_view:cut_word_left",
            "Cut word left",
            EditorAction::CutWordLeft,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("ctrl-w"),
        EditableBinding::new(
            "editor:delete_word_left",
            "Delete word left",
            EditorAction::DeleteWordLeft,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("alt-backspace")
        .with_linux_or_windows_key_binding("ctrl-backspace"),
        EditableBinding::new(
            "editor_view:cut_word_right",
            "Cut word right",
            EditorAction::CutWordRight,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("alt-d"),
        EditableBinding::new("editor_view:delete", "Delete", EditorAction::Delete)
            .with_context_predicate(id!("EditorView") & !id!("EditorView_SingleCursorBufferEnd"))
            .with_key_binding("ctrl-d"),
        EditableBinding::new(
            "editor:delete_word_right",
            "Delete word right",
            EditorAction::DeleteWordRight,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("alt-delete")
        .with_linux_or_windows_key_binding("ctrl-delete"),
        EditableBinding::new(
            "editor_view:clear_lines",
            "Clear selected lines",
            EditorAction::ClearAndCopyLines,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen") & !id!("Vim"))
        // Mac only because otherwise this would conflict with the keybinding to clear all blocks.
        // NOTE ctrl-u exists as a default binding for this action that works across all platforms.
        .with_mac_key_binding("cmd-shift-K"),
        EditableBinding::new(
            "editor_view:cut_all_right",
            "Cut all right",
            EditorAction::CutAllRight,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("ctrl-k"),
        EditableBinding::new(
            "editor_view:delete_all_right",
            "Delete all right",
            EditorAction::DeleteAllRight,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // VSCode only binds a default binding on Mac, see https://github.com/microsoft/vscode/blob/ceda6cc4856841f1550a60327a3eaf3a1d0c306a/src/vs/editor/contrib/linesOperations/browser/linesOperations.ts#L657.
        .with_mac_key_binding("cmd-delete"),
        EditableBinding::new(
            "editor_view:delete_all_left",
            "Delete all left",
            EditorAction::DeleteAllLeft,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Intellij uses `ctrl-Y` to delete a line on Windows/Linux whereas VSCode uses
        // `ctrl-shift-k`. We use the former because `ctrl-shift-k` would interfere with the binding
        // to clear all blocks within the blocklist.
        .with_mac_key_binding("cmd-backspace")
        .with_linux_or_windows_key_binding("ctrl-y"),
        EditableBinding::new(
            "editor_view:insert_newline",
            "Insert newline",
            EditorAction::Newline,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("ctrl-j"),
        // Folds
        EditableBinding::new("editor_view:fold", "Fold", EditorAction::Fold)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_key_binding("alt-cmdorctrl-["),
        EditableBinding::new("editor_view:unfold", "Unfold", EditorAction::Unfold)
            .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
            .with_key_binding("alt-cmdorctrl-]"),
        EditableBinding::new(
            "editor_view:fold_selected_ranges",
            "Fold selected ranges",
            EditorAction::FoldSelectedRanges,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("alt-cmdorctrl-f"),
        EditableBinding::new(
            "editor:insert_last_word_previous_command",
            "Insert last word of previous command",
            EditorAction::InsertLastWordPrevCommand,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("meta-."),
        EditableBinding::new(
            "editor_view:move_backward_one_word",
            "Move Backward One Word",
            EditorAction::MoveBackwardOneWord,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("alt-left")
        .with_linux_or_windows_key_binding("ctrl-left"),
        EditableBinding::new(
            "editor_view:move_forward_one_word",
            "Move Forward One Word",
            EditorAction::MoveForwardOneWord,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_mac_key_binding("alt-right")
        .with_linux_or_windows_key_binding("ctrl-right"),
        EditableBinding::new(
            "editor_view:move_backward_one_subword",
            "Move Backward One Subword",
            EditorAction::MoveBackwardOneSubword,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Only assign a default keybinding for subword navigation on Mac, this is also what VSCode
        // does: https://github.com/microsoft/vscode/blob/e08f57208f087abed82852b24b5f4937357a95c1/src/vs/editor/contrib/wordPartOperations/browser/wordPartOperations.ts#L85.
        .with_mac_key_binding("ctrl-alt-left"),
        EditableBinding::new(
            "editor_view:move_forward_one_subword",
            "Move Forward One Subword",
            EditorAction::MoveForwardOneSubword,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Only assign a default keybinding for subword navigation on Mac, this is also what VSCode
        // does: https://github.com/microsoft/vscode/blob/e08f57208f087abed82852b24b5f4937357a95c1/src/vs/editor/contrib/wordPartOperations/browser/wordPartOperations.ts#L85.
        .with_mac_key_binding("ctrl-alt-right"),
        EditableBinding::new(
            "editor_view:select_left_by_subword",
            "Select one subword to the left",
            EditorAction::SelectLeftBySubword,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Only assign a default keybinding for subword navigation on Mac, this is also what VSCode
        // does: https://github.com/microsoft/vscode/blob/e08f57208f087abed82852b24b5f4937357a95c1/src/vs/editor/contrib/wordPartOperations/browser/wordPartOperations.ts#L85.
        .with_mac_key_binding("ctrl-alt-shift-left"),
        EditableBinding::new(
            "editor_view:select_right_by_subword",
            "Select one subword to the right",
            EditorAction::SelectRightBySubword,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        // Only assign a default keybinding for subword navigation on Mac, this is also what VSCode
        // does: https://github.com/microsoft/vscode/blob/e08f57208f087abed82852b24b5f4937357a95c1/src/vs/editor/contrib/wordPartOperations/browser/wordPartOperations.ts#L85.
        .with_mac_key_binding("ctrl-alt-shift-right"),
        EditableBinding::new(
            ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME,
            "Accept autosuggestion",
            EditorAction::InsertAutosuggestion,
        )
        .with_context_predicate(
            id!("EditorView")
                & !id!("IMEOpen")
                & id!(flags::AUTOSUGGESTIONS_ENABLED_FLAG)
                & id!("Has_Autosuggestion"),
        ),
    ]);

    ctx.register_editable_bindings([
        // With Agent Mode, cmdorctrl-i toggles AI input mode (same as GH Copilot) -- so
        // reassign command x ray to something else.
        EditableBinding::new(
            "editor_view:inspect_command",
            "Inspect Command",
            EditorAction::InspectCommand,
        )
        .with_enabled(|| FeatureFlag::AgentMode.is_enabled())
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen")),
        EditableBinding::new(
            "editor_view:inspect_command",
            "Inspect Command",
            EditorAction::InspectCommand,
        )
        .with_enabled(|| !FeatureFlag::AgentMode.is_enabled())
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_key_binding("cmdorctrl-i"),
    ]);

    ctx.register_editable_bindings([EditableBinding::new(
        "editor_view:clear_buffer",
        "Clear command editor",
        EditorAction::CtrlC,
    )
    .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
    .with_custom_action(CustomAction::ClearEditor)]);

    ctx.register_editable_bindings([
        EditableBinding::new(
            "editor_view:add_cursor_above",
            "Add cursor above",
            EditorAction::AddCursorAbove,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_custom_action(CustomAction::AddCursorAbove),
        EditableBinding::new(
            "editor_view:add_cursor_below",
            "Add cursor below",
            EditorAction::AddCursorBelow,
        )
        .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
        .with_custom_action(CustomAction::AddCursorBelow),
    ]);

    ctx.register_editable_bindings([EditableBinding::new(
        "editor_view:insert_nonexpanding_space",
        "Insert non-expanding space",
        EditorAction::InsertNonExpandingSpace,
    )
    .with_context_predicate(id!("EditorView") & !id!("IMEOpen"))
    .with_key_binding("alt-space")]);

    ctx.register_editable_bindings([EditableBinding::new(
        "editor_view:vim_exit_insert_mode",
        "Exit Vim insert mode",
        EditorAction::VimEscape,
    )
    .with_context_predicate(id!("EditorView") & !id!("IMEOpen") & id!("Vim"))
    .with_key_binding("ctrl-[")]);
    ctx.register_fixed_bindings([FixedBinding::new(
        "ctrl-r",
        EditorAction::Redo,
        id!("EditorView") & !id!("IMEOpen") & id!("VimNormalMode"),
    )]);
}

/// Actions that can be performed on an [`EditorView`].
#[derive(Debug)]
pub enum EditorAction {
    Scroll(Vector2F),
    Select(SelectAction),
    UserInsert(UserInput<String>),
    VimUserInsert(UserInput<String>),
    DragAndDropFiles(Vec<UserInput<String>>),
    SetMarkedText {
        marked_text: UserInput<String>,
        selected_range: Range<usize>,
    },
    ClearMarkedText,
    ImeCommit(UserInput<String>),
    Tab,
    ShiftTab,
    Copy,
    Cut,
    Paste,
    MiddleClickPaste,
    Yank,
    Newline,
    CtrlEnter,
    ShiftEnter,
    AltEnter,
    Enter,
    CutWordLeft,
    CutWordRight,
    DeleteWordLeft,
    DeleteWordRight,
    CutAllRight,
    CutAllLeft,
    DeleteAllRight,
    DeleteAllLeft,
    Delete,
    Backspace,
    VimBackspace,
    Escape,
    VimEscape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    CmdUp,
    CmdDown,
    ClearLines,
    ClearAndCopyLines,
    CtrlC,
    MoveToLineStart,
    MoveToParagraphStart,
    MoveToBufferStart,
    SelectToLineStart,
    MoveToLineEnd,
    MoveToParagraphEnd,
    MoveToBufferEnd,
    SelectToLineEnd,
    MoveToAndSelectBufferStart,
    MoveToAndSelectBufferEnd,
    MoveForwardOneWord,
    MoveBackwardOneWord,
    MoveForwardOneSubword,
    MoveBackwardOneSubword,
    SelectUp,
    SelectDown,
    SelectLeft,
    SelectRight,
    SelectWord(DisplayPoint),
    SelectLine(DisplayPoint),
    SelectAll,
    SelectRightByWord,
    SelectLeftByWord,
    SelectRightBySubword,
    SelectLeftBySubword,
    AddNextOccurrence,
    Fold,
    Unfold,
    FoldSelectedRanges,
    Undo,
    Redo,
    Focus,
    UnhandledModifierKey(Arc<String>),
    ClearParentSelections,
    AddCursorAbove,
    AddCursorBelow,
    CmdEnter,
    InspectCommand,
    /// Requests to try to show the command x-ray overlay when the mouse
    /// is at the given display point.  The x-ray will only actually be shown
    /// if the underlying command is parseable and there is a token under the
    /// displat point that we have a description for.
    TryToShowXRay(DisplayPoint),
    HideXRay,
    InsertLastWordPrevCommand,
    InsertNonExpandingSpace,
    ShowCharacterPalette,
    InsertAutosuggestion,
    EmacsBinding,
    #[cfg(feature = "voice_input")]
    ToggleVoiceInput(voice_input::VoiceInputToggledFrom),
    AttachFiles,
    SetAIContextMenuOpen(bool),
    ReadAndProcessImagesAsync {
        num_images_user_attached: usize,
        file_paths: Vec<String>,
    },
    /// Stores non-image file paths picked via the attach-file button into the pending files state.
    ProcessNonImageFiles {
        file_paths: Vec<String>,
    },
}

impl EditorAction {
    fn should_report_active_cursor_position_updated(&self) -> bool {
        !matches!(
            self,
            EditorAction::Scroll(_)
                | EditorAction::Copy
                | EditorAction::UnhandledModifierKey(_)
                | EditorAction::ShowCharacterPalette
                | EditorAction::TryToShowXRay(_)
                | EditorAction::HideXRay
                | EditorAction::Select(_)
        )
    }

    fn is_new_selection(&self) -> bool {
        matches!(
            self,
            EditorAction::Select(
                SelectAction::Update { .. } | SelectAction::Extend { .. } | SelectAction::End,
            ) | EditorAction::SelectToLineStart
                | EditorAction::SelectToLineEnd
                | EditorAction::SelectUp
                | EditorAction::SelectDown
                | EditorAction::SelectLeft
                | EditorAction::SelectRight
                | EditorAction::SelectWord(_)
                | EditorAction::SelectLine(_)
                | EditorAction::SelectAll
                | EditorAction::SelectRightByWord
                | EditorAction::SelectLeftByWord
                | EditorAction::SelectRightBySubword
                | EditorAction::SelectLeftBySubword
        )
    }
}

/// Configuration for `insert_internal`
/// Whether we want the inserted text to be placed in a selection.
pub enum SelectionInsertion {
    Yes,
    No,
}

#[derive(Debug)]
enum CutDirection {
    Right,
    Left,
}

/// This type is used specifically used to add a cursor above or below.
/// For that reason, when we move the cursor above or below the behavior is
/// different from normal cursor movement in that moving above in the top row
/// will return None instead of the first column and similarly moving below
/// in the bottom row will return None instead of the max point.
enum NewCursorDirection {
    Up,
    Down,
}

struct NewCursorUpOrDownResult {
    point_and_clamp_direction: DisplayPointAndClampDirection,
    goal_column: u32,
}

impl From<MovementResult> for NewCursorUpOrDownResult {
    fn from(result: MovementResult) -> Self {
        NewCursorUpOrDownResult {
            point_and_clamp_direction: result.point_and_clamp_direction,
            goal_column: result.goal_column,
        }
    }
}

impl NewCursorDirection {
    fn move_cursor(
        &self,
        map: &DisplayMap,
        point: DisplayPoint,
        goal_column: Option<u32>,
        clamp_direction: ClampDirection,
    ) -> Option<NewCursorUpOrDownResult> {
        match self {
            NewCursorDirection::Up => match map.up(point, goal_column, clamp_direction) {
                Ok(result) => {
                    if result.is_same_row {
                        None
                    } else {
                        Some(result.into())
                    }
                }
                Err(err) => {
                    log::error!("Error calling map#up {err:?}");
                    None
                }
            },
            NewCursorDirection::Down => match map.down(point, goal_column, clamp_direction) {
                Ok(result) => {
                    if result.is_same_row {
                        None
                    } else {
                        Some(result.into())
                    }
                }
                Err(err) => {
                    log::error!("Error calling map#down {err:?}");
                    None
                }
            },
        }
    }
}

/// Actions that can be performed on a plain text editor view.
#[derive(PartialEq, Eq, PartialOrd, Clone, Copy)]
pub enum PlainTextEditorViewAction {
    InsertChar,
    CursorChanged,
    Yank,
    CutWordLeft,
    ClearBuffer,
    DeleteWordLeft,
    DeleteWordRight,
    CutWordRight,
    Tab,
    Indent,
    Unindent,
    /// The user accepted a completion suggestion; either the full suggestion or a common prefix
    /// amongst all given suggestions.
    AcceptCompletionSuggestion,
    InsertSelectedText,
    SystemInsert,
    Space,
    NonExpandingSpace,
    NewLine,
    Backspace,
    Delete,
    AutoSuggestion,
    CutAll,
    DeleteAll,
    ClearAndCopyLines,
    ClearLines,
    ReplaceBuffer,
    Paste,
    ExpandAlias,
    CycleCompletionSuggestion,
    UpdateMarkedText,
}

impl PlainTextEditorViewAction {
    fn from_inserted_str(c: &str) -> Self {
        if c == " " {
            Self::Space
        } else {
            // TODO: we should probably rename this to "insert text" based on
            // how it's used.
            Self::InsertChar
        }
    }

    fn from_inserted_char(c: char) -> Self {
        if c == ' ' {
            Self::Space
        } else {
            // TODO: we should probably rename this to "insert text" based on
            // how it's used.
            Self::InsertChar
        }
    }
}

#[derive(Clone, Debug)]
pub enum ValidInputType {
    All,
    /// Attempts to parse input as a `u16`.
    PositiveInteger,
    NoSpaces,
}

/// Possible action that could be triggered by enter (including modified enter).
#[derive(Clone)]
pub enum EnterAction {
    /// Emit the event to the parent level.
    Emit,
    /// Insert a new line for this enter action.
    InsertNewLineIfMultiLine,
}

/// Settings for different enter keystrokes.
#[derive(Clone)]
pub struct EnterSettings {
    pub shift_enter: EnterAction,
    pub enter: EnterAction,
    pub alt_enter: EnterAction,
    pub ctrl_enter: EnterAction,
}

impl Default for EnterSettings {
    fn default() -> Self {
        Self {
            shift_enter: EnterAction::InsertNewLineIfMultiLine,
            enter: EnterAction::Emit,
            alt_enter: EnterAction::InsertNewLineIfMultiLine,
            ctrl_enter: EnterAction::InsertNewLineIfMultiLine,
        }
    }
}

#[derive(Clone, Copy)]
pub enum PropagateAndNoOpNavigationKeys {
    Always,
    /// Propagate the up and down arrow keys only when the cursor is at the boundary --
    /// so we will emit EditorEvent::Navigate(Up) when the cursor is at the first row and
    /// emit EditorEvent::Navigate(Down) when the cursor is at the last row.
    AtBoundary,
    Never,
}

#[derive(Clone, Copy, Default)]
pub enum PropagateHorizontalNavigationKeys {
    /// Always propagate horizontal navigation keys to parent
    Always,
    /// Only propagate when cursor is at buffer boundaries (start for left, end for right)
    AtBoundary,
    /// Never propagate - always handle within editor
    #[default]
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropagateAndNoOpEscapeKey {
    /// Let this view's parent handle `esc` first and manually trigger this view's
    /// handlers if necessary. This means the parent will handle an `esc` keypress
    /// before this view exits vim mode. (Example: closing completions in the Input.)
    PropagateFirst,
    /// This view gets precedence for handling `esc`, which means it will exit Vim mode
    /// before the parent gets to handle an `esc` keypress.
    /// (Example: exiting Vim mode before closing a modal.)
    HandleFirst,
}

/// Options for how the editor displays text.
///
/// Each is an `Option` - if `None`, the editor will use the theme default. If a text option is set,
/// the parent view is responsible for keeping it in sync with appearance changes (if applicable).
#[derive(Default, Clone)]
pub struct TextOptions {
    /// If font size is None, we use the settings monospace font size, otherwise we use the given one.
    pub font_size_override: Option<f32>,
    /// Font family to use when rendering the editor. If `None`, the monospace font is used.
    pub font_family_override: Option<FamilyId>,
    /// Default font properties for editor text.
    pub font_properties_override: Option<Properties>,
    /// Colors to use when rendering editor text. If `None`, the theme's text colors are used.
    pub text_colors_override: Option<TextColors>,
}

impl TextOptions {
    /// Create `TextOptions` that use the UI font size. All other overrides have their default values.
    pub fn ui_font_size(appearance: &Appearance) -> Self {
        Self {
            font_size_override: Some(appearance.ui_font_size()),
            font_properties_override: Some(Properties::default()),
            ..Default::default()
        }
    }

    /// Create `TextOptions` to use the UI font family at the given size. All other overrides
    /// have their default value.
    pub fn ui_text(size: Option<f32>, appearance: &Appearance) -> Self {
        Self {
            font_size_override: size,
            font_family_override: Some(appearance.ui_font_family()),
            font_properties_override: Some(Properties::default()),
            ..Default::default()
        }
    }
}

/// Colors to use when rendering cursors and selections in the editor.
#[derive(Clone, Copy)]
pub struct CursorColors {
    /// The color to use for the cursor.
    pub cursor: Fill,
    /// The color to use as the background of selections.
    pub selection: Fill,
}

/// Type alias for the function that returns the Editor Decorator Elements. The first `String` parameter is for the editor buffer contents.
type RenderDecoratorElementsFn = Box<dyn Fn(&AppContext) -> EditorDecoratorElements>;

/// Type alias for a closure that allows parent views to add flags to the EditorView's keymap context.
/// The closure takes the context by mutable reference and can insert additional flags.
pub type KeymapContextModifierFn = Box<dyn Fn(&mut warpui::keymap::Context, &AppContext)>;

/// Enum to choose between different methods of computing the baseline offset for text.
#[derive(Clone, Debug)]
pub enum BaselinePositionComputationMethod {
    /// Calculated "grid-style", where we use solely font metrics (ascent, descent, etc.) to
    /// compute the baseline offset.
    Grid,
    /// The default computation method, which is calculated "line-style", where we use the standard baseline offset
    /// computation for a Line (ultimately uses TOP_BOTTOM_RATIO).
    Default,
}

// Re-export voice transcription types for backwards compatibility
pub use crate::voice::transcriber::{Transcriber, VoiceTranscriber};

/// Similar to [`ImageContext`], but contains un-processed and un-resized image data.
#[derive(Clone)]
pub struct AttachedImage {
    pub data: Vec<u8>,
    pub mime_type: String,
    pub file_name: String,
}

impl fmt::Debug for AttachedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // We log dispatching typed actions (with `AttachedImage` as an argument) and we don't want
        // to log any UGC in prod.
        f.debug_struct("AttachedImage")
            .field("data", &"REDACTED_B64_IMAGE_DATA_UGC")
            .field("mime_type", &self.mime_type)
            .field("file_name", &"REDACTED_FILE_NAME_UGC")
            .finish()
    }
}

/// Interface for picking different options for the editor's behavior.
pub struct EditorOptions {
    pub text: TextOptions,
    pub cursor_colors_fn: CursorColorsFn,
    pub enter_settings: EnterSettings,
    pub propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys,
    pub propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys,
    pub propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey,
    pub autogrow: bool,
    pub single_line: bool,
    pub use_settings_line_height_ratio: bool,
    pub autocomplete_symbols: bool,
    pub soft_wrap: bool,
    pub placeholder_soft_wrap: bool,
    /// Whether or not this editor should acknowledge the AppEditorSettings::vim_mode option.
    pub supports_vim_mode: bool,
    /// Closures that should return a top section, left notch and right notch elements, if we want to paint them
    /// (above, top-left and top-right of EditorElement).
    pub render_decorator_elements: Option<RenderDecoratorElementsFn>,
    pub select_all_on_focus: bool,
    pub clear_selections_on_blur: bool,
    pub max_buffer_len: Option<usize>,
    pub valid_input_type: ValidInputType,
    /// Whether to use a Grid-style baseline position computation (using font metrics), or the standard
    /// baseline position computation for a Line (which uses TOP_BOTTOM_RATIO).
    pub baseline_position_computation_method: BaselinePositionComputationMethod,
    pub middle_click_paste: bool,
    /// If true, the user's [`CursorDisplayType`] will be respected.
    pub allow_user_cursor_preference: bool,
    pub convert_newline_to_space: bool,
    pub include_ai_context_menu: bool,
    /// If true, this editor will delegate handling of paste events to its parent instead of
    /// inserting clipboard contents directly.
    pub delegate_paste_handling: bool,
    /// Optional hook that transforms each dropped path before it's escaped and inserted into
    /// the buffer. Invoked for non-image paths only; image paths are forwarded via
    /// [`Event::DroppedImageFiles`] unchanged so the host can still read them from the
    /// filesystem.
    pub drag_drop_path_transformer: Option<PathTransformerFn>,
    /// If true, this is treated as a password field:
    /// * Text is rendered as dots instead of the actual characters
    /// * Copying is disabled (but paste still works)
    pub is_password: bool,
    /// Optional closure that allows parent views to add flags to the EditorView's keymap context.
    /// This is called during `keymap_context()` and can insert additional flags into the context.
    pub keymap_context_modifier: Option<KeymapContextModifierFn>,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            text: Default::default(),
            enter_settings: Default::default(),
            cursor_colors_fn: Box::new(default_cursor_colors),
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Never,
            propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Never,
            propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::HandleFirst,
            autogrow: false,
            single_line: false,
            use_settings_line_height_ratio: false,
            autocomplete_symbols: false,
            soft_wrap: false,
            placeholder_soft_wrap: false,
            supports_vim_mode: false,
            render_decorator_elements: None,
            select_all_on_focus: false,
            clear_selections_on_blur: false,
            max_buffer_len: None,
            valid_input_type: ValidInputType::All,
            baseline_position_computation_method: BaselinePositionComputationMethod::Default,
            middle_click_paste: true,
            allow_user_cursor_preference: false,
            convert_newline_to_space: false,
            include_ai_context_menu: false,
            delegate_paste_handling: false,
            drag_drop_path_transformer: None,
            is_password: false,
            keymap_context_modifier: None,
        }
    }
}

impl From<SingleLineEditorOptions> for EditorOptions {
    fn from(options: SingleLineEditorOptions) -> Self {
        Self {
            text: options.text,
            enter_settings: options.enter_settings,
            cursor_colors_fn: Box::new(default_cursor_colors),
            propagate_and_no_op_vertical_navigation_keys: options
                .propagate_and_no_op_vertical_navigation_keys,
            propagate_horizontal_navigation_keys: options.propagate_horizontal_navigation_keys,
            propagate_and_no_op_escape_key: options.propagate_and_no_op_escape_key,
            autogrow: false,
            single_line: true,
            use_settings_line_height_ratio: options.use_settings_line_height_ratio,
            autocomplete_symbols: options.autocomplete_symbols,
            soft_wrap: options.soft_wrap,
            placeholder_soft_wrap: options.placeholder_soft_wrap,
            supports_vim_mode: false,
            render_decorator_elements: None,
            select_all_on_focus: options.select_all_on_focus,
            clear_selections_on_blur: options.clear_selections_on_blur,
            max_buffer_len: options.max_buffer_len,
            valid_input_type: options.valid_input_type,
            baseline_position_computation_method: options.baseline_position_computation_method,
            middle_click_paste: options.middle_click_paste,
            allow_user_cursor_preference: options.allow_user_cursor_preference,
            convert_newline_to_space: options.convert_newline_to_space,
            include_ai_context_menu: false,
            delegate_paste_handling: false,
            drag_drop_path_transformer: None,
            is_password: options.is_password,
            keymap_context_modifier: None,
        }
    }
}

/// Interface for picking different options for the editor's behavior.
///
/// There are some fields that are not applicable to single-line editors which
/// are absent from this struct and have the behavior turned off in the `From`
/// implementation that convert a `SingleLineEditorOptions` to an `EditorOptions`.
#[derive(Clone)]
pub struct SingleLineEditorOptions {
    pub text: TextOptions,
    pub enter_settings: EnterSettings,
    pub propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys,
    pub propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys,
    pub propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey,
    pub use_settings_line_height_ratio: bool,
    pub autocomplete_symbols: bool,
    pub soft_wrap: bool,
    pub placeholder_soft_wrap: bool,
    pub select_all_on_focus: bool,
    pub clear_selections_on_blur: bool,
    pub max_buffer_len: Option<usize>,
    pub valid_input_type: ValidInputType,
    /// Whether to use a Grid-style baseline offset computation (using font metrics), or the standard
    /// baseline offset computation for a Line (which uses TOP_BOTTOM_RATIO).
    pub baseline_position_computation_method: BaselinePositionComputationMethod,
    pub middle_click_paste: bool,
    /// If true, the user's [`CursorDisplayType`] will be respected.
    pub allow_user_cursor_preference: bool,
    pub convert_newline_to_space: bool,
    pub is_password: bool,
}

impl Default for SingleLineEditorOptions {
    fn default() -> Self {
        Self {
            text: Default::default(),
            enter_settings: Default::default(),
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Never,
            propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Never,
            propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::HandleFirst,
            use_settings_line_height_ratio: false,
            autocomplete_symbols: false,
            soft_wrap: false,
            placeholder_soft_wrap: false,
            select_all_on_focus: false,
            clear_selections_on_blur: false,
            max_buffer_len: None,
            valid_input_type: ValidInputType::All,
            baseline_position_computation_method: BaselinePositionComputationMethod::Default,
            middle_click_paste: true,
            allow_user_cursor_preference: false,
            convert_newline_to_space: true,
            is_password: false,
        }
    }
}

pub type CursorColorsFn = Box<dyn Fn(&AppContext) -> CursorColors>;

pub type PathTransformerFn = Box<dyn Fn(&str) -> String>;

/// Returns theme-based cursor and selection colors.
pub fn default_cursor_colors(ctx: &AppContext) -> CursorColors {
    let theme = Appearance::as_ref(ctx).theme();
    CursorColors {
        cursor: theme
            .cursor()
            .on_background(theme.background(), MinimumAllowedContrast::Text),
        selection: theme.text_selection_color(),
    }
}

#[derive(Debug)]
pub enum VoiceTranscriptionOptions {
    /// Voice transcription is enabled, possibly showing a microphone button.
    Enabled { show_button: bool },

    /// Voice transcription is disabled.
    Disabled,
}

impl VoiceTranscriptionOptions {
    pub fn is_enabled(&self) -> bool {
        matches!(self, VoiceTranscriptionOptions::Enabled { .. })
    }

    pub fn should_show_button(&self) -> bool {
        matches!(
            self,
            VoiceTranscriptionOptions::Enabled { show_button: true }
        )
    }
}

#[derive(Debug)]
pub enum ImageContextOptions {
    /// Attaching image context is enabled, possibly showing an image button if LLM supports vision.
    Enabled {
        unsupported_model: bool,
        is_processing_attached_images: bool,
        num_images_attached: usize,
        num_images_in_conversation: usize,
    },

    /// Attaching image context is disabled.
    Disabled,
}

impl ImageContextOptions {
    pub fn is_enabled(&self) -> bool {
        match self {
            ImageContextOptions::Enabled {
                unsupported_model,
                is_processing_attached_images,
                num_images_attached,
                num_images_in_conversation,
            } => {
                if *unsupported_model {
                    return false;
                }

                if *is_processing_attached_images {
                    return false;
                }

                if *num_images_attached >= MAX_IMAGE_COUNT_FOR_QUERY {
                    return false;
                }

                let total_images = *num_images_attached + *num_images_in_conversation;
                if total_images >= MAX_IMAGES_PER_CONVERSATION {
                    return false;
                }

                true
            }
            ImageContextOptions::Disabled => false,
        }
    }

    pub fn should_show_button(&self) -> bool {
        matches!(self, ImageContextOptions::Enabled { .. })
    }

    pub fn tooltip_text(&self) -> String {
        if let ImageContextOptions::Enabled {
            unsupported_model,
            is_processing_attached_images,
            num_images_attached,
            num_images_in_conversation,
        } = self
        {
            if *unsupported_model {
                return "Image attachment isn't supported by this model".into();
            }

            if *is_processing_attached_images {
                return "Loading...".into();
            }

            if *num_images_attached >= MAX_IMAGE_COUNT_FOR_QUERY {
                return format!(
                    "Image attachment is disabled — limit is {MAX_IMAGE_COUNT_FOR_QUERY} per query"
                );
            }

            let total_images = *num_images_attached + *num_images_in_conversation;
            if total_images >= MAX_IMAGES_PER_CONVERSATION {
                return format!(
                    "Image attachment is disabled — limit is {MAX_IMAGES_PER_CONVERSATION} per conversation"
                );
            }
        }

        "Attach images".into()
    }

    pub fn num_images_attached(&self) -> usize {
        match self {
            ImageContextOptions::Enabled {
                num_images_attached,
                ..
            } => *num_images_attached,
            _ => 0,
        }
    }

    pub fn num_images_in_conversation(&self) -> usize {
        match self {
            ImageContextOptions::Enabled {
                num_images_in_conversation,
                ..
            } => *num_images_in_conversation,
            _ => 0,
        }
    }

    pub fn is_unsupported_model(&self) -> bool {
        matches!(
            self,
            ImageContextOptions::Enabled {
                unsupported_model: true,
                ..
            }
        )
    }
}

pub struct AIContextMenuState {
    ai_context_menu: ViewHandle<AIContextMenu>,

    /// The mouse handle for the at context menu icon.
    at_context_menu_button_mouse_handle: MouseStateHandle,
}

pub struct EditorView {
    view_id: EntityId,
    editor_model: ModelHandle<EditorModel>,
    scroll_position: Arc<Mutex<Vector2F>>,
    autoscroll_requested: Arc<Mutex<bool>>,
    windowing_state_handle: ModelHandle<WindowManager>,

    text_options: TextOptions,
    use_settings_line_height_ratio: bool,
    focused: bool,
    get_cursor_colors_fn: CursorColorsFn,
    cursors_visible: bool,
    blink_epoch: usize,
    single_line: bool,
    /// A cache for text that is cut within the input using ctrl-k/u. This text is not copied to clipboard.
    internal_clipboard: String,
    /// Sets whether to propagate the vertical navigation keys (up/down) action to parent view.
    propagate_vertical_navigation_keys: PropagateAndNoOpNavigationKeys,
    /// Sets whether to propagate the horizontal navigation keys (left/right) action to parent view.
    propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys,
    /// Sets whether to propagate the escape key action to the parent view.
    propagate_escape_key: PropagateAndNoOpEscapeKey,
    autogrow: bool,
    /// If true, defers to the user's settings for whether to autocomplete
    /// typed symbols.
    autocomplete_symbols_allowed: bool,
    /// A cached copy of autocomplete_symbols from Settings (to avoid performing
    /// a settings read on every typed character).
    autocomplete_symbols_setting: bool,
    /// A cached copy of cursor_display_type from Settings (to avoid performing
    /// a settings read on every typed character).
    /// If `None`, we do not wish to respect the user's cursor display preference.
    cursor_display_override: Option<CursorDisplayType>,
    window_id: WindowId,
    autosuggestion_state: Option<Arc<AutosuggestionState>>,
    next_command_model: Option<ModelHandle<NextCommandModel>>,

    /// The height of the editor at the last render.
    /// This is needed because autosuggestions soft wrap and can increase the height of the editor.
    /// When the user types, the autosuggestion is cleared and recomputed. We don't want the editor height
    /// to jitter during the small amount of time there is no autosuggestion while it's being computed.
    editor_height_shrink_delay: Arc<Mutex<EditorHeightShrinkDelay>>,

    /// Map from prefix to placeholder text.
    /// Empty string prefix "" is the default placeholder (shown when buffer is empty).
    placeholder_texts: Arc<HashMap<String, String>>,
    hover_handle: MouseStateHandle,
    command_x_ray_mouse_handle: CommandXRayMouseStateHandle,
    command_x_ray_state: Option<Arc<Description>>,
    enter_settings: EnterSettings,
    soft_wrap: bool,
    placeholder_soft_wrap: bool,
    /// What the starting text of the view was. Helpful for determining if the editor is
    /// "dirty", i.e., a user has changed the contents from what is persisted elsewhere.
    base_buffer_text: String,

    /// For now, specific editors need to be opted-in to allow vim keybindings. Both the
    /// AppEditorSettings::vim_mode user option and this field need to be true for the vim
    /// keybindings to be active on this editor. There are some editors in which vim keybindings
    /// won't be relevant, e.g. the font size editor.
    supports_vim_mode: bool,
    /// The model that holds the Vim state machine.
    vim_model: ModelHandle<VimModel>,

    /// Optionally, we render left/right decorator elements, using the given closure.
    render_decorator_elements: Option<RenderDecoratorElementsFn>,

    /// Indicates whether or not we want to select the entire editor contents when we focus the editor view.
    select_all_on_focus: bool,
    /// Indicates whether or not we want to clear selections in the editor on blur.
    clear_selections_on_blur: bool,

    /// A map from position ID to buffer point.
    /// Each entry in this map corresponds to a buffer position that will be cached
    /// via the position cache. The cached position will be the top-left of the bounding
    /// box for the corresponding point.
    ///
    /// See [`Self::cache_buffer_point`] for more details.
    // TODO: consider wrapping this in a reference-counted variable
    // if we end up using this API more. Currently, we clone this map
    // for every render, which is inefficient.
    cached_buffer_points: HashMap<Cow<'static, str>, Point>,

    /// Whether to use a Grid-style baseline position computation (using font metrics), or the standard
    /// baseline position computation for a Line (which uses TOP_BOTTOM_RATIO).
    pub baseline_position_computation_method: BaselinePositionComputationMethod,

    /// Whether or not this editor supports middle-click-paste.
    /// This can be configured via [`EditorOptions::middle_click_paste`].
    ///
    /// The main use-case for configuring this is to set it to `false`
    /// to avoid a double-paste bug if the [`EditorView`] has a parent
    /// that already handles middle-click-paste.
    middle_click_paste: bool,

    /// If true, newlines will be converts to space on paste.
    /// This is useful for maintaining the single-line constraint
    convert_newline_to_space: bool,

    /// Shell-specific behavior for this editor, e.g. how to escape special characters in paths.
    shell_family: Option<ShellFamily>,

    accept_autosuggestion_keybinding_view: ViewHandle<AcceptAutosuggestionKeybinding>,
    autosuggestion_ignore_view: ViewHandle<AutosuggestionIgnore>,
    show_autosuggestion_keybinding_hint: bool,
    show_autosuggestion_ignore_button: bool,

    /// The state of voice input for this editor.
    /// Must only be mutated through [`Self::set_voice_input_state`], which keeps
    /// the editor's [`InteractionState`] in sync (locking input during voice).
    #[cfg(feature = "voice_input")]
    voice_input_state: voice::VoiceInputState,

    /// The interaction state before voice input was activated, to restore when voice input ends.
    #[cfg(feature = "voice_input")]
    interaction_state_before_voice: Option<InteractionState>,

    /// Options for voice transcription.
    #[cfg(feature = "voice_input")]
    voice_transcription_options: VoiceTranscriptionOptions,

    /// The mouse handle for the voice transcription icon.
    #[cfg(feature = "voice_input")]
    voice_transcription_button_mouse_handle: MouseStateHandle,

    /// The new feature popup for voice transcription.
    #[cfg(feature = "voice_input")]
    voice_new_feature_popup: ViewHandle<FeaturePopup>,

    context_model: Option<ModelHandle<BlocklistAIContextModel>>,

    /// Options for attaching image context.
    /// Made public to allow terminal input to access image attachment state and limits.
    pub image_context_options: ImageContextOptions,

    /// The mouse handle for the image context icon.
    image_context_button_mouse_handle: MouseStateHandle,

    /// Because the AIContextMenu also contains a text editor,
    /// we need to avoid infinite recursion and selectively
    /// allow the creation of AIContextMenuState.
    pub ai_context_menu_state: Option<AIContextMenuState>,

    /// Whether this editor is in AI input mode.
    is_ai_input: bool,

    /// Whether this editor should delegate handling of paste events to its parent.
    delegate_paste_handling: bool,

    /// Optional hook that transforms each dropped path before it is escaped and inserted into
    /// the buffer. See [`EditorOptions::drag_drop_path_transformer`].
    drag_drop_path_transformer: Option<PathTransformerFn>,

    process_attached_images_future_handle: Option<SpawnedFutureHandle>,

    is_password: bool,

    /// Optional closure that allows parent views to add flags to this editor's keymap context.
    keymap_context_modifier: Option<KeymapContextModifierFn>,
}

pub(super) struct ScrollState {
    pub scroll_position: Arc<Mutex<Vector2F>>,
    pub autoscroll_requested: Arc<Mutex<bool>>,
}

impl ScrollState {
    pub(super) fn scroll_position(&self) -> Vector2F {
        *self.scroll_position.lock()
    }
}

impl From<&EditorView> for ScrollState {
    fn from(view: &EditorView) -> Self {
        ScrollState {
            scroll_position: view.scroll_position.clone(),
            autoscroll_requested: view.autoscroll_requested.clone(),
        }
    }
}

pub struct AutosuggestionState {
    /// Snapshot of the buffer at the time the autosuggestions was set.
    pub buffer_snapshot: String,

    /// The text that was set as the autosuggestion.
    pub original_autosuggestion_text: String,

    /// The current autosuggestion within the editor. This differs from the original autosuggestion
    /// text if the user starts typing a character that matches the autosuggestion.
    pub current_autosuggestion_text: Option<String>,

    /// Optionally, the autosuggestion might exist somewhere besides the very end of the Editor. This
    /// is the case for Notebooks, which support inlined autosuggestions. The location refers to a logical row
    /// number in the editor where the autosuggestion should be rendered. By default, this should be
    /// the end of the buffer.
    pub location: AutosuggestionLocation,

    /// Type of autosuggestion - whether it's a command or AI prompt.
    /// Note we cannot use `type` since that's a reserved Rust keyword.
    pub autosuggestion_type: AutosuggestionType,
}

impl AutosuggestionState {
    pub fn is_active(&self) -> bool {
        self.current_autosuggestion_text
            .as_ref()
            .is_some_and(|text| !text.is_empty())
    }
}

impl VimHandler for EditorView {
    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>) {
        self.user_insert(&c.to_string(), ctx);
    }

    fn keyword_prg(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::TryToShowXRay(CommandXRayAnchor::Cursor));
    }

    // Note that this event is intended for use in [`VimMode::Normal`]. Insertions should leave the
    // cursor _on_ the last char of the insertion, but [`EditorView::user_insert`] leaves the cursor
    // _after_ the last char. So, we have to call [`EditorModel::move_cursors_by_offset`] at
    // the end of this method.
    fn insert_text(
        &mut self,
        text: &str,
        position: &InsertPosition,
        count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        use InsertPosition as I;
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::InsertChar,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| {
                        let text_to_repeat = match position {
                            I::LineAbove => text.to_owned() + "\n",
                            I::LineBelow => String::from("\n") + text,
                            _ => text.to_owned(),
                        };
                        editor_model.insert(
                            text_to_repeat.repeat(count as usize).as_str(),
                            None,
                            ctx,
                        );
                    },
                )
                .with_change_selections(|editor_model, ctx| {
                    match position {
                        I::AtCursor => {}
                        I::AfterCursor => editor_model.move_cursors_by_offset(
                            1,
                            &Direction::Forward,
                            /* keep_selection */ false,
                            /* stop_at_line_boundary */ false,
                            ctx,
                        ),
                        I::LineFirstNonWhitespace => {
                            editor_model.cursor_line_start_non_whitespace(false, ctx)
                        }
                        I::LineAbove => {
                            editor_model.cursor_line_start(/* keep_selection */ false, ctx)
                        }
                        I::LineEnd | I::LineBelow => {
                            editor_model.cursor_line_end(/* keep_selection */ false, ctx)
                        }
                    }
                })
                .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                    editor_model.move_cursors_by_offset(
                        1,
                        &Direction::Backward,
                        /* keep_selection */ false,
                        /* stop_at_line_boundary */ false,
                        ctx,
                    );
                }),
        );
    }

    fn navigate_char(
        &mut self,
        character_count: u32,
        motion: &CharacterMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        // Analogous to how right-arrow accepts an autosuggestion, do this for `l`.
        if *motion == CharacterMotion::Right && self.single_cursor_at_autosuggestion_beginning(ctx)
        {
            self.insert_full_autosuggestion(ctx);
            return;
        }

        // The up-arrow history menu is unable to handle counts >1. Only emit "Up" and "Down"
        // events the parent view if the count is 1.
        // TODO: Update [`NavigationKey::Up`] and [`NavigationKey::Down`] to accept count.
        if *motion == CharacterMotion::Up
            && character_count == 1
            && self.vim_should_propagate_upward_navigation(ctx)
        {
            ctx.emit(Event::Navigate(NavigationKey::Up));
            return;
        }

        if *motion == CharacterMotion::Down
            && character_count == 1
            && self.vim_should_propagate_downward_navigation(ctx)
        {
            ctx.emit(Event::Navigate(NavigationKey::Down));
            return;
        }

        self.change_selections(ctx, |editor_model, ctx| {
            match motion {
                CharacterMotion::Left => {
                    editor_model.move_cursors_by_offset(
                        character_count,
                        &Direction::Backward,
                        /* keep_selection */ false,
                        /* stop_at_line_boundary */ true,
                        ctx,
                    )
                }
                CharacterMotion::Right => {
                    editor_model.move_cursors_by_offset(
                        character_count,
                        &Direction::Forward,
                        /* keep_selection */ false,
                        /* stop_at_line_boundary */ true,
                        ctx,
                    )
                }
                CharacterMotion::WrappingLeft => {
                    editor_model.move_cursor_ignoring_newlines(
                        character_count,
                        &Direction::Backward,
                        /* keep_selection */ false,
                        ctx,
                    )
                }
                CharacterMotion::WrappingRight => {
                    editor_model.move_cursor_ignoring_newlines(
                        character_count,
                        &Direction::Forward,
                        /* keep_selection */ false,
                        ctx,
                    )
                }
                CharacterMotion::Up => editor_model.move_up_by_offset(character_count, ctx),
                CharacterMotion::Down => editor_model.move_down_by_offset(character_count, ctx),
            }
        });
    }

    fn navigate_word(&mut self, word_count: u32, motion: &WordMotion, ctx: &mut ViewContext<Self>) {
        let WordMotion {
            direction,
            bound,
            word_type,
        } = motion;
        match direction {
            Direction::Forward => self.vim_cursor_forward_word(*bound, *word_type, word_count, ctx),
            Direction::Backward => {
                self.vim_cursor_backward_word(*bound, *word_type, word_count, ctx)
            }
        }
    }

    fn navigate_line(&mut self, line_count: u32, motion: &LineMotion, ctx: &mut ViewContext<Self>) {
        match motion {
            LineMotion::Start => self.move_to_line_start(ctx),
            LineMotion::FirstNonWhitespace => {
                self.change_selections(ctx, |editor_model, ctx| {
                    editor_model.cursor_line_start_non_whitespace(false, ctx);
                });
            }
            LineMotion::End => {
                if self.single_cursor_at_autosuggestion_beginning(ctx) {
                    self.insert_full_autosuggestion(ctx);
                } else {
                    // Only moving to the end of the line ($) uses number-repeat.
                    self.change_selections(ctx, |editor_model, ctx| {
                        editor_model.move_down_by_offset(line_count.saturating_sub(1), ctx);
                        editor_model.cursor_line_end(false, ctx);
                    });
                }
            }
        }
    }

    fn first_nonwhitespace_motion(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            match motion {
                FirstNonWhitespaceMotion::Up => editor_model.move_up_by_offset(count, ctx),
                FirstNonWhitespaceMotion::Down => editor_model.move_down_by_offset(count, ctx),
                FirstNonWhitespaceMotion::DownMinusOne => {
                    editor_model.move_down_by_offset(count - 1, ctx)
                }
            };
            editor_model.cursor_line_start_non_whitespace(false /* keep_selection */, ctx);
        });
    }

    fn find_char(
        &mut self,
        occurrence_count: u32,
        motion: &FindCharMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.vim_find_char(
                false, /* keep_selection */
                occurrence_count,
                motion,
                ctx,
            );
        });
    }

    fn navigate_paragraph(
        &mut self,
        count: u32,
        direction: &Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.vim_move_by_paragraph(count, direction, false, ctx);
        });
    }

    fn replace_char(&mut self, c: char, char_count: u32, ctx: &mut ViewContext<Self>) {
        if char_count <= self.distance_to_line_end(ctx) {
            self.replace_characters(c, char_count, ctx);
        }
    }

    fn operation(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        register_name: char,
        replacement_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        // Selection logic is almost the same for all operators, so capture that in a closure first.
        let selection_change =
            |editor_model: &mut EditorModel, ctx: &mut ModelContext<EditorModel>| {
                match operand {
                    VimOperand::Motion {
                        motion,
                        motion_type,
                    } => {
                        match motion {
                            VimMotion::Character(motion) => {
                                editor_model.vim_select_for_char_motion(
                                    motion,
                                    motion_type,
                                    operator,
                                    operand_count,
                                    ctx,
                                );
                            }
                            VimMotion::Word(motion) => {
                                editor_model.vim_select_words(motion, operand_count, ctx);
                            }
                            VimMotion::Line(motion) => {
                                editor_model.vim_select_for_line_motion(motion, operand_count, ctx);
                            }
                            VimMotion::FirstNonWhitespace(motion) => {
                                editor_model.vim_select_for_first_nonwhitespace_motion(
                                    motion,
                                    motion_type,
                                    operator,
                                    operand_count,
                                    ctx,
                                );
                            }
                            VimMotion::FindChar(motion) => {
                                editor_model.vim_find_char(
                                    /* keep_selection */ true,
                                    operand_count,
                                    motion,
                                    ctx,
                                );
                            }
                            VimMotion::JumpToMatchingBracket => {
                                editor_model.vim_select_for_matching_bracket(ctx);
                            }
                            VimMotion::JumpToUnmatchedBracket(bracket) => {
                                editor_model.vim_move_cursor_to_unmatched_bracket(
                                    bracket, /* keep_selection */ true, ctx,
                                );
                            }
                            VimMotion::Paragraph(direction) => {
                                editor_model.vim_move_by_paragraph(
                                    operand_count,
                                    direction,
                                    /* keep_selection */ true,
                                    ctx,
                                );

                                if *motion_type == MotionType::Linewise {
                                    let include_newline = *operator != VimOperator::Change;
                                    editor_model.extend_selection_linewise(include_newline, ctx);
                                }
                            }
                            VimMotion::JumpToLastLine => {
                                editor_model
                                    .move_to_buffer_end(/* keep_selection */ true, ctx);
                                if *motion_type == MotionType::Linewise {
                                    let include_newline = *operator != VimOperator::Change;
                                    editor_model.extend_selection_linewise(include_newline, ctx);
                                }
                            }
                            VimMotion::JumpToFirstLine => {
                                let mut new_selections = editor_model.selections(ctx).clone();
                                for selection in new_selections.iter_mut() {
                                    selection.set_start(Anchor::Start);
                                }
                                editor_model.change_selections(new_selections, ctx);

                                if *motion_type == MotionType::Linewise {
                                    let include_newline = *operator != VimOperator::Change;
                                    editor_model.extend_selection_linewise(include_newline, ctx);
                                }
                            }
                            VimMotion::JumpToLine(_line_number) => {
                                // Jumping to line number not supported
                            }
                        }
                    }
                    VimOperand::Line => {
                        let include_newline = *operator != VimOperator::Change;
                        editor_model.extend_selection_below(operand_count.saturating_sub(1), ctx);
                        editor_model.extend_selection_linewise(include_newline, ctx);
                    }
                    VimOperand::TextObject(VimTextObject {
                        inclusion,
                        object_type,
                    }) => {
                        editor_model.vim_select_text_object(object_type, *inclusion, operator, ctx);
                    }
                }
            };

        let motion_type = match operand {
            VimOperand::Motion { motion_type, .. } => *motion_type,
            VimOperand::TextObject(text_object) => match text_object {
                VimTextObject {
                    object_type: TextObjectType::Paragraph,
                    ..
                } => MotionType::Linewise,
                _ => MotionType::Charwise,
            },
            VimOperand::Line => MotionType::Linewise,
        };

        // Depending on the operator, we may or may not want a new Edit on the UndoStack.
        match operator {
            VimOperator::Delete | VimOperator::Change => {
                self.edit(
                    ctx,
                    Edits::new()
                        .with_update_buffer(
                            PlainTextEditorViewAction::Delete,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| {
                                editor_model.insert(replacement_text, None, ctx);
                            },
                        )
                        .with_change_selections(selection_change)
                        .with_before_buffer_edit(|editor_model, ctx| {
                            editor_model.copy_selection_to_vim_register(
                                register_name,
                                motion_type,
                                ctx,
                            );
                        })
                        .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                            if motion_type == MotionType::Linewise {
                                editor_model.cursor_line_start(false, ctx);
                            }
                        }),
                );
            }
            VimOperator::ToggleCase | VimOperator::Lowercase | VimOperator::Uppercase => {
                self.edit(
                    ctx,
                    Edits::new()
                        .with_update_buffer(
                            PlainTextEditorViewAction::Delete,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| {
                                if operator == &VimOperator::ToggleCase {
                                    editor_model.toggle_selection_case(ctx);
                                } else if operator == &VimOperator::Lowercase {
                                    editor_model.selection_to_lowercase(ctx);
                                } else if operator == &VimOperator::Uppercase {
                                    editor_model.selection_to_uppercase(ctx);
                                }
                            },
                        )
                        .with_change_selections(selection_change),
                );
            }
            VimOperator::Yank => {
                self.change_selections(ctx, |editor_model, ctx| {
                    let existing_selections = editor_model.selections(ctx).clone();
                    selection_change(editor_model, ctx);
                    editor_model.copy_selection_to_vim_register(register_name, motion_type, ctx);
                    // Linewise motions don't alter the cursor position after the yank, but
                    // charwise motions do.
                    if motion_type == MotionType::Linewise {
                        // Reset the selections to what they were before the yank.
                        editor_model.change_selections(existing_selections, ctx);
                    } else {
                        editor_model.deselect(ctx);
                    }
                });
            }
            VimOperator::ToggleComment => {
                // Commenting is not enabled for the EditorView.
            }
        }
    }

    fn change_mode(&mut self, old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>) {
        match new.mode {
            VimMode::Normal => {
                if *old == VimMode::Insert {
                    // when exiting insert mode, move cursor back to cover
                    // the character that was last inserted.
                    self.move_left(/* stop at line start */ true, ctx);
                }
                self.vim_maybe_enforce_cursor_line_cap(ctx);
            }
            VimMode::Insert => self.vim_apply_insert_position(&new.position, ctx),
            VimMode::Visual(_) => self.vim_set_visual_tail(ctx),
            _ => {}
        }
        ctx.notify();
    }

    fn toggle_case(&mut self, char_count: u32, ctx: &mut ViewContext<Self>) {
        let chars_to_toggle = u32::min(char_count, self.distance_to_line_end(ctx));
        self.toggle_character_case(chars_to_toggle, ctx);
    }

    fn search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::Search {
            direction: *direction,
            term: None,
        });
    }

    fn cycle_search(&mut self, _direction: &Direction, _ctx: &mut ViewContext<Self>) {
        // Using "n" and "N" to navigate search is currently not supported for the generic editor view
    }

    /// The "*" and "#" commands are only supported when there is a single cursor.
    fn search_word_at_cursor(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        let Some(point) = self.single_cursor_to_point(ctx) else {
            return;
        };
        let term = self
            .editor_model
            .as_ref(ctx)
            .buffer(ctx)
            .get_word_nearest_to_point(&point);

        ctx.emit(Event::Search {
            direction: *direction,
            term,
        });
    }

    fn ex_command(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::ExCommand);
    }

    fn jump_to_first_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.cursor_top(ctx);
    }

    fn jump_to_last_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_to_buffer_end(false /* keep_selection */, ctx);
            editor_model.cursor_line_start(false /* keep_selection */, ctx);
        });
    }

    fn jump_to_line(&mut self, _line_number: u32, _ctx: &mut ViewContext<Self>) {
        // Jumping to line number not supported
    }

    fn jump_to_matching_bracket(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.vim_move_cursor_to_matching_bracket(/* keep_selection */ false, ctx);
        });
    }

    fn jump_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.vim_move_cursor_to_unmatched_bracket(
                bracket, /* keep_selection */ false, ctx,
            );
        });
    }

    fn paste(
        &mut self,
        count: u32,
        direction: &Direction,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(RegisterContent { text, motion_type }) = VimRegisters::handle(ctx)
            .update(ctx, |registers, ctx| {
                registers.read_from_register(register_name, ctx)
            })
        else {
            return;
        };
        match motion_type {
            MotionType::Charwise => {
                self.edit(
                    ctx,
                    Edits::new()
                        .with_update_buffer(
                            PlainTextEditorViewAction::Paste,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| {
                                for _ in 0..count {
                                    editor_model.insert(&text, None, ctx);
                                }
                            },
                        )
                        .with_change_selections(|editor_model, ctx| {
                            if *direction == Direction::Forward {
                                editor_model.move_cursors_by_offset(
                                    1,
                                    &Direction::Forward,
                                    /* keep_selection */ false,
                                    /* stop_at_line_boundary */ false,
                                    ctx,
                                );
                            }
                        })
                        .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                            editor_model.move_cursors_by_offset(
                                1,
                                &Direction::Backward,
                                /* keep_selection */ false,
                                /* stop_at_line_boundary */ false,
                                ctx,
                            );
                        }),
                );
            }
            MotionType::Linewise => {
                self.edit(
                    ctx,
                    Edits::new()
                        .with_update_buffer(
                            PlainTextEditorViewAction::Paste,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| {
                                let text_to_insert = match direction {
                                    Direction::Backward => text.clone(),
                                    Direction::Forward => {
                                        "\n".to_owned()
                                            + if text.ends_with('\n') {
                                                &text[..text.len() - 1]
                                            } else {
                                                &text
                                            }
                                    }
                                };
                                for _ in 0..count {
                                    editor_model.insert(&text_to_insert, None, ctx);
                                }
                            },
                        )
                        .with_change_selections(|editor_model, ctx| match direction {
                            Direction::Backward => {
                                editor_model.cursor_line_start(/* keep_selection */ false, ctx)
                            }
                            Direction::Forward => {
                                editor_model.cursor_line_end(/* keep_selection */ false, ctx)
                            }
                        })
                        .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                            match direction {
                                Direction::Backward => {
                                    for _ in 0..(text.len() * (count as usize)) {
                                        editor_model.move_cursors_by_offset(
                                            1,
                                            &Direction::Backward,
                                            /* keep_selection */ false,
                                            /* stop_at_line_boundary */ false,
                                            ctx,
                                        );
                                    }
                                }
                                Direction::Forward => {
                                    editor_model.cursor_line_start_non_whitespace(
                                        /* keep_selection */ false, ctx,
                                    );
                                }
                            };
                        }),
                );
            }
        }
    }

    /// Join the current line with the following one(s).
    /// This is implemented by replacing newline characters with spaces.
    fn join_line(&mut self, mut line_count: u32, ctx: &mut ViewContext<Self>) {
        // 1J joins two lines, which is the same as 2J.
        if line_count == 1 {
            line_count = 2;
        }
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Delete,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    let buffer = editor_model.buffer(ctx);
                    let mut new_selections: Vec<LocalSelection> = vec![];
                    for selection in editor_model.selections(ctx).iter() {
                        let Ok(start_offset) = selection.start().to_char_offset(buffer) else {
                            continue;
                        };
                        let Ok(chars) = buffer.chars_at(start_offset) else {
                            continue;
                        };
                        chars
                            .enumerate()
                            .filter(|&(_, c)| c == '\n')
                            .take(line_count.saturating_sub(1) as usize)
                            .for_each(|(addtl_offset, _)| {
                                let Ok(start_anchor) =
                                    buffer.anchor_before(start_offset + addtl_offset)
                                else {
                                    return;
                                };
                                let Ok(end_anchor) =
                                    buffer.anchor_before(start_offset + addtl_offset + 1)
                                else {
                                    return;
                                };

                                new_selections.push(LocalSelection {
                                    selection: Selection {
                                        start: start_anchor,
                                        end: end_anchor,
                                        reversed: false,
                                    },
                                    clamp_direction: Default::default(),
                                    goal_start_column: None,
                                    goal_end_column: None,
                                })
                            });
                    }
                    if let Ok(new_selections) = Vec1::try_from_vec(new_selections) {
                        editor_model.change_selections(new_selections, ctx);
                        editor_model.insert(" ", None, ctx);
                        editor_model.move_cursors_by_offset(
                            1,
                            &Direction::Backward,
                            /* keep_selection */ false,
                            /* stop_at_line_boundary */ true,
                            ctx,
                        );
                        editor_model.change_selections(
                            vec1![editor_model.last_selection(ctx).clone()],
                            ctx,
                        );
                    }
                },
            ),
        );
    }

    fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        self.undo(ctx);
    }

    fn visual_operator(
        &mut self,
        operator: &VimOperator,
        motion_type: MotionType,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let selection_change =
            |editor_model: &mut EditorModel, ctx: &mut ModelContext<EditorModel>| {
                let include_newline = *operator != VimOperator::Change;
                editor_model.vim_visual_selection_range(motion_type, include_newline, ctx);
            };
        match operator {
            VimOperator::Delete | VimOperator::Change => {
                self.edit(
                    ctx,
                    Edits::new()
                        .with_update_buffer(
                            PlainTextEditorViewAction::Delete,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| {
                                editor_model.insert("", None, ctx);
                            },
                        )
                        .with_change_selections(selection_change)
                        .with_before_buffer_edit(|editor_model, ctx| {
                            editor_model.copy_selection_to_vim_register(
                                register_name,
                                motion_type,
                                ctx,
                            );
                        })
                        .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                            if motion_type == MotionType::Linewise {
                                editor_model.cursor_line_start(false, ctx);
                            }
                        }),
                );
            }
            VimOperator::ToggleCase | VimOperator::Lowercase | VimOperator::Uppercase => {
                self.edit(
                    ctx,
                    Edits::new()
                        .with_update_buffer(
                            PlainTextEditorViewAction::InsertChar,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| {
                                if operator == &VimOperator::ToggleCase {
                                    editor_model.toggle_selection_case(ctx);
                                } else if operator == &VimOperator::Lowercase {
                                    editor_model.selection_to_lowercase(ctx);
                                } else if operator == &VimOperator::Uppercase {
                                    editor_model.selection_to_uppercase(ctx);
                                }
                            },
                        )
                        .with_change_selections(selection_change),
                );
            }
            VimOperator::Yank => {
                self.change_selections(ctx, |editor_model, ctx| {
                    selection_change(editor_model, ctx);
                    editor_model.copy_selection_to_vim_register(register_name, motion_type, ctx);
                    editor_model.deselect(ctx);
                });
            }
            VimOperator::ToggleComment => {
                // Commenting is not enabled for the EditorView.
            }
        }
    }

    fn visual_paste(
        &mut self,
        motion_type: MotionType,
        read_register_name: char,
        write_register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(RegisterContent {
            text,
            motion_type: yanked_motion_type,
        }) = VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
            registers.read_from_register(read_register_name, ctx)
        })
        else {
            return;
        };
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::Paste,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| {
                        editor_model.insert(&text, None, ctx);
                    },
                )
                .with_change_selections(
                    |editor_model: &mut EditorModel, ctx: &mut ModelContext<EditorModel>| {
                        let include_newline = motion_type == MotionType::Linewise
                            && yanked_motion_type == MotionType::Linewise;
                        editor_model.vim_visual_selection_range(motion_type, include_newline, ctx);
                    },
                )
                .with_before_buffer_edit(|editor_model, ctx| {
                    editor_model.copy_selection_to_vim_register(
                        write_register_name,
                        motion_type,
                        ctx,
                    );
                })
                .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                    if motion_type == MotionType::Linewise {
                        editor_model.cursor_line_start(false, ctx);
                    }
                }),
        );
    }

    fn visual_text_object(&mut self, text_object: &VimTextObject, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            // Text objects in visual mode actually change the selection tail from what we had set when we first entered visual mode.
            // Once we figure out the bounds for the text object(s), store the new tails in this Vec.
            let mut visual_tails = vec![];
            let VimTextObject {
                inclusion,
                object_type,
            } = text_object;

            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let Ok(offset) = selection.head().to_char_offset(buffer) else {
                    continue;
                };
                let selection_range = match (object_type, inclusion) {
                    (TextObjectType::Word(word_type), TextObjectInclusion::Around) => {
                        vim_a_word(buffer, offset, *word_type)
                    }
                    (TextObjectType::Word(word_type), TextObjectInclusion::Inner) => {
                        vim_inner_word(buffer, offset, *word_type)
                    }
                    // `vim_a_paragraph` and `vim_inner_paragraph` do _not_ include the trailing
                    // newline. For other operators, we rely on [`Self::extend_selection_linewise`]
                    // to grow the range to include the newline, but we don't call that for visual
                    // mode expansion of text objects.
                    (TextObjectType::Paragraph, TextObjectInclusion::Around) => {
                        vim_a_paragraph(buffer, offset).map(|range| range.start..range.end + 1)
                    }
                    (TextObjectType::Paragraph, TextObjectInclusion::Inner) => {
                        vim_inner_paragraph(buffer, offset).map(|range| range.start..range.end + 1)
                    }
                    (TextObjectType::Quote(quote_type), TextObjectInclusion::Around) => {
                        vim_a_quote(buffer, offset, *quote_type)
                    }
                    (TextObjectType::Quote(quote_type), TextObjectInclusion::Inner) => {
                        vim_inner_quote(buffer, offset, *quote_type)
                    }
                    (TextObjectType::Block(bracket_type), TextObjectInclusion::Around) => {
                        vim_a_block(buffer, offset, *bracket_type)
                    }
                    (TextObjectType::Block(bracket_type), TextObjectInclusion::Inner) => {
                        vim_inner_block(buffer, offset, *bracket_type, false)
                    }
                };
                let Some(Range { start, mut end }) = selection_range else {
                    continue;
                };
                if end > start {
                    end -= 1;
                }
                let Ok(mut end_point) = buffer.point_for_offset(end) else {
                    continue;
                };
                // Cursor always snaps to column 0 on paragraph text objects.
                if let TextObjectType::Paragraph = text_object.object_type {
                    end_point.column = 0;
                }
                let Ok(new_head) = buffer.anchor_at(end_point, AnchorBias::Left) else {
                    continue;
                };
                let Ok(new_tail) = buffer.anchor_at(start, AnchorBias::Left) else {
                    continue;
                };
                selection.set_start(new_head.clone());
                selection.set_end(new_head);
                visual_tails.push(new_tail);
            }
            editor_model.change_selections(new_selections, ctx);
            editor_model.vim_set_visual_tails(visual_tails);
        });
    }

    fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        self.backspace(ctx);
    }

    fn delete_forward(&mut self, ctx: &mut ViewContext<Self>) {
        self.delete(ctx);
    }

    fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        self.escape(ctx);
    }
}

impl EditorView {
    fn snapshot(&self, ctx: &AppContext) -> ViewSnapshot {
        let font_cache = ctx.font_cache();
        let appearance = Appearance::as_ref(ctx);
        ViewSnapshot {
            view_id: self.view_id,
            is_focused: self.is_focused(),
            editor_model: self.editor_model.clone(),

            can_select: self.can_select(ctx),

            font_size: self.font_size(appearance),
            font_family: self.font_family(appearance),
            placeholder_font_family: self.placeholder_font_family(appearance),
            font_properties: self.font_properties(appearance),
            line_height: self.line_height(font_cache, appearance),
            line_height_ratio: self.line_height_ratio(appearance),
            em_width: self.em_width(font_cache, appearance),

            autogrow: self.autogrow,
            is_empty: self.is_empty(ctx),

            placeholder_texts: self.placeholder_texts.clone(),

            autosuggestion_state: self.autosuggestion_state.clone(),
            command_xray: self.get_command_x_ray(),

            cached_buffer_points: self.cached_buffer_points.clone(),

            baseline_position_computation_method: self.baseline_position_computation_method.clone(),

            #[cfg(feature = "voice_input")]
            voice_input_state: self.voice_input_state.clone(),

            editor_height_shrink_delay: self.editor_height_shrink_delay.clone(),
        }
    }

    pub fn set_shell_family(&mut self, shell_family: ShellFamily) {
        self.shell_family = Some(shell_family);
    }

    pub fn shell_family(&self) -> Option<ShellFamily> {
        self.shell_family
    }

    pub fn set_drag_drop_path_transformer(&mut self, transformer: Option<PathTransformerFn>) {
        self.drag_drop_path_transformer = transformer;
    }

    fn clipboard_content(&mut self, ctx: &mut ViewContext<Self>) -> String {
        let content = ctx.clipboard().read();
        self.clipboard_text_content(content)
    }

    pub fn clipboard_text_content(&self, content: ClipboardContent) -> String {
        clipboard_content_with_escaped_paths(
            content,
            self.shell_family,
            self.convert_newline_to_space,
        )
    }

    fn middle_click_paste_content(&mut self, ctx: &mut ViewContext<Self>) -> Option<String> {
        let content = SelectionSettings::handle(ctx).update(ctx, |selection, ctx| {
            selection.read_for_middle_click_paste(ctx)
        });
        content.map(|content| self.clipboard_text_content(content))
    }

    /// Creates a single-line [`EditorView`] with an empty buffer
    /// and with behaviour specified by `options`.
    pub fn single_line(options: SingleLineEditorOptions, ctx: &mut ViewContext<Self>) -> Self {
        let options: EditorOptions = options.into();
        Self::new(options, ctx)
    }

    /// Creates an [`EditorView`] with an empty buffer
    /// and with behaviour specified by `options`.
    pub fn new(options: EditorOptions, ctx: &mut ViewContext<Self>) -> Self {
        Self::new_internal("", options, ctx)
    }

    pub fn with_next_command_model(
        self,
        next_command_model: ModelHandle<NextCommandModel>,
    ) -> Self {
        Self {
            next_command_model: Some(next_command_model),
            ..self
        }
    }

    pub fn with_context_model(self, context_model: ModelHandle<BlocklistAIContextModel>) -> Self {
        Self {
            context_model: Some(context_model),
            ..self
        }
    }

    /// Creates an [`EditorView`] with the initial text
    /// equal to `base_text` and with behaviour specified by `options`.
    #[cfg(test)]
    pub fn new_with_base_text(
        base_text: impl Into<String>,
        options: EditorOptions,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_internal(base_text, options, ctx)
    }

    fn new_internal(
        base_text: impl Into<String>,
        options: EditorOptions,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let base_text = base_text.into();
        let windowing_state_handle = WindowManager::handle(ctx);
        ctx.subscribe_to_model(
            &windowing_state_handle,
            |editor, _handle, evt, ctx| match evt {
                windowing::StateEvent::ValueChanged { current, previous } => {
                    editor.handle_windowing_state_event((current, previous), ctx);
                }
            },
        );

        #[cfg(feature = "voice_input")]
        {
            use crate::workspaces::user_workspaces::UserWorkspaces;

            ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _handle, _event, ctx| {
                me.update_voice_transcription_options(Self::voice_options(ctx), ctx);
                // Re-render if teams-related data changed that may affect whether features such as voice input are enabled.
                ctx.notify();
            });

            ctx.subscribe_to_model(
                &AISettings::handle(ctx),
                |editor, _, event, ctx| match event {
                    AISettingsChangedEvent::VoiceInputEnabled { .. } => {
                        editor.update_voice_transcription_options(Self::voice_options(ctx), ctx)
                    }
                    AISettingsChangedEvent::VoiceInputToggleKey { .. } => ctx.notify(),
                    _ => {}
                },
            );
        }

        let editor_model = ctx.add_model(|ctx| {
            EditorModel::new(
                base_text.clone(),
                DEFAULT_TAB_SIZE,
                options.max_buffer_len,
                options.valid_input_type,
                ctx,
            )
        });
        ctx.subscribe_to_model(&editor_model, Self::handle_model_event);

        let vim_model = ctx.add_model(|_| VimModel::new());

        ctx.subscribe_to_model(&vim_model, Self::handle_vim_event);

        let editor_settings_handle = &AppEditorSettings::handle(ctx);
        ctx.subscribe_to_model(
            editor_settings_handle,
            Self::handle_app_editor_settings_update,
        );

        let accept_autosuggestion_keybinding_view =
            ctx.add_typed_action_view(AcceptAutosuggestionKeybinding::new);
        let autosuggestion_ignore_view = ctx.add_typed_action_view(|_| AutosuggestionIgnore::new());
        let cursor_display_override = if options.allow_user_cursor_preference {
            Some(*editor_settings_handle.as_ref(ctx).cursor_display_type)
        } else {
            None
        };

        ctx.subscribe_to_view(
            &autosuggestion_ignore_view,
            |_me, _, event, ctx| match event {
                AutosuggestionIgnoreEvent::IgnoreAutosuggestion { suggestion } => {
                    ctx.emit(Event::IgnoreAutosuggestion {
                        suggestion: suggestion.clone(),
                    });
                }
            },
        );

        let ai_context_menu_state = if options.include_ai_context_menu {
            let ai_context_menu = ctx.add_typed_action_view(AIContextMenu::new);
            ctx.subscribe_to_view(
                &ai_context_menu,
                |me, _, event: &AIContextMenuEvent, ctx| {
                    let is_udi_enabled =
                        InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
                    let current_input_mode = if me.is_ai_input {
                        InputType::AI
                    } else {
                        InputType::Shell
                    };
                    match event {
                        AIContextMenuEvent::Close {
                            item_count,
                            query_length,
                        } => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::AtMenuInteracted {
                                    action: "cancelled".to_string(),
                                    item_count: *item_count,
                                    query_length: Some(*query_length),
                                    is_udi_enabled,
                                    current_input_mode,
                                },
                                ctx
                            );

                            ctx.emit(Event::SetAIContextMenuOpen(false));
                            ctx.focus_self();
                            ctx.notify();
                        }
                        AIContextMenuEvent::ResultAccepted {
                            action,
                            item_count,
                            query_length,
                        } => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::AtMenuInteracted {
                                    action: "item_selected".to_string(),
                                    item_count: *item_count,
                                    query_length: Some(*query_length),
                                    is_udi_enabled,
                                    current_input_mode,
                                },
                                ctx
                            );

                            ctx.emit(Event::AcceptAIContextMenuItem(action.clone()));
                            ctx.focus_self();
                            ctx.notify();
                        }
                        AIContextMenuEvent::CategorySelected { category } => {
                            ctx.emit(Event::SelectAIContextMenuCategory(*category));
                            ctx.focus_self();
                            ctx.notify();
                        }
                    }
                },
            );

            Some(AIContextMenuState {
                at_context_menu_button_mouse_handle: Default::default(),
                ai_context_menu,
            })
        } else {
            None
        };

        Self {
            view_id: ctx.view_id(),
            editor_model,
            scroll_position: Arc::new(Mutex::new(Vector2F::zero())),
            autoscroll_requested: Arc::new(Mutex::new(false)),
            text_options: options.text,
            use_settings_line_height_ratio: options.use_settings_line_height_ratio,
            windowing_state_handle,
            focused: false,
            cursors_visible: false,
            blink_epoch: 0,
            single_line: options.single_line,
            internal_clipboard: "".to_string(),
            propagate_vertical_navigation_keys: options
                .propagate_and_no_op_vertical_navigation_keys,
            propagate_horizontal_navigation_keys: options.propagate_horizontal_navigation_keys,
            propagate_escape_key: options.propagate_and_no_op_escape_key,
            autogrow: options.autogrow,
            window_id: ctx.window_id(),
            autocomplete_symbols_allowed: options.autocomplete_symbols,
            autocomplete_symbols_setting: *editor_settings_handle.as_ref(ctx).autocomplete_symbols,
            cursor_display_override,
            autosuggestion_state: None,
            next_command_model: None,
            editor_height_shrink_delay: Arc::new(Mutex::new(EditorHeightShrinkDelay {
                editor_height_before_shrink: 0.,
                editor_height_shrink_start: None,
            })),
            placeholder_texts: Arc::new(HashMap::new()),
            hover_handle: Default::default(),
            command_x_ray_mouse_handle: Default::default(),
            command_x_ray_state: None,
            enter_settings: options.enter_settings,
            soft_wrap: options.soft_wrap,
            placeholder_soft_wrap: options.placeholder_soft_wrap,
            base_buffer_text: base_text,
            supports_vim_mode: options.supports_vim_mode,
            vim_model,
            render_decorator_elements: options.render_decorator_elements,
            select_all_on_focus: options.select_all_on_focus,
            clear_selections_on_blur: options.clear_selections_on_blur,
            get_cursor_colors_fn: options.cursor_colors_fn,
            cached_buffer_points: Default::default(),
            baseline_position_computation_method: options.baseline_position_computation_method,
            middle_click_paste: options.middle_click_paste,
            shell_family: None,
            accept_autosuggestion_keybinding_view,
            autosuggestion_ignore_view,
            show_autosuggestion_keybinding_hint: *editor_settings_handle
                .as_ref(ctx)
                .autosuggestion_keybinding_hint,
            show_autosuggestion_ignore_button: *editor_settings_handle
                .as_ref(ctx)
                .show_autosuggestion_ignore_button,
            #[cfg(feature = "voice_input")]
            voice_transcription_button_mouse_handle: Default::default(),
            #[cfg(feature = "voice_input")]
            voice_input_state: Default::default(),
            #[cfg(feature = "voice_input")]
            interaction_state_before_voice: None,
            #[cfg(feature = "voice_input")]
            voice_transcription_options: Self::voice_options(ctx),
            #[cfg(feature = "voice_input")]
            voice_new_feature_popup: Self::create_voice_new_feature_popup(ctx),
            is_ai_input: false,
            convert_newline_to_space: options.convert_newline_to_space,
            context_model: None,
            image_context_options: ImageContextOptions::Disabled,
            image_context_button_mouse_handle: Default::default(),
            ai_context_menu_state,
            delegate_paste_handling: options.delegate_paste_handling,
            drag_drop_path_transformer: options.drag_drop_path_transformer,
            process_attached_images_future_handle: None,
            is_password: options.is_password,
            keymap_context_modifier: options.keymap_context_modifier,
        }
    }

    pub fn set_is_ai_input(&mut self, is_ai_input: bool, ctx: &mut ViewContext<Self>) {
        self.is_ai_input = is_ai_input;
        if !self.is_ai_input && !FeatureFlag::AtMenuOutsideOfAIMode.is_enabled() {
            ctx.emit(Event::SetAIContextMenuOpen(false));
        }
        ctx.notify();
    }

    pub fn update_image_context_options(
        &mut self,
        options: ImageContextOptions,
        ctx: &mut ViewContext<Self>,
    ) {
        log::debug!("update_image_context_options: {options:?}");
        self.image_context_options = options;
        ctx.notify();
    }

    pub fn abort_attached_images_future_handle(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(process_attached_images_future_handle) =
            self.process_attached_images_future_handle.take()
        {
            process_attached_images_future_handle.abort();
        }
        ctx.emit(Event::ProcessingAttachedImages(false));
    }

    /// The replica ID of the collaborative buffer.
    pub fn replica_id<C: ModelAsRef>(&self, ctx: &C) -> ReplicaId {
        self.editor_model.as_ref(ctx).replica_id(ctx)
    }

    /// Forces the collaborative buffer to be recreated.
    /// If a replica ID is provided, the buffer will be initialized with that replica ID.
    /// Otherwise, the new buffer will have the same replica ID as before.
    pub fn reinitialize_buffer(
        &mut self,
        replica_id: Option<ReplicaId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor_model.update(ctx, |model, ctx| {
            model.recreate_buffer(replica_id, ctx);
        });
        ctx.emit(Event::BufferReinitialized);
    }

    pub fn register_remote_peer(
        &mut self,
        replica_id: ReplicaId,
        selection_data: PeerSelectionData,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model().update(ctx, |model, ctx| {
            model.register_remote_peer(replica_id, selection_data, ctx);
        });

        ctx.notify();
    }

    pub fn unregister_all_remote_peers(&mut self, ctx: &mut ViewContext<Self>) {
        self.model().update(ctx, |model, ctx| {
            model.unregister_all_remote_peers(ctx);
        });

        ctx.notify();
    }

    pub fn unregister_remote_peer(&mut self, replica_id: &ReplicaId, ctx: &mut ViewContext<Self>) {
        self.model().update(ctx, |model, ctx| {
            model.unregister_remote_peer(replica_id, ctx);
        });

        ctx.notify();
    }

    pub fn set_remote_peer_selection_data(
        &mut self,
        replica_id: &ReplicaId,
        selection_data: PeerSelectionData,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model().update(ctx, |model, ctx| {
            model.set_remote_peer_selection_data(replica_id, selection_data, ctx);
        });

        ctx.notify();
    }

    /// A helper function to make arbitrary edits more ergonomic.
    fn edit<C, F, U, B>(&mut self, ctx: &mut ViewContext<Self>, edits: Edits<C, F, U, B>)
    where
        C: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
        F: FnOnce(&EditorModel, &mut ModelContext<EditorModel>),
        U: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
        B: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        self.editor_model
            .update(ctx, |model, ctx| model.edit(ctx, edits))
    }

    /// A helper function to make bare selection changes more ergonomic.
    fn change_selections<C>(&mut self, ctx: &mut ViewContext<Self>, change_selections: C)
    where
        C: FnOnce(&mut EditorModel, &mut ModelContext<EditorModel>),
    {
        self.edit(ctx, Edits::new().with_change_selections(change_selections))
    }

    /// Applies incoming edits from peers to the underlying buffer.
    pub fn apply_remote_operations(
        &mut self,
        operations: Vec<CrdtOperation>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editor_model.update(ctx, |model, ctx| {
            model.apply_remote_operations(operations, ctx);
        });
    }

    /// Sets the font family to use when rendering editor text to `font_family_id`.
    pub fn set_font_family(&mut self, font_family_id: FamilyId, ctx: &mut ViewContext<Self>) {
        self.text_options.font_family_override = Some(font_family_id);
        ctx.notify();
    }

    pub fn set_font_size(&mut self, new_font_size: f32, ctx: &mut ViewContext<Self>) {
        self.text_options.font_size_override = Some(new_font_size);
        ctx.notify();
    }

    pub fn set_text_colors(&mut self, text_colors: TextColors, ctx: &mut ViewContext<Self>) {
        self.text_options.text_colors_override = Some(text_colors);
        ctx.notify();
    }

    fn handle_windowing_state_event(
        &mut self,
        (current, previous): (&windowing::State, &windowing::State),
        ctx: &mut ViewContext<Self>,
    ) {
        let prev_cursor_active = self.focused && previous.active_window == Some(ctx.window_id());
        let curr_cursor_active = self.focused && current.active_window == Some(ctx.window_id());

        // Check if the windowing state update caused the state of our cursor to change
        match (prev_cursor_active, curr_cursor_active) {
            (false, true) => {
                // The windowing state update caused the cursor to go from inactive to active
                // Immediately show the cursor and update the blink timer
                self.reset_cursor_blink_timer(ctx);
            }
            (true, false) => {
                // The windowing state update caused the cursor to go from active to inactive
                // Notify the UI framework so that it is hidden on the next render
                ctx.notify();
            }
            _ => {
                // Otherwise, there was no change, so we don't need to call notify
            }
        }
    }

    /// Caches the given point via the position cache.
    /// The position will be cached with the ID `editor_{editor_id}:{position_id}`.
    /// See [`position_id_for_cached_point`].
    ///
    /// Note that the position is cached indefinitely. The caller is responsible for
    /// insuring that the point is valid before relying on the position.
    /// If the point is no longer valid, the cached position will be a foobar value.
    pub fn cache_buffer_point(
        &mut self,
        point: Point,
        position_id: impl Into<Cow<'static, str>>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.cached_buffer_points.insert(position_id.into(), point);

        // Force a re-render so that the position is cached via the editor element.
        ctx.notify();
    }

    #[cfg(test)]
    pub fn get_cached_buffer_point(
        &self,
        position_id: impl Into<Cow<'static, str>>,
    ) -> Option<Point> {
        self.cached_buffer_points.get(&position_id.into()).cloned()
    }

    fn handle_app_editor_settings_update(
        &mut self,
        _: ModelHandle<AppEditorSettings>,
        evt: &AppEditorSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ensure our cached copy of these settings are up-to-date.
        match evt {
            AppEditorSettingsChangedEvent::CursorDisplayState { .. } => {
                if self.cursor_display_override.is_some() {
                    self.cursor_display_override =
                        Some(*AppEditorSettings::as_ref(ctx).cursor_display_type);
                }
            }
            AppEditorSettingsChangedEvent::AutocompleteSymbols { .. } => {
                self.autocomplete_symbols_setting =
                    *AppEditorSettings::as_ref(ctx).autocomplete_symbols;
            }
            AppEditorSettingsChangedEvent::AutosuggestionKeybindingHint { .. } => {
                self.show_autosuggestion_keybinding_hint =
                    *AppEditorSettings::as_ref(ctx).autosuggestion_keybinding_hint;
            }
            AppEditorSettingsChangedEvent::ShowAutosuggestionIgnoreButton { .. } => {
                self.show_autosuggestion_ignore_button =
                    *AppEditorSettings::as_ref(ctx).show_autosuggestion_ignore_button;
            }
            _ => {}
        }
    }

    /// Returns the buffer point for the corresponding offset.
    pub fn point_for_offset(&self, offset: impl ToCharOffset, ctx: &AppContext) -> Result<Point> {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        let char_offset = offset.to_char_offset(buffer)?;
        buffer.to_point(char_offset)
    }

    fn next_command_state<'a, A: ModelAsRef>(&self, ctx: &'a A) -> &'a NextCommandSuggestionState {
        self.next_command_model
            .as_ref()
            .map_or(&NextCommandSuggestionState::None, |model| {
                model.as_ref(ctx).get_state()
            })
    }

    /// Set an autosuggestion that is rendered natively within the editor as "ghosted" text. This
    /// autosuggestion will continue to be displayed as long as the text in the editor stays the
    /// same or a user inserts a prefix of autosuggestion text.
    pub fn set_autosuggestion(
        &mut self,
        text: impl Into<String>,
        location: AutosuggestionLocation,
        autosuggestion_type: AutosuggestionType,
        ctx: &mut ViewContext<Self>,
    ) {
        let buffer_snapshot = self.autosuggestion_basis(location, ctx);

        if let Some(buffer_text) = buffer_snapshot {
            let text = text.into();
            let full_suggestion = format!("{buffer_text}{text}");

            self.autosuggestion_state = Some(Arc::new(AutosuggestionState {
                buffer_snapshot: buffer_text,
                original_autosuggestion_text: text.clone(),
                current_autosuggestion_text: Some(text),
                autosuggestion_type,
                location,
            }));

            self.autosuggestion_ignore_view.update(ctx, |view, _ctx| {
                view.set_current_autosuggestion(Some(full_suggestion));
            });

            ctx.notify();
        }
    }

    /// Clears any existing autosuggestions (intelligent or not) that weren't for the current input_type.
    /// If there's an empty buffer, populates the input with an intelligent autosuggestion for the input_type.
    pub fn maybe_populate_intelligent_autosuggestion(
        &mut self,
        input_type: InputType,
        ctx: &mut ViewContext<Self>,
    ) {
        // If our existing autosuggestion is not meant for the current input type, clear it.
        if self
            .autosuggestion_state
            .as_ref()
            .is_some_and(|state| !state.autosuggestion_type.matches_input_type(input_type))
        {
            self.clear_autosuggestion(ctx);
        }
        if input_type.is_ai() {
            // The server does not return AI query suggestions currently.
            // If we switched to AI input, clear the next command state.
            // This way when switching back to shell input, there should be no next command suggestion populated.
            self.clear_next_command_state(ctx);
        } else if let Some(command) = self
            .next_command_state(ctx)
            .command_suggestion()
            .map(|command| command.to_owned())
        {
            // Check if this suggestion is ignored before applying it
            let is_ignored = IgnoredSuggestionsModel::as_ref(ctx)
                .is_ignored(&command, SuggestionType::ShellCommand);

            if !is_ignored {
                // If input type is shell, populate with suggested shell command.
                // The suggestion must contain the current buffer text as a prefix.
                let Some(autosuggestion) = command.strip_prefix(self.buffer_text(ctx).as_str())
                else {
                    return;
                };
                self.set_autosuggestion(
                    autosuggestion,
                    AutosuggestionLocation::EndOfBuffer,
                    AutosuggestionType::Command {
                        was_intelligent_autosuggestion: true,
                    },
                    ctx,
                );
            }
        }
    }

    /// Set placeholder text that appears when buffer matches the given prefix.
    /// Use empty string prefix "" for the default placeholder (shown when buffer is empty).
    pub fn set_placeholder_text_with_prefix(
        &mut self,
        prefix: impl Into<String>,
        text: impl Into<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        Arc::make_mut(&mut self.placeholder_texts).insert(prefix.into(), text.into());
        ctx.notify();
    }

    /// Convenience method for setting the default placeholder (empty prefix).
    pub fn set_placeholder_text(&mut self, text: impl Into<String>, ctx: &mut ViewContext<Self>) {
        self.set_placeholder_text_with_prefix("", text, ctx);
    }

    #[cfg(test)]
    pub fn placeholder_text(&self, prefix: &str) -> Option<&str> {
        self.placeholder_texts.get(prefix).map(String::as_str)
    }

    pub fn set_base_buffer_text(&mut self, base_content: String, ctx: &mut ViewContext<Self>) {
        self.base_buffer_text = base_content;
        ctx.notify();
    }

    /// Sets the current command x-ray state.  Even though the x-ray rendering is
    /// handled at the level of the input, the x-ray description needs to be cached
    /// on the editor view so that the editor element can do hit testing on the token
    /// boundaries when there is a mouse move.
    pub fn set_command_x_ray(&mut self, description: Arc<Description>) {
        self.command_x_ray_state = Some(description);
    }

    /// Returns the current command x ray description, if there is one set.
    pub fn get_command_x_ray(&self) -> Option<Arc<Description>> {
        self.command_x_ray_state.clone()
    }

    /// Clears any current command x-ray state, and marks the
    pub fn clear_command_x_ray(&mut self) {
        let state = &mut *self
            .command_x_ray_mouse_handle
            .lock()
            .expect("should get mouse handle lock");
        if let Some(state) = state {
            state.user_dismissed = true;
        }
        self.command_x_ray_state = None;
    }

    /// Clears the current autosuggestion (ghosted text).
    /// Next command state is not cleared so it may be used to populate an autosuggestion again.
    pub fn clear_autosuggestion(&mut self, ctx: &mut ViewContext<Self>) {
        self.autosuggestion_state.take();

        // Clear the current autosuggestion from the ignore view
        self.autosuggestion_ignore_view.update(ctx, |view, _ctx| {
            view.set_current_autosuggestion(None);
        });

        ctx.notify();
    }

    /// Clears any next command state. Autosuggestion (ghosted text) is not cleared.
    fn clear_next_command_state(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(next_command_model) = &self.next_command_model {
            next_command_model.update(ctx, |model, _| {
                model.clear_state();
            });
        }
        ctx.notify();
    }

    /// Remove a specific placeholder by prefix.
    pub fn clear_placeholder_text_with_prefix(
        &mut self,
        prefix: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        Arc::make_mut(&mut self.placeholder_texts).remove(prefix);
        ctx.notify();
    }

    /// Convenience method for clearing the default placeholder (empty prefix).
    pub fn clear_placeholder_text(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_placeholder_text_with_prefix("", ctx);
    }

    /// Clear all placeholder texts.
    pub fn clear_all_placeholder_text(&mut self) {
        self.placeholder_texts = Arc::new(HashMap::new());
    }

    pub fn is_single_line(&self) -> bool {
        self.single_line
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_autogrow(&mut self, autogrow: bool) {
        self.autogrow = autogrow;
    }

    /// Clears the transient editor-height shrink-delay state.
    ///
    /// The shrink-delay is useful when height briefly drops during autosuggestion churn, but
    /// certain mode transitions (for example entering Agent View from multiline PS1/classic input)
    /// intentionally remove decorator content in one step. In that case, carrying over the
    /// previous baseline for even one frame causes visible layout jitter.
    pub fn reset_height_shrink_delay(&mut self, ctx: &mut ViewContext<Self>) {
        let mut editor_height_shrink_delay = self.editor_height_shrink_delay.lock();
        editor_height_shrink_delay.editor_height_before_shrink = 0.;
        editor_height_shrink_delay.editor_height_shrink_start = None;
        ctx.notify();
    }

    pub fn set_propagate_vertical_navigation_keys(
        &mut self,
        propagate_vertical_navigation_keys: PropagateAndNoOpNavigationKeys,
    ) {
        self.propagate_vertical_navigation_keys = propagate_vertical_navigation_keys;
    }

    pub fn set_propagate_horizontal_navigation_keys(
        &mut self,
        propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys,
    ) {
        self.propagate_horizontal_navigation_keys = propagate_horizontal_navigation_keys;
    }

    pub fn can_edit<C: ModelAsRef>(&self, ctx: &C) -> bool {
        self.model().as_ref(ctx).can_edit()
    }

    pub fn can_select<C: ModelAsRef>(&self, ctx: &C) -> bool {
        self.model().as_ref(ctx).can_select()
    }

    pub fn interaction_state<C: ModelAsRef>(&self, ctx: &C) -> InteractionState {
        self.editor_model.as_ref(ctx).interaction_state()
    }

    pub fn set_interaction_state(
        &mut self,
        interaction_state: InteractionState,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(feature = "voice_input")]
        if self.is_voice_input_active() {
            // Voice has locked the editor to Selectable. Stash the requested
            // state so it's restored correctly when voice ends.
            self.interaction_state_before_voice = Some(interaction_state);
            return;
        }

        self.editor_model.update(ctx, |model, _| {
            model.set_interaction_state(interaction_state);
        });
        ctx.notify();
    }

    pub fn set_autocomplete_symbols_allowed(&mut self, enable: bool) {
        self.autocomplete_symbols_allowed = enable;
    }

    /// This should not be exposed as a public function.
    /// If you (as a consumer) need access to the model, you
    /// should interface through the [`EditorView`]`.
    fn model(&self) -> &ModelHandle<EditorModel> {
        &self.editor_model
    }

    #[cfg(test)]
    pub fn displayed_text(&self, ctx: &AppContext) -> String {
        self.editor_model.as_ref(ctx).displayed_text(ctx)
    }

    /// Returns whether or not the editor is empty.
    pub fn is_empty(&self, ctx: &AppContext) -> bool {
        self.editor_model.as_ref(ctx).buffer(ctx).is_empty()
    }

    // Returns the buffer characters count (similar to String::Chars::Count).
    pub fn buffer_size(&self, ctx: &AppContext) -> CharOffset {
        self.editor_model.as_ref(ctx).buffer(ctx).len()
    }

    pub fn buffer_text(&self, ctx: &AppContext) -> String {
        self.editor_model.as_ref(ctx).buffer_text(ctx)
    }

    /// Returns the buffer text before the last edit.
    /// If there weren't any edits, returns the base text.
    /// In the case of an undo / redo, the last buffer text would be the
    /// buffer text before the undo / redo.
    ///
    /// Example:
    /// 1. Suppose the buffer text starts as "a".
    /// 2. "b" is inserted.
    /// 3. Undo.
    /// The last buffer text is now "ab".
    pub fn last_buffer_text<'a>(&self, ctx: &'a AppContext) -> &'a str {
        self.editor_model.as_ref(ctx).last_buffer_text()
    }

    /// Gets the last action in the editor model's undo stack.
    pub fn get_last_action<C: ModelAsRef>(&self, ctx: &C) -> Option<PlainTextEditorViewAction> {
        self.editor_model.as_ref(ctx).last_action(ctx)
    }

    fn scroll(&mut self, scroll_position: Vector2F, ctx: &mut ViewContext<Self>) {
        *self.scroll_position.lock() = Vector2F::new(scroll_position.x(), scroll_position.y());
        ctx.notify();
    }

    fn select(&mut self, arg: &SelectAction, ctx: &mut ViewContext<Self>) {
        self.maybe_commit_incomplete_ime_text(ctx);
        if self.can_select(ctx) {
            if let SelectAction::Update { .. } | SelectAction::Extend { .. } = arg {
                self.vim_force_insert_mode(ctx);
            }
            match arg {
                SelectAction::Begin { position, add } => {
                    self.begin_selection(position.point, position.clamp_direction, *add, ctx)
                }
                SelectAction::Update {
                    position,
                    scroll_position,
                } => self.update_selection(*position, *scroll_position, ctx),
                SelectAction::Extend {
                    position,
                    scroll_position,
                } => self.extend_selection(*position, *scroll_position, ctx),
                SelectAction::End => self.end_selection(ctx),
            }
        }
    }

    fn begin_selection(
        &mut self,
        position: DisplayPoint,
        clamp_direction: ClampDirection,
        add: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.focused {
            ctx.focus_self();
            ctx.emit(Event::Activate);
            if self.select_all_on_focus {
                self.select_all(ctx);
                return;
            }
        }

        self.change_selections(ctx, |editor_model, ctx| {
            let cursor = display_point_to_anchor_clamped(editor_model, position, Bias::Left, ctx);

            let selection = LocalSelection {
                selection: Selection {
                    start: cursor.clone(),
                    end: cursor,
                    reversed: false,
                },
                clamp_direction,
                goal_start_column: None,
                goal_end_column: None,
            };

            let pending = Some(LocalPendingSelection {
                selection: selection.clone(),
                selection_mode: SelectionMode::Chars,
                starting_selection: selection.clone(),
                is_single_selection: !add,
            });

            let selections = if !add {
                vec1![selection]
            } else {
                editor_model.selections(ctx).clone()
            };

            editor_model.change_selections(
                LocalSelections {
                    pending,
                    selections,
                    marked_text_state: Default::default(),
                },
                ctx,
            );
        });
    }

    fn update_selection(
        &mut self,
        position: DisplayPoint,
        scroll_position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            if let Some(mut pending_selection) = editor_model.pending_selection(ctx).cloned() {
                let cursor =
                    display_point_to_anchor_clamped(editor_model, position, Bias::Left, ctx);

                pending_selection
                    .selection
                    .set_head(editor_model.buffer(ctx), cursor.clone());

                let new_selections = if pending_selection.is_single_selection {
                    let buffer = editor_model.buffer(ctx);
                    let mut selection = editor_model.first_selection(ctx).clone();
                    let selection_mode = pending_selection.selection_mode;
                    let starting_selection = pending_selection.starting_selection.clone();

                    // Variables used to compare position of current cursor relative to starting_selection
                    let ordering = cursor
                        .cmp(starting_selection.head(), buffer)
                        .expect("Anchors should be comparable");

                    let is_cursor_in_range = starting_selection.range(buffer).contains(
                        &cursor
                            .to_point(buffer)
                            .expect("Should be able to get point from cursor"),
                    );

                    // Update current selection based on head and tails of cursor and starting_selection,
                    // extending to nearby words or lines based on selection mode
                    let mut head = starting_selection.head().clone();
                    let mut tail = starting_selection.tail().clone();

                    match selection_mode {
                        SelectionMode::Lines => {
                            if ordering == Ordering::Greater {
                                // Select next line(s)
                                match buffer.line_len(position.row()) {
                                    Ok(end_col) => {
                                        head = buffer
                                            .anchor_before(Point::new(position.row(), end_col))
                                            .expect("Anchor should exist")
                                    }
                                    Err(_) => log::error!(
                                        "Update selection is called with invalid position"
                                    ),
                                }
                            } else if is_cursor_in_range || ordering == Ordering::Equal {
                                selection = starting_selection.clone();
                            } else {
                                // Select previous line(s)
                                tail = starting_selection.head().clone();
                                head = buffer
                                    .anchor_before(Point::new(position.row(), 0))
                                    .expect("Anchor should exist");
                            }
                        }
                        SelectionMode::Words => {
                            let selection_model = SemanticSelection::as_ref(ctx);
                            if ordering == Ordering::Greater {
                                // Select next word(s)
                                head = editor_model.get_word_end(
                                    position,
                                    selection_model.word_boundary_policy(),
                                    ctx,
                                );
                            } else if is_cursor_in_range || ordering == Ordering::Equal {
                                selection = starting_selection.clone();
                            } else {
                                // Select previous word(s)
                                tail = starting_selection.head().clone();
                                head = editor_model.get_word_start(
                                    position,
                                    selection_model.word_boundary_policy(),
                                    ctx,
                                );
                            }
                        }
                        SelectionMode::Chars => {
                            head = cursor;
                        }
                    }

                    selection.set_head(buffer, head);
                    selection.set_tail(buffer, tail);

                    // Update the current selection to display the correct selection while dragging.
                    // And update the pending selection to display the correct selection after release.
                    LocalSelections {
                        selections: vec1![selection.clone()],
                        pending: Some(LocalPendingSelection {
                            selection,
                            selection_mode,
                            starting_selection,
                            is_single_selection: true,
                        }),
                        marked_text_state: Default::default(),
                    }
                } else {
                    LocalSelections {
                        pending: Some(pending_selection.clone()),
                        selections: editor_model.selections(ctx).clone(),
                        marked_text_state: Default::default(),
                    }
                };

                editor_model.change_selections(new_selections, ctx);
            } else {
                log::error!("update_selection dispatched with no pending selection");
            }
        });

        *self.scroll_position.lock() = scroll_position;
    }

    fn extend_selection(
        &mut self,
        position: DisplayPoint,
        scroll_position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.focused {
            ctx.focus_self();
            ctx.emit(Event::Activate);
        }

        self.change_selections(ctx, |editor_model, ctx| {
            let cursor = display_point_to_anchor_clamped(editor_model, position, Bias::Left, ctx);

            let mut selection = editor_model.first_selection(ctx).clone();
            selection.set_head(editor_model.buffer(ctx), cursor);
            editor_model.change_selections(vec1![selection], ctx);
        });

        *self.scroll_position.lock() = scroll_position;
        ctx.notify();
    }

    fn end_selection(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            if let Some(pending_selection) = editor_model.pending_selection(ctx).cloned() {
                // Insert the starting selection.
                let new_selections = if pending_selection.is_single_selection {
                    vec1![pending_selection.starting_selection]
                } else {
                    let mut new_selections = editor_model.selections(ctx).clone();
                    let ix = editor_model.selection_insertion_index(
                        pending_selection.starting_selection.start(),
                        ctx,
                    );
                    new_selections.insert(ix, pending_selection.starting_selection);
                    new_selections
                };
                // This will also clear the pending selection.
                editor_model.change_selections(new_selections, ctx);

                // Insert the pending selection itself.
                let mut new_selections = editor_model.selections(ctx).clone();
                let ix = editor_model
                    .selection_insertion_index(pending_selection.selection.start(), ctx);
                new_selections.insert(ix, pending_selection.selection);
                editor_model.change_selections(new_selections, ctx);
            } else {
                log::error!("end_selection dispatched with no pending selection");
            }
        });
    }

    pub fn select_line(&mut self, position: &DisplayPoint, ctx: &mut ViewContext<Self>) {
        let row = position.row();

        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            match buffer.line_len(row) {
                Ok(end_col) => {
                    let selection = LocalSelection {
                        selection: Selection {
                            start: buffer
                                .anchor_after(Point::new(row, 0))
                                .expect("Anchor should exist"),
                            end: buffer
                                .anchor_before(Point::new(row, end_col))
                                .expect("Anchor should exist"),
                            reversed: false,
                        },
                        clamp_direction: Default::default(),
                        goal_start_column: None,
                        goal_end_column: None,
                    };

                    // Continue expecting selection
                    editor_model.change_selections(
                        LocalSelections {
                            pending: Some(LocalPendingSelection {
                                selection: selection.clone(),
                                selection_mode: SelectionMode::Lines,
                                starting_selection: selection.clone(),
                                is_single_selection: true,
                            }),
                            selections: vec1![selection],
                            marked_text_state: Default::default(),
                        },
                        ctx,
                    );
                }
                Err(_) => {
                    log::error!("select_line is called with invalid position");
                }
            }
        });
    }

    /// This method connects the Editor to the SemanticSelection to implement smart-select. There
    /// is also a GridHandler::smart_search equivalent. As the SemanticSelection operates on a
    /// string and a byte index, we need some glue code to extract a string from the Editor's
    /// Buffer and convert the position of the click into a byte position. Smart-select is based
    /// on regex and that crate returns matches as byte offsets, not char offsets.
    /// The steps are as follows:
    ///
    /// 1. Get the "window", the consecutive non-space characters next to the cursor that might
    ///    contain a smart-select pattern, and extract as a string.
    /// 2. Convert the start of that window to a byte offset.
    /// 3. Convert the click position from a DisplayPoint to a byte offset.
    /// 4. Pass to SemanticSelection::smart_search to see if a smart-select patterns exist.
    /// 5. If a pattern is found, it will be returned as a range of byte offsets, which need to be
    ///    converted to Anchors so the Editor can turn it into a selection.
    fn smart_select(
        click_point: DisplayPoint,
        editor: &mut EditorModel,
        ctx: &mut ModelContext<EditorModel>,
    ) -> Option<(Anchor, Anchor)> {
        let word_start =
            editor.get_word_start(click_point, WordBoundariesPolicy::OnlyWhitespace, ctx);
        let word_end = editor.get_word_end(click_point, WordBoundariesPolicy::OnlyWhitespace, ctx);

        let buffer = editor.buffer(ctx);
        let nonblank_word = buffer.text_for_range(word_start.clone()..word_end).ok()?;
        let start_offset = word_start.to_byte_offset(buffer).ok()?;

        let map = editor.display_map(ctx);
        let click_offset = map
            .anchor_before(click_point, Bias::Left, ctx)
            .ok()?
            .to_byte_offset(buffer)
            .ok()?;

        SemanticSelection::as_ref(ctx)
            .smart_search(&nonblank_word, click_offset - start_offset)
            .and_then(|range| {
                let match_start = buffer.point_for_offset(start_offset + range.start).ok()?;
                let match_end = buffer.point_for_offset(start_offset + range.end).ok()?;
                Some((
                    map.anchor_before(
                        match_start.to_display_point(map, ctx).ok()?,
                        Bias::Left,
                        ctx,
                    )
                    .ok()?,
                    map.anchor_before(match_end.to_display_point(map, ctx).ok()?, Bias::Right, ctx)
                        .ok()?,
                ))
            })
    }

    /// This method is triggered by the initial double-click selection (not a drag). The way the
    /// selection range expands depends on user settings stored in SemanticSelection. Smart-select may
    /// be enabled, and if not, the word-breaking characters may have been overridden.
    pub fn select_word(&mut self, position: &DisplayPoint, ctx: &mut ViewContext<Self>) {
        let position = *position;

        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            // SemanticSelection determines the editor's word definition via the policy
            let policy = SemanticSelection::as_ref(ctx).word_boundary_policy();

            // first, get the normal, non-smart-selection
            let mut start = editor_model.get_word_start(position, policy.clone(), ctx);
            let mut end = editor_model.get_word_end(position, policy, ctx);

            // run smart-select. reassign the selection start/end only if the results are a
            // LARGER bound than normal selection. this is because smart-selection might
            // sometimes match smaller parts that shouldn't actually override normal selection
            if let Some((smart_start, smart_end)) = Self::smart_select(position, editor_model, ctx)
            {
                if let Ok(Ordering::Less) = smart_start.cmp(&start, editor_model.buffer(ctx)) {
                    start = smart_start
                }
                if let Ok(Ordering::Greater) = smart_end.cmp(&end, editor_model.buffer(ctx)) {
                    end = smart_end
                }
            }

            let selection = LocalSelection {
                selection: Selection {
                    start,
                    end,
                    reversed: false,
                },
                clamp_direction: Default::default(),
                goal_start_column: None,
                goal_end_column: None,
            };

            // Update selections to select word
            editor_model.change_selections(
                LocalSelections {
                    pending: Some(LocalPendingSelection {
                        selection: selection.clone(),
                        selection_mode: SelectionMode::Words,
                        starting_selection: selection.clone(),
                        is_single_selection: true,
                    }),
                    selections: vec1![selection],
                    marked_text_state: Default::default(),
                },
                ctx,
            );
        });
    }

    pub fn is_selecting(&self, ctx: &AppContext) -> bool {
        self.editor_model.as_ref(ctx).is_selecting(ctx)
    }

    #[cfg(test)]
    pub fn select_ranges<T>(&mut self, ranges: T, ctx: &mut ViewContext<Self>) -> Result<()>
    where
        T: IntoIterator<Item = Range<DisplayPoint>>,
    {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model
                .select_ranges_by_display_point(ranges, ctx)
                .unwrap();
        });

        Ok(())
    }

    pub fn copy(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_password {
            return;
        }

        if self.can_select(ctx) {
            let text = self.selected_text(ctx);
            if text.is_empty() {
                ctx.emit(Event::Copy);
            } else {
                ctx.clipboard().write(ClipboardContent::plain_text(text));
            }
        }
    }

    pub fn cut(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_password {
            return;
        }
        self.copy(ctx);
        self.backspace(ctx);
    }

    pub fn paste(&mut self, ctx: &mut ViewContext<Self>) {
        // If this editor does not delegate paste handling, insert clipboard text content.
        // When paste handling is delegated, the parent view (e.g. the terminal input) is
        // responsible for processing the paste.
        if !self.delegate_paste_handling {
            // Read clipboard contents
            let content = ctx.clipboard().read();

            let clipboard_content_str = self.clipboard_text_content(content);
            self.user_initiated_insert(
                &clipboard_content_str,
                PlainTextEditorViewAction::Paste,
                ctx,
            );
        }

        ctx.emit(Event::Paste);
    }

    fn middle_click_paste(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.middle_click_paste {
            return;
        }

        let clipboard_content = self.middle_click_paste_content(ctx);
        if let Some(clipboard_content) = clipboard_content {
            self.user_initiated_insert(&clipboard_content, PlainTextEditorViewAction::Paste, ctx);
            ctx.emit(Event::MiddleClickPaste);
        }
        ctx.focus_self();
    }

    // Pastes the most recently cut text within the shell.
    fn yank(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.internal_clipboard.clone();
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Yank,
                EditOrigin::UserInitiated,
                move |editor_model, ctx| {
                    editor_model.insert(text.as_str(), None, ctx);
                },
            ),
        );
    }

    /// Returns whether or not the editor's current buffer text is equal to
    /// the base buffer text it was supplied with.
    pub fn is_dirty(&self, ctx: &AppContext) -> bool {
        !self.buffer_text(ctx).eq(&self.base_buffer_text)
    }

    pub fn selected_text<C: ModelAsRef>(&self, ctx: &C) -> String {
        self.editor_model.as_ref(ctx).selected_text(ctx)
    }

    pub fn selected_text_strings(&self, ctx: &mut ViewContext<Self>) -> Vec<String> {
        self.editor_model.as_ref(ctx).selected_text_strings(ctx)
    }

    pub fn start_byte_index_of_first_selection<A: ModelAsRef>(&self, ctx: &A) -> ByteOffset {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        model
            .first_selection(ctx)
            .head()
            .to_byte_offset(buffer)
            .expect("Selection must be convertible to byte offset")
    }

    /// Finds the start byte of the token under the given point (the offset at
    /// the start word boundary of the underlying token)
    pub fn start_byte_offset_at_point(
        &self,
        point: &DisplayPoint,
        ctx: &AppContext,
    ) -> Option<ByteOffset> {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        let map = model.display_map(ctx);

        let position = point.to_buffer_point(map, Bias::Left, ctx).ok()?;
        let mut word_starts = buffer
            .word_starts_backward_from_offset_inclusive(position)
            .ok()?;
        word_starts
            .next()
            .and_then(|word_start| word_start.to_byte_offset(buffer).ok())
    }

    /// Returns the starting byte index position of the last selection.
    pub fn start_byte_index_of_last_selection<A: ModelAsRef>(&self, ctx: &A) -> ByteOffset {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        model.last_selection(ctx).to_byte_offset(buffer).sorted().0
    }

    /// Returns the ending byte index position of the last selection.
    pub fn end_byte_index_of_last_selection<A: ModelAsRef>(&self, ctx: &A) -> ByteOffset {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        model.last_selection(ctx).to_byte_offset(buffer).sorted().1
    }

    // Clears editor buffer but does not clear the undo/redo stack.
    pub fn clear_buffer(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ClearBuffer,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.clear_buffer(ctx);
                },
            ),
        );
    }

    /// Clears editor buffer if the vim mode allows for it, but does not
    /// clear the undo/redo stack.
    pub fn handle_ctrl_c(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "voice_input")]
        {
            self.stop_voice_input(true, ctx);
        }

        #[cfg(windows)]
        // On Windows, if there is selected text, users expect ctrl-c to copy.
        if !self.selected_text(ctx).is_empty() {
            self.copy(ctx);
            return;
        }

        if !self.can_edit(ctx) {
            return;
        }

        let terminal_view = ctx
            .windows()
            .active_window()
            .and_then(|active_window| {
                ctx
                    // Need to get the workspace info since we don't have access to the terminal view
                    // from the ClearBuffer action.
                    .views_of_type::<Workspace>(active_window)
                    .and_then(|views| views.first().cloned())
            })
            .and_then(|workspace| {
                workspace
                    .as_ref(ctx)
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
            });

        // If an agent is responding, we don't want ctrl+c to clear the persistent input.
        let is_agent_responding = terminal_view
            .as_ref()
            .and_then(|terminal_view| {
                BlocklistAIHistoryModel::as_ref(ctx).active_conversation(terminal_view.id())
            })
            .is_some_and(|conversation| {
                conversation.status().is_in_progress() && conversation.exchange_count() > 0
            });

        // If there is a pending passive ai block, we don't want ctrl+c to clear the buffer.
        let is_pending_passive_ai_block = terminal_view.is_some_and(|terminal_view| {
            let terminal_model = terminal_view.as_ref(ctx).model.lock();
            terminal_model
                .block_list()
                .last_non_hidden_ai_block_handle(ctx)
                .is_some_and(|ai_block| {
                    let block = ai_block.as_ref(ctx);
                    // Ctrl+c should dismiss the passive ai block only if the keybindings for the block are not hidden.
                    let is_pending_code_diff = block.find_undismissed_code_diff(ctx).is_some();
                    let is_pending_suggested_prompt = block
                        .pending_unit_test_suggestion(ctx)
                        .is_some_and(|suggested_prompt| {
                            !suggested_prompt.as_ref(ctx).is_keybindings_hidden()
                        });
                    block.is_passive_conversation(ctx)
                        && (is_pending_code_diff || is_pending_suggested_prompt)
                })
        });

        let mut cleared_buffer_len = 0;
        if (!self.vim_mode_enabled(ctx)
            || self
                .vim_mode(ctx)
                .is_some_and(|vim_mode| matches![vim_mode, VimMode::Normal | VimMode::Insert]))
            && !is_agent_responding
            && !is_pending_passive_ai_block
        {
            cleared_buffer_len = self.buffer_size(ctx).as_usize();
            self.clear_buffer(ctx);
        }
        if !is_agent_responding || !is_pending_passive_ai_block {
            self.vim_interrupt(ctx);
        }

        ctx.emit(Event::CtrlC { cleared_buffer_len });
    }

    // Clears editor buffer and conditionally resets the undo stack.
    pub fn system_clear_buffer(&mut self, reset_undo_stack: bool, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ClearBuffer,
                EditOrigin::SystemEdit,
                |editor_model, ctx| {
                    editor_model.clear_buffer(ctx);
                },
            ),
        );

        if reset_undo_stack {
            // Reset the undo stack _after_ clearing the buffer so that the change
            // is not pushed onto the stack.
            self.editor_model.update(ctx, |model, ctx| {
                model.reset_undo_redo_stack(ctx);
            });
        }
    }

    // Clears editor buffer but also resets the undo/redo stack.
    pub fn clear_buffer_and_reset_undo_stack(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ClearBuffer,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.clear_buffer(ctx);
                },
            ),
        );

        // Reset the undo stack _after_ clearing the buffer so that the change
        // is not pushed onto the stack.
        self.editor_model.update(ctx, |model, ctx| {
            model.reset_undo_redo_stack(ctx);
        });
    }

    // Replaces the current editor buffer with temporary contents (e.g. commands from history up).
    // This will not create new undo items in the undo/redo stack.
    pub fn set_buffer_text_ignoring_undo(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer_options(
                PlainTextEditorViewAction::ReplaceBuffer,
                EditOrigin::UserInitiated,
                UpdateBufferOption::IsEphemeral,
                |model, ctx| {
                    model.clear_buffer(ctx);
                    model.insert(content, None, ctx);
                },
            ),
        );
    }

    // Inserts selected text into buffer, without creating new undo items.
    pub fn insert_selected_text_to_buffer_ignoring_undo(
        &mut self,
        content: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer_options(
                PlainTextEditorViewAction::InsertSelectedText,
                EditOrigin::UserInitiated,
                UpdateBufferOption::IsEphemeral,
                |model, ctx| {
                    model.insert_selected_text(content, ctx);
                },
            ),
        );
    }

    /// As a system edit, reset the buffer to the given text and clear the undo/redo stack. This is
    /// useful when new buffer contents are synced from another source.
    ///
    /// Also see [`Self::set_buffer_text`], which makes similar updates, but as a user-initiated
    /// edit visible in the undo stack.
    pub fn system_reset_buffer_text(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        let interaction_state = self.interaction_state(ctx);
        self.set_interaction_state(InteractionState::Editable, ctx);

        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ReplaceBuffer,
                EditOrigin::SystemEdit,
                |editor_model, ctx| {
                    editor_model.clear_buffer(ctx);
                    editor_model.insert(content, None, ctx);
                },
            ),
        );

        // Reset the undo stack _after_ replacing the buffer so that the change
        // is not pushed onto the stack.
        self.editor_model.update(ctx, |model, ctx| {
            model.reset_undo_redo_stack(ctx);
        });

        self.set_interaction_state(interaction_state, ctx);
        ctx.notify();
    }

    /// Replaces the current editor buffer. This will create a new undo item in undo/redo stack.
    /// This function works regardless of the interaction state (i.e. it'll allow
    /// the contents to change even if the interaction state is disabled or selectable).
    pub fn set_buffer_text(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        let interaction_state = self.interaction_state(ctx);
        self.set_interaction_state(InteractionState::Editable, ctx);

        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ReplaceBuffer,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.clear_buffer(ctx);
                    editor_model.insert(content, None, ctx);
                },
            ),
        );

        self.set_interaction_state(interaction_state, ctx);
        ctx.notify();
    }

    pub fn set_buffer_text_with_base_buffer(&mut self, content: &str, ctx: &mut ViewContext<Self>) {
        self.set_base_buffer_text(content.to_string(), ctx);
        self.set_buffer_text(content, ctx);
    }

    /// Replaces the current editor buffer. This will create a new undo item in undo/redo stack.
    /// Specifies that the source of the change is the synced inputs feature
    pub fn set_buffer_text_for_syncing_inputs(
        &mut self,
        content: Arc<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        // without this can_edit check, we'lll send text to long-running commands (e.g. vim)
        if self.can_edit(ctx) {
            self.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::SyncedTerminalInput,
                    |editor_model, ctx| {
                        editor_model.clear_buffer(ctx);
                        editor_model.insert_internal(&content, None, SelectionInsertion::No, ctx);
                    },
                ),
            );
            ctx.notify();
        }
    }

    pub fn replace_first_n_characters(
        &mut self,
        n: CharOffset,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ReplaceBuffer,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.replace_first_n_characters(n, text, ctx);
                },
            ),
        );
    }

    pub fn replace_last_n_characters(
        &mut self,
        n: CharOffset,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ReplaceBuffer,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.replace_last_n_characters(n, text, ctx);
                },
            ),
        );
    }

    pub fn cut_word_left(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::CutWordLeft,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.shell_cut(ctx),
                )
                .with_change_selections(|editor_model, ctx| {
                    editor_model.select_word_left(ctx);
                }),
        );
    }

    pub fn delete_word_left(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::DeleteWordLeft,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.insert("", None, ctx),
                )
                .with_change_selections(|editor_model, ctx| editor_model.select_word_left(ctx)),
        );
    }

    pub fn delete_word_right(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::DeleteWordRight,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.insert("", None, ctx),
                )
                .with_change_selections(|editor_model, ctx| editor_model.select_word_right(ctx)),
        );
    }

    pub fn cut_word_right(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::CutWordRight,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.shell_cut(ctx),
                )
                .with_change_selections(|editor_model, ctx| {
                    editor_model.select_word_right(ctx);
                }),
        );
    }

    /// Clear any selections, leaving the cursor at the end of the first selection.
    pub fn clear_selections(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.clear_selections(ctx);
        });
    }

    fn tab(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            match self.propagate_vertical_navigation_keys {
                PropagateAndNoOpNavigationKeys::Always => {
                    ctx.emit(Event::Navigate(NavigationKey::Tab))
                }
                _ => self.handle_tab(ctx),
            };
        }
    }

    pub fn handle_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        // In the zero-state case, we always accept any active autosuggestion, if one exists.
        // Note that this is applied even if TAB is assigned to the completions menu, since
        // there is no conflict in the zero-state case.
        if self.active_autosuggestion() && buffer.is_empty() {
            self.insert_full_autosuggestion(ctx);
            return;
        }
        let selections = self.editor_model.as_ref(ctx).selections(ctx);
        match selections.len() {
            1 => {
                let selection = selections.first();
                // Indent selection if there is a selection
                // Indent is analogous to indenting in a code editor, where we
                // want to move the selected line(s) forward, and we don't
                // want to move the cursor.
                if !selection.is_cursor_only(buffer) {
                    self.indent(ctx);
                } else {
                    self.edit(
                        ctx,
                        Edits::new().with_update_buffer(
                            PlainTextEditorViewAction::Tab,
                            EditOrigin::UserInitiated,
                            |editor_model, ctx| editor_model.insert("    ", None, ctx),
                        ),
                    );
                }
            }
            _ => self.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::Tab,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.insert("    ", None, ctx),
                ),
            ),
        }
    }

    fn indent(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Indent,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.indent(ctx);
                },
            ),
        );
    }

    fn shift_tab(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            match self.propagate_vertical_navigation_keys {
                PropagateAndNoOpNavigationKeys::Always => {
                    ctx.emit(Event::Navigate(NavigationKey::ShiftTab))
                }
                _ => self.unindent(ctx),
            };
        }
    }

    pub fn unindent(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Indent,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.unindent(ctx);
                },
            ),
        );
    }

    pub fn insert_selected_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::InsertSelectedText,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.insert_selected_text(text, ctx);
                },
            ),
        );
    }

    /// Selects the given ranges and replaces the text at the selection with the given `text`.
    pub fn select_and_replace(
        &mut self,
        text: &str,
        selection_ranges: impl IntoIterator<Item = Range<ByteOffset>>,
        action: PlainTextEditorViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(action, EditOrigin::UserInitiated, |editor_model, ctx| {
                    editor_model.insert(text, None /* text_style */, ctx);
                })
                .with_change_selections(|editor_model, ctx| {
                    editor_model
                        .select_ranges_by_offset(selection_ranges, ctx)
                        .expect("byte index selection should be insertable")
                }),
        );
    }

    pub fn user_initiated_insert(
        &mut self,
        text: &str,
        action: PlainTextEditorViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                action,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    editor_model.insert(text, None, ctx);
                },
            ),
        );
    }

    pub fn system_insert(
        &mut self,
        text: &str,
        action: PlainTextEditorViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(action, EditOrigin::SystemEdit, |editor_model, ctx| {
                editor_model.insert(text, None, ctx);
            }),
        );
    }

    /// Performs a delete operation as a system edit (not user-initiated)
    /// Deletes text in the specified range
    pub fn system_delete(&mut self, range: Range<ByteOffset>, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Delete,
                EditOrigin::SystemEdit,
                |editor_model, ctx| {
                    // Convert ByteOffset to CharOffset properly to handle multi-byte characters
                    let buffer = editor_model.buffer(ctx);
                    match (range.start.to_char_offset(buffer), range.end.to_char_offset(buffer)) {
                        (Ok(start_char), Ok(end_char)) => {
                            let char_range = start_char..end_char;
                            if let Err(error) = editor_model.buffer_edit([char_range], "", ctx) {
                                log::error!("error performing system delete: {error}");
                            }
                        }
                        (Err(error), _) | (_, Err(error)) => {
                            log::error!("error converting byte offset to char offset for system delete: {error}");
                        }
                    }
                },
            ),
        );
    }

    /// Selects ranges by `ByteOffset`. Note if the selections specified in `ranges` are empty, the
    /// selections are not set since there must be at least one selection.
    pub fn select_ranges_by_byte_offset(
        &mut self,
        selection_ranges: impl IntoIterator<Item = Range<ByteOffset>>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model
                .select_ranges_by_offset(selection_ranges, ctx)
                .expect("byte index selection should be insertable");
        });
    }

    pub fn text_style_runs<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> impl 'a + Iterator<Item = TextRun> {
        let model = self.editor_model.as_ref(ctx);
        model.buffer(ctx).text_style_runs()
    }

    pub fn clear_text_style_runs(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |model, ctx| {
            model.clear_all_styles(ctx);
        });
    }

    pub fn update_buffer_styles<I, S>(
        &mut self,
        old_ranges: I,
        text_style_operation: TextStyleOperation,
        ctx: &mut ViewContext<Self>,
    ) where
        I: IntoIterator<Item = Range<S>>,
        S: ToCharOffset,
    {
        self.editor_model.update(ctx, |model, ctx| {
            model.update_buffer_styles(old_ranges, text_style_operation, ctx)
        });
    }

    /// Inserts styles into the editor, rendering the ranges marked in `sorted_styles` with the
    /// necessary style.
    ///
    /// Note it up to the caller to ensure `sorted_styles` is sorted in ascending order based on the
    /// start of the range and that each range is disjoint. If a range is not disjoint, we will log a warning and skip it.
    pub fn insert_with_styles(
        &mut self,
        text: &str,
        sorted_styles: &[(Range<ByteOffset>, TextStyle)],
        action: PlainTextEditorViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut final_styles = vec![];
        let mut last_range_end = ByteOffset::from(0);

        for (range, style) in sorted_styles {
            match text.get(last_range_end.as_usize()..range.start.as_usize()) {
                Some(last_range_end_to_curr_range_start) => {
                    final_styles.push((last_range_end_to_curr_range_start, TextStyle::new()));
                }
                None => log::warn!("last_range_end_to_curr_range_start is not a valid range"),
            }

            match text.get(range.start.as_usize()..range.end.as_usize()) {
                Some(curr_range_start_to_end) => {
                    final_styles.push((curr_range_start_to_end, *style));
                }
                None => log::warn!("curr_range_start_to_end is not a valid range"),
            }

            last_range_end = range.end;
        }

        match text.get(last_range_end.as_usize()..text.len()) {
            Some(remaining_text) => {
                final_styles.push((remaining_text, TextStyle::new()));
            }
            None => log::warn!("remaining_text is not a valid range"),
        }

        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                action,
                EditOrigin::UserInitiated,
                move |editor_model, ctx| {
                    for (text, style) in final_styles {
                        editor_model.insert(text, Some(style), ctx);
                    }
                },
            ),
        );
    }

    pub fn vim_mode_enabled(&self, ctx: &AppContext) -> bool {
        self.supports_vim_mode && AppEditorSettings::as_ref(ctx).vim_mode_enabled()
    }

    pub fn user_insert(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        let should_autocomplete_symbols =
            self.autocomplete_symbols_allowed && self.autocomplete_symbols_setting;
        let action = PlainTextEditorViewAction::from_inserted_str(text);
        self.edit(
            ctx,
            Edits::new().with_update_buffer(action, EditOrigin::UserTyped, |editor_model, ctx| {
                if should_autocomplete_symbols {
                    editor_model.insert_and_maybe_autocomplete_symbols(text, ctx);
                } else {
                    editor_model.insert_internal(text, None, SelectionInsertion::No, ctx);
                }
            }),
        );
    }

    fn voice_input_toggle_key_code(&self, ctx: &AppContext) -> Option<KeyCode> {
        let ai_settings_handle = &AISettings::handle(ctx);
        ai_settings_handle
            .as_ref(ctx)
            .voice_input_toggle_key
            .value()
            .to_key_code()
    }

    pub fn attach_files(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let view_id = self.view_id;

        let file_picker_config = FilePickerConfiguration::new().allow_multi_select();

        let is_unsupported_model = self.image_context_options.is_unsupported_model();
        let num_images_attached = self.image_context_options.num_images_attached();
        let num_images_in_conversation = self.image_context_options.num_images_in_conversation();

        ctx.open_file_picker(
            move |result, ctx| {
                match result {
                    Ok(paths) => {
                        // Split picked paths into image and non-image files by MIME type.
                        let mut image_paths = Vec::new();
                        let mut non_image_paths = Vec::new();
                        for path in &paths {
                            let mime = mime_guess::from_path(path)
                                .first_or_octet_stream()
                                .to_string();
                            if CLIPBOARD_IMAGE_MIME_TYPES.contains(&mime.as_str()) {
                                image_paths.push(path.clone());
                            } else {
                                non_image_paths.push(path.clone());
                            }
                        }

                        // If the model doesn't support vision, show toast and clear images.
                        if !image_paths.is_empty() && is_unsupported_model {
                            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                toast_stack.add_ephemeral_toast(
                                    DismissibleToast::error(
                                        "The selected model does not support images as context."
                                            .to_string(),
                                    ),
                                    window_id,
                                    ctx,
                                );
                            });
                            image_paths.clear();
                        }

                        // Apply image count limits.
                        let num_images_user_attached = image_paths.len();
                        let num_excess_images_by_query_limit = (image_paths.len()
                            + num_images_attached)
                            .saturating_sub(MAX_IMAGE_COUNT_FOR_QUERY);
                        let num_excess_images_by_conversation_limit =
                            (image_paths.len() + num_images_attached + num_images_in_conversation)
                                .saturating_sub(MAX_IMAGES_PER_CONVERSATION);
                        let num_excess_images = num_excess_images_by_query_limit
                            .max(num_excess_images_by_conversation_limit);

                        if num_excess_images > 0 {
                            let limit_reason = if num_excess_images
                                == num_excess_images_by_query_limit
                            {
                                format!("limit is {MAX_IMAGE_COUNT_FOR_QUERY} per query")
                            } else {
                                format!("limit is {MAX_IMAGES_PER_CONVERSATION} per conversation")
                            };

                            let message = if num_excess_images == 1 {
                                format!("1 image wasn't attached - {limit_reason}.")
                            } else {
                                format!(
                                    "{num_excess_images} images weren't attached - {limit_reason}."
                                )
                            };

                            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                toast_stack.add_persistent_toast(
                                    DismissibleToast::error(message),
                                    window_id,
                                    ctx,
                                );
                            });
                        }

                        // Process image paths (excluding excess).
                        let image_paths_to_process: Vec<String> =
                            image_paths[0..(image_paths.len() - num_excess_images)].to_vec();

                        if !image_paths_to_process.is_empty() {
                            ctx.dispatch_typed_action_for_view(
                                window_id,
                                view_id,
                                &EditorAction::ReadAndProcessImagesAsync {
                                    num_images_user_attached,
                                    file_paths: image_paths_to_process,
                                },
                            );
                        }

                        // Process non-image file paths.
                        if !non_image_paths.is_empty() {
                            ctx.dispatch_typed_action_for_view(
                                window_id,
                                view_id,
                                &EditorAction::ProcessNonImageFiles {
                                    file_paths: non_image_paths,
                                },
                            );
                        }
                    }
                    Err(err) => {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_persistent_toast(
                                DismissibleToast::error(format!("{err}")),
                                window_id,
                                ctx,
                            );
                        });
                    }
                }
            },
            file_picker_config,
        );

        ctx.notify();
    }

    /// Reads and processes images asynchronously from file paths.
    ///
    /// This function reads image files from the given paths, validates they are supported formats,
    /// and processes them for AI context attachment via `process_and_attach_images_as_ai_context`.
    pub fn read_and_process_images_async(
        &mut self,
        num_images_user_attached: usize,
        file_paths: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.image_context_options.is_enabled() {
            if self.image_context_options.is_unsupported_model() {
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(
                            "The selected model does not support images as context".to_owned(),
                        ),
                        window_id,
                        ctx,
                    );
                });
            }
            return;
        }

        let window_id = ctx.window_id();

        ctx.spawn(
            async move {
                let mut images = vec![];
                let mut num_unsupported_images: usize = 0;
                let mut num_read_errors: usize = 0;

                for path_str in &file_paths {
                    match async_fs::read(path_str).await {
                        Ok(bytes) => {
                            let path = Path::new(path_str);
                            let Some(file_name) = path
                                .file_name()
                                .and_then(|path| path.to_str())
                                .map(|path| path.to_string())
                            else {
                                continue;
                            };

                            let mime_type = from_path(path).first_or_octet_stream().to_string();

                            if !CLIPBOARD_IMAGE_MIME_TYPES.contains(&mime_type.as_str()) {
                                num_unsupported_images += 1;
                                continue;
                            }

                            images.push(AttachedImage {
                                data: bytes,
                                mime_type,
                                file_name,
                            });
                        }
                        Err(e) => {
                            safe_error!(
                                safe: ("Failed to read file: {e}"),
                                full: ("Failed to read file {path_str}: {e}")
                            );
                            num_read_errors += 1;
                        }
                    }
                }

                (images, num_unsupported_images, num_read_errors)
            },
            move |this, (images, num_unsupported_images, num_read_errors), ctx| {
                if num_unsupported_images > 0 {
                    let message = if num_unsupported_images == 1 && num_images_user_attached == 1 {
                        "Image cannot be attached - supported types are PNG, JPG, GIF, WEBP.".into()
                    } else if num_unsupported_images == 1 {
                        "1 image wasn't attached - supported types are PNG, JPG, GIF, WEBP.".into()
                    } else {
                        format!("{num_unsupported_images} images weren't attached - supported types are PNG, JPG, GIF, WEBP.")
                    };

                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_persistent_toast(
                            DismissibleToast::error(message),
                            window_id,
                            ctx,
                        );
                    });
                }

                if num_read_errors > 0 {
                    let message = if num_read_errors == 1 && num_images_user_attached == 1 {
                        "Image cannot be attached - failed to read file.".into()
                    } else if num_read_errors == 1 {
                        "1 image wasn't attached - failed to read file.".into()
                    } else {
                        format!("{num_read_errors} images weren't attached - failed to read files.")
                    };

                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_persistent_toast(
                            DismissibleToast::error(message),
                            window_id,
                            ctx,
                        );
                    });
                }

                if !images.is_empty() {
                    this.process_and_attach_images_as_ai_context(num_images_user_attached, images, ctx);
                }
            },
        );
    }

    /// Processes and attaches images to the AI context model.
    ///
    /// This function handles the final step of image attachment after validation,
    /// updating the context model and UI state accordingly.
    pub fn process_and_attach_images_as_ai_context(
        &mut self,
        num_images_user_attached: usize,
        pending_images: Vec<AttachedImage>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.image_context_options.is_enabled() {
            if self.image_context_options.is_unsupported_model() {
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(
                            "The selected model does not support images as context".to_owned(),
                        ),
                        window_id,
                        ctx,
                    );
                });
            }
            return;
        }

        let is_udi_enabled = InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);

        send_telemetry_from_ctx!(
            TelemetryEvent::AttachedImagesToAgentModeQuery {
                num_images: pending_images.len(),
                is_udi_enabled,
            },
            ctx
        );

        self.process_attached_images_future_handle = Some(ctx.spawn(
            async move {
                let mut processed_pending_images = vec![];
                let mut num_oversized_images: usize = 0;
                let mut num_unprocessed_images: usize = 0;

                for image in pending_images {
                    let is_figma = is_figma_png(&image.data);

                    let resized_image_bytes = match resize_image(&image.data) {
                        Ok(resized_image_bytes) => resized_image_bytes,
                        Err(err) => {
                            num_unprocessed_images += 1;
                            log::warn!("Error resizing attached image {err:?}");
                            continue;
                        }
                    };

                    if resized_image_bytes.len() > MAX_IMAGE_SIZE_BYTES {
                        num_oversized_images += 1;
                        continue;
                    }

                    let base64_str = general_purpose::STANDARD.encode(&resized_image_bytes);

                    processed_pending_images.push(ImageContext {
                        data: base64_str,
                        mime_type: image.mime_type,
                        file_name: image.file_name,
                        is_figma,
                    });
                }

                (
                    num_oversized_images,
                    num_unprocessed_images,
                    processed_pending_images,
                )
            },
            move |this, (num_oversized_images, num_unprocessed_images, pending_images), ctx| {
                // Future was aborted
                if this.process_attached_images_future_handle.is_none() {
                    return;
                }

                let window_id = ctx.window_id();

                if num_oversized_images > 0 {
                    let message = if num_oversized_images == 1 && num_images_user_attached == 1 {
                        "Image cannot be attached - file is too large.".into()
                    } else if num_oversized_images == 1 {
                        "1 image wasn't attached — file is too large.".into()
                    } else {
                        format!(
                            "{num_oversized_images} images weren't attached — files are too large."
                        )
                    };

                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_persistent_toast(
                            DismissibleToast::error(message),
                            window_id,
                            ctx,
                        );
                    });
                }

                if num_unprocessed_images > 0 {
                    let message = if num_unprocessed_images == 1 && num_images_user_attached == 1 {
                        "Image cannot be attached - error processing.".into()
                    } else if num_unprocessed_images == 1 {
                        "1 image wasn't attached - error processing.".into()
                    } else {
                        format!(
                            "{num_unprocessed_images} images weren't attached - error processing."
                        )
                    };

                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_persistent_toast(
                            DismissibleToast::error(message),
                            window_id,
                            ctx,
                        );
                    });
                }

                if let Some(context_model) = &this.context_model {
                    context_model.update(ctx, |context_model, ctx| {
                        context_model.append_pending_images(pending_images, ctx);
                    });
                }

                ctx.emit(Event::ProcessingAttachedImages(false));
            },
        ));

        ctx.emit(Event::ProcessingAttachedImages(true));
    }

    /// Stores non-image files selected via the file picker into the pending files context.
    fn process_non_image_files(&mut self, file_paths: Vec<String>, ctx: &mut ViewContext<Self>) {
        let attachments: Vec<PendingAttachment> = file_paths
            .iter()
            .filter_map(|path_str| {
                let path = std::path::Path::new(path_str);
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())?;
                let mime_type = from_path(path).first_or_octet_stream().to_string();
                Some(PendingAttachment::File(PendingFile {
                    file_name,
                    file_path: path.to_path_buf(),
                    mime_type,
                }))
            })
            .collect();

        if let Some(context_model) = &self.context_model {
            context_model.update(ctx, |context_model, ctx| {
                context_model.append_pending_attachments(attachments, ctx);
            });
        }
    }

    /// Alternate path to Self::user_insert for when Vim mode is enabled. Forwards character
    /// commands to the VimFSA for interpretation.
    fn vim_user_insert(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.vim_model.update(ctx, |vim_model, ctx| {
            for c in text.chars() {
                vim_model.typed_character(c, ctx);
            }
        });
        ctx.emit(Event::VimStatusUpdate)
    }

    fn ime_commit(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        // If Vim is in Normal or Visual mode, we don't want to insert any text.
        if matches!(
            self.vim_mode(ctx),
            Some(VimMode::Normal) | Some(VimMode::Visual(_))
        ) {
            return;
        }

        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::UpdateMarkedText,
                EditOrigin::UserTyped,
                |editor_model, ctx| editor_model.clear_marked_text_and_commit(text, ctx),
            ),
        )
    }

    /// Similar to Self::vim_user_insert, but for keystrokes which aren't represented by a char.
    pub fn vim_keystroke(&mut self, keystroke: &Keystroke, ctx: &mut ViewContext<Self>) {
        self.vim_model.update(ctx, |vim_model, ctx| {
            vim_model.keypress(keystroke, ctx);
        });
        ctx.emit(Event::VimStatusUpdate)
    }

    /// Clears the VimFSA's pending command state and enters insert mode.
    fn vim_force_insert_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_model.update(ctx, |vim, ctx| {
            vim.force_insert_mode(ctx);
        });
    }

    /// Clears the VimFSA's pending command state and switches modes if necessary.
    fn vim_interrupt(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_model.update(ctx, |vim, ctx| {
            vim.interrupt(ctx);
        });
    }

    fn add_non_expanding_space(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::NonExpandingSpace,
                EditOrigin::UserTyped,
                |editor_model, ctx| {
                    editor_model.insert_internal(" ", None, SelectionInsertion::No, ctx);
                },
            ),
        );
    }

    fn enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            match self.enter_settings.enter {
                EnterAction::InsertNewLineIfMultiLine if !self.single_line => {
                    self.newline_internal(ctx)
                }
                _ => {
                    // On submitting a command for execution, we reset the Vim FSA
                    // to Insert mode. Since the contents of the buffer will be cleared,
                    // it makes more sense to put the user in a state where they can
                    // immediately be ready to enter the next command.
                    // This behavior mirrors bash and zsh's vi mode implementations.
                    self.vim_force_insert_mode(ctx);
                    ctx.emit(Event::Enter)
                }
            }
        }
    }

    fn shift_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            match self.enter_settings.shift_enter {
                EnterAction::InsertNewLineIfMultiLine if !self.single_line => {
                    self.newline_internal(ctx)
                }
                _ => ctx.emit(Event::ShiftEnter),
            }
        }
    }

    fn alt_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            match self.enter_settings.alt_enter {
                EnterAction::InsertNewLineIfMultiLine if !self.single_line => {
                    self.newline_internal(ctx)
                }
                _ => ctx.emit(Event::AltEnter),
            }
        }
    }

    fn newline_internal(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::NewLine,
                EditOrigin::UserInitiated,
                |editor_model, ctx| editor_model.insert("\n", None, ctx),
            ),
        );
    }

    fn newline(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            if self.single_line {
                // For single-line editors, inserting a newline results in us propagating the
                // shift-enter event to the parent view.
                ctx.emit(Event::ShiftEnter);
            } else {
                self.newline_internal(ctx);
            }
        }
    }

    fn ctrl_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            match self.enter_settings.ctrl_enter {
                EnterAction::InsertNewLineIfMultiLine if !self.single_line => {
                    self.newline_internal(ctx)
                }
                _ => (),
            }
            ctx.emit(Event::CtrlEnter)
        }
    }

    fn newline_after(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::InsertChar,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.insert("\n", None, ctx),
                )
                .with_change_selections(|editor_model, ctx| {
                    editor_model.cursor_line_end(/* keep_selection */ false, ctx);
                }),
        );
    }

    fn newline_before(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::InsertChar,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| editor_model.insert("\n", None, ctx),
                )
                .with_change_selections(|editor_model, ctx| {
                    editor_model.cursor_line_start(/* keep_selection */ false, ctx);
                })
                .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                    editor_model.move_cursors_by_offset(
                        1,
                        &Direction::Backward,
                        false, /* keep_selection */
                        /* stop_at_line_boundary */ false,
                        ctx,
                    );
                }),
        );
    }

    pub fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        if self.buffer_text(ctx).is_empty() {
            ctx.emit(Event::BackspaceOnEmptyBuffer);
        } else if self.single_cursor_at_buffer_start(ctx) {
            ctx.emit(Event::BackspaceAtBeginningOfBuffer);
        }

        // Only select left for empty selections
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Backspace,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    if editor_model.consecutive_autocomplete_insertion_edits_counter() > 0 {
                        editor_model.remove_before_and_after_cursor(ctx);
                    } else {
                        editor_model.backspace(ctx);
                    }
                },
            ),
        );
    }

    /// For all cursors, delete the character to the right of the cursor. For all selections, delete
    /// the characters in the selection.
    pub fn delete(&mut self, ctx: &mut ViewContext<Self>) {
        // Do not perform this edit if it would do nothing (to avoid adding to the UndoStack).
        if self.single_cursor_at_buffer_end(false /* respect_line_cap */, ctx) {
            return;
        }
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::Delete,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    if editor_model.consecutive_autocomplete_insertion_edits_counter() > 0 {
                        editor_model.remove_before_and_after_cursor(ctx);
                    } else {
                        editor_model.delete(ctx);
                    }
                },
            ),
        );
    }

    /// Get the number of characters between the current selection's endpoint and
    /// the end of the line.
    fn distance_to_line_end(&self, ctx: &mut ViewContext<Self>) -> u32 {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        let current_position = model
            .last_selection(ctx)
            .to_offset(buffer)
            .end
            .to_point(buffer)
            .expect("Last selection's end must be a valid Point");
        let line_end = buffer.line_len(current_position.row).unwrap_or_default();
        line_end.saturating_sub(current_position.column)
    }

    /// Replace characters under the cursor with the given character `c`.
    fn replace_characters(&mut self, c: char, char_count: u32, ctx: &mut ViewContext<Self>) {
        let action = PlainTextEditorViewAction::from_inserted_char(c);

        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(action, EditOrigin::UserInitiated, |editor_model, ctx| {
                    editor_model.insert(&c.to_string().repeat(char_count as usize), None, ctx);
                })
                .with_change_selections(|editor_model, ctx| {
                    editor_model.move_cursors_by_offset(
                        char_count,
                        &Direction::Forward,
                        /* keep_selection */ true,
                        /* stop_at_line_boundary */ true,
                        ctx,
                    );
                })
                .with_post_buffer_edit_change_selections(|editor_model, ctx| {
                    editor_model.move_cursors_by_offset(
                        u32::min(1, char_count),
                        &Direction::Backward,
                        /* keep_selection */ false,
                        /* stop_at_line_boundary */ true,
                        ctx,
                    );
                }),
        );
    }

    /// Toggle case for characters under and after the cursor.
    fn toggle_character_case(&mut self, char_count: u32, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::InsertChar,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| {
                        editor_model.toggle_selection_case(ctx);
                    },
                )
                .with_change_selections(|editor_model, ctx| {
                    editor_model.move_cursors_by_offset(
                        char_count,
                        &Direction::Forward,
                        /* keep_selection */ true,
                        /* stop_at_line_boundary */ true,
                        ctx,
                    );
                }),
        );
    }

    pub fn move_left(&mut self, stop_at_line_start: bool, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let map = editor_model.display_map(ctx);
            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let start = selection.start().to_display_point(map, ctx).unwrap();
                let end = selection.end().to_display_point(map, ctx).unwrap();

                if start != end {
                    selection.set_end(selection.start().clone());
                } else {
                    let cursor = map
                        .anchor_before(
                            movement::left(map, start, ctx, stop_at_line_start)
                                .expect("moving left should return a valid DisplayPoint"),
                            Bias::Left,
                            ctx,
                        )
                        .expect("DisplayPoint should convert to an Anchor");
                    selection.set_start(cursor.clone());
                    selection.set_end(cursor);
                }
                selection.set_reversed(false);
                selection.goal_start_column = None;
                selection.goal_end_column = None;
                // When we are moving left, we should keep the cursor on the same line
                // in favor of the line above.
                selection.clamp_direction = ClampDirection::Down;
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    pub fn move_to_line_start(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.cursor_line_start(false, ctx);
        });
    }

    pub fn move_to_paragraph_start(&mut self, ctx: &mut ViewContext<Self>) {
        // warp doesn't wrap the text, so basically each line is a paragraph.
        // this moves to the start of the paragraph (and the previous one if used multiple times).
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_cursor(
                false,
                |buffer, selection| {
                    let start = selection.head().to_point(buffer).unwrap();
                    if start.row > 0 && start.column == 0 {
                        return Point::new(start.row - 1, 0);
                    }
                    Point::new(start.row, 0)
                },
                ctx,
            );
        });
    }
    pub fn move_to_buffer_start(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_cursor(false, |_, _| Point::new(0, 0), ctx);
        });
    }

    pub fn select_to_line_start(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.cursor_line_start(true, ctx);
        });
    }

    pub fn cursor_home(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let map = editor_model.display_map(ctx);
            let buffer = editor_model.buffer(ctx);

            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let start = selection.start().to_display_point(map, ctx).unwrap();
                let string_start = buffer
                    .chars_at(
                        DisplayPoint::new(start.row(), 0)
                            .to_buffer_point(map, Bias::Left, ctx)
                            .unwrap(),
                    )
                    .unwrap()
                    .take_while(|c| c.is_whitespace())
                    .count();
                let cursor_start = {
                    if string_start == start.column() as usize {
                        0
                    } else {
                        string_start
                    }
                };
                let cursor = map
                    .anchor_before(
                        DisplayPoint::new(start.row(), cursor_start as u32),
                        Bias::Left,
                        ctx,
                    )
                    .unwrap();
                selection.set_selection(Selection {
                    start: cursor.clone(),
                    end: cursor,
                    reversed: false,
                });
                selection.goal_start_column = None;
                selection.goal_end_column = None;
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    pub fn cmd_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            let point = Point::new(0, 0);
            let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
            if self.single_cursor_on_first_row(ctx)
                && self.first_selection(ctx).start().to_point(buffer).unwrap() == point
            {
                ctx.emit(Event::CmdUpOnFirstRow);
            } else {
                self.cursor_top(ctx);
            }
        }
    }

    pub fn cursor_top(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let point = Point::new(0, 0);
            editor_model.reset_selections_to_point(&point, ctx);
        });
    }

    pub fn cursor_bottom(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let row = buffer.max_point().row;
            let col = buffer.line_len(row).unwrap();
            let point = Point::new(row, col);
            editor_model.reset_selections_to_point(&point, ctx);
        });
    }

    pub fn reset_selections_to_point(&mut self, point: &Point, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.reset_selections_to_point(point, ctx);
        });
    }

    pub fn move_to_line_end(&mut self, ctx: &mut ViewContext<Self>) {
        if self.single_cursor_at_autosuggestion_beginning(ctx) {
            self.insert_full_autosuggestion(ctx);
        } else {
            self.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_end(false, ctx);
            });
        }
    }

    pub fn move_to_line_end_no_autosuggestions(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.cursor_line_end(false, ctx);
        });
    }

    pub fn move_to_buffer_end(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_to_buffer_end(false /* keep_selection */, ctx);
        });
    }

    pub fn move_to_paragraph_end(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_cursor(
                false,
                |buffer, selection| {
                    let end = selection.end().to_point(buffer).unwrap();
                    let paragraph_end = buffer.line_len(end.row).unwrap();
                    let max_point = buffer.max_point();
                    // we're at the end of paragraph, but not at the end of the text
                    if end.column == paragraph_end && end.row < max_point.row {
                        return Point::new(end.row + 1, buffer.line_len(end.row + 1).unwrap());
                    }
                    Point::new(end.row, buffer.line_len(end.row).unwrap())
                },
                ctx,
            );
        });
    }

    pub fn select_to_line_end(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.cursor_line_end(true, ctx);
        });
    }

    pub fn move_and_select_to_buffer_start(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_cursor(true, |_, _| Point::new(0, 0), ctx);
        });
    }

    pub fn move_and_select_to_buffer_end(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model.move_cursor(true, |buffer, _| buffer.max_point(), ctx);
        });
    }

    fn insert_autosuggestion_from_action(&mut self, ctx: &mut ViewContext<Self>) {
        // This method is only called from an `InsertAutosuggestion` action, so if
        // this boolean is true, then this method was triggered by a Tab keypress.
        let is_insert_autosuggestion_bound_to_tab =
            keybinding_name_to_keystroke(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, ctx)
                .map(|keystroke| keystroke.key == "tab")
                .unwrap_or_default();

        // If there is an active autosuggestion, we always insert it. Otherwise,
        // if a Tab keypress triggered this handler, we fall back to the handler
        // for Tab. We need to do this so setting Tab to insert autosuggestions
        // doesn't prevent users from using Tab to navigate between editor fields.
        //
        // Ideally we would handle the InsertAutosuggestion action in the Input
        // view to avoid this, but limitations in the UI Framework prevent this
        // from being a valid solution currently.
        if self.active_autosuggestion() {
            self.insert_full_autosuggestion(ctx);
        } else if is_insert_autosuggestion_bound_to_tab {
            self.tab(ctx);
        }
    }

    /// Wrapper method around insert_autosuggestion to accept the whole suggestion.
    fn insert_full_autosuggestion(&mut self, ctx: &mut ViewContext<Self>) {
        self.insert_autosuggestion(|text| text.chars().count(), ctx);
    }

    /// Inserts autosuggestion text into the buffer.
    /// The insert_fn closure is used to determine how much of the autosuggestion should get
    /// accepted. Usually this is the whole thing, but it may be less for word-based navigation.
    /// Note the insert_full_autosuggestion wrapper method for accepting the full autosuggestion.
    fn insert_autosuggestion<F, B>(&mut self, insert_fn: F, ctx: &mut ViewContext<Self>)
    where
        B: Into<CharOffset>,
        F: Fn(&str) -> B,
    {
        if !self.can_edit(ctx) {
            return;
        }
        let Some(current_autosuggestion_state) = self.autosuggestion_state.take() else {
            return;
        };
        let Some(current_autosuggestion_text) = current_autosuggestion_state
            .current_autosuggestion_text
            .as_ref()
        else {
            return;
        };

        // Ensure the cursor is at the end of the line before inserting
        self.move_to_buffer_end(ctx);

        let split_char_offset = insert_fn(current_autosuggestion_text).into();

        let (insertion_text, remaining_autosuggestion): (String, String) =
            current_autosuggestion_text
                .chars()
                .enumerate()
                .partition_map(|(i, c)| {
                    if i < split_char_offset.as_usize() {
                        Either::Left(c)
                    } else {
                        Either::Right(c)
                    }
                });

        if !remaining_autosuggestion.is_empty() {
            let new_autosuggestion_state = AutosuggestionState {
                buffer_snapshot: current_autosuggestion_state.buffer_snapshot.clone(),
                original_autosuggestion_text: current_autosuggestion_state
                    .original_autosuggestion_text
                    .clone(),
                current_autosuggestion_text: Some(remaining_autosuggestion.to_owned()),
                location: current_autosuggestion_state.location,
                autosuggestion_type: current_autosuggestion_state.autosuggestion_type.clone(),
            };
            self.autosuggestion_state = Some(Arc::new(new_autosuggestion_state));
        }

        let buffer_char_length = self.buffer_text(ctx).chars().count();
        ctx.emit(Event::AutosuggestionAccepted {
            insertion_length: insertion_text.len(),
            buffer_char_length,
            autosuggestion_type: current_autosuggestion_state.autosuggestion_type.clone(),
        });

        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::AutoSuggestion,
                EditOrigin::UserInitiated,
                move |editor_model, ctx| {
                    editor_model.insert_internal(
                        &insertion_text,
                        None,
                        SelectionInsertion::No,
                        ctx,
                    );
                },
            ),
        );
    }

    pub fn cursor_end(&mut self, ctx: &mut ViewContext<Self>) {
        if self.single_cursor_at_autosuggestion_beginning(ctx) {
            self.insert_full_autosuggestion(ctx);
        } else {
            self.change_selections(ctx, |editor_model, ctx| {
                let map = editor_model.display_map(ctx);
                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let end = selection
                        .end()
                        .to_display_point(map, ctx)
                        .expect("Should be able to get end point of selection.");
                    let cursor = map
                        .anchor_before(
                            DisplayPoint::new(
                                end.row(),
                                map.line_len(end.row(), ctx).expect(
                                    "Should be able to get length of line at selection end.",
                                ),
                            ),
                            Bias::Right,
                            ctx,
                        )
                        .unwrap();
                    selection.set_selection(Selection::single_cursor(cursor));
                    selection.goal_start_column = None;
                    selection.goal_end_column = None;
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    pub fn select_left(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let map = editor_model.display_map(ctx);
            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let head = selection.head().to_display_point(map, ctx).unwrap();
                let cursor = map
                    .anchor_before(
                        movement::left(map, head, ctx, /* stop at line start */ false).unwrap(),
                        Bias::Left,
                        ctx,
                    )
                    .unwrap();
                selection.set_head(buffer, cursor);
                selection.goal_start_column = None;
                selection.goal_end_column = None;
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    pub fn move_right(&mut self, stop_at_line_end: bool, ctx: &mut ViewContext<Self>) {
        if self.single_cursor_at_autosuggestion_beginning(ctx) {
            self.insert_full_autosuggestion(ctx);
        } else {
            self.change_selections(ctx, |editor_model, ctx| {
                let map = editor_model.display_map(ctx);
                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let start = selection.start().to_display_point(map, ctx).unwrap();
                    let end = selection.end().to_display_point(map, ctx).unwrap();

                    if start != end {
                        selection.set_start(selection.end().clone());
                    } else {
                        let cursor = map
                            .anchor_before(
                                movement::right(map, end, ctx, stop_at_line_end)
                                    .expect("moving right should return a valid DisplayPoint"),
                                Bias::Right,
                                ctx,
                            )
                            .expect("DisplayPoint should convert to an Anchor");
                        selection.set_start(cursor.clone());
                        selection.set_end(cursor);
                    }
                    selection.set_reversed(false);
                    selection.goal_start_column = None;
                    selection.goal_end_column = None;
                    // When moving right, we should keep the cursor on the
                    // same line in favor of the line below.
                    selection.clamp_direction = ClampDirection::Up;
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    pub fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        if self.propagate_escape_key == PropagateAndNoOpEscapeKey::HandleFirst
            && self.vim_mode_enabled(ctx)
            && self.vim_has_pending(ctx)
        {
            self.vim_escape(ctx);
        } else if self.can_select(ctx) {
            if FeatureFlag::ClearAutosuggestionOnEscape.is_enabled()
                && (!self.vim_mode_enabled(ctx) || self.vim_mode(ctx) == Some(VimMode::Normal))
            {
                self.clear_autosuggestion(ctx);
            }

            if self.editor_model.as_ref(ctx).is_single_cursor_only(ctx) {
                ctx.emit(Event::Escape);
            } else {
                self.change_selections(ctx, |editor_model, ctx| {
                    editor_model.clear_selections(ctx);
                });
            }
        }
        #[cfg(feature = "voice_input")]
        self.stop_voice_input(true, ctx);
    }

    fn delete_all(&mut self, direction: CutDirection, cut: bool, ctx: &mut ViewContext<Self>) {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        let map = self.editor_model.as_ref(ctx).display_map(ctx);

        let ranges = self
            .editor_model
            .as_ref(ctx)
            .selections(ctx)
            .iter()
            .map(|selection| {
                let start = selection.start().to_display_point(map, ctx).unwrap();
                let end = selection.end().to_display_point(map, ctx).unwrap();
                match direction {
                    CutDirection::Right => {
                        start..DisplayPoint::new(end.row(), buffer.line_len(end.row()).unwrap())
                    }
                    CutDirection::Left => {
                        let mut start = DisplayPoint::new(start.row(), 0);
                        if start == end {
                            // if the line was empty, move to the previous one
                            let head = selection.head().to_char_offset(buffer).unwrap();
                            start = head
                                .saturating_sub(&1.into())
                                .to_point(buffer)
                                .expect("Head offset should exist")
                                .to_display_point(map, ctx)
                                .expect("points should exist");
                        };
                        start..end
                    }
                }
            })
            .collect::<Vec<_>>();

        if cut {
            self.edit(
                ctx,
                Edits::new()
                    .with_update_buffer(
                        PlainTextEditorViewAction::CutAll,
                        EditOrigin::UserInitiated,
                        |editor_model, ctx| {
                            editor_model.shell_cut(ctx);
                        },
                    )
                    .with_change_selections(move |editor_model, ctx| {
                        editor_model
                            .select_ranges_by_display_point(ranges, ctx)
                            .expect("points should exist");
                    }),
            );
        } else {
            let ranges = ranges
                .iter()
                .map(|range| {
                    range
                        .start
                        .to_char_offset(map, Bias::Left, buffer, ctx)
                        .unwrap()
                        .as_usize()
                        ..range
                            .end
                            .to_char_offset(map, Bias::Left, buffer, ctx)
                            .unwrap()
                            .as_usize()
                })
                .collect::<Vec<_>>();

            self.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::DeleteAll,
                    EditOrigin::UserInitiated,
                    move |editor_model, ctx| {
                        if let Err(error) = editor_model.buffer_edit(
                            merge_ranges(ranges).into_iter().map(|range| {
                                CharOffset::from(range.start)..CharOffset::from(range.end)
                            }),
                            "",
                            ctx,
                        ) {
                            log::error!("error deleting all (direction {direction:?}): {error}");
                        };
                    },
                ),
            );
        }
    }

    pub fn clear_and_copy_lines(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new()
                .with_update_buffer(
                    PlainTextEditorViewAction::ClearAndCopyLines,
                    EditOrigin::UserInitiated,
                    |editor_model, ctx| {
                        editor_model.shell_cut(ctx);
                    },
                )
                .with_change_selections(|editor_model, ctx| {
                    let map = editor_model.display_map(ctx);
                    let buffer = editor_model.buffer(ctx);

                    let ranges = editor_model
                        .selections(ctx)
                        .iter()
                        .map(|selection| {
                            let position = selection.start().to_display_point(map, ctx).unwrap();
                            let row = position.row();
                            DisplayPoint::new(row, 0)
                                ..DisplayPoint::new(row, buffer.line_len(row).unwrap())
                        })
                        .collect::<Vec<_>>();

                    editor_model
                        .select_ranges_by_display_point(ranges, ctx)
                        .expect("range should exist");
                }),
        );
    }

    pub fn clear_lines(&mut self, ctx: &mut ViewContext<Self>) {
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::ClearLines,
                EditOrigin::UserInitiated,
                |editor_model, ctx| {
                    let buffer = editor_model.buffer(ctx);
                    let row_ranges = merge_ranges(
                        editor_model
                            .selections(ctx)
                            .iter()
                            .map(|selection| {
                                let start = selection.start().to_point(buffer).unwrap().row;
                                let end = selection.end().to_point(buffer).unwrap().row;
                                (start as usize)..(end as usize + 1)
                            })
                            .collect::<Vec<_>>(),
                    );

                    if let Err(error) = editor_model.buffer_edit(
                        row_ranges
                            .iter()
                            .map(|row_range| {
                                Point::new(row_range.start as u32, 0)
                                    .to_char_offset(buffer)
                                    .unwrap()
                                    ..Point::new(
                                        row_range.end as u32 - 1,
                                        buffer.line_len(row_range.end as u32 - 1).unwrap(),
                                    )
                                    .to_char_offset(buffer)
                                    .unwrap()
                            })
                            .collect::<Vec<_>>(),
                        "",
                        ctx,
                    ) {
                        log::error!("error clearing lines: {error}");
                    }
                },
            ),
        );
    }

    fn cursor_forward_one_word(&mut self, select: bool, ctx: &mut ViewContext<Self>) {
        if self.single_cursor_at_autosuggestion_beginning(ctx) {
            self.insert_autosuggestion(move_single_word, ctx);
        } else {
            if select {
                self.vim_force_insert_mode(ctx);
            }
            self.change_selections(ctx, |editor_model, ctx| {
                let buffer = editor_model.buffer(ctx);
                let map = editor_model.display_map(ctx);

                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let end_position = if select {
                        selection.head().to_point(buffer).unwrap()
                    } else {
                        selection.end().to_point(buffer).unwrap()
                    };
                    if let Ok(mut boundaries) = buffer.word_ends_from_offset_exclusive(end_position)
                    {
                        let word_start = boundaries.next().unwrap_or(end_position);
                        let cursor = map
                            .anchor_before(
                                word_start.to_display_point(map, ctx).unwrap(),
                                Bias::Right,
                                ctx,
                            )
                            .unwrap();
                        if select {
                            selection.set_head(buffer, cursor);
                        } else {
                            selection.set_selection(Selection::single_cursor(cursor));
                        }
                        selection.goal_start_column = None;
                        selection.goal_end_column = None;
                    }
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    fn cursor_backward_one_word(&mut self, select: bool, ctx: &mut ViewContext<Self>) {
        if select {
            self.vim_force_insert_mode(ctx);
        }
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let map = editor_model.display_map(ctx);
            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let start_position = if select {
                    selection.head().to_point(buffer).unwrap()
                } else {
                    selection.start().to_point(buffer).unwrap()
                };
                if let Ok(mut word_starts) =
                    buffer.word_starts_backward_from_offset_exclusive(start_position)
                {
                    let word_start = word_starts.next().unwrap_or(start_position);
                    let cursor = map
                        .anchor_before(
                            word_start.to_display_point(map, ctx).unwrap(),
                            Bias::Left,
                            ctx,
                        )
                        .unwrap_or(Anchor::Start);
                    if select {
                        selection.set_head(buffer, cursor);
                    } else {
                        selection.set_selection(Selection::single_cursor(cursor));
                    }
                    selection.goal_start_column = None;
                    selection.goal_end_column = None;
                }
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    fn cursor_forward_one_subword(&mut self, select: bool, ctx: &mut ViewContext<Self>) {
        if select {
            self.vim_force_insert_mode(ctx);
        }
        if self.single_cursor_at_autosuggestion_beginning(ctx) {
            self.insert_autosuggestion(
                |text| {
                    SubwordBoundaries::forward_subword_ends_exclusive(
                        CharOffset::zero(),
                        text.chars(),
                        text,
                    )
                    .next()
                    // Note that we're using a plain-old str as the [`TextBuffer`] so
                    // there is only one row (even if the str contains newlines).
                    .map(|point| CharOffset::from(point.column as usize))
                    .unwrap_or(CharOffset::zero())
                },
                ctx,
            );
        } else {
            self.change_selections(ctx, |editor_model, ctx| {
                let buffer = editor_model.buffer(ctx);
                let map = editor_model.display_map(ctx);

                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let end_position = if select {
                        selection
                            .head()
                            .to_point(buffer)
                            .expect("Selection head must be convertible to a Point")
                    } else {
                        selection
                            .end()
                            .to_point(buffer)
                            .expect("Selection end must be convertible to a Point")
                    };
                    if let Ok(mut boundaries) =
                        buffer.subword_ends_from_offset_exclusive(end_position)
                    {
                        let subword_start = boundaries.next().unwrap_or(end_position);
                        let cursor = map
                            .anchor_before(
                                subword_start
                                    .to_display_point(map, ctx)
                                    .expect("Subword start must be convertible to a DisplayPoint"),
                                Bias::Right,
                                ctx,
                            )
                            .unwrap_or(Anchor::End);
                        if select {
                            selection.set_head(buffer, cursor);
                        } else {
                            selection.set_selection(Selection::single_cursor(cursor));
                        }
                        selection.goal_start_column = None;
                        selection.goal_end_column = None;
                    }
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    fn cursor_backward_one_subword(&mut self, select: bool, ctx: &mut ViewContext<Self>) {
        if select {
            self.vim_force_insert_mode(ctx);
        }
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let map = editor_model.display_map(ctx);

            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let start_position = if select {
                    selection
                        .head()
                        .to_point(buffer)
                        .expect("Selection head must be convertible to a Point")
                } else {
                    selection
                        .start()
                        .to_point(buffer)
                        .expect("Selection start must be convertible to a Point")
                };
                if let Ok(mut boundaries) =
                    buffer.subword_backward_starts_from_offset_exclusive(start_position)
                {
                    let subword_start = boundaries.next().unwrap_or(start_position);
                    let cursor = map
                        .anchor_before(
                            subword_start
                                .to_display_point(map, ctx)
                                .expect("Subword start must be convertible to a DisplayPoint"),
                            Bias::Left,
                            ctx,
                        )
                        .unwrap_or(Anchor::Start);
                    if select {
                        selection.set_head(buffer, cursor);
                    } else {
                        selection.set_selection(Selection::single_cursor(cursor));
                    }
                    selection.goal_start_column = None;
                    selection.goal_end_column = None;
                }
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    pub fn select_right(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let map = editor_model.display_map(ctx);

            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let head = selection.head().to_display_point(map, ctx).unwrap();
                let cursor = map
                    .anchor_before(
                        movement::right(map, head, ctx, false).unwrap(),
                        Bias::Right,
                        ctx,
                    )
                    .unwrap();
                selection.set_head(buffer, cursor);
                selection.goal_start_column = None;
                selection.goal_end_column = None;
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    /// Determine if an upward navigation command (i.e. `Up` or `PageUp`) should be propagated
    /// to the parent View
    fn should_propagate_upward_navigation(&self, ctx: &mut ViewContext<Self>) -> bool {
        match self.propagate_vertical_navigation_keys {
            PropagateAndNoOpNavigationKeys::Always => true,
            PropagateAndNoOpNavigationKeys::Never => false,
            PropagateAndNoOpNavigationKeys::AtBoundary => self.single_cursor_on_first_row(ctx),
        }
    }

    /// Determine if a downward navigation command (i.e. `Down` or `PageDown`) should be propagated
    /// to the parent View
    fn should_propagate_downward_navigation(&self, ctx: &mut ViewContext<Self>) -> bool {
        match self.propagate_vertical_navigation_keys {
            PropagateAndNoOpNavigationKeys::Always => true,
            PropagateAndNoOpNavigationKeys::Never => false,
            PropagateAndNoOpNavigationKeys::AtBoundary => self.single_cursor_on_last_row(ctx),
        }
    }

    /// Determine if a rightward navigation command (i.e. `Right`) should be propagated
    /// to the parent View
    fn should_propagate_rightward_navigation(&self, ctx: &mut ViewContext<Self>) -> bool {
        match self.propagate_horizontal_navigation_keys {
            PropagateHorizontalNavigationKeys::Always => true,
            PropagateHorizontalNavigationKeys::Never => false,
            PropagateHorizontalNavigationKeys::AtBoundary => {
                self.single_cursor_at_buffer_end(/* respect_line_cap */ false, ctx)
            }
        }
    }

    /// Vim keybinding-specific function to determine if an upward motion (i.e. `k`)
    /// should be propagated to the parent View.
    ///
    /// When Vim keybindings are enabled,
    /// j/k navigation should be prioritize in-editor navigation.
    /// whereas arrow keys should propagate as usual.
    fn vim_should_propagate_upward_navigation(&self, ctx: &mut ViewContext<Self>) -> bool {
        match self.propagate_vertical_navigation_keys {
            PropagateAndNoOpNavigationKeys::Never => false,
            PropagateAndNoOpNavigationKeys::Always | PropagateAndNoOpNavigationKeys::AtBoundary => {
                self.single_cursor_on_first_line(ctx)
            }
        }
    }

    /// Vim keybinding-specific function to determine if a downward motion (i.e. `j`)
    /// should be propagated to the parent View.
    ///
    /// When Vim keybindings are enabled,
    /// j/k navigation should be prioritize in-editor navigation.
    /// whereas arrow keys should propagate as usual.
    fn vim_should_propagate_downward_navigation(&self, ctx: &mut ViewContext<Self>) -> bool {
        match self.propagate_vertical_navigation_keys {
            PropagateAndNoOpNavigationKeys::Never => false,
            PropagateAndNoOpNavigationKeys::Always | PropagateAndNoOpNavigationKeys::AtBoundary => {
                self.single_cursor_on_last_line(ctx)
            }
        }
    }

    pub fn up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            if self.should_propagate_upward_navigation(ctx) {
                ctx.emit(Event::Navigate(NavigationKey::Up));
            } else {
                self.move_up(ctx);
            }
        }
    }

    pub fn down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            if self.should_propagate_downward_navigation(ctx) {
                ctx.emit(Event::Navigate(NavigationKey::Down));
            } else {
                self.move_down(ctx);
            }
        }
    }

    pub fn right(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            if self.should_propagate_rightward_navigation(ctx) {
                ctx.emit(Event::Navigate(NavigationKey::Right));
            }
            self.move_right(/* stop at line end */ false, ctx);
        }
    }

    pub fn single_cursor_to_point(&self, ctx: &AppContext) -> Option<Point> {
        let model = self.editor_model.as_ref(ctx);
        if !model.is_single_cursor_only(ctx) {
            None
        } else {
            let buffer = model.buffer(ctx);
            Some(model.first_selection(ctx).start().to_point(buffer).unwrap())
        }
    }

    /// Return the end position of the first selection, used for positioning argument suggestions in the editor input
    pub fn first_selection_end_to_point(&self, ctx: &AppContext) -> Point {
        let model = self.editor_model.as_ref(ctx);
        let buffer = model.buffer(ctx);
        model.first_selection(ctx).end().to_point(buffer).unwrap()
    }

    pub fn single_cursor_at_buffer_start(&self, ctx: &AppContext) -> bool {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        self.editor_model.as_ref(ctx).is_single_cursor_only(ctx)
            && self.first_selection(ctx).end().to_point(buffer).unwrap() == Point::new(0, 0)
    }

    /// Check if there is only one cursor and if it is at the end of the editor. The "end" may
    /// depend on if Vim mode is active and we're in normal mode, as the normal mode block cursor
    /// cannot go past the final character as the beam cursor can. We call this limitation "line
    /// capping". The `respect_line_cap` parameter being true means this Vim cursor behavior is
    /// acknowledged.
    pub fn single_cursor_at_buffer_end(&self, respect_line_cap: bool, ctx: &AppContext) -> bool {
        let single_cursor = self.editor_model.as_ref(ctx).is_single_cursor_only(ctx);
        if !single_cursor {
            return false;
        }
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        let max_point = buffer.max_point();
        let cursor = self
            .first_selection(ctx)
            .end()
            .to_point(buffer)
            .expect("There is always at least one selection");
        let at_max_row = max_point.row == cursor.row;
        let at_max_col = if self.vim_mode(ctx) == Some(VimMode::Normal)
            && respect_line_cap
            && max_point.column > 0
        {
            max_point.column - 1 == cursor.column
        } else {
            max_point.column == cursor.column
        };
        single_cursor && at_max_row && at_max_col
    }

    /// Whether the cursor appears on the first visual row, taking into account the line's
    /// current soft-wrapping.
    pub fn single_cursor_on_first_row(&self, ctx: &AppContext) -> bool {
        let map = self.editor_model.as_ref(ctx).display_map(ctx);
        if self.editor_model.as_ref(ctx).is_single_cursor_only(ctx) {
            let selection = self.first_selection(ctx);
            let display_point = selection
                .start()
                .to_display_point(map, ctx)
                .expect("Should be able to convert selection to display point");
            map.to_soft_wrap_point(display_point, selection.clamp_direction)
                .map(|soft_wrap_point| soft_wrap_point.row() == 0)
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Whether the cursor is on the first logical line, regardless of the line's
    /// current soft-wrapping.
    pub fn single_cursor_on_first_line(&self, ctx: &AppContext) -> bool {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        if self.editor_model.as_ref(ctx).is_single_cursor_only(ctx) {
            let selection = self.first_selection(ctx);
            let point = selection
                .start()
                .to_point(buffer)
                .expect("Should be able to convert selection to point");
            point.row == 0
        } else {
            false
        }
    }

    /// Whether the cursor is on the last visual row, taking into account the line's
    /// current soft-wrapping.
    pub fn single_cursor_on_last_row(&self, ctx: &AppContext) -> bool {
        let map = self.editor_model.as_ref(ctx).display_map(ctx);
        self.editor_model.as_ref(ctx).is_single_cursor_only(ctx)
            && self
                .first_selection(ctx)
                .start()
                .to_display_point(map, ctx)
                .expect("Should be able to convert selection to display point")
                .row()
                == map.max_point(ctx).row()
    }

    /// Whether the cursor is on the last logical line, regardless of the line's
    /// current soft-wrapping.
    pub fn single_cursor_on_last_line(&self, ctx: &AppContext) -> bool {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        self.editor_model.as_ref(ctx).is_single_cursor_only(ctx)
            && self
                .first_selection(ctx)
                .start()
                .to_point(buffer)
                .expect("Should be able to convert selection to point")
                .row
                == buffer.max_point().row
    }

    /// If the autosuggestion is location-less (e.g., the input view), the autosuggestion beginning is the
    /// very end of the buffer. If the autosuggestion has a location (e.g., notebooks), it's
    /// the end of the line that is the basis for the autosuggestion.
    pub fn single_cursor_at_autosuggestion_beginning(&mut self, ctx: &AppContext) -> bool {
        if let Some(location) = self.autosuggestion_location() {
            match location {
                AutosuggestionLocation::EndOfBuffer => {
                    self.single_cursor_at_buffer_end(true /* respect_line_cap */, ctx)
                }
                AutosuggestionLocation::Inline(line_ix) => {
                    if self.editor_model.as_ref(ctx).is_single_cursor_only(ctx) {
                        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
                        let line_length = buffer
                            .line_len(line_ix as u32)
                            .expect("line should contain a length");
                        let cursor = self
                            .first_selection(ctx)
                            .end()
                            .to_point(buffer)
                            .expect("selection anchor should convert to point");
                        cursor.row == line_ix as u32 && cursor.column == line_length
                    } else {
                        false
                    }
                }
            }
        } else {
            false
        }
    }

    /// Returns the line number where the single cursor exists. If there's more than one selection
    /// or the selection is more than just a cursor, returns None.
    pub fn single_cursor_line_index(&self, ctx: &AppContext) -> Option<u32> {
        let model = self.editor_model.as_ref(ctx);
        if model.selections(ctx).len() == 1 {
            let selection = model.first_selection(ctx);
            let buffer = model.buffer(ctx);
            selection.is_cursor_only(buffer).then(|| {
                let cursor = selection.end();
                cursor
                    .to_point(buffer)
                    .expect("cursor should convert to point")
                    .row
            })
        } else {
            None
        }
    }

    pub fn move_up(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.single_line {
            self.change_selections(ctx, |editor_model, ctx| {
                let map = editor_model.display_map(ctx);

                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let start = selection.start().to_display_point(map, ctx).unwrap();
                    let end = selection.end().to_display_point(map, ctx).unwrap();
                    if start != end {
                        selection.goal_start_column = None;
                        selection.goal_end_column = None;
                    }

                    match map.up(end, selection.goal_end_column, selection.clamp_direction) {
                        Ok(result) => {
                            // We can't assume the display point is always valid because in the very rare case,
                            // two editor actions could be dispatched so close together that the re-render hasn't
                            // started for the first action. This would cause the SoftWrappedState to fall behind
                            // the FoldMap which is updated immediately upon buffer change and subsequently cause
                            // the returned MovementResult to be invalid.
                            //
                            // TODO: We should fix this synchronization issue eventually.
                            let cursor = match map.anchor_before(
                                result.point_and_clamp_direction.point,
                                Bias::Left,
                                ctx,
                            ) {
                                Ok(cursor) => cursor,
                                Err(e) => {
                                    log::warn!("Couldn't convert display point to anchor: {e}");
                                    return;
                                }
                            };

                            selection.set_selection(Selection::single_cursor(cursor));
                            selection.goal_start_column = Some(result.goal_column);
                            selection.goal_end_column = Some(result.goal_column);
                            selection.clamp_direction =
                                result.point_and_clamp_direction.clamp_direction;
                        }
                        Err(err) => {
                            log::error!("Failed to call DisplayMap#up {err:?}");
                        }
                    }
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    pub fn select_up(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.single_line {
            self.vim_force_insert_mode(ctx);
            self.change_selections(ctx, |editor_model, ctx| {
                let buffer = editor_model.buffer(ctx);
                let map = editor_model.display_map(ctx);
                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let head = selection
                        .head()
                        .to_display_point(map, ctx)
                        .expect("Should be able to convert Anchor to DisplayPoint");

                    match map.up(head, selection.goal_end_column, selection.clamp_direction) {
                        Ok(result) => {
                            // We can't assume the display point is always valid because in the very rare case,
                            // two editor actions could be dispatched so close together that the re-render hasn't
                            // started for the first action. This would cause the SoftWrappedState to fall behind
                            // the FoldMap which is updated immediately upon buffer change and subsequently cause
                            // the returned MovementResult to be invalid.
                            //
                            // TODO: We should fix this synchronization issue eventually.
                            let cursor = match map.anchor_before(
                                result.point_and_clamp_direction.point,
                                Bias::Left,
                                ctx,
                            ) {
                                Ok(cursor) => cursor,
                                Err(e) => {
                                    log::warn!("Couldn't convert display point to anchor: {e}");
                                    return;
                                }
                            };

                            selection.set_head(buffer, cursor);
                            selection.goal_start_column = Some(result.goal_column);
                            selection.goal_end_column = Some(result.goal_column);
                            selection.clamp_direction =
                                result.point_and_clamp_direction.clamp_direction;
                        }
                        Err(err) => {
                            log::warn!("Failed to call DisplayMap#up {err:?}");
                        }
                    }
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    pub fn move_down(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.single_line {
            self.change_selections(ctx, |editor_model, ctx| {
                let map = editor_model.display_map(ctx);

                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let start = selection.start().to_display_point(map, ctx).unwrap();
                    let end = selection.end().to_display_point(map, ctx).unwrap();
                    if start != end {
                        selection.goal_start_column = None;
                        selection.goal_end_column = None;
                    }
                    match map.down(end, selection.goal_end_column, selection.clamp_direction) {
                        Ok(result) => {
                            // We can't assume the display point is always valid because in the very rare case,
                            // two editor actions could be dispatched so close together that the re-render hasn't
                            // started for the first action. This would cause the SoftWrappedState to fall behind
                            // the FoldMap which is updated immediately upon buffer change and subsequently cause
                            // the returned MovementResult to be invalid.
                            //
                            // TODO: We should fix this synchronization issue eventually.
                            let cursor = match map.anchor_before(
                                result.point_and_clamp_direction.point,
                                Bias::Right,
                                ctx,
                            ) {
                                Ok(cursor) => cursor,
                                Err(e) => {
                                    log::warn!("Couldn't convert display point to anchor: {e}");
                                    return;
                                }
                            };
                            selection.set_selection(Selection::single_cursor(cursor));
                            selection.goal_start_column = Some(result.goal_column);
                            selection.goal_end_column = Some(result.goal_column);
                            selection.clamp_direction =
                                result.point_and_clamp_direction.clamp_direction;
                        }
                        Err(err) => {
                            log::warn!("Failed to call DisplayMap#down {err:?}");
                        }
                    }
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    pub fn select_down(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.single_line {
            self.vim_force_insert_mode(ctx);
            self.change_selections(ctx, |editor_model, ctx| {
                let buffer = editor_model.buffer(ctx);
                let map = editor_model.display_map(ctx);

                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let head = selection
                        .head()
                        .to_display_point(map, ctx)
                        .expect("Should be able to convert Anchor to DisplayPoint");
                    match map.down(head, selection.goal_end_column, selection.clamp_direction) {
                        Ok(result) => {
                            // We can't assume the display point is always valid because in the very rare case,
                            // two editor actions could be dispatched so close together that the re-render hasn't
                            // started for the first action. This would cause the SoftWrappedState to fall behind
                            // the FoldMap which is updated immediately upon buffer change and subsequently cause
                            // the returned MovementResult to be invalid.
                            //
                            // TODO: We should fix this synchronization issue eventually.
                            let cursor = match map.anchor_before(
                                result.point_and_clamp_direction.point,
                                Bias::Right,
                                ctx,
                            ) {
                                Ok(cursor) => cursor,
                                Err(e) => {
                                    log::warn!("Couldn't convert display point to anchor: {e}");
                                    return;
                                }
                            };

                            selection.set_head(buffer, cursor);
                            selection.goal_start_column = Some(result.goal_column);
                            selection.goal_end_column = Some(result.goal_column);
                            selection.clamp_direction =
                                result.point_and_clamp_direction.clamp_direction;
                        }
                        Err(err) => log::error!("Failed to call DisplayMap#down {err:?}"),
                    }
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    pub fn select_all(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::EmacsBindingUsed);

        self.vim_force_insert_mode(ctx);
        self.change_selections(ctx, |editor_model, ctx| {
            editor_model
                .select_ranges_by_offset([CharOffset::zero()..editor_model.buffer_len(ctx)], ctx)
                .unwrap();
        });
    }

    pub fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |editor_model, ctx| {
            editor_model.undo(ctx);
        });
    }

    pub fn redo(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |editor_model, ctx| {
            editor_model.redo(ctx);
        });
    }

    fn expand_selections(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let map = editor_model.display_map(ctx);
            let mut new_selections = Vec::new();
            for selection in editor_model.selections(ctx).clone().iter_mut() {
                // Keep non-cursor-only selections the same
                if !selection.is_cursor_only(buffer) {
                    new_selections.push(selection.clone());
                    continue;
                }
                let start = selection.tail();
                let position = start
                    .to_display_point(map, ctx)
                    .unwrap()
                    .to_buffer_point(map, Bias::Left, ctx)
                    .unwrap();
                let word_end = buffer
                    .word_ends_from_offset_inclusive(position)
                    .unwrap()
                    .next();
                let word_start = buffer
                    .word_starts_backward_from_offset_inclusive(position)
                    .unwrap()
                    .next();
                if let (Some(start), Some(end)) = (word_start, word_end) {
                    new_selections.push(LocalSelection {
                        selection: Selection {
                            start: buffer.anchor_before(start).unwrap(),
                            end: buffer.anchor_before(end).unwrap(),
                            reversed: false,
                        },
                        clamp_direction: selection.clamp_direction,
                        goal_start_column: None,
                        goal_end_column: None,
                    });
                }
            }

            if let Ok(new_selections) = Vec1::try_from_vec(new_selections) {
                editor_model.change_selections(new_selections, ctx);
            }
        });
    }

    fn select_next_occurrence(
        &mut self,
        ctx: &mut ViewContext<Self>,
        selected_text: String,
        start_offset: ByteOffset,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let map = editor_model.display_map(ctx);

            let buffer_text: String = buffer.text();
            let len = selected_text.len();
            let (before, after): (Vec<_>, Vec<_>) = buffer_text
                .match_indices(selected_text.as_str())
                .partition(|s| s.0 < start_offset.as_usize());

            for (i, _) in after.iter().chain(before.iter()) {
                let Ok(start) = buffer
                    .point_for_offset(ByteOffset::from(*i))
                    .and_then(|p| p.to_display_point(map, ctx))
                else {
                    log::warn!("Failed to convert start offset to point");
                    return;
                };
                let Ok(anchor_before_start) = map.anchor_before(start, Bias::Left, ctx) else {
                    log::warn!("Failed to find anchor before start point");
                    return;
                };

                let Ok(end) = buffer
                    .point_for_offset(ByteOffset::from(*i + len))
                    .and_then(|p| p.to_display_point(map, ctx))
                else {
                    log::warn!("Failed to convert end offset to point");
                    return;
                };
                let Ok(anchor_before_end) = map.anchor_before(end, Bias::Left, ctx) else {
                    log::warn!("Failed to find anchor before end point");
                    return;
                };

                if editor_model
                    .local_selections_intersecting_range(start..end, ctx)
                    .next()
                    .is_none()
                {
                    let selection = LocalSelection {
                        selection: Selection {
                            start: anchor_before_start,
                            end: anchor_before_end,
                            reversed: false,
                        },
                        clamp_direction: Default::default(),
                        goal_start_column: None,
                        goal_end_column: None,
                    };

                    let ix = editor_model.selection_insertion_index(selection.start(), ctx);
                    let mut new_selections = editor_model.selections(ctx).clone();
                    new_selections.insert(ix, selection);
                    editor_model.change_selections(new_selections, ctx);
                    return;
                }
            }

            // If we get there, then we didn't find a next occurrence.
            log::warn!("Unable to select next occurrence");
        });
    }

    fn should_select_next_occurrence(&mut self, ctx: &ViewContext<Self>) -> bool {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        let mut last_selected_text: Option<String> = None;

        for selection in self.selections(ctx).iter() {
            if selection.is_cursor_only(buffer) {
                return false;
            }
            let start = selection.start().to_char_offset(buffer).unwrap();
            let end = selection.end().to_char_offset(buffer).unwrap();
            let selected_text: String = buffer
                .chars_at(start)
                .unwrap()
                .take(end.as_usize() - start.as_usize())
                .collect();
            if last_selected_text.is_none() {
                last_selected_text = Some(selected_text.clone());
            }
            if last_selected_text != Some(selected_text) {
                return false;
            }
        }
        last_selected_text.is_some()
    }

    pub fn add_next_occurrence(&mut self, ctx: &mut ViewContext<Self>) {
        let buffer = self.editor_model.as_ref(ctx).buffer(ctx);
        if self.should_select_next_occurrence(ctx) {
            let first_selection = self.first_selection(ctx);
            let start_offset = first_selection.start().to_byte_offset(buffer).unwrap();
            let start = first_selection.start().to_char_offset(buffer).unwrap();
            let end = first_selection.end().to_char_offset(buffer).unwrap();
            let selected_text: String = buffer
                .chars_at(start)
                .unwrap()
                .take(end.as_usize() - start.as_usize())
                .collect();

            self.select_next_occurrence(ctx, selected_text, start_offset);
        } else {
            self.expand_selections(ctx);
        }
    }

    /// Adds a cursor above/below all existing selections.
    ///
    /// For non-empty selections [s..e], the characters above/below [s..e] (if any) become
    /// a new selection [s'..e'] and the new cursor is anchored to the end of e'.
    fn add_cursor(&mut self, direction: NewCursorDirection, ctx: &mut ViewContext<Self>) {
        self.change_selections(ctx, |editor_model, ctx| {
            let map = editor_model.display_map(ctx);
            let mut new_selections = Vec::new();

            for selection in editor_model.selections(ctx).iter() {
                // Find the endpoints of selection as DPs.
                let start = selection
                    .start()
                    .to_display_point(map, ctx)
                    .expect("Should be able to get start point of selection.");
                let end = selection
                    .end()
                    .to_display_point(map, ctx)
                    .expect("Should be able to get end point of selection");

                // Determine the new start and end cursors after the vertical move.
                let start_result = match direction.move_cursor(
                    map,
                    start,
                    selection.goal_start_column,
                    selection.clamp_direction,
                ) {
                    Some(values) => values,
                    None => continue,
                };
                let end_result = match direction.move_cursor(
                    map,
                    end,
                    selection.goal_end_column,
                    selection.clamp_direction,
                ) {
                    Some(values) => values,
                    None => continue,
                };

                let new_start_cursor = map
                    .anchor_before(
                        start_result.point_and_clamp_direction.point,
                        Bias::Left,
                        ctx,
                    )
                    .expect("Should be able to get new start cursor based on new start point.");
                let new_end_cursor = map
                    .anchor_before(end_result.point_and_clamp_direction.point, Bias::Left, ctx)
                    .expect("Should be able to get new end cursor based on new end point");

                // Create a new selection at (new_start, new_end).
                new_selections.push(LocalSelection {
                    selection: Selection {
                        start: new_start_cursor,
                        end: new_end_cursor,
                        reversed: selection.reversed(),
                    },
                    clamp_direction: end_result.point_and_clamp_direction.clamp_direction,
                    goal_start_column: Some(start_result.goal_column),
                    goal_end_column: Some(end_result.goal_column),
                });
            }

            // Add all the new selections to the existing selections.
            for new_selection in new_selections {
                let ix = editor_model.selection_insertion_index(new_selection.start(), ctx);
                let mut new_selections = editor_model.selections(ctx).clone();
                new_selections.insert(ix, new_selection);
                editor_model.change_selections(new_selections, ctx);
            }
        });
    }

    pub fn page_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            if self.should_propagate_upward_navigation(ctx) {
                ctx.emit(Event::Navigate(NavigationKey::PageUp));
            } else {
                self.move_page_up(ctx);
            }
        }
    }

    pub fn page_down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            if self.should_propagate_downward_navigation(ctx) {
                ctx.emit(Event::Navigate(NavigationKey::PageDown));
            } else {
                self.move_page_down(ctx);
            }
        }
    }

    pub fn move_page_up(&mut self, ctx: &mut ViewContext<Self>) {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        let first_cursor_col: u32 = model
            .first_selection(ctx)
            .start()
            .to_point(buffer)
            .unwrap()
            .column;
        let first_row_new_col: u32 = cmp::min(buffer.line_len(0).unwrap(), first_cursor_col);

        // Single Cursor
        if model.is_single_cursor_only(ctx) {
            let cursor_row = model
                .first_selection(ctx)
                .start()
                .to_point(buffer)
                .unwrap()
                .row;

            if cursor_row == 0 {
                // Move to line start
                self.move_to_line_start(ctx)
            } else {
                // Move to col min(first_cursor_col, length of first row) on first row
                self.change_selections(ctx, |editor_model, ctx| {
                    let point = Point::new(0, first_row_new_col);
                    editor_model.reset_selections_to_point(&point, ctx);
                });
            }
            return;
        }

        // Multiple Cursors
        if model.selections(ctx).iter().all(|selection| {
            let is_cursor = selection.is_cursor_only(buffer);
            let same_col = selection.start().to_point(buffer).unwrap().column == first_cursor_col;
            is_cursor && same_col
        }) {
            // Multiple cursors, all same column
            self.change_selections(ctx, |editor_model, ctx| {
                let point = Point::new(0, first_row_new_col);
                editor_model.reset_selections_to_point(&point, ctx);
            });
        } else {
            // Multiple cursors, different columns
            self.cursor_top(ctx);
        }
    }

    pub fn move_page_down(&mut self, ctx: &mut ViewContext<Self>) {
        let model = self.model().as_ref(ctx);
        let buffer = model.buffer(ctx);
        let map = self.editor_model.as_ref(ctx).display_map(ctx);
        let last_row = map.max_point(ctx).row();
        let first_cursor_col: u32 = model
            .first_selection(ctx)
            .start()
            .to_point(buffer)
            .unwrap()
            .column;
        let last_row_new_col: u32 = cmp::min(buffer.line_len(last_row).unwrap(), first_cursor_col);

        // Single Cursor
        if model.is_single_cursor_only(ctx) {
            let cursor_row = model
                .first_selection(ctx)
                .start()
                .to_point(buffer)
                .unwrap()
                .row;

            if cursor_row == last_row {
                // Move to line end
                self.move_to_line_end(ctx)
            } else {
                // Move to col min(first_cursor_col, length of last row) on last row
                self.change_selections(ctx, |editor_model, ctx| {
                    let point = Point::new(last_row, last_row_new_col);
                    editor_model.reset_selections_to_point(&point, ctx);
                });
            }
            return;
        }

        // Multiple Cursors
        if model.selections(ctx).iter().all(|selection| {
            let is_cursor = selection.is_cursor_only(buffer);
            let same_col = selection.start().to_point(buffer).unwrap().column == first_cursor_col;
            is_cursor && same_col
        }) {
            // Multiple cursors, all same column
            self.change_selections(ctx, |editor_model, ctx| {
                let point = Point::new(last_row, last_row_new_col);
                editor_model.reset_selections_to_point(&point, ctx);
            });
        } else {
            // Multiple cursors, different column
            self.cursor_bottom(ctx);
        }
    }

    pub fn fold(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |model, ctx| {
            model.fold(ctx);
        });
        *self.autoscroll_requested.lock() = true;
    }

    pub fn fold_selected_ranges(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |model, ctx| {
            model.fold_selected_ranges(ctx);
        });
        *self.autoscroll_requested.lock() = true;
    }

    pub fn unfold(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |model, ctx| {
            model.unfold(ctx);
        });
        *self.autoscroll_requested.lock() = true;
    }

    pub fn line_len(&self, display_row: u32, ctx: &AppContext) -> Result<u32> {
        self.editor_model.as_ref(ctx).line_len(display_row, ctx)
    }

    pub fn max_point(&self, ctx: &AppContext) -> DisplayPoint {
        self.editor_model.as_ref(ctx).max_point(ctx)
    }

    /// Font size of this editor.
    pub fn font_size(&self, appearance: &Appearance) -> f32 {
        self.text_options
            .font_size_override
            .unwrap_or_else(|| appearance.monospace_font_size())
    }

    /// Font family of this editor.
    pub fn font_family(&self, appearance: &Appearance) -> FamilyId {
        if self.is_password {
            return appearance.password_font_family();
        }
        self.text_options
            .font_family_override
            .unwrap_or_else(|| appearance.monospace_font_family())
    }

    /// Font family of this editor's placeholder text.
    pub fn placeholder_font_family(&self, appearance: &Appearance) -> FamilyId {
        self.text_options
            .font_family_override
            .unwrap_or_else(|| appearance.monospace_font_family())
    }

    pub fn line_height_ratio(&self, appearance: &Appearance) -> f32 {
        if self.use_settings_line_height_ratio {
            return appearance.line_height_ratio();
        }
        DEFAULT_UI_LINE_HEIGHT_RATIO
    }

    fn font_properties(&self, appearance: &Appearance) -> Properties {
        self.text_options
            .font_properties_override
            .unwrap_or_else(|| Properties::default().weight(appearance.monospace_font_weight()))
    }

    fn text_colors(&self, appearance: &Appearance) -> TextColors {
        self.text_options
            .text_colors_override
            .clone()
            .unwrap_or_else(|| TextColors::from_appearance(appearance))
    }

    pub fn line_height(&self, font_cache: &FontCache, appearance: &Appearance) -> f32 {
        // Copy the font family ID so that it can be moved into the closure.
        let font_family = self.font_family(appearance);
        match self.baseline_position_computation_method {
            BaselinePositionComputationMethod::Grid => grid_cell_dimensions(
                font_cache,
                font_family,
                self.font_size(appearance),
                self.line_height_ratio(appearance),
            )
            .y(),
            BaselinePositionComputationMethod::Default => font_cache.line_height(
                self.font_size(appearance),
                self.line_height_ratio(appearance),
            ),
        }
    }

    pub fn em_width(&self, font_cache: &FontCache, appearance: &Appearance) -> f32 {
        font_cache.em_width(self.font_family(appearance), self.font_size(appearance))
    }

    pub fn placeholder_text_exists(&self) -> bool {
        !self.placeholder_texts.is_empty()
    }

    pub fn active_autosuggestion(&self) -> bool {
        self.autosuggestion_state
            .as_ref()
            .is_some_and(|s| s.is_active())
    }

    pub fn active_autosuggestion_type(&self) -> Option<&AutosuggestionType> {
        self.autosuggestion_state
            .as_ref()
            .map(|s| &s.autosuggestion_type)
    }

    pub fn current_autosuggestion_text(&self) -> Option<&str> {
        self.autosuggestion_state
            .as_ref()
            .and_then(|state| state.current_autosuggestion_text.as_deref())
    }

    pub fn autosuggestion_location(&self) -> Option<AutosuggestionLocation> {
        self.autosuggestion_state
            .as_ref()
            .map(|state| state.location)
    }

    fn next_blink_epoch(&mut self) -> usize {
        self.blink_epoch += 1;
        self.blink_epoch
    }

    /// Mark the cursor as visible and schedule the next cursor blink at the right interval.
    fn reset_cursor_blink_timer(&mut self, ctx: &mut ViewContext<Self>) {
        self.cursors_visible = true;
        ctx.notify();

        let epoch = self.next_blink_epoch();
        let _ = ctx.spawn(
            async move {
                Timer::after(CURSOR_BLINK_INTERVAL).await;
                epoch
            },
            Self::blink_cursors,
        );
    }

    /// Returns a snapshot of the editor.
    pub fn snapshot_model(&self, ctx: &AppContext) -> EditorSnapshot {
        self.editor_model.as_ref(ctx).as_snapshot(ctx)
    }

    fn blink_cursors(&mut self, epoch: usize, ctx: &mut ViewContext<Self>) {
        let cursor_blink = &AppEditorSettings::as_ref(ctx).cursor_blink;
        if epoch == self.blink_epoch
            && self.focused_in_active_window(ctx)
            && (
                // Allow this method to recurse on itself (and continue blinking)
                // if blinking is enabled. Otherwise, it can only recurse if
                // cursors_visible is false, then it may recurse just 1 more time
                // to make the cursor visible again and remain that way. We need
                // to allow that in case blinking gets disabled while
                // cursors_visible is false
                cursor_blink.value() == &CursorBlink::Enabled || !self.cursors_visible
            )
        {
            self.cursors_visible = !self.cursors_visible;
            ctx.notify();

            let epoch = self.next_blink_epoch();
            let _ = ctx.spawn(
                async move {
                    Timer::after(CURSOR_BLINK_INTERVAL).await;
                    epoch
                },
                Self::blink_cursors,
            );
        }
    }

    fn focused_in_active_window(&self, ctx: &AppContext) -> bool {
        let active = self
            .windowing_state_handle
            .as_ref(ctx)
            .state()
            .active_window;
        Some(self.window_id) == active && self.focused
    }

    fn should_draw_cursors(&self, ctx: &AppContext) -> bool {
        // Always draw cursors when voice input is active.
        #[cfg(feature = "voice_input")]
        if self.voice_input_state.is_active() {
            return true;
        }

        self.cursors_visible && self.focused_in_active_window(ctx) && self.can_edit(ctx)
    }

    fn handle_model_event(
        &mut self,
        _: ModelHandle<EditorModel>,
        event: &EditorModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorModelEvent::Edited { edit_origin } => {
                // Update the autosuggestion state after an edit.
                self.update_autosuggestion_state(ctx);
                self.vim_maybe_enforce_cursor_line_cap(ctx);
                self.accept_autosuggestion_keybinding_view
                    .update(ctx, |view, ctx| view.close_menu(ctx));
                ctx.emit(Event::Edited(*edit_origin))
            }
            EditorModelEvent::BufferReplaced => ctx.emit(Event::BufferReplaced),
            EditorModelEvent::SelectionsChanged => {
                self.vim_maybe_enforce_cursor_line_cap(ctx);
                ctx.emit(Event::SelectionChanged);
            }
            EditorModelEvent::ShellCut(text) => {
                self.internal_clipboard.clone_from(text);
                self.internal_clipboard.shrink_to_fit();
            }
            EditorModelEvent::UpdatePeers { operations } => {
                ctx.emit(Event::UpdatePeers {
                    operations: operations.clone(),
                });
                // This event is sent in conjunction with the main event reflecting
                // the change (e.g. `Edited`) so no need to do anything else here.
                return;
            }
            // In these cases, we just need to re-render, which is handled
            // by the invalidation below.
            EditorModelEvent::DisplayMapUpdated | EditorModelEvent::StylesUpdated => {}
        }

        // The editor model can change even when we aren't focused, so we only want to reset the
        // blink timer when we are active
        if self.focused_in_active_window(ctx) {
            self.reset_cursor_blink_timer(ctx);
        }
        *self.autoscroll_requested.lock() = true;
        ctx.notify();
    }

    /// When in Vim mode, specifically normal mode, the block cursor cannot go past the last
    /// character on the line as the beam cursor can. We call this "line capping." This helper
    /// method determines if line capping needs to be enforced, and if so, enforces it.
    ///
    /// TODO: ideally, this wouldn't be treated as a separate 'edit' from the 'edit' that was
    /// produced by the initial user-action.
    fn vim_maybe_enforce_cursor_line_cap(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(VimMode::Normal) = self.vim_mode(ctx) {
            if self.editor_model.as_ref(ctx).vim_needs_line_capping(ctx) {
                self.edit(
                    ctx,
                    Edits::new().with_change_selections(|model, ctx| {
                        model.vim_enforce_cursor_line_cap(ctx);
                    }),
                );
            }
        }
    }

    /// Returns an option around the string that represents the current state of whatever content
    /// was used to generate the autosuggestion. Either the entire buffer OR one line of it.
    fn autosuggestion_basis(
        &self,
        location: AutosuggestionLocation,
        ctx: &mut ViewContext<Self>,
    ) -> Option<String> {
        match location {
            AutosuggestionLocation::EndOfBuffer => Some(self.buffer_text(ctx)),
            AutosuggestionLocation::Inline(line_ix) => {
                let buffer = self.model().as_ref(ctx).buffer(ctx);
                buffer.line(line_ix as u32).ok()
            }
        }
    }

    /// Update autosuggestion state based on the result of an edit. First check if the buffer text
    /// when the autosuggestion was initially inserted is still in the buffer. If so, remove the
    /// characters from the end of the buffer that weren't in the buffer at the time the
    /// autosuggestion was set. If these characters are a prefix of the original autosuggestion,
    /// update the current autosuggestion text to be the original autosuggestion text minus the
    /// prefix that is already inserted in the buffer.
    fn update_autosuggestion_state(&mut self, ctx: &mut ViewContext<EditorView>) {
        if let Some(autosuggestion_state) = self.autosuggestion_state.take() {
            // Retrieve the current state of the buffer. If the autosuggestion is location-less,
            // the buffer is the entire buffer text. If the autosuggestion belongs to a line,
            // the buffer is just that line
            let autosuggestion_basis =
                self.autosuggestion_basis(autosuggestion_state.location, ctx);
            if autosuggestion_basis.is_none() {
                return;
            }
            let current_buffer_text_for_autosuggestion =
                autosuggestion_basis.expect("autosuggestion basis should exist");

            // Ensure the buffer text at the time the autosuggestion was sent is still in the
            // buffer.
            if let Some(added_buffer_text) = current_buffer_text_for_autosuggestion
                .strip_prefix(autosuggestion_state.buffer_snapshot.as_str())
            {
                // Update the current autosuggestion text if any of text added to the buffer is a
                // prefix of the autosuggestion.
                let mut new_autosuggestion_text = None;
                // For PowerShell, the prefix may be case-insensitive since PowerShell
                // values/commands are case-insensitive.
                if self.shell_family == Some(ShellFamily::PowerShell)
                    && autosuggestion_state
                        .original_autosuggestion_text
                        .to_lowercase()
                        .starts_with(&added_buffer_text.to_lowercase())
                {
                    let suffix = &autosuggestion_state.original_autosuggestion_text
                        [added_buffer_text.len()..];
                    new_autosuggestion_text = Some(suffix.to_owned());
                }
                // For other shells, do case-sensitive prefix match.
                else if let Some(suffix) = autosuggestion_state
                    .original_autosuggestion_text
                    .strip_prefix(added_buffer_text)
                {
                    new_autosuggestion_text = Some(suffix.to_owned());
                }

                if let Some(new_autosuggestion_text) = new_autosuggestion_text {
                    let new_autosuggestion_state = AutosuggestionState {
                        buffer_snapshot: autosuggestion_state.buffer_snapshot.clone(),
                        original_autosuggestion_text: autosuggestion_state
                            .original_autosuggestion_text
                            .clone(),
                        current_autosuggestion_text: Some(new_autosuggestion_text.to_owned()),
                        location: autosuggestion_state.location,
                        autosuggestion_type: autosuggestion_state.autosuggestion_type.clone(),
                    };
                    self.autosuggestion_state = Some(Arc::new(new_autosuggestion_state));
                    return;
                }
            }

            self.autosuggestion_state = Some(Arc::new(AutosuggestionState {
                buffer_snapshot: autosuggestion_state.buffer_snapshot.clone(),
                original_autosuggestion_text: autosuggestion_state
                    .original_autosuggestion_text
                    .clone(),
                current_autosuggestion_text: None,
                location: autosuggestion_state.location,
                autosuggestion_type: autosuggestion_state.autosuggestion_type.clone(),
            }))
        }
    }

    /// If vim keybindings are enabled, return the [`VimMode`]. Otherwise, return None.
    pub fn vim_mode(&self, ctx: &AppContext) -> Option<VimMode> {
        self.vim_state(ctx).map(|state| state.mode)
    }

    /// If vim keybindings are enabled, return the [`VimState`]. Otherwise, return None.
    pub fn vim_state<'a>(&self, ctx: &'a AppContext) -> Option<VimState<'a>> {
        self.vim_mode_enabled(ctx)
            .then(|| self.vim_model.as_ref(ctx).state())
    }

    /// Return whether the vim state has any pending commands or mode changes.
    fn vim_has_pending(&self, ctx: &AppContext) -> bool {
        self.vim_state(ctx).is_some_and(|vim_state| {
            (vim_state.mode != VimMode::Normal) || (!vim_state.showcmd.is_empty())
        })
    }

    /// Send the 'escape' keystroke to the VimFSA.
    fn vim_escape(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_keystroke(&Keystroke::parse("escape").expect("escape parses"), ctx)
    }

    fn vim_apply_insert_position(
        &mut self,
        position: &InsertPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        match position {
            InsertPosition::AtCursor => {}
            InsertPosition::AfterCursor => {
                self.change_selections(ctx, |editor_model, ctx| {
                    editor_model.move_cursors_by_offset(
                        1,
                        &Direction::Forward,
                        /* keep_selection */ false,
                        /* stop_at_line_boundary */ true,
                        ctx,
                    );
                });
            }
            InsertPosition::LineFirstNonWhitespace => {
                self.change_selections(ctx, |editor_model, ctx| {
                    editor_model.cursor_line_start_non_whitespace(false, ctx);
                });
            }
            InsertPosition::LineEnd => self.move_to_line_end_no_autosuggestions(ctx),
            InsertPosition::LineAbove => self.newline_before(ctx),
            InsertPosition::LineBelow => self.newline_after(ctx),
        }
    }

    fn vim_set_visual_tail(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_model.update(ctx, |editor_model, ctx| {
            editor_model.vim_set_visual_tail_to_selection_heads(ctx);
        });
    }

    fn vim_cursor_forward_word(
        &mut self,
        bound: WordBound,
        word_type: WordType,
        word_count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.single_cursor_at_autosuggestion_beginning(ctx) {
            self.insert_autosuggestion(
                |text| {
                    let Ok(iter) = vim_word_iterator_from_offset(
                        0,
                        text,
                        Direction::Forward,
                        // NOTE: We use `WordBound::End` here instead of the `bound` parameter.
                        // This converts a `w` motion to a `e` with the exact same reasoning that
                        // Vim converts `cw` to `ce`, see docs:
                        // https://vimhelp.org/motion.txt.html#WORD:~:text=before%20the%20fold.-,Special%20case,-%3A%20%22cw%22%20and
                        WordBound::End,
                        word_type,
                    ) else {
                        return CharOffset::zero();
                    };
                    iter.take(word_count as usize)
                        .last()
                        .map(|offset| {
                            // We have to add 1 to the offset because the char the block cursor is
                            // on should be included. The cursor line-capping will take care of
                            // moving the cursor back 1 after the autosuggestion is partially
                            // accepted.
                            offset + 1
                        })
                        .unwrap_or(CharOffset::zero())
                },
                ctx,
            );
        } else {
            self.change_selections(ctx, |editor_model, ctx| {
                let buffer = editor_model.buffer(ctx);

                let mut new_selections = editor_model.selections(ctx).clone();
                for selection in new_selections.iter_mut() {
                    let Ok(end_offset) = selection.end().to_char_offset(buffer) else {
                        continue;
                    };

                    let Ok(boundaries) = vim_word_iterator_from_offset(
                        end_offset,
                        buffer,
                        Direction::Forward,
                        bound,
                        word_type,
                    ) else {
                        continue;
                    };

                    let cursor = buffer
                        .anchor_at(
                            boundaries
                                .take(word_count as usize)
                                .last()
                                .unwrap_or(end_offset),
                            AnchorBias::Right,
                        )
                        .unwrap_or_else(|_| selection.end().clone());

                    selection.set_selection(Selection::single_cursor(cursor));
                    selection.goal_start_column = None;
                    selection.goal_end_column = None;
                }
                editor_model.change_selections(new_selections, ctx);
            });
        }
    }

    fn vim_cursor_backward_word(
        &mut self,
        bound: WordBound,
        word_type: WordType,
        word_count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_selections(ctx, |editor_model, ctx| {
            let buffer = editor_model.buffer(ctx);
            let mut new_selections = editor_model.selections(ctx).clone();
            for selection in new_selections.iter_mut() {
                let Ok(end_offset) = selection.end().to_char_offset(buffer) else {
                    continue;
                };

                let Ok(boundaries) = vim_word_iterator_from_offset(
                    end_offset,
                    buffer,
                    Direction::Backward,
                    bound,
                    word_type,
                ) else {
                    continue;
                };

                let cursor = buffer
                    .anchor_at(
                        boundaries
                            .take(word_count as usize)
                            .last()
                            .unwrap_or(end_offset),
                        AnchorBias::Left,
                    )
                    .unwrap_or_else(|_| selection.end().clone());

                selection.set_selection(Selection::single_cursor(cursor));
                selection.goal_start_column = None;
                selection.goal_end_column = None;
            }
            editor_model.change_selections(new_selections, ctx);
        });
    }

    fn first_selection<'a, C: ModelAsRef>(&'a self, ctx: &'a C) -> &'a LocalSelection {
        self.editor_model.as_ref(ctx).first_selection(ctx)
    }

    fn selections<'a, C: ModelAsRef>(&'a self, ctx: &'a C) -> &'a Vec1<LocalSelection> {
        self.editor_model.as_ref(ctx).selections(ctx)
    }

    pub fn num_selections<C: ModelAsRef>(&self, ctx: &C) -> usize {
        self.selections(ctx).len()
    }

    pub fn any_selections_span_entire_buffer<C: ModelAsRef>(&self, ctx: &C) -> bool {
        self.editor_model
            .as_ref(ctx)
            .any_selections_span_entire_buffer(ctx)
    }

    /// For every selection, we return an iterator of the chars preceding that selection.
    pub fn chars_preceding_selections<'a, C: ModelAsRef>(
        &'a self,
        ctx: &'a C,
    ) -> impl Iterator<Item = Chars<'a>> + 'a {
        self.editor_model
            .as_ref(ctx)
            .chars_preceding_selections(ctx)
    }

    pub fn is_single_cursor_only<C: ModelAsRef>(&self, ctx: &C) -> bool {
        self.editor_model.as_ref(ctx).is_single_cursor_only(ctx)
    }

    /// Returns drawable selections data for local and remote peers.
    pub fn all_drawable_selections_data(
        &self,
        font_size: f32,
        avatar_size: f32,
        ctx: &AppContext,
    ) -> (
        LocalDrawableSelectionData,
        HashMap<ReplicaId, RemoteDrawableSelectionData>,
    ) {
        let local_selection_data = LocalDrawableSelectionData {
            colors: (self.get_cursor_colors_fn)(ctx),
            should_draw_cursors: self.should_draw_cursors(ctx),
        };

        // Convert a remote peer's selection data into an avatar component
        let appearance = Appearance::as_ref(ctx);
        let avatar_styles = UiComponentStyles {
            width: Some(avatar_size),
            height: Some(avatar_size),
            border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
            border_width: Some(1.),
            font_color: Some(ColorU::black()),
            font_family_id: Some(appearance.ui_font_family()),
            font_weight: Some(Weight::Bold),
            font_size: Some(font_size),
            ..Default::default()
        };

        let remote_selections_data = self
            .editor_model
            .as_ref(ctx)
            .registered_peers(ctx)
            .iter()
            .map(|(replica_id, peer)| {
                let color = peer.selection_data.colors.cursor;
                let avatar = Avatar::new(
                    peer.selection_data
                        .image_url
                        .clone()
                        .map(|url| AvatarContent::Image {
                            url,
                            display_name: peer.selection_data.display_name.clone(),
                        })
                        .unwrap_or(AvatarContent::DisplayName(
                            peer.selection_data.display_name.clone(),
                        )),
                    UiComponentStyles {
                        border_color: Some(color.into()),
                        background: Some(color.into()),
                        ..avatar_styles
                    },
                );

                let drawable_selections_data = RemoteDrawableSelectionData {
                    colors: peer.selection_data.colors,
                    should_draw_cursors: peer.selection_data.should_draw_cursors,
                    avatar,
                };

                (replica_id.clone(), drawable_selections_data)
            })
            .collect::<HashMap<_, _>>();

        (local_selection_data, remote_selections_data)
    }

    fn drag_and_drop_files(&mut self, paths: &[UserInput<String>], ctx: &mut ViewContext<Self>) {
        // Image paths are forwarded to the parent unchanged so the host can still read them
        // from the filesystem (the path transformer, if any, only applies to text insertion).
        let paths_as_strings: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
        let image_filepaths =
            warpui::clipboard_utils::get_image_filepaths_from_paths(&paths_as_strings);

        // If we have image file paths, emit event for parent to handle terminal-specific processing
        let num_image_files = image_filepaths.len();
        if num_image_files > 0 {
            ctx.emit(Event::DroppedImageFiles(image_filepaths));

            // If dropped only image file paths, we are done
            if num_image_files == paths.len() {
                return; // Return early, don't insert file paths as text
            }
        }

        let transformed_paths: Vec<String> = match &self.drag_drop_path_transformer {
            Some(transformer) => paths_as_strings.iter().map(|p| transformer(p)).collect(),
            None => paths_as_strings,
        };

        let input =
            warpui::clipboard_utils::escaped_paths_str(&transformed_paths, self.shell_family);

        self.user_insert(&input, ctx);
    }

    fn render_menu_button_tooltip(
        &self,
        tooltip_text: String,
        appearance: &Appearance,
    ) -> Box<dyn FnOnce() -> Box<dyn Element>> {
        let tooltip_background = appearance.theme().surface_1().into_solid();
        let tooltip_text_color = appearance
            .theme()
            .main_text_color(tooltip_background.into())
            .into_solid();
        let ui_builder = appearance.ui_builder().clone();

        Box::new(move || {
            let tool_tip_style = UiComponentStyles {
                background: Some(elements::Fill::Solid(tooltip_background)),
                font_color: Some(tooltip_text_color),
                ..Default::default()
            };

            ui_builder
                .tool_tip(tooltip_text)
                .with_style(tool_tip_style)
                .build()
                .finish()
        })
    }

    fn render_image_context_button(
        &self,
        disabled: bool,
        tooltip_text: String,
        icon_size: f32,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let button = icon_button(
            appearance,
            icons::Icon::Image,
            false,
            self.image_context_button_mouse_handle.clone(),
        )
        .with_tooltip_position(ButtonTooltipPosition::Above)
        .with_tooltip(self.render_menu_button_tooltip(tooltip_text, appearance))
        .with_style(UiComponentStyles {
            width: Some(icon_size),
            height: Some(icon_size),
            padding: Some(Coords::uniform(icon_size / 10.)),
            ..Default::default()
        });

        let button = if disabled {
            button
                .with_style(UiComponentStyles {
                    font_color: Some(
                        appearance
                            .theme()
                            .disabled_text_color(appearance.theme().background())
                            .into(),
                    ),
                    ..Default::default()
                })
                .with_hovered_styles(UiComponentStyles {
                    background: None,
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::Arrow)
        } else {
            button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(EditorAction::AttachFiles);
                })
                .with_cursor(Cursor::PointingHand)
        };

        button.finish()
    }

    pub fn render_ai_context_menu(&self) -> Option<Box<dyn Element>> {
        if let Some(ai_context_menu_state) = &self.ai_context_menu_state {
            Some(ChildView::new(&ai_context_menu_state.ai_context_menu).finish())
        } else {
            None
        }
    }

    pub fn ai_context_menu(&self) -> Option<&ViewHandle<AIContextMenu>> {
        self.ai_context_menu_state
            .as_ref()
            .map(|state| &state.ai_context_menu)
    }

    fn render_at_context_menu_button(
        &self,
        icon_size: f32,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let Some(ai_context_menu_state) = &self.ai_context_menu_state else {
            return None;
        };

        let button = icon_button(
            appearance,
            icons::Icon::AtSign,
            false,
            ai_context_menu_state
                .at_context_menu_button_mouse_handle
                .clone(),
        )
        .with_style(UiComponentStyles {
            width: Some(icon_size),
            height: Some(icon_size),
            padding: Some(Coords::uniform(icon_size / 10.)),
            ..Default::default()
        });
        let button =
            button
                .with_tooltip_position(ButtonTooltipPosition::Above)
                .with_tooltip(self.render_menu_button_tooltip(
                    "Search files and directories".to_string(),
                    appearance,
                ))
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(EditorAction::SetAIContextMenuOpen(true));
                })
                .finish();

        Some(button)
    }

    /// Commits the currently composed text from the IME (if there is any) to properly handle one of the following:
    /// - a new selection
    /// - clicking outside of the editor
    fn maybe_commit_incomplete_ime_text(&mut self, ctx: &mut ViewContext<Self>) {
        let marked_text_state = self
            .editor_model
            .read(ctx, |editor_model, ctx| editor_model.marked_text_state(ctx));
        if matches!(marked_text_state, MarkedTextState::Inactive) {
            return;
        }
        self.edit(
            ctx,
            Edits::new().with_update_buffer(
                PlainTextEditorViewAction::UpdateMarkedText,
                EditOrigin::UserTyped,
                |editor_model, ctx| {
                    editor_model.commit_incomplete_marked_text(ctx);
                },
            ),
        );
        ctx.notify();
    }

    fn set_marked_text(
        &mut self,
        marked_text: &str,
        selected_range: &Range<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::ImeMarkedText.is_enabled() {
            return;
        }

        // If in Normal or Visual mode, we don't want to insert any text.
        if matches!(
            self.vim_mode(ctx),
            Some(VimMode::Normal) | Some(VimMode::Visual(_))
        ) {
            return;
        }

        let cursor_colors = (self.get_cursor_colors_fn)(ctx);
        self.edit(
            ctx,
            Edits::new().with_update_buffer_options(
                PlainTextEditorViewAction::UpdateMarkedText,
                EditOrigin::UserTyped,
                // We don't want marked text updates to be sent to be a CRDT operation.
                UpdateBufferOption::IsEphemeral,
                |editor_model, ctx| {
                    editor_model.update_marked_text(
                        marked_text,
                        Some(TextStyle::new().with_underline_color(cursor_colors.cursor.into())),
                        selected_range,
                        ctx,
                    );
                },
            ),
        );
        ctx.notify();
    }

    fn clear_marked_text(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::ImeMarkedText.is_enabled() {
            return;
        }

        self.editor_model.update(ctx, |editor_model, ctx| {
            editor_model.clear_marked_text(ctx);
        });
    }

    /// If the editor should show any controls, render them.
    /// Otherwise, return the child element.
    fn render_controls(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "voice_input")] {
                let should_show_voice = self.voice_transcription_options.should_show_button();
            } else {
                let should_show_voice = false;
            }
        }
        let input_settings = InputSettings::as_ref(ctx);
        let is_universal_input_enabled = input_settings.is_universal_developer_input_enabled(ctx);
        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let should_show_image = !FeatureFlag::AgentView.is_enabled()
            && self.image_context_options.should_show_button()
            && !is_universal_input_enabled;
        let should_show_at_context_menu = !FeatureFlag::AgentView.is_enabled()
            && !is_universal_input_enabled
            && is_any_ai_enabled
            && {
                if !self.is_ai_input {
                    // In terminal mode, check the setting
                    if !*InputSettings::as_ref(ctx).at_context_menu_in_terminal_mode {
                        false
                    } else {
                        self.ai_context_menu_state
                            .as_ref()
                            .map(|state| state.ai_context_menu.as_ref(ctx).should_render(ctx))
                            .unwrap_or(false)
                    }
                } else {
                    // In AI mode, always allow if available
                    self.ai_context_menu_state
                        .as_ref()
                        .map(|state| state.ai_context_menu.as_ref(ctx).should_render(ctx))
                        .unwrap_or(false)
                }
            };

        if !should_show_voice && !should_show_image && !should_show_at_context_menu {
            return None;
        }

        let appearance = Appearance::as_ref(ctx);
        let font_cache = ctx.font_cache();
        let icon_size = self.line_height(font_cache, appearance);

        let mut controls = Flex::row().with_main_axis_size(MainAxisSize::Min);

        if should_show_at_context_menu {
            let at_context_menu_button = self.render_at_context_menu_button(icon_size, appearance);
            if let Some(at_context_menu_button) = at_context_menu_button {
                controls.add_child(
                    Container::new(at_context_menu_button)
                        .with_margin_left(4.)
                        .finish(),
                );
            }
        }

        if should_show_image {
            controls.add_child(
                Container::new(self.render_image_context_button(
                    !self.image_context_options.is_enabled(),
                    self.image_context_options.tooltip_text(),
                    icon_size,
                    appearance,
                ))
                .with_margin_left(4.)
                .finish(),
            );
        }

        #[cfg(feature = "voice_input")]
        if should_show_voice {
            controls.add_child(
                Container::new(self.render_voice_transcription_button(icon_size, appearance, ctx))
                    .with_margin_left(4.)
                    .finish(),
            );

            if self.should_show_voice_new_feature_popup(ctx) {
                controls.add_child(
                    Container::new(ChildView::new(&self.voice_new_feature_popup).finish())
                        .with_margin_left(4.)
                        .finish(),
                );
            }
        }

        Some(controls.finish())
    }
}

/// Try to convert display point to an anchor. If it is not possible, clamp to anchoring at end of buffer.
fn display_point_to_anchor_clamped(
    editor_model: &EditorModel,
    position: DisplayPoint,
    bias: Bias,
    ctx: &AppContext,
) -> Anchor {
    match editor_model
        .display_map(ctx)
        .anchor_before(position, bias, ctx)
    {
        Ok(anchor) => anchor,
        Err(e) => {
            log::warn!("Should be able to get anchor {e}");
            Anchor::End
        }
    }
}

fn move_single_word(text: &str) -> CharOffset {
    text.word_ends_from_offset_exclusive(CharOffset::zero())
        .ok()
        .and_then(|mut iter| iter.next())
        // Note that we're using a plain-old str as the [`TextBuffer`] so
        // there is only one row (even if the str contains newlines).
        .map(|point| CharOffset::from(point.column as usize))
        .unwrap_or(CharOffset::zero())
}

/// The anchor for showing command x-ray information
#[derive(Debug)]
pub enum CommandXRayAnchor {
    /// Show x-ray info based on the cursor positon
    Cursor,

    /// Show x-ray info based on a hover at the given display point
    Hover(DisplayPoint),
}

#[derive(Debug)]
pub enum Event {
    Activate,
    Edited(EditOrigin),
    Blurred,
    Focused,
    SelectionChanged,
    AutosuggestionAccepted {
        insertion_length: usize,
        buffer_char_length: usize,
        autosuggestion_type: AutosuggestionType,
    },
    Navigate(NavigationKey),
    Enter,
    ShiftEnter,
    AltEnter,
    CtrlEnter,
    /// Attempt to remove text when there is a single cursor placed at the beginning of the editor
    /// buffer.
    BackspaceAtBeginningOfBuffer,
    BackspaceOnEmptyBuffer,
    CtrlC {
        cleared_buffer_len: usize,
    },
    BufferReplaced,
    BufferReinitialized,
    CmdUpOnFirstRow,
    Copy,
    Escape,
    UnhandledModifierKeyOnEditor(Arc<String>),
    ClearParentSelections,
    CmdEnter,
    /// Requests to try to show the command x-ray overlay when the mouse
    /// is at the given display point.  The x-ray will only actually be shown
    /// if the underlying command is parseable and there is a token under the
    /// display point that we have a description for.
    TryToShowXRay(CommandXRayAnchor),
    HideXRay,
    InsertLastWordPrevCommand,
    /// EditorView-initiated search experience, e.g. triggered by some Vim keybindings. The way the
    /// parent view handles this will depend on the parent view.
    Search {
        /// For parent views that cycle through results, analogous to how Vim does it, indicate the
        /// direction in which to cycle.
        direction: Direction,
        /// Sometimes there is no initial search time, as in "/" or "?", and sometimes there is, as
        /// in "*" or "#".
        term: Option<String>,
    },
    /// Open a menu (command-line mode) to accept an ex-command. See ":help :" in Vim.
    ExCommand,
    /// Notify subscribers that the VimFSA state may have changed. The EditorView itself doesn't
    /// display status info for Vim. Parent views may show it instead and they need to get
    /// notified.
    VimStatusUpdate,
    /// Signifies that something has been pasted into the view
    Paste,
    MiddleClickPaste,
    /// Emitted when the 'delete all left' keybinding is triggered (cmd-delete on mac, ctrl-y on
    /// linux).
    DeleteAllLeft,
    /// Notify the user that they're using a MacOS-style binding that conflicts with a non-MacOS-style binding.
    EmacsBindingUsed,
    UpdatePeers {
        operations: Rc<Vec<CrdtOperation>>,
    },
    SetAIContextMenuOpen(bool),
    AcceptAIContextMenuItem(AIContextMenuSearchableAction),
    SelectAIContextMenuCategory(AIContextMenuCategory),
    ProcessingAttachedImages(bool),
    VoiceStateUpdated {
        is_listening: bool,
        is_transcribing: bool,
    },
    /// Request parent to process image file paths from drag-and-drop
    DroppedImageFiles(Vec<String>),
    IgnoreAutosuggestion {
        suggestion: String,
    },
}

impl Entity for EditorView {
    type Event = Event;
}

impl TypedActionView for EditorView {
    type Action = EditorAction;

    fn action_accessibility_contents(
        &mut self,
        action: &EditorAction,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        match action {
            EditorAction::UserInsert(text) => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help(text.to_string(), WarpA11yRole::UserAction),
            ),
            EditorAction::SelectLeft
            | EditorAction::SelectToLineEnd
            | EditorAction::SelectLine(_)
            | EditorAction::SelectLeftByWord
            | EditorAction::SelectLeftBySubword
            | EditorAction::SelectToLineStart
            | EditorAction::Select(_)
            | EditorAction::SelectUp
            | EditorAction::SelectAll
            | EditorAction::SelectDown
            | EditorAction::SelectWord(_)
            | EditorAction::SelectRight
            | EditorAction::SelectRightByWord
            | EditorAction::SelectRightBySubword
            | EditorAction::MoveToLineStart
            | EditorAction::MoveToParagraphStart
            | EditorAction::MoveToBufferStart
            | EditorAction::MoveToLineEnd
            | EditorAction::MoveToParagraphEnd
            | EditorAction::MoveToBufferEnd
            | EditorAction::Left
            | EditorAction::Right
            | EditorAction::Home
            | EditorAction::End
            | EditorAction::MoveForwardOneWord
            | EditorAction::MoveBackwardOneWord
            | EditorAction::MoveForwardOneSubword
            | EditorAction::MoveBackwardOneSubword
            | EditorAction::Delete
            | EditorAction::Backspace => ActionAccessibilityContent::Empty,
            EditorAction::Paste => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    format!("Pasting: {}", self.clipboard_content(ctx)),
                    WarpA11yRole::UserAction,
                ))
            }
            _ => ActionAccessibilityContent::from_debug(),
        }
    }

    fn handle_action(&mut self, action: &EditorAction, ctx: &mut ViewContext<Self>) {
        use EditorAction::*;
        match action {
            Scroll(position) => self.scroll(*position, ctx),
            Select(action) => self.select(action, ctx),
            UserInsert(text) => self.user_insert(text.as_ref(), ctx),
            #[cfg(feature = "voice_input")]
            ToggleVoiceInput(source) => {
                self.toggle_voice_input(source, ctx);
            }
            AttachFiles => self.attach_files(ctx),
            ReadAndProcessImagesAsync {
                num_images_user_attached,
                file_paths,
            } => self.read_and_process_images_async(
                *num_images_user_attached,
                file_paths.clone(),
                ctx,
            ),
            ProcessNonImageFiles { file_paths } => {
                self.process_non_image_files(file_paths.clone(), ctx);
            }
            Tab => self.tab(ctx),
            ShiftTab => self.shift_tab(ctx),
            Copy => self.copy(ctx),
            Cut => self.cut(ctx),
            Paste => self.paste(ctx),
            MiddleClickPaste => self.middle_click_paste(ctx),
            Yank => self.yank(ctx),
            Newline => self.newline(ctx),
            ShiftEnter => self.shift_enter(ctx),
            CtrlEnter => self.ctrl_enter(ctx),
            AltEnter => self.alt_enter(ctx),
            Enter => self.enter(ctx),
            CutWordLeft => self.cut_word_left(ctx),
            CutWordRight => self.cut_word_right(ctx),
            DeleteWordLeft => self.delete_word_left(ctx),
            DeleteWordRight => self.delete_word_right(ctx),
            CutAllLeft => self.delete_all(CutDirection::Left, true /* cut */, ctx),
            CutAllRight => self.delete_all(CutDirection::Right, true /* cut */, ctx),
            DeleteAllLeft => {
                self.delete_all(CutDirection::Left, false /* cut */, ctx);
                ctx.emit(Event::DeleteAllLeft);
            }
            DeleteAllRight => self.delete_all(CutDirection::Right, false /* cut */, ctx),
            Delete => self.delete(ctx),
            Backspace => self.backspace(ctx),
            Escape => self.escape(ctx),
            Up => self.up(ctx),
            Down => self.down(ctx),
            Left => self.move_left(/* stop at line start */ false, ctx),
            Right => self.right(ctx),
            Home => self.cursor_home(ctx),
            End => self.cursor_end(ctx),
            PageUp => self.page_up(ctx),
            PageDown => self.page_down(ctx),
            CmdUp => self.cmd_up(ctx),
            CmdDown => self.cursor_bottom(ctx),
            ClearLines => self.clear_lines(ctx),
            ClearAndCopyLines => self.clear_and_copy_lines(ctx),
            CtrlC => self.handle_ctrl_c(ctx),
            MoveToLineStart => self.move_to_line_start(ctx),
            MoveToParagraphStart => self.move_to_paragraph_start(ctx),
            MoveToBufferStart => self.move_to_buffer_start(ctx),
            SelectToLineStart => self.select_to_line_start(ctx),
            MoveToLineEnd => self.move_to_line_end(ctx),
            MoveToParagraphEnd => self.move_to_paragraph_end(ctx),
            MoveToBufferEnd => self.move_to_buffer_end(ctx),
            SelectToLineEnd => self.select_to_line_end(ctx),
            MoveToAndSelectBufferStart => self.move_and_select_to_buffer_start(ctx),
            MoveToAndSelectBufferEnd => self.move_and_select_to_buffer_end(ctx),
            MoveForwardOneWord => self.cursor_forward_one_word(false /* select */, ctx),
            MoveBackwardOneWord => self.cursor_backward_one_word(false /* select */, ctx),
            MoveForwardOneSubword => self.cursor_forward_one_subword(false /* select */, ctx),
            MoveBackwardOneSubword => {
                self.cursor_backward_one_subword(false /* select */, ctx)
            }
            SelectUp => self.select_up(ctx),
            SelectDown => self.select_down(ctx),
            SelectLeft => self.select_left(ctx),
            SelectRight => self.select_right(ctx),
            SelectWord(position) => self.select_word(position, ctx),
            SelectLine(position) => self.select_line(position, ctx),
            SelectAll => self.select_all(ctx),
            SelectRightByWord => self.cursor_forward_one_word(true /* select */, ctx),
            SelectLeftByWord => self.cursor_backward_one_word(true /* select */, ctx),
            SelectRightBySubword => self.cursor_forward_one_subword(true /* select */, ctx),
            SelectLeftBySubword => self.cursor_backward_one_subword(true /* select */, ctx),
            AddNextOccurrence => self.add_next_occurrence(ctx),
            Fold => self.fold(ctx),
            Unfold => self.unfold(ctx),
            FoldSelectedRanges => self.fold_selected_ranges(ctx),
            Undo => self.undo(ctx),
            Redo => self.redo(ctx),
            Focus => {
                ctx.emit(Event::Focused);
                ctx.focus_self();
            }
            UnhandledModifierKey(keystroke) => {
                if self.can_select(ctx) {
                    // This event helps us to keep track of what key bindings users
                    // try to use in the editor but are currently not available in Warp.
                    ctx.emit(Event::UnhandledModifierKeyOnEditor(keystroke.clone()))
                }
            }
            ClearParentSelections => {
                self.maybe_commit_incomplete_ime_text(ctx);
                if self.can_select(ctx) {
                    ctx.emit(Event::ClearParentSelections)
                }
            }
            CmdEnter => {
                if self.can_edit(ctx) {
                    ctx.emit(Event::CmdEnter)
                }
            }
            InspectCommand => {
                if self.command_x_ray_state.is_some() {
                    ctx.emit(Event::HideXRay);
                } else {
                    ctx.emit(Event::TryToShowXRay(CommandXRayAnchor::Cursor));
                }
            }
            TryToShowXRay(position) => {
                ctx.emit(Event::TryToShowXRay(CommandXRayAnchor::Hover(*position)))
            }
            HideXRay => ctx.emit(Event::HideXRay),
            AddCursorAbove => self.add_cursor(NewCursorDirection::Up, ctx),
            AddCursorBelow => self.add_cursor(NewCursorDirection::Down, ctx),
            InsertLastWordPrevCommand => {
                if self.can_edit(ctx) {
                    ctx.emit(Event::InsertLastWordPrevCommand)
                }
            }
            InsertNonExpandingSpace => self.add_non_expanding_space(ctx),
            ShowCharacterPalette => {
                if self.can_edit(ctx) {
                    ctx.open_character_palette();
                }
            }
            InsertAutosuggestion => self.insert_autosuggestion_from_action(ctx),
            VimUserInsert(text) => self.vim_user_insert(text.as_ref(), ctx),
            VimBackspace => self.vim_keystroke(
                &Keystroke::parse("backspace").expect("backspace parses"),
                ctx,
            ),
            VimEscape => self.vim_escape(ctx),
            EmacsBinding => ctx.emit(Event::EmacsBindingUsed),
            DragAndDropFiles(paths) => {
                self.drag_and_drop_files(paths, ctx);
            }
            SetAIContextMenuOpen(open) => {
                if !self.is_ai_input && *open {
                    // In terminal mode, check the setting before opening
                    let input_settings = InputSettings::as_ref(ctx);
                    if *input_settings.at_context_menu_in_terminal_mode {
                        ctx.emit(Event::SetAIContextMenuOpen(*open));
                    }
                    // If setting is false, don't emit the event to open the menu
                } else {
                    // In AI mode or when closing, always allow
                    ctx.emit(Event::SetAIContextMenuOpen(*open));
                }
            }
            ImeCommit(text) => self.ime_commit(text, ctx),
            SetMarkedText {
                marked_text,
                selected_range,
            } => self.set_marked_text(marked_text, selected_range, ctx),
            ClearMarkedText => self.clear_marked_text(ctx),
        }

        if self.is_focused() {
            if action.should_report_active_cursor_position_updated() {
                ctx.report_active_cursor_position_update();
            }
            if action.is_new_selection() {
                SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
                    selection_settings.maybe_write_to_linux_selection_clipboard(
                        |ctx| ClipboardContent::plain_text(self.selected_text(ctx)),
                        ctx,
                    );
                });
            }
        }
    }
}

impl View for EditorView {
    fn ui_name() -> &'static str {
        "EditorView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let soft_wrap_state = self.model().as_ref(ctx).display_map(ctx).soft_wrap_state();

        let appearance = Appearance::as_ref(ctx);
        let text_colors = self.text_colors(appearance);

        let editor_decorator_elements = self
            .render_decorator_elements
            .as_ref()
            .map(|f| f(ctx))
            .unwrap_or_default();

        let view_snapshot = self.snapshot(ctx);
        let scroll_state = self.into();

        let (local_selection_data, remote_selections_data) = self.all_drawable_selections_data(
            view_snapshot.cursor_avatar_font_size(),
            view_snapshot.cursor_avatar_size(),
            ctx,
        );

        let editor_element = EditorElement::new(
            view_snapshot,
            scroll_state,
            self.command_x_ray_mouse_handle.clone(),
            self.soft_wrap,
            soft_wrap_state,
            self.placeholder_soft_wrap,
            self.vim_mode(ctx),
            text_colors,
            editor_decorator_elements,
            local_selection_data,
            remote_selections_data,
            self.cursor_display_override,
            self.voice_input_toggle_key_code(ctx),
        )
        .with_input_editor_icons(
            &self.accept_autosuggestion_keybinding_view,
            &self.autosuggestion_ignore_view,
            self.show_autosuggestion_keybinding_hint,
            self.show_autosuggestion_ignore_button,
            self.next_command_state(ctx).is_cycling(),
            ctx,
        );

        #[cfg(feature = "voice_input")]
        let editor_element = self.configure_editor_element_voice(editor_element, appearance);

        let hoverable = Hoverable::new(self.hover_handle.clone(), |_state| editor_element.finish())
            .with_cursor(Cursor::IBeam)
            .finish();

        if let Some(controls) = self.render_controls(ctx) {
            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::End);
            row.add_child(Shrinkable::new(1., hoverable).finish());
            row.add_child(controls);
            row.finish()
        } else {
            hoverable
        }
    }

    fn keymap_context(&self, ctx: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        if self.single_cursor_at_buffer_end(false /* respect_line_cap */, ctx) {
            context.set.insert("EditorView_SingleCursorBufferEnd");
        }

        let editor_settings = AppEditorSettings::as_ref(ctx);
        if *editor_settings.enable_autosuggestions {
            context.set.insert(flags::AUTOSUGGESTIONS_ENABLED_FLAG);
        }

        if self.autosuggestion_state.is_some() {
            context.set.insert("Has_Autosuggestion");
        }

        if let Some(vim_mode) = self.vim_mode(ctx) {
            context.set.insert("Vim");
            if vim_mode == VimMode::Normal {
                context.set.insert("VimNormalMode");
            }
        }

        if self.is_ai_input {
            context.set.insert("AIInput");
        }

        // Allow parent views to add additional flags to the context
        if let Some(modifier) = &self.keymap_context_modifier {
            modifier(&mut context, ctx);
        }

        context
    }

    fn on_window_transferred(
        &mut self,
        _source_window_id: WindowId,
        target_window_id: WindowId,
        _ctx: &mut ViewContext<Self>,
    ) {
        self.window_id = target_window_id;
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            self.blink_cursors(self.blink_epoch, ctx);
            if self.select_all_on_focus {
                self.select_all(ctx);
            }
            ctx.report_active_cursor_position_update();
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            self.cursors_visible = false;
            if self.clear_selections_on_blur {
                self.clear_selections(ctx);
            }

            self.maybe_commit_incomplete_ime_text(ctx);
            ctx.emit(Event::Blurred);
            ctx.notify();
        }
    }

    fn active_cursor_position(&self, ctx: &ViewContext<Self>) -> Option<CursorInfo> {
        let cursor_id = position_id_for_cursor(ctx.view_id());
        let appearance = Appearance::as_ref(ctx);
        let font_size = self.font_size(appearance);
        ctx.element_position_by_id(cursor_id)
            .map(|position| CursorInfo {
                position,
                font_size,
            })
    }
}

/// Returns the canonical position ID for a cached point in an editor.
pub fn position_id_for_cached_point(editor_view_id: EntityId, position_id: &str) -> String {
    format!("editor_{editor_view_id}:{position_id}")
}

/// Returns the canonical position ID for the cursor in an editor.
pub fn position_id_for_cursor(editor_view_id: EntityId) -> String {
    format!("editor:cursor_{editor_view_id}")
}

/// Returns the canonical position ID for the first cursor in an editor.
/// Necessary because the above `position_id_for_cursor` refers to the
/// last cursor position, when there are multiple.
pub fn position_id_for_first_cursor(editor_view_id: EntityId) -> String {
    format!("editor:first_cursor_{editor_view_id}")
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
