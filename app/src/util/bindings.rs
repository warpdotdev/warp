use crate::keyboard::{remove_custom_keybinding, write_custom_keybinding, UserDefinedKeybinding};
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use enum_iterator::{all, Sequence};
use fuzzy_match::match_indices_case_insensitive;
use itertools::Itertools;
use lazy_static::lazy_static;

use regex::Regex;
use std::borrow::Cow;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::Arc,
};
use warpui::keymap::{BindingId, IsBindingValid};
use warpui::platform::OperatingSystem;
use warpui::{
    actions::StandardAction,
    keymap::{
        BindingDescription, BindingLens, CustomTag, DescriptionContext, EditableBindingLens,
        Keystroke, Trigger,
    },
    Action,
};
use warpui::{AppContext, SingletonEntity};

pub const MAC_MENUS_CONTEXT: DescriptionContext = DescriptionContext::Custom("mac_menus");

// CustomActions are attached to menu items, and may be attached to Bindings.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Sequence)]
#[repr(isize)]
pub enum CustomAction {
    NewTab,
    NewFile,
    ShowAboutWarp,
    ShowSettings,
    ConfigureKeybindings,
    ShowAccount,
    ShowAppearance,
    ReferAFriend,
    ViewChangelog,
    FocusInput,
    ClearBlocks,
    AddNextOccurrence,
    AddCursorAbove,
    AddCursorBelow,
    CycleNextSession,
    CyclePrevSession,
    Cut,
    Copy,
    Paste,
    Undo,
    Redo,
    CommandPalette,
    AISearch,
    ClearEditor,
    Find,
    SelectAll,
    Workflows,
    HistorySearch,
    SaveCurrentConfig,
    History,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    IncreaseZoom,
    DecreaseZoom,
    ResetZoom,
    RenameTab,
    SplitPaneRight,
    SplitPaneLeft,
    SplitPaneUp,
    SplitPaneDown,
    MoveTabLeft,
    MoveTabRight,
    ActivateNextTab,
    ActivatePreviousTab,
    ActivateNextPane,
    ActivatePreviousPane,
    NavigationPalette,
    SelectBlockAbove,
    SelectBlockBelow,
    SelectAllBlocks,
    CreateBlockPermalink,
    ToggleBookmarkBlock,
    FindWithinBlock,
    CopyBlock,
    CopyBlockCommand,
    CopyBlockOutput,
    ViewSharedBlocks,
    CloseTab,
    CloseOtherTabs,
    CloseTabsRight,
    ToggleMaximizePane,
    LaunchConfigPalette,
    FilesPalette,
    TriggerWelcomeBlock,
    CommandSearch,
    ToggleResourceCenter,
    ToggleKeybindingsPage,
    ScrollToTopOfSelectedBlocks,
    ScrollToBottomOfSelectedBlocks,
    ToggleSyncAllTerminalInputsInAllTabs,
    ToggleSyncTerminalInputsInCurrentTab,
    DisableSyncTerminalInputs,
    ReopenClosedSession,
    ToggleWarpDrive,
    AddWindow,
    CloseCurrentSession,
    CloseWindow,
    NewPersonalWorkflow,
    NewPersonalNotebook,
    NewPersonalEnvVars,
    NewTeamWorkflow,
    NewTeamNotebook,
    NewTeamEnvVars,
    SearchDrive,
    OpenTeamSettings,
    ShareCurrentSession,
    SharePaneContents,
    #[cfg(windows)]
    WindowsPaste,
    #[cfg(windows)]
    WindowsCopy,
    /// Also applies to legacy Warp AI (toggles the panel)
    NewAgentModePane,
    /// Also applies to legacy Warp AI (attaches the selection to the panel editor)
    AttachSelectionAsAgentModeContext,
    OpenAIFactCollection,
    OpenMCPServerCollection,
    ToggleProjectExplorer,
    NewPersonalAIPrompt,
    NewTeamAIPrompt,
    OpenRepository,
    NewTerminalTab,
    NewAgentTab,
    GoToLine,
    ToggleGlobalSearch,
    ToggleConversationListView,
    RenamePane,
}

