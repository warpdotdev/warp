//! Surface-neutral state for the `@` context menu.
//!
//! This module owns the parts of the menu whose meaning is identical on every
//! front-end: which categories are available, the navigation state machine
//! between the category list and a category's results, the `@` trigger grammar,
//! and the buffer text each accepted action inserts.
//!
//! Front-ends keep their own selection model, rendering, key handling, and
//! search-mixer ownership. The GUI drives this state from
//! [`crate::search::ai_context_menu::view::AIContextMenu`]; the TUI drives it
//! from its own inline-menu model.

#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::AppContext;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity as _;

use crate::drive::settings::WarpDriveSettings;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::settings::InputSettings;

/// How many consecutive empty result sets the user may type past before the
/// menu gives up and closes itself.
const MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIContextMenuCategory {
    CurrentFolderFiles,
    RepoFiles,
    Commands,
    Blocks,
    Workflows,
    Notebooks,
    Plans,
    Diffs,
    Docs,
    Tasks,
    Rules,
    Servers,
    Terminal,
    Web,
    RecentDiff,
    RecentBlock,
    Code,
    DiffSet,
    Conversations,
    Skills,
}

impl AIContextMenuCategory {
    pub fn name(&self) -> &'static str {
        match self {
            AIContextMenuCategory::CurrentFolderFiles => "Files and folders",
            AIContextMenuCategory::RepoFiles => "Files and folders",
            AIContextMenuCategory::Commands => "Commands",
            AIContextMenuCategory::Blocks => "Blocks",
            AIContextMenuCategory::Workflows => "Workflows",
            AIContextMenuCategory::Notebooks => "Notebooks",
            AIContextMenuCategory::Plans => "Plans",
            AIContextMenuCategory::Diffs => "Diffs",
            AIContextMenuCategory::Docs => "Docs",
            AIContextMenuCategory::Tasks => "Past tasks",
            AIContextMenuCategory::Rules => "Rules",
            AIContextMenuCategory::Servers => "Servers and integrations",
            AIContextMenuCategory::Terminal => "Terminal",
            AIContextMenuCategory::Web => "Web",
            AIContextMenuCategory::RecentDiff => "Most recent diff",
            AIContextMenuCategory::RecentBlock => "Most recent block",
            AIContextMenuCategory::Code => "Code",
            AIContextMenuCategory::DiffSet => "Diff sets",
            AIContextMenuCategory::Conversations => "Conversations",
            AIContextMenuCategory::Skills => "Skills",
        }
    }
}

/// The different navigation states for the AI context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationState {
    /// The main menu showing all categories.
    MainMenu,
    /// Viewing items from a specific category.
    Category(AIContextMenuCategory),
    /// Viewing search results from all categories combined.
    AllCategories,
}

/// Surface conditions that gate which categories the menu offers.
///
/// Each front-end supplies the values that are meaningful for it; conditions a
/// front-end has no concept of stay `false`. The headless TUI, for instance,
/// has no ambient-agent session and no shared-session viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtContextMenuGates {
    /// Whether the input is in AI or autodetect mode rather than locked to the shell.
    pub is_ai_or_autodetect_mode: bool,
    /// Whether the surface is viewing somebody else's shared session.
    pub is_shared_session_viewer: bool,
    /// Whether the surface is an ambient agent session.
    pub is_in_ambient_agent: bool,
    /// Whether the surface is a CLI agent rich input, which only understands
    /// files, folders, and code symbols.
    pub is_cli_agent_input: bool,
}

impl Default for AtContextMenuGates {
    fn default() -> Self {
        Self {
            is_ai_or_autodetect_mode: true,
            is_shared_session_viewer: false,
            is_in_ambient_agent: false,
            is_cli_agent_input: false,
        }
    }
}