lazy_static! {
    /// Maps for converting from custom tags back to the action enum
    /// This layer of indirection is necessary because the UI framework can't
    /// know about particular Warp specific actions, so it deals with all actions
    /// as plain isizes.  Within Warp though we want to deal with them as the enum type.
    pub static ref CUSTOM_TAG_TO_ACTION: HashMap<isize, CustomAction> = HashMap::from_iter(all::<CustomAction>().map(|action| {
        (action as isize, action)
    }));

    /// Regex that matches whether the the normalized form of a [`Keystroke`] matches a control
    /// character. ASCII control characters constitute the first 31 values of ASCII characters.
    /// Though they have their own ASCII codepoints, they are typed into the keyboard using
    /// `ctrl-XX`, see <https://en.wikipedia.org/wiki/Caret_notation>.
    ///
    /// As an example, the ETX character (represented as `^C` in caret notation) is sent to
    /// the PTY when the user presses `ctrl-c`.
    ///
    /// ## Control Characters List
    /// The full list of these control characters (and their corresponding name) are documented
    /// below:
    /// * `^@`: Null
    /// * `^A`: Start of Header
    /// * `^B`: Start of Text
    /// * `^C`: End of Text
    /// * `^D`: End of Transmission
    /// * `^E`: Enquiry
    /// * `^F`: Acknowledge
    /// * `^G`: Bell
    /// * `^H`: BackSpace
    /// * `^I`: Horizontal Tabulation
    /// * `^J`: Line Feed
    /// * `^K`: Vertical Tabulation
    /// * `^L`: Form Feed
    /// * `^M`: Carriage Return
    /// * `^N`: Shift Out
    /// * `^O`: Shift In
    /// * `^P`: Data Link Escape
    /// * `^Q`: Device Control 1 (XON)
    /// * `^R`: Device Control 2
    /// * `^S`: Device Control 3 (XOFF)
    /// * `^T`: Device Control 4
    /// * `^U`: Negative acknowledge
    /// * `^V`: Synchronous Idle
    /// * `^W`: End of Transmission Block
    /// * `^X`: Cancel
    /// * `^Y`: End of Medium
    /// * `^Z`: Substitute
    /// * `^[`: Escape
    /// * `^\`: File Separator
    /// * `^]`: Group Separator
    /// * `^^`: Record Separator
    /// * `^_`: Unit Separator
    /// * `^?`: Delete
    ///
    /// ## Note
    /// Though caret notation uses uppercase letters (`^C` instead of `^c`), we validate using
    /// _lowercase_ characters because it is impossible to create a [`Keystroke`] of the form
    /// `ctrl-[A-Z]`. See [`Keystroke::parse`].
    pub static ref CONTROL_CHARACTER_KEY_REGEX: Regex = Regex::new(r"^ctrl-[a-z@\[\\\]^_?]$").expect("should be able to construct regex");

    /// Set of actions on Mac that should be considered valid bindings even though they aren't PTY
    /// compliant. We weren't always diligent about avoiding bindings that could conflict with
    /// character codes, unfortunately some bindings on Mac currently conflict with the PTY. We have
    /// this allowlist to special case these legacy actions for the purposes of binding validation.
    pub static ref MAC_PTY_NON_COMPLIANT_ACTIONS: HashSet<&'static str> = HashSet::from_iter(["terminal:warpify_subshell", "terminal:open_block_list_context_menu_via_keybinding"]);

    /// Set of actions on Windows that should be considered valid bindings even though they aren't
    /// PTY compliant. Windows users expect pasting to work using both `ctrl-v` and `ctrl-shift-v`,
    /// so we allowlist the terminal paste action for the purposes of binding validation.
    pub static ref WINDOWS_PTY_NON_COMPLIANT_KEYSTROKES: HashSet<Keystroke> = HashSet::from_iter([Keystroke::parse("ctrl-v").expect("should be able to construct ctrl-v keystroke")]);

    /// Set of keystrokes that should be considered valid bindings on all platforms even though
    /// they aren't PTY compliant.
    pub static ref PTY_NON_COMPLIANT_KEYSTROKES: HashSet<Keystroke> = HashSet::from_iter([
        // Windows users expect ctrl-c to copy any selected text to the clipboard. To avoid
        // introducing multiple codepaths for handling ctrl-c, we register ctrl-c as a binding
        // on TerminalView on all platforms.
        Keystroke::parse("ctrl-c").expect("should be able to construct ctrl-c keystroke"),
        // The resume conversation binding uses cmd-shift-R on Mac and should be allowed
        Keystroke::parse("cmd-shift-R").expect("should be able to construct cmd-shift-R keystroke")
    ]);
}

impl From<CustomAction> for CustomTag {
    fn from(action: CustomAction) -> Self {
        action as CustomTag
    }
}

impl From<CustomTag> for CustomAction {
    fn from(tag: CustomTag) -> Self {
        *CUSTOM_TAG_TO_ACTION
            .get(&tag)
            .expect("All custom actions are handled.")
    }
}