/// What the caller must do after the core absorbs a new filter query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtContextMenuQueryTransition {
    /// The navigation state did not change; run the query against the sources
    /// that are already installed.
    SourcesUnchanged,
    /// No category name matched the query, so the menu fell through to
    /// searching every available category and its sources must be reinstalled.
    EnteredAllCategories,
}

/// State shared by the GUI and TUI `@` context menus.
///
/// This deliberately excludes selection over *results*: each front-end already
/// has a selection model wired to its own list rendering (the GUI's
/// `SearchBarState`, the TUI's `TuiInlineMenuListState`). Selection over
/// *categories* lives here, because the category list is derived from this
/// state rather than from a mixer.
#[derive(Debug, Clone)]
pub struct AtContextMenuCoreState {
    navigation_state: NavigationState,
    gates: AtContextMenuGates,
    /// Working directory of the surface's session, used to decide whether
    /// repo-scoped categories are available. Front-ends keep this current as
    /// their session navigates.
    working_directory: Option<LocalOrRemotePath>,
    /// Query used to filter the category list while the main menu is showing.
    main_menu_query: String,
    selected_category_index: usize,
    /// How many times in a row results came back empty while the user kept
    /// extending the query. Used to close a menu that is making no progress.
    consecutive_empty_results_events: usize,
}

impl AtContextMenuCoreState {
    /// Builds the initial state for a surface. `navigation_state` starts on the
    /// category list, which [`Self::refresh_categories`] immediately narrows to
    /// a single category when only one is available.
    pub fn new(gates: AtContextMenuGates) -> Self {
        Self {
            navigation_state: NavigationState::MainMenu,
            gates,
            working_directory: None,
            main_menu_query: String::new(),
            selected_category_index: 0,
            consecutive_empty_results_events: 0,
        }
    }

    pub fn navigation_state(&self) -> NavigationState {
        self.navigation_state
    }

    pub fn gates(&self) -> AtContextMenuGates {
        self.gates
    }

    pub fn main_menu_query(&self) -> &str {
        &self.main_menu_query
    }

    pub fn selected_category_index(&self) -> usize {
        self.selected_category_index
    }

    /// Replaces the availability gates, returning whether anything changed so
    /// callers can skip recomputing category-dependent state.
    pub fn set_gates(&mut self, gates: AtContextMenuGates) -> bool {
        let changed = self.gates != gates;
        self.gates = gates;
        changed
    }

    /// Replaces the working directory, returning whether it changed.
    pub fn set_working_directory(&mut self, working_directory: Option<LocalOrRemotePath>) -> bool {
        let changed = self.working_directory != working_directory;
        self.working_directory = working_directory;
        changed
    }

    pub fn working_directory(&self) -> Option<&LocalOrRemotePath> {
        self.working_directory.as_ref()
    }