pub fn trigger_to_keystroke(trigger: &Trigger) -> Option<Keystroke> {
    match trigger {
        Trigger::Keystrokes(keys) => keys.first().cloned(),
        // Custom actions don't have keyboard shortcuts associated with the actions themselves,
        // they are set separately in app/src/lib.rs as part of creating the Menu. As a result,
        // we need to map those to the appropriate keyboard shortcut.
        Trigger::Custom(custom) => custom_tag_to_keystroke(*custom),
        // Similarly, Standard Actions have their keyboard shortcuts set when creating the menu
        Trigger::Standard(standard) => match standard {
            StandardAction::Close => mac_only_keystroke("cmd-shift-W"),
            // "cmd-q" to quit and "cmd-h" to hide are the standard bindings for these actions on
            // Mac.
            StandardAction::Quit => mac_only_keystroke("cmd-q"),
            StandardAction::Hide => mac_only_keystroke("cmd-h"),
            StandardAction::HideOtherApps => Keystroke::parse("cmdorctrl-alt-h").ok(),
            StandardAction::ToggleFullScreen => mac_only_keystroke("cmd-ctrl-f"),
            StandardAction::Paste => Keystroke::parse(cmd_or_ctrl_shift("v")).ok(),
            StandardAction::ShowAllApps
            | StandardAction::BringAllToFront
            | StandardAction::Minimize
            | StandardAction::Zoom => None,
        },
        Trigger::Empty => None,
    }
}