    /// The categories available for the current gates, in display order.
    ///
    /// A single-element result means the menu should skip the category list
    /// entirely and open straight into that category.
    pub fn available_categories(&self, app: &AppContext) -> Vec<AIContextMenuCategory> {
        let show_warp_drive = WarpDriveSettings::is_warp_drive_enabled(app);
        let is_active_dir_in_git_repo = self.is_working_directory_in_git_repo(app);
        let AtContextMenuGates {
            is_ai_or_autodetect_mode,
            is_shared_session_viewer,
            is_in_ambient_agent,
            is_cli_agent_input,
        } = self.gates;

        // For CLI agent input, use a positive allowlist of categories that CLI agents
        // can interpret. This is safer than a blocklist because new categories added
        // to the enum in the future won't accidentally leak into the CLI agent menu.
        if is_cli_agent_input {
            let mut categories = vec![];
            if !is_shared_session_viewer {
                categories.push(self.files_category(is_active_dir_in_git_repo));
            }
            if self.is_code_category_enabled(is_active_dir_in_git_repo, app)
                && !is_shared_session_viewer
            {
                categories.push(AIContextMenuCategory::Code);
            }
            return categories;
        }

        // For ambient agent sessions, only show limited categories
        if is_in_ambient_agent {
            let mut categories = vec![];
            if show_warp_drive {
                if FeatureFlag::DriveObjectsAsContext.is_enabled() {
                    categories.push(AIContextMenuCategory::Workflows);
                    categories.push(AIContextMenuCategory::Notebooks);
                    categories.push(AIContextMenuCategory::Plans);
                }
                categories.push(AIContextMenuCategory::Rules);
            }
            return categories;
        }

        if is_ai_or_autodetect_mode {
            let mut categories = vec![];

            // Hide file options for shared session viewers
            if !is_shared_session_viewer {
                categories.push(self.files_category(is_active_dir_in_git_repo));
            }

            if FeatureFlag::AIContextMenuCommands.is_enabled() {
                categories.push(AIContextMenuCategory::Commands);
            }
            categories.push(AIContextMenuCategory::Blocks);
            if self.is_code_category_enabled(is_active_dir_in_git_repo, app)
                && !is_shared_session_viewer
            {
                categories.push(AIContextMenuCategory::Code);
            }
            if show_warp_drive && FeatureFlag::DriveObjectsAsContext.is_enabled() {
                categories.push(AIContextMenuCategory::Workflows);
                categories.push(AIContextMenuCategory::Notebooks);
                categories.push(AIContextMenuCategory::Plans);
            }
            if FeatureFlag::DiffSetAsContext.is_enabled()
                && is_active_dir_in_git_repo
                && !is_shared_session_viewer
            {
                categories.push(AIContextMenuCategory::DiffSet);
            }
            if FeatureFlag::ConversationsAsContext.is_enabled() {
                categories.push(AIContextMenuCategory::Conversations);
            }
            if show_warp_drive {
                categories.push(AIContextMenuCategory::Rules);
            }
            categories.push(AIContextMenuCategory::Skills);
            categories
        } else if !is_shared_session_viewer {
            // Terminal mode: show Files and Code categories (when enabled)
            let mut categories = vec![self.files_category(is_active_dir_in_git_repo)];

            // Also show Code category in terminal mode when enabled
            if self.is_code_category_enabled(is_active_dir_in_git_repo, app) {
                categories.push(AIContextMenuCategory::Code);
            }

            categories
        } else {
            // File searching is not available in shared session viewers
            vec![]
        }
    }

    fn files_category(&self, is_active_dir_in_git_repo: bool) -> AIContextMenuCategory {
        if is_active_dir_in_git_repo {
            AIContextMenuCategory::RepoFiles
        } else {
            AIContextMenuCategory::CurrentFolderFiles
        }
    }

    fn is_code_category_enabled(&self, is_active_dir_in_git_repo: bool, app: &AppContext) -> bool {
        FeatureFlag::AIContextMenuCode.is_enabled()
            && *InputSettings::as_ref(app)
                .outline_codebase_symbols_for_at_context_menu
                .value()
            && is_active_dir_in_git_repo
    }

    #[cfg(not(target_family = "wasm"))]
    fn is_working_directory_in_git_repo(&self, app: &AppContext) -> bool {
        self.working_directory.as_ref().is_some_and(|dir| {
            DetectedRepositories::as_ref(app)
                .get_root_for_canonical_path(dir)
                .is_some()
        })
    }

    /// Repository detection is unavailable in WASM builds, where
    /// `DetectedRepositories` is never registered.
    #[cfg(target_family = "wasm")]
    fn is_working_directory_in_git_repo(&self, _app: &AppContext) -> bool {
        false
    }

    /// The subset of [`Self::available_categories`] whose names match the
    /// main-menu filter query.
    pub fn filtered_categories(&self, app: &AppContext) -> Vec<AIContextMenuCategory> {
        let categories = self.available_categories(app);
        if self.main_menu_query.is_empty() {
            return categories;
        }
        let query_lower = self.main_menu_query.trim().to_lowercase();
        categories
            .into_iter()
            .filter(|category| category.name().to_lowercase().contains(&query_lower))
            .collect()
    }

    /// Resets category-derived state after the gates or working directory
    /// change, opening straight into the only category when just one is
    /// available.
    pub fn refresh_categories(&mut self, app: &AppContext) {
        let categories = self.available_categories(app);

        // Zero categories has no single destination to open into, and more than
        // one needs the list, so both land on the main menu.
        self.navigation_state = match categories.as_slice() {
            [only] => NavigationState::Category(*only),
            _ => NavigationState::MainMenu,
        };
        self.clear_category_filter();
    }

    /// Returns to the category list, which is only worth showing when more than
    /// one category is available. Returns whether that list is showable, so
    /// callers can skip work that only matters once it is visible.
    pub fn return_to_main_menu_if_multiple_categories(&mut self, app: &AppContext) -> bool {
        if self.available_categories(app).len() <= 1 {
            return false;
        }
        self.navigation_state = NavigationState::MainMenu;
        true
    }

    /// Clears the category-list filter query and its selection.
    pub fn clear_category_filter(&mut self) {
        self.main_menu_query = String::new();
        self.selected_category_index = 0;
    }

    /// Moves the category selection down the filtered list, wrapping at the end.
    pub fn select_next_category(&mut self, app: &AppContext) {
        let count = self.filtered_categories(app).len();
        if count == 0 {
            return;
        }
        self.selected_category_index = (self.selected_category_index + 1) % count;
    }

    /// Moves the category selection up the filtered list, wrapping at the start.
    pub fn select_previous_category(&mut self, app: &AppContext) {
        let count = self.filtered_categories(app).len();
        if count == 0 {
            return;
        }
        self.selected_category_index = self
            .selected_category_index
            .checked_sub(1)
            .unwrap_or(count - 1);
    }

    /// The category the main-menu selection currently points at.
    pub fn selected_category(&self, app: &AppContext) -> Option<AIContextMenuCategory> {
        self.filtered_categories(app)
            .get(self.selected_category_index)
            .copied()
    }

    /// Opens a category's result list.
    ///
    /// The category selection index is left alone: it is only meaningful while
    /// the category list is showing, and returning to that list resets it.
    pub fn enter_category(&mut self, category: AIContextMenuCategory) {
        self.navigation_state = NavigationState::Category(category);
        self.main_menu_query = String::new();
    }

    /// Absorbs a new filter query typed after the `@`.
    ///
    /// While the category list is showing, the query filters category names; if
    /// nothing matches, the menu falls through to searching every available
    /// category instead of showing an empty list.
    pub fn set_query(&mut self, query: &str, app: &AppContext) -> AtContextMenuQueryTransition {
        if self.navigation_state != NavigationState::MainMenu {
            return AtContextMenuQueryTransition::SourcesUnchanged;
        }

        self.main_menu_query = query.to_owned();

        let filtered_count = self.filtered_categories(app).len();
        if self.selected_category_index >= filtered_count {
            self.selected_category_index = 0;
        }
        if filtered_count > 0 {
            return AtContextMenuQueryTransition::SourcesUnchanged;
        }

        self.navigation_state = NavigationState::AllCategories;
        AtContextMenuQueryTransition::EnteredAllCategories
    }

    /// Absorbs a results update and returns whether the menu should close
    /// because the user has typed past several consecutive empty result sets.
    ///
    /// `query_grew` means the new query still contains the previous one, i.e.
    /// the user narrowed rather than rewrote. The category list is exempt: it is
    /// filtered locally and has no mixer sources, so empty mixer results there
    /// say nothing about the user's progress.
    #[must_use]
    pub fn record_results_update(
        &mut self,
        results_are_empty: bool,
        is_loading: bool,
        query_grew: bool,
    ) -> bool {
        let counts_as_no_progress = self.navigation_state != NavigationState::MainMenu
            && results_are_empty
            && !is_loading
            && query_grew;
        if counts_as_no_progress {
            self.consecutive_empty_results_events += 1;
        } else {
            self.consecutive_empty_results_events = 0;
        }
        self.consecutive_empty_results_events >= MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS
    }