/// Returns the corresponding [`Keystroke`], if any, of a [`CustomTag`].
pub fn custom_tag_to_keystroke(custom: CustomTag) -> Option<Keystroke> {
    match custom.into() {
        CustomAction::FocusInput => Keystroke::parse(cmd_or_ctrl_shift("l")).ok(),
        CustomAction::NewTab => Keystroke::parse(cmd_or_ctrl_shift("t")).ok(),
        CustomAction::Cut => Keystroke::parse("cmdorctrl-x").ok(),
        CustomAction::Copy => Keystroke::parse(cmd_or_ctrl_shift("c")).ok(),
        CustomAction::Paste => Keystroke::parse(cmd_or_ctrl_shift("v")).ok(),
        #[cfg(windows)]
        CustomAction::WindowsPaste => Keystroke::parse("ctrl-v").ok(),
        #[cfg(windows)]
        CustomAction::WindowsCopy => Keystroke::parse("ctrl-c").ok(),
        CustomAction::Undo => Keystroke::parse("cmdorctrl-z").ok(),
        CustomAction::Redo => Keystroke::parse("cmdorctrl-shift-Z").ok(),
        CustomAction::ClearEditor => Keystroke::parse("ctrl-c").ok(),
        CustomAction::CycleNextSession => Keystroke::parse("ctrl-tab").ok(),
        CustomAction::CyclePrevSession => Keystroke::parse("ctrl-shift-tab").ok(),
        CustomAction::ShowSettings => Keystroke::parse("cmdorctrl-,").ok(),
        CustomAction::AddNextOccurrence => Keystroke::parse("ctrl-g").ok(),
        CustomAction::AddCursorAbove => Keystroke::parse("ctrl-shift-up").ok(),
        CustomAction::AddCursorBelow => Keystroke::parse("ctrl-shift-down").ok(),
        CustomAction::CommandPalette => Keystroke::parse(cmd_or_ctrl_shift("p")).ok(),
        CustomAction::AISearch => Keystroke::parse("ctrl-`").ok(),
        CustomAction::Find => Keystroke::parse(cmd_or_ctrl_shift("f")).ok(),
        CustomAction::SelectAll => Keystroke::parse("cmdorctrl-a").ok(),
        CustomAction::CommandSearch => Keystroke::parse("ctrl-r").ok(),
        CustomAction::Workflows => Keystroke::parse("ctrl-shift-R").ok(),
        CustomAction::History => Keystroke::parse("up").ok(),
        CustomAction::IncreaseFontSize => Keystroke::parse("shift-cmdorctrl-+").ok(),
        CustomAction::DecreaseFontSize => Keystroke::parse("shift-cmdorctrl-_").ok(),
        CustomAction::ResetFontSize => Keystroke::parse("cmdorctrl-0").ok(),
        CustomAction::IncreaseZoom => Keystroke::parse("cmdorctrl-=").ok(),
        CustomAction::DecreaseZoom => Keystroke::parse("cmdorctrl--").ok(),
        CustomAction::ResetZoom => Keystroke::parse("cmdorctrl-0").ok(),
        CustomAction::SplitPaneRight => Keystroke::parse(cmd_or_ctrl_shift("d")).ok(),
        CustomAction::SplitPaneDown => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-shift-D").ok()
            } else {
                // On non-Mac platforms, we can't use `ctrl-shift-D` for `SplitPaneRight` since
                // we already  use that for `SplitPaneRight` above. Instead we use
                // `ctrl-shift-E`, which matches what Hyper uses. See https://github.com/vercel/hyper/blob/9c72409f5138c03a5a74fcc4dba9109217b4524a/app/keymaps/linux.json#L32.
                Keystroke::parse("ctrl-shift-E").ok()
            }
        }
        CustomAction::MoveTabLeft => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("shift-ctrl-left").ok()
            } else {
                Keystroke::parse("shift-ctrl-pageup").ok()
            }
        }
        CustomAction::MoveTabRight => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("shift-ctrl-right").ok()
            } else {
                Keystroke::parse("shift-ctrl-pagedown").ok()
            }
        }
        CustomAction::ActivateNextTab => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("shift-cmd-}").ok()
            } else {
                Keystroke::parse("ctrl-pagedown").ok()
            }
        }
        CustomAction::ActivatePreviousTab => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("shift-cmd-{").ok()
            } else {
                Keystroke::parse("ctrl-pageup").ok()
            }
        }
        CustomAction::ActivateNextPane => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-]").ok()
            } else {
                Keystroke::parse("ctrl-shift-}").ok()
            }
        }
        CustomAction::ActivatePreviousPane => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-[").ok()
            } else {
                Keystroke::parse("ctrl-shift-{").ok()
            }
        }
        CustomAction::NavigationPalette => mac_only_keystroke("cmd-shift-P"),
        CustomAction::LaunchConfigPalette => mac_only_keystroke("ctrl-cmd-l"),
        CustomAction::FilesPalette => Keystroke::parse(cmd_or_ctrl_shift("o")).ok(),
        CustomAction::ClearBlocks => Keystroke::parse(cmd_or_ctrl_shift("k")).ok(),
        CustomAction::SelectBlockAbove => Keystroke::parse("cmdorctrl-up").ok(),
        CustomAction::SelectBlockBelow => Keystroke::parse("cmdorctrl-down").ok(),
        // Set this to mac-only. On Linux this conflicts with the binding to save a workflow.
        CustomAction::CreateBlockPermalink => mac_only_keystroke("cmd-shift-S"),
        CustomAction::ToggleBookmarkBlock => Keystroke::parse(cmd_or_ctrl_shift("b")).ok(),
        CustomAction::CopyBlockOutput => Keystroke::parse("cmdorctrl-alt-shift-C").ok(),
        // Set this to mac-only. On Linux this conflicts with the general binding to copy.
        CustomAction::CopyBlockCommand => mac_only_keystroke("cmd-shift-C"),
        // Set this to mac-only. On Linux this conflicts with the cmd-enter keybindings
        // (used for actions on the input suggestions menu, and for accepting passive code diffs).
        CustomAction::ToggleMaximizePane => mac_only_keystroke("cmd-shift-enter"),
        // Note: The base character '/' is used instead of '?' as mac registers keybindings
        // differently compared to the app which saves the resulting character used with shift
        // TODO: resolve these keybinding differences
        CustomAction::ToggleResourceCenter => Keystroke::parse("ctrl-shift-/").ok(),
        CustomAction::ToggleKeybindingsPage => Keystroke::parse("cmdorctrl-/").ok(),
        CustomAction::ScrollToTopOfSelectedBlocks => Keystroke::parse("cmdorctrl-shift-up").ok(),
        CustomAction::ScrollToBottomOfSelectedBlocks => {
            Keystroke::parse("cmdorctrl-shift-down").ok()
        }
        CustomAction::CopyBlock => Keystroke::parse(cmd_or_ctrl_shift("c")).ok(),
        CustomAction::FindWithinBlock => Keystroke::parse(cmd_or_ctrl_shift("f")).ok(),
        CustomAction::ToggleSyncTerminalInputsInCurrentTab => {
            Keystroke::parse("alt-cmdorctrl-i").ok()
        }
        CustomAction::ReopenClosedSession => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-shift-T").ok()
            } else {
                // Use a custom keybinding for linux/windows since the binding would otherwise
                // conflict with the binding for creating a new tab.
                Keystroke::parse("ctrl-alt-t").ok()
            }
        }

        // This is one of the app's hardcoded keybindings.
        CustomAction::AddWindow => Keystroke::parse(cmd_or_ctrl_shift("n")).ok(),
        CustomAction::ToggleWarpDrive => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-\\").ok()
            } else {
                Keystroke::parse("ctrl-shift-|").ok()
            }
        }
        CustomAction::CloseWindow => mac_only_keystroke("cmd-shift-W"),
        CustomAction::CloseCurrentSession => Keystroke::parse(cmd_or_ctrl_shift("w")).ok(),
        CustomAction::ViewChangelog => Keystroke::parse(cmd_or_ctrl_shift("alt-o")).ok(),
        CustomAction::NewAgentModePane => Keystroke::parse("ctrl-space").ok(),
        CustomAction::AttachSelectionAsAgentModeContext => {
            Keystroke::parse("ctrl-shift-space").ok()
        }
        CustomAction::ToggleProjectExplorer => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("ctrl-2").ok()
            } else {
                Keystroke::parse("ctrl-shift-2").ok()
            }
        }
        CustomAction::OpenRepository => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("cmd-shift-O").ok()
            } else {
                Keystroke::parse("alt-shift-O").ok()
            }
        }
        CustomAction::GoToLine => Keystroke::parse("ctrl-g").ok(),
        CustomAction::ToggleGlobalSearch => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("ctrl-3").ok()
            } else {
                Keystroke::parse("alt-3").ok()
            }
        }
        CustomAction::ToggleConversationListView => {
            if OperatingSystem::get().is_mac() {
                Keystroke::parse("ctrl-1").ok()
            } else {
                Keystroke::parse("alt-1").ok()
            }
        }
        CustomAction::NewTerminalTab
        | CustomAction::NewFile
        | CustomAction::ShowAboutWarp
        | CustomAction::SplitPaneLeft
        | CustomAction::SelectAllBlocks
        | CustomAction::SplitPaneUp
        | CustomAction::ConfigureKeybindings
        | CustomAction::RenameTab
        | CustomAction::RenamePane
        | CustomAction::CloseTab
        | CustomAction::CloseOtherTabs
        | CustomAction::CloseTabsRight
        | CustomAction::ReferAFriend
        | CustomAction::ViewSharedBlocks
        | CustomAction::ShowAccount
        | CustomAction::ShowAppearance
        | CustomAction::SaveCurrentConfig
        | CustomAction::TriggerWelcomeBlock
        | CustomAction::HistorySearch
        | CustomAction::DisableSyncTerminalInputs
        | CustomAction::ToggleSyncAllTerminalInputsInAllTabs
        | CustomAction::NewPersonalWorkflow
        | CustomAction::NewPersonalNotebook
        | CustomAction::NewPersonalEnvVars
        | CustomAction::NewTeamWorkflow
        | CustomAction::NewTeamNotebook
        | CustomAction::NewTeamEnvVars
        | CustomAction::SearchDrive
        | CustomAction::OpenTeamSettings
        | CustomAction::ShareCurrentSession
        | CustomAction::SharePaneContents
        | CustomAction::OpenAIFactCollection
        | CustomAction::OpenMCPServerCollection
        | CustomAction::NewPersonalAIPrompt
        | CustomAction::NewTeamAIPrompt
        | CustomAction::NewAgentTab => None,
    }
}

/// Get the keystroke currently assigned to a binding. Returns `None` if the binding does not exist
/// or is unassigned.
pub fn keybinding_name_to_keystroke(binding_name: &str, ctx: &AppContext) -> Option<Keystroke> {
    ctx.get_binding_by_name(binding_name)
        .and_then(|binding| trigger_to_keystroke(binding.trigger))
}

/// Get keybinding display string from binding name. Unset keybindings will return None.
pub fn keybinding_name_to_display_string(binding_name: &str, ctx: &AppContext) -> Option<String> {
    keybinding_name_to_keystroke(binding_name, ctx).map(|keystroke| keystroke.displayed())
}

/// Get normalized keybinding string from binding name. Unset keybindings will return None.
pub fn keybinding_name_to_normalized_string(
    binding_name: &str,
    ctx: &AppContext,
) -> Option<String> {
    keybinding_name_to_keystroke(binding_name, ctx).map(|keystroke| keystroke.normalized())
}

/// Sets a custom keybinding for an editable binding using the given keystroke. Will
/// persist the keybinding to the user's config file and emit a KeybindingChangedEvent.
pub fn set_custom_keybinding(binding_name: &str, keystroke: &Keystroke, ctx: &mut AppContext) {
    ctx.set_custom_trigger(
        binding_name.into(),
        Trigger::Keystrokes(vec![keystroke.clone()]),
    );
    write_custom_keybinding(
        binding_name.into(),
        UserDefinedKeybinding::keystroke(keystroke.clone()),
    );
    KeybindingChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
        ctx.emit(KeybindingChangedEvent::BindingChanged {
            binding_name: binding_name.into(),
            new_trigger: Some(keystroke.clone()),
        })
    });
}

/// Reset an editable binding back to its default trigger. Will persist this change to
/// the user's config file and emit a KeybindingChangedEvent with the default trigger.
/// Returns the default keystroke for the binding.
pub fn reset_keybinding_to_default(binding_name: &str, ctx: &mut AppContext) -> Option<Keystroke> {
    ctx.remove_custom_trigger(binding_name);
    remove_custom_keybinding(binding_name);

    let default_keystroke = ctx
        .editable_bindings()
        .find(|binding| binding.name == binding_name)
        .and_then(|binding| trigger_to_keystroke(binding.trigger));

    KeybindingChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
        ctx.emit(KeybindingChangedEvent::BindingChanged {
            binding_name: binding_name.into(),
            new_trigger: default_keystroke.clone(),
        })
    });

    default_keystroke
}

#[derive(Clone, Debug)]
pub struct CommandBinding {
    pub name: String,
    pub description: BindingDescription,
    pub trigger: Option<Keystroke>,
    pub action: Option<Arc<dyn Action>>,
    pub group: Option<BindingGroup>,
    /// The ID of the binding.  If the [`CommandBinding`] was created from an
    /// [`EditableBindingLens`] or [`BindingLens`] this is the id of the lens. Otherwise a new ID
    /// is constructed.
    pub id: BindingId,
}