    /// Clears the no-progress counter, for use when the menu closes.
    pub fn reset_results_progress(&mut self) {
        self.consecutive_empty_results_events = 0;
    }
}

/// Returns whether an `@` just typed at `cursor_position` sits in a position
/// that should open the context menu.
///
/// `cursor_position` is the offset immediately after the typed `@`. An `@` opens
/// the menu at the start of the buffer or after any non-alphanumeric character,
/// so `foo@bar` (an email or a package spec) does not trigger it.
///
/// Callers layer their own surface conditions on top; the GUI additionally
/// suppresses the menu for package-installer command lines in shell mode.
pub fn is_at_menu_trigger(buffer_text: &str, cursor_position: usize) -> bool {
    if cursor_position == 0 {
        return false;
    }

    if buffer_text
        .chars()
        .nth(cursor_position.saturating_sub(1))
        .is_none_or(|c| c != '@')
    {
        return false;
    }

    // '@' at the very start of the buffer is always a valid trigger.
    if cursor_position == 1 {
        return true;
    }

    buffer_text
        .chars()
        .nth(cursor_position.saturating_sub(2))
        .is_some_and(|c| !c.is_alphanumeric())
}

/// Returns whether an open menu anchored at `at_symbol_position` should close
/// given the current buffer and cursor.
///
/// The menu closes when the cursor moves behind the `@`, when a newline or any
/// non-space whitespace appears between the `@` and the cursor, or when the
/// filter accumulates two consecutive spaces — at which point the user is
/// writing prose rather than picking context.
pub fn should_close_at_menu(
    buffer_text: &str,
    cursor_position: usize,
    at_symbol_position: usize,
) -> bool {
    // If the cursor is to the left of the "@", we should close the AI context menu.
    if cursor_position < at_symbol_position {
        return true;
    }

    let chars_before_cursor: Vec<char> = buffer_text.chars().take(cursor_position).collect();
    let mut prev_char_was_space = false;
    for c in chars_before_cursor.into_iter().rev() {
        if c.is_whitespace() && c != ' ' {
            return true;
        }
        if c == '@' {
            return prev_char_was_space;
        }
        if c == ' ' {
            if prev_char_was_space {
                return true;
            }
            prev_char_was_space = true;
        } else {
            prev_char_was_space = false;
        }
    }
    true
}

/// The buffer text an accepted action inserts, for actions whose inserted
/// representation is identical on every surface.
///
/// Returns `None` for the two actions that need surface-specific handling:
/// [`AIContextMenuSearchableAction::InsertFilePath`], which the GUI rewrites
/// relative to the session's working directory when the input is locked to the
/// shell, and [`AIContextMenuSearchableAction::InsertDiffSet`], which attaches
/// context rather than inserting text.
pub fn shared_inserted_text(action: &AIContextMenuSearchableAction) -> Option<String> {
    match action {
        AIContextMenuSearchableAction::InsertText { text } => Some(text.clone()),
        AIContextMenuSearchableAction::InsertDriveObject {
            object_type,
            object_uid,
        } => Some(format!("<{object_type}:{object_uid}>")),
        AIContextMenuSearchableAction::InsertPlan { ai_document_uid } => {
            Some(format!("<plan:{ai_document_uid}>"))
        }
        AIContextMenuSearchableAction::InsertConversation { conversation_id } => {
            Some(format!("<convo:{conversation_id}>"))
        }
        AIContextMenuSearchableAction::InsertSkill { name } => Some(format!("/{name}")),
        AIContextMenuSearchableAction::InsertFilePath { .. }
        | AIContextMenuSearchableAction::InsertDiffSet { .. } => None,
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