/// SearchScore is a helper struct for ranking the result of the search.
/// `keystroke_score` represents the proximity between search term keystroke with
/// the candidate keystroke. If the score is None, it means the candidate keystroke is not
/// a valid subset of the search term keystroke.
/// The fuzzy_search_score helps on the secondary ranking -- if two candidate keystrokes are
/// both not valid subset of the search term keystroke, then we rank these by the score
/// of the fuzzy_search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SearchScore {
    keystroke_score: Option<usize>,
    fuzzy_search_score: i64,
}

impl Ord for SearchScore {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.keystroke_score.cmp(&other.keystroke_score) {
            Ordering::Equal => self.fuzzy_search_score.cmp(&other.fuzzy_search_score),
            ordering => ordering,
        }
    }
}

impl PartialOrd for SearchScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn convert_search_term_to_keystroke(search_term: &str) -> Option<Keystroke> {
    let mut search_keystroke: Keystroke = Default::default();
    let mut key_set = false;

    for element in search_term.split_whitespace() {
        match element.to_ascii_lowercase().as_str() {
            "command" | "cmd" => {
                if search_keystroke.cmd {
                    return None;
                }
                search_keystroke.cmd = true
            }
            "control" | "ctrl" => {
                if search_keystroke.ctrl {
                    return None;
                }
                search_keystroke.ctrl = true
            }
            "alt" | "option" => {
                if search_keystroke.alt {
                    return None;
                }
                search_keystroke.alt = true
            }
            "shift" => {
                if search_keystroke.shift {
                    return None;
                }
                search_keystroke.shift = true
            }
            "meta" => {
                if search_keystroke.meta {
                    return None;
                }
                search_keystroke.meta = true
            }
            key => {
                if key_set {
                    return None;
                }
                key_set = true;
                search_keystroke.key = key.to_string()
            }
        }
    }

    // Internally we uppercase the key when shift modifier is true.
    if search_keystroke.shift && search_keystroke.key.len() == 1 {
        search_keystroke.key = search_keystroke.key.to_ascii_uppercase();
    }

    Some(search_keystroke)
}

pub fn filter_bindings_including_keystroke<'a>(
    bindings_iter: impl Iterator<Item = &'a CommandBinding>,
    search_term: &str,
    description_for: DescriptionContext,
) -> impl Iterator<Item = (Option<Vec<usize>>, &'a CommandBinding)> {
    let search_keystroke = convert_search_term_to_keystroke(search_term);

    bindings_iter
        .filter_map(move |binding| {
            if search_term.is_empty() {
                Some((Default::default(), None, binding))
            } else {
                let keystroke_search_score = if let Some(search_keystroke) = &search_keystroke {
                    binding.trigger.as_ref().and_then(|candidate_keystroke| {
                        let score = keystroke_includes(search_keystroke, candidate_keystroke);
                        if score > 0 {
                            Some(score)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };

                let fuzzy_search_result = match_indices_case_insensitive(
                    binding.description.in_context(description_for),
                    search_term,
                );

                match (keystroke_search_score, fuzzy_search_result) {
                    // If keystroke matched, don't include fuzzy search highlights.
                    (Some(keystroke_score), Some(fuzzy_search_result)) => Some((
                        SearchScore {
                            keystroke_score: Some(keystroke_score),
                            fuzzy_search_score: fuzzy_search_result.score,
                        },
                        None,
                        binding,
                    )),
                    (None, Some(fuzzy_search_result)) => Some((
                        SearchScore {
                            fuzzy_search_score: fuzzy_search_result.score,
                            ..Default::default()
                        },
                        None,
                        binding,
                    )),
                    (Some(keystroke_score), None) => Some((
                        SearchScore {
                            keystroke_score: Some(keystroke_score),
                            ..Default::default()
                        },
                        None,
                        binding,
                    )),
                    _ => None,
                }
            }
        })
        .sorted_by(|(score1, _, _), (score2, _, _)| score2.cmp(score1))
        .map(|(_, indices, binding)| (indices, binding))
}

/// Check if the keystroke could be a possible candidate of the search keystroke and give a score
/// based on proximity.
/// The scores are generated as follows: for each field of the keystroke (alt, cmd, etc), the comparison
/// between the search and candidate keystroke could yield three possible results - a strict
/// match (candidate and search has the same value), a potential match (candidate has the value
/// set to true but search is missing the value), a mismatch (candidate has the value set to
/// false but search is set to true).
/// For a strict match, we increase the score by multiplying it by two. For a potential match,
/// we keep the original score by multiplying it by one. For a mismatch, we multiply by zero
/// to mark that the candidate keystroke is not a valid subset of the search keystroke.
fn keystroke_includes(search_keystroke: &Keystroke, candidate_keystroke: &Keystroke) -> usize {
    fn modifier_match(
        search_keystroke_condition: bool,
        candidate_keystroke_condition: bool,
    ) -> usize {
        match (search_keystroke_condition, candidate_keystroke_condition) {
            (false, false) | (true, true) => 2, // match gives a score of 2.
            (false, true) => 1, // keep the same score if the keystroke term is true but search_keystroke does not include the term.
            (true, false) => 0, // if the keystroke term is false but search_keystorke is true, return 0 score
        }
    }

    let key_match_score = if search_keystroke.key == candidate_keystroke.key {
        2
    } else {
        usize::from(search_keystroke.key.is_empty())
    };

    modifier_match(search_keystroke.alt, candidate_keystroke.alt)
        * modifier_match(search_keystroke.cmd, candidate_keystroke.cmd)
        * modifier_match(search_keystroke.ctrl, candidate_keystroke.ctrl)
        * modifier_match(search_keystroke.meta, candidate_keystroke.meta)
        * modifier_match(search_keystroke.shift, candidate_keystroke.shift)
        * key_match_score
}

impl CommandBinding {
    pub fn new(name: String, description: String, trigger: Option<Keystroke>) -> Self {
        CommandBinding {
            name,
            description: BindingDescription::new(description),
            trigger,
            action: None,
            group: None,
            id: BindingId::new(),
        }
    }

    /// Materializes a [`CommandBinding`] from a [`BindingLens`], resolving
    /// any dynamic description against `ctx` so downstream consumers that
    /// have no `&AppContext` (fuzzy/full-text search indices, accessibility
    /// labels, render paths that only see an `Appearance`) observe a plain
    /// string.
    ///
    /// This is intentionally the only way to build a `CommandBinding` from
    /// a lens; taking `&AppContext` by value here forces every cache-
    /// population site to thread context through, which in turn guarantees
    /// that a future dynamic description cannot silently go unresolved.
    ///
    /// Returns `None` when the source binding has no description.
    pub fn from_lens(lens: BindingLens<'_>, ctx: &AppContext) -> Option<Self> {
        lens.description.map(|desc| CommandBinding {
            description: materialize_description(desc, ctx),
            trigger: trigger_to_keystroke(lens.trigger),
            action: Some(lens.action.clone()),
            name: lens.name.to_string(),
            group: lens.group.and_then(BindingGroup::from_str),
            id: lens.id,
        })
    }

    /// Materializes a [`CommandBinding`] from an [`EditableBindingLens`].
    /// See [`Self::from_lens`] for why this takes `&AppContext`.
    pub fn from_editable_lens(lens: EditableBindingLens<'_>, ctx: &AppContext) -> Self {
        Self {
            description: materialize_description(lens.description, ctx),
            trigger: trigger_to_keystroke(lens.trigger),
            action: Some(lens.action.clone()),
            name: lens.name.into(),
            group: lens.group.and_then(BindingGroup::from_str),
            id: lens.id,
        }
    }

    pub fn placeholder(placeholder: String) -> Self {
        CommandBinding {
            name: Default::default(),
            description: placeholder.into(),
            trigger: None,
            action: None,
            group: None,
            id: BindingId::new(),
        }
    }
}

fn materialize_description(desc: &BindingDescription, ctx: &AppContext) -> BindingDescription {
    if desc.has_dynamic_override() {
        desc.materialized(ctx)
    } else {
        desc.clone()
    }
}

/// Possible groups a Binding can be part of. The string representation (produced in
/// [`BindingGroup::as_str`]) is used as the group identifier within
/// [`warpui::keymap::FixedBinding`] or [`EditableBinding`].
#[derive(Copy, Clone, Debug, Sequence)]
pub enum BindingGroup {
    Settings,
    Close,
    Navigation,
    WarpAi,
    Workflow,
    Notebooks,
    Folders,
    KeyboardShortcuts,
    AutoUpdate,
    Notifications,
    EnvVarCollection,
    Terminal,
}

impl BindingGroup {
    /// Returns a string representation of the [`BindingGroup`].
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::WarpAi => "warp_ai",
            Self::Navigation => "navigation",
            Self::Workflow => "workflows",
            Self::Notebooks => "notebooks",
            Self::Folders => "folders",
            Self::KeyboardShortcuts => "keyboard_shortcuts",
            Self::Close => "close",
            Self::AutoUpdate => "autoupdate",
            Self::Notifications => "notifications",
            Self::EnvVarCollection => "env_var_collections",
            Self::Terminal => "terminal",
        }
    }

    /// Creates a [`BindingGroup`] from a str. Returns `None` if there is no group that corresponds
    /// to the `str`.
    fn from_str(str: &'static str) -> Option<Self> {
        all::<Self>().find(|&item| item.as_str() == str)
    }
}

/// Constructs a keybinding that is the `cmd-key` on Mac or `ctrl-shift-key` otherwise. This is
/// useful when constructing a binding that needs to be compliant with the PTY. The typical pattern
/// of using `cmdorctrl-XX` to construct a platform agnostic keybinding does not work here because
/// `ctrl-XX` would conflict with the PTY because it is reserved as a control character.
///
/// Bindings of the form `ctrl-[a-z@[\]^_?]` are reserved as control characters. We don't want to
/// create bindings for in-app actions that would conflict with these control characters because we
/// would end up preventing the user from sending these control characters to the PTY. To avoid
/// this, we follow other terminals and use `ctrl-shift-XX` for in-app bindings if the binding would
/// otherwise conflict with the PTY.
///
/// ## Panics
/// Panics if debug assertions are enabled and a non "A-Z" key was passed in an environment where `ctrl-shift` would be
/// used. This is because the passed key needs to modified by the shift character in order to be valid in our UI
/// framework and we can't easily produce the shift-modified version of the key ourselves. In this case the recommended
/// solution is to to create separate [`Keystroke`]s for the Mac and non-Mac cases. For example:
/// ```
/// use warpui::keymap::Keystroke;
/// use warpui::platform::OperatingSystem;
/// let keystroke = if OperatingSystem::get().is_mac() {
///    Keystroke::parse("cmd-[")
/// } else {
///     Keystroke::parse("ctrl-shift-{")
/// };
/// ```
pub fn cmd_or_ctrl_shift(key: &str) -> String {
    if OperatingSystem::get().is_mac() {
        format!("cmd-{key}")
    } else {
        let key = if Keystroke::is_valid_special_key(key) {
            // Valid keys don't need to be uppercase (we don't want to create a binding that looks
            // like `ctrl-shift-ENTER`).
            Cow::Borrowed(key)
        } else {
            if cfg!(debug_assertions) {
                let stroke = key.chars().next().expect("Character should exist");

                if !stroke.is_ascii_lowercase() {
                    panic!(
                        "Tried to register a ctrl-shift-{key} shortcut which is invalid because the {key} character needs to be modified by the shift character."
                    );
                }
            }
            // The need to uppercase the key because of the addition of the `shift`.
            // Keystroke::parse debug asserts if this the modifier is lowercase:
            // https://github.com/warpdotdev/warp-internal/blob/c225b8cedd94fdba33e957cf1efb99d84768d193/ui/src/keymap.rs#L637/
            key.to_ascii_uppercase().into()
        };
        format!("ctrl-shift-{key}")
    }
}

/// Returns whether the given [`BindingLens`] is compliant with the PTY.
/// A binding is considered PTY compliant if it does not interfere with a control character that
/// needs to be sent to the PTY. A binding is considered to be a control character if the only
/// modifier set is `ctrl` and the key is one of `a-z@[\]^_?`.
pub fn is_binding_pty_compliant(binding: BindingLens) -> IsBindingValid {
    let trigger = binding.original_trigger.unwrap_or(binding.trigger);
    let Some(keystroke) = trigger_to_keystroke(trigger) else {
        return IsBindingValid::Yes;
    };

    let is_binding_in_allowlist = (OperatingSystem::get().is_mac()
        && MAC_PTY_NON_COMPLIANT_ACTIONS.contains(binding.name))
        || (OperatingSystem::get().is_windows()
            && WINDOWS_PTY_NON_COMPLIANT_KEYSTROKES.contains(&keystroke))
        || PTY_NON_COMPLIANT_KEYSTROKES.contains(&keystroke);

    if CONTROL_CHARACTER_KEY_REGEX.is_match(keystroke.normalized().as_str())
        && !is_binding_in_allowlist
    {
        // The binding interferes with a control character so it is not valid.
        IsBindingValid::No
    } else {
        IsBindingValid::Yes
    }
}

/// Validates all that bindings are cross-platform by returning [`IsBindingValid::No`] if a `cmd-*`
/// binding is used on non-mac platforms.
pub fn is_binding_cross_platform(binding: BindingLens) -> IsBindingValid {
    if OperatingSystem::get().is_mac() {
        return IsBindingValid::Yes;
    };

    let trigger = binding.original_trigger.unwrap_or(binding.trigger);
    let Some(keystroke) = trigger_to_keystroke(trigger) else {
        return IsBindingValid::Yes;
    };

    if keystroke.cmd {
        IsBindingValid::No
    } else {
        IsBindingValid::Yes
    }
}

/// Attempts to construct a [`Keystroke`] from the given source string if the current
/// [`OperatingSystem`] is mac. Returns `None` if not on Mac or if a [`Keystroke`] was unable to be
/// constructed from the source string.
fn mac_only_keystroke(source: &str) -> Option<Keystroke> {
    if OperatingSystem::get().is_mac() {
        Keystroke::parse(source).ok()
    } else {
        None
    }
}

#[cfg(test)]
#[path = "bindings_tests.rs"]
mod tests;
