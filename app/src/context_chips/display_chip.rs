use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Flex, Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, Stack, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::fonts::{Cache, FamilyId, Properties, Weight};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EntityId, Gradient, ModelHandle, SingletonEntity, TypedActionView,
    View, ViewContext, ViewHandle,
};

use super::directory_fetcher::{
    DirectoryFetcher, DirectoryFetcherEvent, DirectoryItem, DirectoryType,
};
use super::display_menu::{
    ChipMenuType, DisplayChipMenu, FixedFooter, GenericMenuItem, PromptDisplayMenuEvent,
};
use super::{
    agent_view_chip_color, github_pr_display_text_from_url, render_text_from_kind, ChipResult,
    ChipValue, ContextChipKind,
};
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::prompt::plan_and_todo_list::{PlanAndTodoListEvent, PlanAndTodoListView};
use crate::ai::blocklist::{BlocklistAIContextModel, BlocklistAIInputModel};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::appearance::Appearance;
use crate::code::editor::{add_color, remove_color};
use crate::code_review::code_review_view::CODE_REVIEW_TOOLTIP_TEXT;
use crate::code_review::diff_state::DiffStats;
use crate::completer::SessionContext;
use crate::context_chips::git_branch_on_click::{
    is_plausible_new_branch_name, GitBranchOnClickValue,
};
use crate::context_chips::node_version_popup::{NodeVersionPopupEvent, NodeVersionPopupView};
use crate::context_chips::spacing;
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::terminal::model::session::SessionType;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::util::truncation::truncate_from_beginning;
use crate::view_components::action_button::{ActionButtonTheme, NakedTheme};
use crate::view_components::{FeaturePopup, NewFeaturePopupEvent, NewFeaturePopupLabel};
use crate::workspace::view::TOGGLE_RIGHT_PANEL_BINDING_NAME;

/// Helper function to render git diff stats content (file icon or +- icons, file count, bullet, +/- counts)
/// Used by both the context chips and the AI control panel
pub fn render_git_diff_stats_content(
    line_changes: &GitLineChanges,
    icon_size: f32,
    font_family: FamilyId,
    font_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let mut git_content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Determine if there are any changes
    let has_changes = line_changes.lines_added > 0 || line_changes.lines_removed > 0;

    // Add icon based on whether there are changes
    let icon_element = if has_changes {
        // Use file icon when there are changes
        Icon::File
            .to_warpui_icon(Fill::Solid(internal_colors::neutral_6(theme)))
            .finish()
    } else {
        // Use diff icon when there are no changes
        Icon::Diff
            .to_warpui_icon(Fill::Solid(internal_colors::neutral_6(theme)))
            .finish()
    };

    git_content.add_child(
        Container::new(
            ConstrainedBox::new(icon_element)
                .with_height(icon_size)
                .with_width(icon_size)
                .finish(),
        )
        .with_margin_right(4.)
        .finish(),
    );

    let mut counts_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Add file count
    counts_row.add_child(
        Text::new_inline(
            line_changes.files_changed.to_string(),
            font_family,
            font_size,
        )
        .with_color(Fill::Solid(internal_colors::neutral_6(theme)).into())
        .with_line_height_ratio(appearance.line_height_ratio())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish(),
    );

    // Only add bullet separator if there are changes to display
    if has_changes {
        counts_row.add_child(
            Text::new_inline(" • ", font_family, font_size)
                .with_color(Fill::Solid(internal_colors::neutral_6(theme)).into())
                .with_line_height_ratio(appearance.line_height_ratio())
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        );
    }

    // Display git line changes using the parsed struct data
    if line_changes.lines_added > 0 {
        // Add green text for additions
        counts_row.add_child(
            Text::new_inline(
                format!("+{}", line_changes.lines_added),
                font_family,
                font_size,
            )
            .with_color(add_color(appearance))
            .with_line_height_ratio(appearance.line_height_ratio())
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish(),
        );
    }

    if line_changes.lines_removed > 0 {
        // Add space if we already have additions
        if line_changes.lines_added > 0 {
            counts_row.add_child(Text::new_inline(" ", font_family, font_size).finish());
        }

        // Add red text for deletions
        counts_row.add_child(
            Text::new_inline(
                format!("-{}", line_changes.lines_removed),
                font_family,
                font_size,
            )
            .with_color(remove_color(appearance))
            .with_line_height_ratio(appearance.line_height_ratio())
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish(),
        );
    }
    git_content.add_child(counts_row.finish());

    git_content.finish()
}

fn git_branch_status_icon(icon: Icon, color: ColorU, icon_size: f32) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(color)).finish())
            .with_height(icon_size)
            .with_width(icon_size)
            .finish(),
    )
    .finish()
}

fn git_branch_status_text(
    text: String,
    color: ColorU,
    font_family: FamilyId,
    font_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Text::new_inline(text, font_family, font_size)
        .with_color(Fill::Solid(color).into())
        .with_line_height_ratio(appearance.line_height_ratio())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish()
}

fn git_branch_status_count(
    icon: Icon,
    count: String,
    color: ColorU,
    font_family: FamilyId,
    font_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    add_git_branch_status_child(
        &mut content,
        git_branch_status_icon(icon, color, GIT_BRANCH_STATUS_STATUS_ICON_SIZE),
        GIT_BRANCH_STATUS_COUNT_GAP,
    );
    content.add_child(git_branch_status_text(
        count,
        color,
        font_family,
        font_size,
        appearance,
    ));
    content.finish()
}

fn add_git_branch_status_child(content: &mut Flex, child: Box<dyn Element>, margin_right: f32) {
    content.add_child(
        Container::new(child)
            .with_margin_right(margin_right)
            .finish(),
    );
}

const PROMPT_CHIP_DISPLAY_ID: &str = "PromptChipDisplay";
const DROP_SHADOW_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 32,
};

const CHIP_MARGIN_RIGHT: f32 = 8.;
const UDI_CHIP_MAX_NUM_CHARACTERS: usize = 40;

const CHIP_CORNER_RADIUS: f32 = 4.0;
const GIT_BRANCH_STATUS_CHIP_HORIZONTAL_PADDING: f32 = 8.0;
const GIT_BRANCH_STATUS_CHIP_VERTICAL_PADDING: f32 = 4.0;
const GIT_BRANCH_STATUS_MAIN_GAP: f32 = 4.0;
const GIT_BRANCH_STATUS_COUNT_GAP: f32 = 2.0;
const GIT_BRANCH_STATUS_BRANCH_ICON_SIZE: f32 = 14.0;
const GIT_BRANCH_STATUS_STATUS_ICON_SIZE: f32 = 12.0;
pub(crate) const CHIP_BORDER_WIDTH: f32 = 1.0;
/// Inner rounded corners are 1px smaller than the outer border radius
const CHIP_INNER_CORNER_RADIUS: f32 = CHIP_CORNER_RADIUS - CHIP_BORDER_WIDTH;

/// Standard positioning for tooltip overlays on UDI chips
fn udi_tooltip_positioning() -> OffsetPositioning {
    OffsetPositioning::offset_from_parent(
        vec2f(0., -8.),
        ParentOffsetBounds::WindowByPosition,
        ParentAnchor::TopLeft,
        ChildAnchor::BottomLeft,
    )
}

/// Configuration for creating a unified UDI chip
pub(crate) struct UdiChipConfig {
    /// The icon to display
    icon: Option<Icon>,
    keystroke: Option<Keystroke>,
    /// The color for both icon and text
    color: ColorU,
    /// The text content to display
    text: String,
    /// Whether to truncate text to UDI_CHIP_MAX_NUM_CHARACTERS
    truncate_text: bool,
    border_override: Option<Border>,
    is_in_agent_view: bool,
    /// When `true`, the chip paints its hover background instead of its
    /// default background. The chip's background is opaque, so callers must
    /// toggle this from their `Hoverable` rather than wrap the chip in an
    /// outer container (which would be occluded).
    hovered: bool,
}

impl UdiChipConfig {
    pub(crate) fn new(color: ColorU, text: String) -> Self {
        Self {
            icon: None,
            keystroke: None,
            color,
            text,
            truncate_text: true,
            border_override: None,
            is_in_agent_view: false,
            hovered: false,
        }
    }

    pub(crate) fn new_with_icon(icon: Icon, color: ColorU, text: String) -> Self {
        Self {
            icon: Some(icon),
            keystroke: None,
            color,
            text,
            truncate_text: true,
            border_override: None,
            is_in_agent_view: false,
            hovered: false,
        }
    }

    fn new_with_keystroke(color: ColorU, text: String, keystroke: Keystroke) -> Self {
        Self {
            icon: None,
            keystroke: Some(keystroke),
            color,
            text,
            truncate_text: true,
            border_override: None,
            is_in_agent_view: false,
            hovered: false,
        }
    }

    pub(crate) fn with_truncate_text(mut self, truncate: bool) -> Self {
        self.truncate_text = truncate;
        self
    }

    pub(crate) fn with_border_override(mut self, border: Border) -> Self {
        self.border_override = Some(border);
        self
    }

    pub(crate) fn with_hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    fn for_agent_view(mut self) -> Self {
        self.is_in_agent_view = true;
        self
    }
}

#[derive(Debug, Clone)]
pub enum DisplayChipAction {
    CloseMenu,
    ToggleMenu,
    ToggleCodeReview,
    OpenBranchSelector,
    OpenGithubPullRequest(String),
}

pub struct DisplayChip {
    mouse_state: MouseStateHandle,
    diff_stats_mouse_state: MouseStateHandle,
    text: String,
    value: Option<ChipValue>,
    chip_kind: ContextChipKind,
    display_chip_kind: DisplayChipKind,
    next_chip_kind: Option<ContextChipKind>,
    on_click_values: Vec<String>,
    quota_reset_popup: ViewHandle<FeaturePopup>,
    session_context: Option<SessionContext>,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    agent_view_controller: ModelHandle<AgentViewController>,
    is_shared_session_viewer: bool,
    is_in_agent_view: bool,
    /// Optional because `DisplayChip` sometimes should be disabled, depending on if it is in an ambient agent view.
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    /// Cached display string for the code review keybinding.
    code_review_keybinding: Option<String>,
    /// The terminal view this chip belongs to, used to check CLI agent session state.
    terminal_view_id: EntityId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitLineChanges {
    pub files_changed: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
}

impl GitLineChanges {
    /// Convert GitDiffData to GitLineChanges
    pub fn from_diff_stats(diff_stats: &DiffStats) -> Self {
        Self {
            files_changed: diff_stats.files_changed as u32,
            lines_added: diff_stats.total_additions as u32,
            lines_removed: diff_stats.total_deletions as u32,
        }
    }

    /// Parse git diff --shortstat output into GitLineChanges struct
    /// Input example: " 1 file changed, 2 insertions(+), 17 deletions(-)"
    pub fn parse_from_git_output(raw_output: &str) -> Option<Self> {
        let line = raw_output.trim();

        if line.is_empty() {
            return None;
        }

        let mut files_changed = 0;
        let mut lines_added = 0;
        let mut lines_removed = 0;

        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            if let Ok(num) = word.parse::<u32>() {
                if let Some(next_word) = words.get(i + 1) {
                    if next_word.starts_with("file") {
                        files_changed = num;
                    } else if next_word.starts_with("insertion") {
                        lines_added = num;
                    } else if next_word.starts_with("deletion") {
                        lines_removed = num;
                    }
                }
            }
        }

        Some(Self {
            files_changed,
            lines_added,
            lines_removed,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitBranchTrackingStatus {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub counts_available: bool,
    #[serde(default)]
    pub rebased: bool,
}

impl GitBranchTrackingStatus {
    pub fn new(branch: String, upstream: Option<String>, ahead: u32, behind: u32) -> Self {
        let counts_available = upstream.is_some();
        Self {
            branch,
            upstream,
            ahead,
            behind,
            counts_available,
            rebased: false,
        }
    }

    pub fn without_counts(branch: String, upstream: Option<String>) -> Self {
        Self {
            branch,
            upstream,
            ahead: 0,
            behind: 0,
            counts_available: false,
            rebased: false,
        }
    }

    pub fn rebased(branch: String, upstream: String) -> Self {
        Self {
            branch,
            upstream: Some(upstream),
            ahead: 0,
            behind: 0,
            counts_available: true,
            rebased: true,
        }
    }

    pub fn from_display_text(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }

        let Some((branch, status_text)) = text.rsplit_once(" • ") else {
            return Some(Self {
                branch: text.to_string(),
                upstream: None,
                ahead: 0,
                behind: 0,
                counts_available: false,
                rebased: false,
            });
        };

        let Some((ahead, behind, rebased)) = Self::parse_display_status(status_text) else {
            return Some(Self {
                branch: text.to_string(),
                upstream: None,
                ahead: 0,
                behind: 0,
                counts_available: false,
                rebased: false,
            });
        };

        let branch = branch.trim();
        if branch.is_empty() {
            return None;
        }

        Some(Self {
            branch: branch.to_string(),
            upstream: None,
            ahead,
            behind,
            counts_available: true,
            rebased,
        })
    }

    pub fn display_text(&self) -> String {
        let mut parts = Vec::new();
        if self.is_rebased() {
            parts.push("⇅".to_string());
        } else {
            if let Some(ahead) = self.ahead_display_count() {
                parts.push(format!("↑{ahead}"));
            }
            if let Some(behind) = self.behind_display_count() {
                parts.push(format!("↓{behind}"));
            }
        }

        if parts.is_empty() {
            self.branch.clone()
        } else {
            format!("{} • {}", self.branch, parts.join(" "))
        }
    }

    pub fn is_rebased(&self) -> bool {
        self.counts_available && self.rebased
    }

    pub fn ahead_display_count(&self) -> Option<String> {
        (!self.is_rebased() && self.counts_available && self.ahead > 0)
            .then(|| Self::format_display_count(self.ahead))
    }

    pub fn behind_display_count(&self) -> Option<String> {
        (!self.is_rebased() && self.counts_available && self.behind > 0)
            .then(|| Self::format_display_count(self.behind))
    }

    fn format_display_count(count: u32) -> String {
        const MAX_DISPLAY_COUNT: u32 = 999;
        if count > MAX_DISPLAY_COUNT {
            format!("{MAX_DISPLAY_COUNT}+")
        } else {
            count.to_string()
        }
    }

    fn parse_display_count(count: &str) -> Option<u32> {
        if let Some(capped_count) = count.strip_suffix('+') {
            capped_count.parse::<u32>().ok()?.checked_add(1)
        } else {
            count.parse::<u32>().ok()
        }
    }

    fn parse_display_status(status_text: &str) -> Option<(u32, u32, bool)> {
        let mut ahead = 0;
        let mut behind = 0;
        let mut rebased = false;
        let mut saw_status_token = false;

        for part in status_text.split_whitespace() {
            saw_status_token = true;
            if part == "⇅" {
                rebased = true;
            } else if let Some(ahead_count) = part.strip_prefix('↑') {
                ahead = Self::parse_display_count(ahead_count)?;
            } else if let Some(behind_count) = part.strip_prefix('↓') {
                behind = Self::parse_display_count(behind_count)?;
            } else {
                return None;
            }
        }

        saw_status_token.then_some((ahead, behind, rebased))
    }

    fn tooltip_text(&self) -> String {
        match &self.upstream {
            Some(upstream) if self.is_rebased() => {
                format!("Tracking {upstream} • branch was rebased")
            }
            Some(upstream) if self.counts_available => format!(
                "Tracking {upstream} • ahead {}, behind {}",
                self.ahead, self.behind
            ),
            Some(upstream) => {
                format!("Tracking {upstream}; ahead/behind counts are unavailable")
            }
            None if self.is_rebased() => {
                "Branch was rebased; upstream name is unavailable".to_string()
            }
            None if self.counts_available => format!(
                "Ahead {}, behind {}; upstream name is unavailable",
                self.ahead, self.behind
            ),
            None => "No upstream configured".to_string(),
        }
    }
}

impl std::fmt::Display for GitBranchTrackingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_text())
    }
}

#[derive(Debug, Clone)]
pub enum DisplayChipKind {
    Text,
    WorkingDirectory {
        show_menu: bool,
        menu_open: bool,
        menu: ViewHandle<DisplayChipMenu>,
        directory_fetcher: ModelHandle<DirectoryFetcher>,
    },
    Ssh,
    Subshell,
    VirtualEnvironment,
    CondaEnvironment,
    NodeVersion {
        popup_open: bool,
        popup: ViewHandle<crate::context_chips::node_version_popup::NodeVersionPopupView>,
    },
    AgentPlanAndTodoList {
        plan_and_todo_list: ViewHandle<PlanAndTodoListView>,
    },
    GitBranch {
        menu_open: bool,
        menu: ViewHandle<DisplayChipMenu>,
    },
    GitBranchStatus {
        tracking_status: Option<GitBranchTrackingStatus>,
    },
    GithubPullRequest,
    GitDiffStats {
        line_changes_info: Option<GitLineChanges>,
    },
}

impl DisplayChipKind {
    pub fn has_open_menu(&self) -> bool {
        match self {
            DisplayChipKind::WorkingDirectory { menu_open, .. } => *menu_open,
            DisplayChipKind::NodeVersion { popup_open, .. } => *popup_open,
            DisplayChipKind::GitBranch { menu_open, .. } => *menu_open,
            DisplayChipKind::GithubPullRequest
            | DisplayChipKind::GitBranchStatus { .. }
            | DisplayChipKind::GitDiffStats { .. }
            | DisplayChipKind::Text
            | DisplayChipKind::Ssh
            | DisplayChipKind::Subshell
            | DisplayChipKind::VirtualEnvironment
            | DisplayChipKind::CondaEnvironment
            | DisplayChipKind::AgentPlanAndTodoList { .. } => false,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum SelectAction {
    Prev,
    Next,
    Index(usize),
}

#[derive(Debug, Clone)]
pub enum MenuItemType {
    GitBranch,
    File,
    TextFile,
    Directory,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub name: String,
    pub item_type: MenuItemType,
}

/// Configuration for creating a DisplayChip
#[derive(Clone)]
pub struct DisplayChipConfig {
    pub ai_input_model: ModelHandle<BlocklistAIInputModel>,
    pub ai_context_model: ModelHandle<BlocklistAIContextModel>,
    pub terminal_view_id: EntityId,
    pub menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    pub session_context: Option<SessionContext>,
    pub current_repo_path: Option<PathBuf>,
    pub model_events: ModelHandle<ModelEventDispatcher>,
    pub is_shared_session_viewer: bool,
    pub agent_view_controller: ModelHandle<AgentViewController>,
    /// Optional because `DisplayChip` sometimes should be disabled, depending on if it is in an ambient agent view.
    pub ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
}

#[derive(Debug, Clone)]
pub struct GitBranch(String);

impl GitBranch {
    fn prompt_chip_command(&self) -> PromptChipShellCommand {
        let branch = GitBranchOnClickValue::decode(&self.0);
        if let Some(worktree_path) = branch.worktree_path {
            return PromptChipShellCommand::ChangeDirectory {
                dir_name: worktree_path,
            };
        }

        if branch.is_linked_worktree {
            return PromptChipShellCommand::Echo {
                message: "The branch is already checked out in another worktree, but Warp couldn't find its path.",
            };
        }

        PromptChipShellCommand::GitCheckout {
            branch_name: branch.branch_name,
        }
    }

    fn icon_for_menu(&self) -> Icon {
        let branch = GitBranchOnClickValue::decode(&self.0);
        if branch.is_linked_worktree {
            Icon::Dataflow02
        } else {
            Icon::GitBranch
        }
    }
}

impl GenericMenuItem for GitBranch {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> String {
        GitBranchOnClickValue::decode(&self.0).branch_name
    }

    fn icon(&self, _app: &AppContext) -> Option<Icon> {
        Some(self.icon_for_menu())
    }

    fn action_data(&self) -> String {
        self.0.clone()
    }
}

/// Synthetic menu entry shown in the branch switcher when the user types a
/// query that does not match any existing branch. Selecting it runs
/// `git checkout -b <branch>` so the user can create the branch they were
/// about to switch to without leaving the picker.
#[derive(Debug, Clone)]
pub(crate) struct CreateGitBranch(String);

impl CreateGitBranch {
    pub(crate) fn new(branch_name: String) -> Self {
        Self(branch_name.trim().to_string())
    }

    pub(crate) fn branch_name(&self) -> &str {
        &self.0
    }

    fn prompt_chip_command(&self) -> PromptChipShellCommand {
        PromptChipShellCommand::GitCreateAndCheckoutBranch {
            branch_name: self.0.clone(),
        }
    }
}

impl GenericMenuItem for CreateGitBranch {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> String {
        format!("Create new branch \"{}\"", self.0)
    }

    fn icon(&self, _app: &AppContext) -> Option<Icon> {
        Some(Icon::Plus)
    }

    fn action_data(&self) -> String {
        self.0.clone()
    }
}

impl DisplayChip {
    /// Convert MenuPositioning to appropriate anchor pair for overlay positioning
    fn positioning_to_anchors(positioning: MenuPositioning) -> (ParentAnchor, ChildAnchor) {
        match positioning {
            MenuPositioning::BelowInputBox => (ParentAnchor::BottomLeft, ChildAnchor::TopLeft),
            MenuPositioning::AboveInputBox => (ParentAnchor::TopLeft, ChildAnchor::BottomLeft),
        }
    }

    pub fn new(
        ctx: &mut ViewContext<Self>,
        chip_result: ChipResult,
        next_chip_kind: Option<ContextChipKind>,
        config: DisplayChipConfig,
    ) -> Self {
        Self::new_internal(chip_result, next_chip_kind, config, false, ctx)
    }

    pub fn new_for_agent_view(
        chip_result: ChipResult,
        next_chip_kind: Option<ContextChipKind>,
        config: DisplayChipConfig,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_internal(chip_result, next_chip_kind, config, true, ctx)
    }

    fn new_internal(
        chip_result: ChipResult,
        next_chip_kind: Option<ContextChipKind>,
        config: DisplayChipConfig,
        is_in_agent_view: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Re-render this chip whenever Agent Mode state changes so UDI font/color updates
        // immediately on enter/exit.
        ctx.subscribe_to_model(&config.agent_view_controller, |_me, _model, _event, ctx| {
            ctx.notify();
        });

        let display_chip_kind = match chip_result.kind {
            ContextChipKind::AgentPlanAndTodoList => {
                let context_model = config.ai_context_model.clone();
                let view_id = config.terminal_view_id;
                let plan_and_todo_list = ctx.add_typed_action_view(|ctx| {
                    PlanAndTodoListView::new(
                        context_model,
                        config.menu_positioning_provider.clone(),
                        view_id,
                        is_in_agent_view,
                        ctx,
                    )
                });

                ctx.subscribe_to_view(&plan_and_todo_list, |_me, _, event, ctx| match event {
                    PlanAndTodoListEvent::OpenAIDocument {
                        document_id,
                        document_version,
                    } => {
                        ctx.emit(PromptDisplayChipEvent::OpenAIDocument {
                            document_id: *document_id,
                            document_version: *document_version,
                        });
                    }
                });

                DisplayChipKind::AgentPlanAndTodoList { plan_and_todo_list }
            }
            ContextChipKind::ShellGitBranch => {
                // Convert git branch strings to GitBranch items
                let git_branch_items: Vec<GitBranch> = chip_result
                    .on_click_values
                    .iter()
                    .map(|branch_name| GitBranch(branch_name.clone()))
                    .collect();

                let menu_view = ctx.add_typed_action_view(move |ctx| {
                    DisplayChipMenu::new(
                        git_branch_items,
                        None, // No fixed footer for git branches
                        ChipMenuType::Branches,
                        ctx,
                    )
                    .with_create_item_from_query(Arc::new(|query: &str| {
                        if !is_plausible_new_branch_name(query) {
                            return None;
                        }
                        let create_branch = CreateGitBranch::new(query.to_string());
                        let arc: Arc<dyn GenericMenuItem> = Arc::new(create_branch);
                        Some(arc)
                    }))
                });
                ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
                    PromptDisplayMenuEvent::MenuAction(generic_event) => {
                        let action_item = generic_event.action_item.as_any();
                        let command =
                            if let Some(git_branch) = action_item.downcast_ref::<GitBranch>() {
                                git_branch.prompt_chip_command()
                            } else if let Some(create_branch) =
                                action_item.downcast_ref::<CreateGitBranch>()
                            {
                                create_branch.prompt_chip_command()
                            } else {
                                log::warn!(
                                "MenuAction event should contain a GitBranch or CreateGitBranch \
                                 action item"
                            );
                                return;
                            };

                        ctx.emit(PromptDisplayChipEvent::TryExecuteCommand(command));
                        me.close_git_branch_menu(ctx);
                        ctx.notify();
                    }
                    PromptDisplayMenuEvent::CloseMenu => {
                        me.close_git_branch_menu(ctx);
                        ctx.emit(PromptDisplayChipEvent::ToggleMenu { open: false });
                        ctx.notify();
                    }
                });

                DisplayChipKind::GitBranch {
                    menu_open: false,
                    menu: menu_view,
                }
            }
            ContextChipKind::GitDiffStats => DisplayChipKind::GitDiffStats {
                line_changes_info: None,
            },
            ContextChipKind::GitBranchStatus => DisplayChipKind::GitBranchStatus {
                tracking_status: chip_result
                    .value
                    .as_ref()
                    .and_then(|value| value.as_git_branch_tracking_status())
                    .cloned(),
            },
            ContextChipKind::GithubPullRequest => DisplayChipKind::GithubPullRequest,
            ContextChipKind::WorkingDirectory => {
                let dir_path = chip_result
                    .value
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_default();

                let directory_fetcher = ctx.add_model(|ctx| {
                    DirectoryFetcher::new(dir_path.clone(), config.session_context.clone(), ctx)
                });

                let menu_view = ctx.add_typed_action_view(move |ctx| {
                    DisplayChipMenu::new(
                        Vec::<DirectoryItem>::new(),
                        Some(FixedFooter::new(Arc::new(DirectoryItem {
                            name: ".. (Parent Directory)".to_string(),
                            directory_type: DirectoryType::NavigateToParent,
                        }))), // Show parent directory option
                        ChipMenuType::Directories,
                        ctx,
                    )
                });

                // Subscribe to DirectoryFetcher events to update menu
                ctx.subscribe_to_model(
                    &directory_fetcher,
                    move |display_chip, model, event, ctx| {
                        match event {
                            DirectoryFetcherEvent::DirectoryContentsUpdated => {
                                // Update the existing menu with new directory contents
                                if let DisplayChipKind::WorkingDirectory { menu, .. } =
                                    &mut display_chip.display_chip_kind
                                {
                                    let new_files = model
                                        .read(ctx, |fetcher, _| fetcher.cached_files().to_vec());
                                    // Update the existing menu with new content instead of recreating it
                                    menu.update(ctx, |menu_view, menu_ctx| {
                                        // Update the menu items using the new method
                                        menu_view.update_menu_items(new_files, menu_ctx);
                                    });
                                }
                                ctx.notify();
                            }
                            DirectoryFetcherEvent::FetchStarted => {
                                log::debug!("Directory fetch started");
                            }
                            DirectoryFetcherEvent::FetchCompleted { success } => {
                                log::debug!("Directory fetch completed: success={success}");
                            }
                        }
                    },
                );

                ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
                    PromptDisplayMenuEvent::MenuAction(generic_event) => {
                        let action_item = generic_event.action_item.clone();
                        let Some(directory_item) =
                            action_item.as_any().downcast_ref::<DirectoryItem>()
                        else {
                            log::warn!("MenuAction event should contain DirectoryItem action item");
                            return;
                        };
                        match directory_item.directory_type {
                            DirectoryType::Directory => {
                                // For directories, navigate action is change directory
                                ctx.emit(PromptDisplayChipEvent::TryExecuteCommand(
                                    PromptChipShellCommand::ChangeDirectory {
                                        dir_name: directory_item.name.clone(),
                                    },
                                ));
                                me.close_working_directory_menu(ctx);
                                ctx.notify();
                            }
                            DirectoryType::TextFile => {
                                // For text files, secondary action is open in code editor
                                ctx.emit(PromptDisplayChipEvent::OpenTextFileInCodeEditor(
                                    directory_item.name.clone(),
                                ));
                                me.close_working_directory_menu(ctx);
                                ctx.notify();
                            }
                            DirectoryType::OtherFile => {
                                // For other files, primary action is default open
                                ctx.emit(PromptDisplayChipEvent::OpenFile(
                                    directory_item.name.clone(),
                                ));
                                me.close_working_directory_menu(ctx);
                                ctx.notify();
                            }
                            DirectoryType::NavigateToParent => {
                                ctx.emit(PromptDisplayChipEvent::TryExecuteCommand(
                                    PromptChipShellCommand::ChangeDirectory {
                                        dir_name: "..".to_string(),
                                    },
                                ));
                                me.close_working_directory_menu(ctx);
                                ctx.notify();
                            }
                        }
                    }
                    PromptDisplayMenuEvent::CloseMenu => {
                        me.close_working_directory_menu(ctx);
                        ctx.emit(PromptDisplayChipEvent::ToggleMenu { open: false });
                        ctx.notify();
                    }
                });

                DisplayChipKind::WorkingDirectory {
                    show_menu: true,
                    menu_open: false,
                    menu: menu_view,
                    directory_fetcher,
                }
            }
            ContextChipKind::Ssh => DisplayChipKind::Ssh,
            ContextChipKind::Subshell => DisplayChipKind::Subshell,
            ContextChipKind::VirtualEnvironment => DisplayChipKind::VirtualEnvironment,
            ContextChipKind::CondaEnvironment => DisplayChipKind::CondaEnvironment,
            ContextChipKind::NodeVersion => {
                let current_version = chip_result.value.as_ref().map(|v| v.to_string());
                let model_events = &config.model_events;
                let popup_view = ctx.add_typed_action_view(move |ctx| {
                    NodeVersionPopupView::new(current_version, model_events, ctx)
                });

                ctx.subscribe_to_view(&popup_view, |me, _, event, ctx| match event {
                    NodeVersionPopupEvent::Close => {
                        me.close_node_version_popup(ctx);
                        ctx.focus_self();
                    }
                    NodeVersionPopupEvent::SelectVersion { version } => {
                        ctx.emit(PromptDisplayChipEvent::TryExecuteCommand(
                            PromptChipShellCommand::NvmUse {
                                version: version.clone(),
                            },
                        ));
                        me.close_node_version_popup(ctx);
                        ctx.focus_self();
                    }
                    NodeVersionPopupEvent::InstallNvm => {
                        ctx.emit(PromptDisplayChipEvent::RunAgentQuery(if cfg!(windows) {
                            // nvm-windows has documented issues when installed alongside an existing Node.js installation.
                            // https://github.com/coreybutler/nvm-windows?tab=readme-ov-file#star-star-uninstall-any-pre-existing-node-installations-star-star
                            // Prompt the agent to remove this first.
                            "Uninstall existing Node.js installation and install nvm for me"
                                .to_string()
                        } else {
                            "Install nvm for me".to_string()
                        }));
                        me.close_node_version_popup(ctx);
                    }
                    NodeVersionPopupEvent::InstallLatestNodeVersion => {
                        ctx.emit(PromptDisplayChipEvent::TryExecuteCommand(
                            PromptChipShellCommand::NvmInstallLatestNode,
                        ));
                        me.close_node_version_popup(ctx);
                    }
                });

                DisplayChipKind::NodeVersion {
                    popup_open: false,
                    popup: popup_view,
                }
            }
            _ => DisplayChipKind::Text,
        };

        let quota_reset_popup = ctx.add_typed_action_view(|_| {
            FeaturePopup::alert_icon(NewFeaturePopupLabel::FromString(
                "Monthly AI credits reset!".to_string(),
            ))
        });

        ctx.subscribe_to_view(&quota_reset_popup, |_, _, event, ctx| match event {
            NewFeaturePopupEvent::Dismissed => {
                AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                    ai_settings.mark_quota_banner_as_dismissed(ctx);
                    ctx.notify();
                });
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::AIRequestQuotaInfoSetting { .. }
            ) {
                ctx.notify();
            }
        });

        // Subscribe to ambient agent model changes to re-render when the state changes
        if let Some(ref ambient_agent_model) = config.ambient_agent_view_model {
            ctx.subscribe_to_model(ambient_agent_model, |_, _, _, ctx| {
                ctx.notify();
            });
        }

        // Cache the code review keybinding and subscribe to changes.
        let code_review_keybinding =
            keybinding_name_to_display_string(TOGGLE_RIGHT_PANEL_BINDING_NAME, ctx);
        ctx.subscribe_to_model(
            &KeybindingChangedNotifier::handle(ctx),
            |me, _, event, ctx| {
                let KeybindingChangedEvent::BindingChanged {
                    binding_name,
                    new_trigger,
                } = event;
                if binding_name == TOGGLE_RIGHT_PANEL_BINDING_NAME {
                    me.code_review_keybinding = new_trigger.as_ref().map(|k| k.displayed());
                    ctx.notify();
                }
            },
        );

        Self {
            mouse_state: Default::default(),
            diff_stats_mouse_state: Default::default(),
            text: chip_result
                .value
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            value: chip_result.value,
            chip_kind: chip_result.kind,
            display_chip_kind,
            next_chip_kind,
            on_click_values: chip_result.on_click_values,
            quota_reset_popup,
            session_context: config.session_context,
            menu_positioning_provider: config.menu_positioning_provider,
            is_shared_session_viewer: config.is_shared_session_viewer,
            agent_view_controller: config.agent_view_controller.clone(),
            is_in_agent_view,
            ambient_agent_view_model: config.ambient_agent_view_model,
            code_review_keybinding,
            terminal_view_id: config.terminal_view_id,
        }
    }

    /// Returns `true` when a CLI agent session is active for this chip's terminal,
    /// meaning interactive behaviors (menus, hover, click) should be suppressed.
    fn is_cli_agent_session_active(&self, app: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(app)
            .session(self.terminal_view_id)
            .is_some()
    }

    fn close_node_version_popup(&mut self, ctx: &mut ViewContext<'_, DisplayChip>) {
        if let DisplayChipKind::NodeVersion { popup_open, .. } = &mut self.display_chip_kind {
            *popup_open = false;
            ctx.notify();
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn value(&self) -> Option<&ChipValue> {
        self.value.as_ref()
    }

    pub fn chip_kind(&self) -> &ContextChipKind {
        &self.chip_kind
    }

    pub fn display_chip_kind(&self) -> &DisplayChipKind {
        &self.display_chip_kind
    }

    pub(crate) fn on_click_values(&self) -> &[String] {
        &self.on_click_values
    }

    pub fn close_git_branch_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if let DisplayChipKind::GitBranch { menu_open, menu } = &mut self.display_chip_kind {
            *menu_open = false;
            menu.update(ctx, |menu, _| {
                menu.reset_selected_index();
            });
        }
    }

    pub fn close_working_directory_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if let DisplayChipKind::WorkingDirectory {
            menu_open, menu, ..
        } = &mut self.display_chip_kind
        {
            *menu_open = false;

            menu.update(ctx, |menu, _| {
                menu.reset_selected_index();
            });
        }
    }

    /// Try to focus an open menu if this chip has one. Returns true if a menu was focused.
    pub fn try_focus_open_menu(&self, ctx: &mut ViewContext<Self>) -> bool {
        match &self.display_chip_kind {
            DisplayChipKind::WorkingDirectory {
                menu_open, menu, ..
            } => {
                if *menu_open {
                    ctx.focus(menu);
                    return true;
                }
            }
            DisplayChipKind::GitBranch { menu_open, menu } => {
                if *menu_open {
                    ctx.focus(menu);
                    return true;
                }
            }
            DisplayChipKind::GitDiffStats { .. }
            | DisplayChipKind::GitBranchStatus { .. }
            | DisplayChipKind::Text
            | DisplayChipKind::Ssh
            | DisplayChipKind::Subshell
            | DisplayChipKind::VirtualEnvironment
            | DisplayChipKind::CondaEnvironment
            | DisplayChipKind::NodeVersion { .. }
            | DisplayChipKind::AgentPlanAndTodoList { .. }
            | DisplayChipKind::GithubPullRequest => {}
        }
        false
    }

    pub fn maybe_set_git_line_changes_info(
        &mut self,
        git_line_changes_info: Option<GitLineChanges>,
    ) {
        if let DisplayChipKind::GitDiffStats {
            line_changes_info, ..
        } = &mut self.display_chip_kind
        {
            *line_changes_info = git_line_changes_info;
        }
    }

    /// Update the session context (useful when it becomes available later)
    pub fn update_session_context(
        &mut self,
        session_context: Option<SessionContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.session_context = session_context.clone();

        // If this is a working directory chip, update the DirectoryFetcher's session context
        if let DisplayChipKind::WorkingDirectory {
            directory_fetcher, ..
        } = &self.display_chip_kind
        {
            directory_fetcher.update(ctx, |fetcher, model_ctx| {
                fetcher.update_session_context(session_context, model_ctx);
            });
        }
    }

    pub fn em_width(&self, font_cache: &Cache, appearance: &Appearance) -> f32 {
        font_cache.em_width(
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
    }

    fn render_keystroke_with_label(
        keystroke: &Option<Keystroke>,
        label: impl Into<Cow<'static, str>>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let prompt_height = app.font_cache().line_height(
            appearance.monospace_font_size(),
            DEFAULT_UI_LINE_HEIGHT_RATIO,
        );
        let theme = appearance.theme();
        let keybinding_style = UiComponentStyles {
            height: Some(prompt_height),
            width: Some(prompt_height),
            font_size: Some(appearance.monospace_font_size() - 4.),
            font_color: Some(blended_colors::text_main(theme, theme.surface_2())),
            background: Some(theme.surface_2().into()),
            foreground: Some(theme.foreground().into()),
            padding: Some(Coords::default()),
            margin: Some(Coords {
                left: 2.,
                right: 2.,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(keystroke) = keystroke {
            row.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .keyboard_shortcut(keystroke)
                        .with_style(keybinding_style)
                        .build()
                        .finish(),
                )
                .finish(),
            );
        }
        row.add_child(
            Text::new_inline(
                label,
                appearance.monospace_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(blended_colors::text_disabled(theme, theme.background()))
            .finish(),
        );
        row.finish()
    }

    pub fn should_render(&self, app: &AppContext) -> bool {
        match &self.display_chip_kind {
            DisplayChipKind::AgentPlanAndTodoList { plan_and_todo_list } => {
                plan_and_todo_list.as_ref(app).should_render(app)
            }
            _ => true,
        }
    }

    fn git_branch_chip(
        &self,
        menu_open: bool,
        menu: &ViewHandle<DisplayChipMenu>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let font_color = if self.is_in_agent_view {
            agent_view_chip_color(appearance)
        } else {
            appearance.theme().ansi_fg_green()
        };

        let is_interactive =
            !self.is_shared_session_viewer && !self.is_cli_agent_session_active(app);
        let is_in_agent_view = self.is_in_agent_view;
        let chip_text = self.text.clone();
        let hover = Hoverable::new(self.mouse_state.clone(), move |state| {
            let hovered = state.is_hovered() && is_interactive;
            let mut config =
                UdiChipConfig::new_with_icon(Icon::GitBranch, font_color, chip_text.clone())
                    .with_hovered(hovered);
            if is_in_agent_view {
                config = config.for_agent_view();
            }
            let chip_element = render_udi_chip(config, appearance);

            let mut stack = Stack::new().with_child(chip_element);
            if state.is_hovered() && is_interactive && !menu_open {
                let tool_tip = appearance
                    .ui_builder()
                    .tool_tip("Change git branch".to_string())
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(tool_tip, udi_tooltip_positioning());
            }
            stack.finish()
        });

        let hover = if !is_interactive {
            hover.finish()
        } else {
            hover
                .on_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(DisplayChipAction::OpenBranchSelector);
                })
                .with_cursor(Cursor::PointingHand)
                .finish()
        };

        let mut stack = Stack::new().with_child(hover);

        if menu_open {
            let positioning = self.menu_positioning_provider.menu_position(app);
            let (parent_anchor, child_anchor) = Self::positioning_to_anchors(positioning);
            let offset = match positioning {
                MenuPositioning::BelowInputBox => vec2f(0., 4.),
                MenuPositioning::AboveInputBox => vec2f(0., -4.),
            };
            stack.add_positioned_overlay_child(
                ChildView::new(menu).finish(),
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::WindowByPosition,
                    parent_anchor,
                    child_anchor,
                ),
            );
        }

        stack.finish()
    }

    fn github_pull_request_chip(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let font_color = if self.is_in_agent_view {
            agent_view_chip_color(appearance)
        } else {
            appearance.theme().ansi_fg_green()
        };
        let chip_text =
            github_pr_display_text_from_url(&self.text).unwrap_or_else(|| self.text.clone());
        let url = self.text.clone();
        let is_in_agent_view = self.is_in_agent_view;

        let hover = Hoverable::new(self.mouse_state.clone(), move |state| {
            let mut config =
                UdiChipConfig::new_with_icon(Icon::Github, font_color, chip_text.clone())
                    .with_hovered(state.is_hovered());
            if is_in_agent_view {
                config = config.for_agent_view();
            }
            let chip_element = render_udi_chip(config, appearance);

            let mut stack = Stack::new().with_child(chip_element);
            if state.is_hovered() {
                let tool_tip = appearance
                    .ui_builder()
                    .tool_tip("View pull request".to_string())
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(tool_tip, udi_tooltip_positioning());
            }
            stack.finish()
        });

        hover
            .on_click(move |ctx, _app, _position| {
                if !url.trim().is_empty() {
                    ctx.dispatch_typed_action(DisplayChipAction::OpenGithubPullRequest(
                        url.clone(),
                    ));
                }
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    fn git_branch_status_chip(
        &self,
        tracking_status: &Option<GitBranchTrackingStatus>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_color = internal_colors::neutral_6(theme);
        let font_family = appearance.ui_font_family();
        let font_size = udi_font_size(appearance);
        let fallback_branch = self.text.clone();
        let tracking_status = tracking_status
            .clone()
            .or_else(|| GitBranchTrackingStatus::from_display_text(&self.text));
        let tooltip_text = tracking_status
            .as_ref()
            .map(GitBranchTrackingStatus::tooltip_text);

        Hoverable::new(self.mouse_state.clone(), move |state| {
            let branch = tracking_status
                .as_ref()
                .map(|status| status.branch.clone())
                .unwrap_or_else(|| fallback_branch.clone());

            let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            add_git_branch_status_child(
                &mut content,
                git_branch_status_icon(
                    Icon::GitBranch,
                    font_color,
                    GIT_BRANCH_STATUS_BRANCH_ICON_SIZE,
                ),
                GIT_BRANCH_STATUS_MAIN_GAP,
            );
            add_git_branch_status_child(
                &mut content,
                git_branch_status_text(branch, font_color, font_family, font_size, appearance),
                GIT_BRANCH_STATUS_MAIN_GAP,
            );

            if let Some(status) = tracking_status.as_ref() {
                let show_rebased = status.is_rebased();
                let ahead = status.ahead_display_count();
                let behind = status.behind_display_count();
                if show_rebased || ahead.is_some() || behind.is_some() {
                    add_git_branch_status_child(
                        &mut content,
                        git_branch_status_text(
                            "•".to_string(),
                            font_color,
                            font_family,
                            font_size,
                            appearance,
                        ),
                        GIT_BRANCH_STATUS_MAIN_GAP,
                    );
                }

                if show_rebased {
                    content.add_child(git_branch_status_icon(
                        Icon::SwitchVertical02,
                        font_color,
                        GIT_BRANCH_STATUS_STATUS_ICON_SIZE,
                    ));
                } else {
                    if let Some(ahead) = ahead {
                        add_git_branch_status_child(
                            &mut content,
                            git_branch_status_count(
                                Icon::ArrowUp,
                                ahead,
                                font_color,
                                font_family,
                                font_size,
                                appearance,
                            ),
                            GIT_BRANCH_STATUS_MAIN_GAP,
                        );
                    }
                    if let Some(behind) = behind {
                        content.add_child(git_branch_status_count(
                            Icon::ArrowDown,
                            behind,
                            font_color,
                            font_family,
                            font_size,
                            appearance,
                        ));
                    }
                }
            }

            let mut chip_element = Container::new(content.finish())
                .with_background(theme.surface_1())
                .with_border(
                    Border::all(CHIP_BORDER_WIDTH)
                        .with_border_color(internal_colors::neutral_3(theme)),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CHIP_CORNER_RADIUS)))
                .with_vertical_padding(GIT_BRANCH_STATUS_CHIP_VERTICAL_PADDING)
                .with_horizontal_padding(GIT_BRANCH_STATUS_CHIP_HORIZONTAL_PADDING);
            if state.is_hovered() {
                chip_element = chip_element.with_background(theme.surface_2());
            }

            let mut stack = Stack::new().with_child(chip_element.finish());
            if state.is_hovered() {
                if let Some(tooltip_text) = tooltip_text.clone() {
                    let tool_tip = appearance
                        .ui_builder()
                        .tool_tip(tooltip_text)
                        .build()
                        .finish();
                    stack.add_positioned_overlay_child(tool_tip, udi_tooltip_positioning());
                }
            }
            stack.finish()
        })
        .finish()
    }

    fn git_diff_stats_chip(
        &self,
        line_changes_info: &Option<GitLineChanges>,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let Some(line_changes_info) = line_changes_info else {
            return None;
        };

        if self.is_shared_session_viewer {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let udi_icon_size = udi_icon_size(appearance, app);
        let font_size = udi_font_size(appearance);
        let font_family = if self.is_in_agent_view {
            appearance.ui_font_family()
        } else if FeatureFlag::AgentView.is_enabled() {
            appearance.monospace_font_family()
        } else {
            appearance.ui_font_family()
        };

        let git_diff_stats_content = render_git_diff_stats_content(
            line_changes_info,
            udi_icon_size,
            font_family,
            font_size,
            appearance,
        );

        // Code review is only supported on local sessions and
        // on remote sessions with a connected host ID.
        let supports_code_review = self
            .session_context
            .as_ref()
            .map(|ctx| match ctx.session.session_type() {
                SessionType::Local => true,
                SessionType::WarpifiedRemote { host_id: Some(_) } => {
                    FeatureFlag::RemoteCodeReview.is_enabled()
                }
                SessionType::WarpifiedRemote { host_id: None } => false,
            })
            .unwrap_or(false);

        let diff_stats_display = if supports_code_review {
            // Get the keybinding for the tooltip
            let code_review_keybinding = self.code_review_keybinding.clone().unwrap_or_default();

            Hoverable::new(self.diff_stats_mouse_state.clone(), |state| {
                let base_container = Container::new(git_diff_stats_content)
                    .with_vertical_padding(2.)
                    .with_horizontal_padding(4.);

                let base_container = if state.is_hovered() {
                    base_container
                        .with_background(appearance.theme().surface_2())
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                            CHIP_INNER_CORNER_RADIUS,
                        )))
                        .finish()
                } else {
                    base_container.finish()
                };

                let mut stack = Stack::new().with_child(base_container);
                if state.is_hovered() {
                    let tool_tip = appearance
                        .ui_builder()
                        .tool_tip_with_sublabel(
                            CODE_REVIEW_TOOLTIP_TEXT.to_string(),
                            code_review_keybinding.clone(),
                        )
                        .build()
                        .finish();
                    stack.add_positioned_overlay_child(tool_tip, udi_tooltip_positioning());
                }
                stack.finish()
            })
            .on_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(DisplayChipAction::ToggleCodeReview);
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
        } else {
            Container::new(git_diff_stats_content)
                .with_vertical_padding(2.)
                .with_horizontal_padding(4.)
                .finish()
        };

        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(diff_stats_display)
            .finish();

        let button = Container::new(content)
            .with_background(theme.surface_1())
            .with_border(Border::all(1.0).with_border_color(internal_colors::neutral_3(theme)))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CHIP_CORNER_RADIUS)))
            .finish();

        Some(button)
    }

    fn working_directory_chip(
        &self,
        show_menu: bool,
        menu: &ViewHandle<DisplayChipMenu>,
        menu_open: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // Note: Menu contents are updated via model subscriptions when directory contents change
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Check if we're in an ambient agent conversation.
        // If so, the directory chip should be non-interactive.
        let is_in_active_ambient_agent = self
            .ambient_agent_view_model
            .as_ref()
            .map(|model| {
                let m = model.as_ref(app);
                m.is_ambient_agent() && !m.is_configuring_ambient_agent()
            })
            .unwrap_or(false);

        let mut stack = Stack::new();

        // Menu is only allowed when the caller requests it and we're not in an active ambient
        // agent session or CLI agent session.
        let is_cli_agent_active = self.is_cli_agent_session_active(app);
        let allow_show_menu = show_menu && !is_in_active_ambient_agent && !is_cli_agent_active;

        let button = if allow_show_menu {
            let chip_text = self.text.clone();
            let font_color = if self.is_in_agent_view {
                agent_view_chip_color(appearance)
            } else {
                theme.ansi_fg_cyan()
            };

            let is_in_agent_view = self.is_in_agent_view;
            Hoverable::new(self.mouse_state.clone(), move |state| {
                let hovered = !menu_open && state.is_hovered();
                let mut config =
                    UdiChipConfig::new_with_icon(Icon::Folder, font_color, chip_text.clone())
                        .with_hovered(hovered);
                if is_in_agent_view {
                    config = config.for_agent_view();
                }

                let chip_element = render_udi_chip(config, appearance);
                let mut stack = Stack::new().with_child(chip_element);

                if state.is_hovered() {
                    let tool_tip = appearance
                        .ui_builder()
                        .tool_tip("Change working directory".to_string())
                        .build()
                        .finish();

                    stack.add_positioned_overlay_child(tool_tip, udi_tooltip_positioning());
                }

                stack.finish()
            })
            .on_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(DisplayChipAction::ToggleMenu);
            })
            .with_cursor(Cursor::PointingHand)
            .finish()
        } else {
            // Non-interactive chip (either show_menu is false or in active ambient agent)
            let font_color = if self.is_in_agent_view {
                // Use disabled text color when in active ambient agent
                if is_in_active_ambient_agent {
                    theme
                        .disabled_text_color(blended_colors::neutral_1(theme).into())
                        .into_solid()
                } else {
                    // In agent view but the chip is non-interactive for reasons other than an active
                    // ambient agent session. Keep the normal agent-view subtext styling (not disabled).
                    agent_view_chip_color(appearance)
                }
            } else {
                theme.ansi_fg_cyan()
            };

            let chip_text = self.text.clone();
            let is_in_agent_view = self.is_in_agent_view;

            Hoverable::new(self.mouse_state.clone(), move |state| {
                let mut config =
                    UdiChipConfig::new_with_icon(Icon::Folder, font_color, chip_text.clone());
                if is_in_agent_view {
                    config = config.for_agent_view();
                }

                let chip_element = render_udi_chip(config, appearance);
                let mut stack = Stack::new().with_child(chip_element);

                if state.is_hovered() && !is_cli_agent_active {
                    let tool_tip = appearance
                        .ui_builder()
                        .tool_tip("Working directory".to_string())
                        .build()
                        .finish();

                    stack.add_positioned_overlay_child(tool_tip, udi_tooltip_positioning());
                }

                stack.finish()
            })
            .with_cursor(Cursor::Arrow)
            .finish()
        };

        stack.add_child(button);

        if menu_open {
            let positioning = self.menu_positioning_provider.menu_position(app);
            let (parent_anchor, child_anchor) = Self::positioning_to_anchors(positioning);

            let offset = match positioning {
                MenuPositioning::BelowInputBox => vec2f(0., 4.),
                MenuPositioning::AboveInputBox => vec2f(0., -4.),
            };
            stack.add_positioned_overlay_child(
                ChildView::new(menu).finish(),
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::WindowByPosition,
                    parent_anchor,
                    child_anchor,
                ),
            );
        }

        stack.finish()
    }

    fn ssh_chip(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let color = if self.is_in_agent_view {
            agent_view_chip_color(appearance)
        } else {
            appearance.theme().ansi_fg_blue()
        };

        let mut config = UdiChipConfig::new_with_icon(Icon::User, color, self.text.clone());
        if self.is_in_agent_view {
            config = config.for_agent_view();
        }
        render_udi_chip(config, appearance)
    }

    fn subshell_chip(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let color = if self.is_in_agent_view {
            agent_view_chip_color(appearance)
        } else {
            appearance.theme().ansi_fg_blue()
        };
        let mut config = UdiChipConfig::new_with_icon(Icon::Terminal, color, self.text.clone());
        if self.is_in_agent_view {
            config = config.for_agent_view();
        }

        render_udi_chip(config, appearance)
    }

    fn virtual_environment_chip(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let color = if self.is_in_agent_view {
            agent_view_chip_color(appearance)
        } else {
            appearance.theme().ansi_fg_yellow()
        };
        let mut config = UdiChipConfig::new_with_icon(Icon::Terminal, color, self.text.clone());
        if self.is_in_agent_view {
            config = config.for_agent_view();
        }

        render_udi_chip(config, appearance)
    }

    fn conda_environment_chip(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let color = if self.is_in_agent_view {
            agent_view_chip_color(appearance)
        } else {
            appearance.theme().ansi_fg_yellow()
        };
        let mut config = UdiChipConfig::new_with_icon(Icon::Terminal, color, self.text.clone());
        if self.is_in_agent_view {
            config = config.for_agent_view();
        }

        render_udi_chip(config, appearance)
    }

    fn node_version_chip(
        &self,
        popup: &ViewHandle<crate::context_chips::node_version_popup::NodeVersionPopupView>,
        popup_open: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let chip_text = self.text.clone();
        let is_in_agent_view = self.is_in_agent_view;
        let hoverable = Hoverable::new(self.mouse_state.clone(), move |state| {
            let color = if is_in_agent_view {
                agent_view_chip_color(appearance)
            } else {
                appearance.theme().ansi_fg_green()
            };
            let hovered = state.is_hovered() && !popup_open;
            let mut config = UdiChipConfig::new_with_icon(Icon::NodeJS, color, chip_text.clone())
                .with_hovered(hovered);
            if is_in_agent_view {
                config = config.for_agent_view();
            }
            render_udi_chip(config, appearance)
        })
        .on_click(|ctx, _app, _pos| {
            ctx.dispatch_typed_action(DisplayChipAction::ToggleMenu);
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let mut stack = Stack::new().with_child(hoverable);
        if popup_open {
            let positioning = self.menu_positioning_provider.menu_position(app);
            let (parent_anchor, child_anchor) = Self::positioning_to_anchors(positioning);
            let offset = match positioning {
                MenuPositioning::BelowInputBox => vec2f(0., 4.),
                MenuPositioning::AboveInputBox => vec2f(0., -4.),
            };
            stack.add_positioned_overlay_child(
                ChildView::new(popup).finish(),
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::WindowByPosition,
                    parent_anchor,
                    child_anchor,
                ),
            );
        }
        stack.finish()
    }

    fn render_chip(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(app);
        let font_family = if self.is_in_agent_view || !FeatureFlag::AgentView.is_enabled() {
            appearance.ui_font_family()
        } else {
            appearance.monospace_font_family()
        };
        let font_size = udi_font_size(appearance);

        match &self.display_chip_kind {
            DisplayChipKind::WorkingDirectory {
                show_menu,
                menu,
                menu_open,
                ..
            } => Some(self.working_directory_chip(*show_menu, menu, *menu_open, app)),
            DisplayChipKind::Ssh => Some(self.ssh_chip(app)),
            DisplayChipKind::Subshell => Some(self.subshell_chip(app)),
            DisplayChipKind::VirtualEnvironment => Some(self.virtual_environment_chip(app)),
            DisplayChipKind::NodeVersion { popup, popup_open } => {
                Some(self.node_version_chip(popup, *popup_open, app))
            }
            DisplayChipKind::CondaEnvironment => Some(self.conda_environment_chip(app)),
            DisplayChipKind::AgentPlanAndTodoList { plan_and_todo_list } => {
                Some(ChildView::new(plan_and_todo_list).finish())
            }
            DisplayChipKind::GitBranch { menu_open, menu } => {
                Some(self.git_branch_chip(*menu_open, menu, app))
            }
            DisplayChipKind::GitBranchStatus { tracking_status } => {
                Some(self.git_branch_status_chip(tracking_status, app))
            }
            DisplayChipKind::GithubPullRequest => Some(self.github_pull_request_chip(app)),
            DisplayChipKind::GitDiffStats { line_changes_info } => {
                self.git_diff_stats_chip(line_changes_info, app)
            }
            _ => {
                let mut text = Text::new_inline(String::new(), font_family, font_size)
                    .with_line_height_ratio(appearance.line_height_ratio());

                // This is temporary, until we design more generic chip formatting.
                // In sync with `ContextChipKind::display_value`.
                render_text_from_kind(
                    &mut text,
                    self.chip_kind.clone(),
                    self.text.clone(),
                    self.is_in_agent_view,
                    appearance,
                );

                Some(chip_container(text.finish(), None, appearance).finish())
            }
        }
    }
}

impl Entity for DisplayChip {
    type Event = PromptDisplayChipEvent;
}

impl View for DisplayChip {
    fn ui_name() -> &'static str {
        "DisplayChip"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if let Some(chip) = self.render_chip(app) {
            if self.is_in_agent_view {
                chip
            } else {
                Container::new(chip)
                    .with_margin_right(CHIP_MARGIN_RIGHT)
                    .finish()
            }
        } else {
            Empty::new().finish()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptChipShellCommand {
    GitCheckout {
        branch_name: String,
    },
    GitCreateAndCheckoutBranch {
        branch_name: String,
    },
    ChangeDirectory {
        dir_name: String,
    },
    NvmUse {
        version: String,
    },
    NvmInstallLatestNode,
    Echo {
        /// The message to echo.
        ///
        /// This is very intentionally a `&'static str` to ensure that the message is a compile-time constant.
        /// This is to prevent accidental injection of user input into the message.
        message: &'static str,
    },
}

pub enum PromptDisplayChipEvent {
    OpenFile(String),
    OpenTextFileInCodeEditor(String),
    ToggleMenu {
        open: bool,
    },
    OpenCodeReview,
    OpenConversationHistory,
    OpenCommandPaletteFiles,
    TryExecuteCommand(PromptChipShellCommand),
    RunAgentQuery(String),
    OpenAIDocument {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
}

impl TypedActionView for DisplayChip {
    type Action = DisplayChipAction;

    fn handle_action(&mut self, action: &DisplayChipAction, ctx: &mut ViewContext<Self>) {
        match action {
            DisplayChipAction::CloseMenu => match &mut self.display_chip_kind {
                DisplayChipKind::GitBranch { menu_open, menu } => {
                    *menu_open = false;
                    menu.update(ctx, |menu, _| {
                        menu.reset_selected_index();
                    });
                    ctx.notify();
                }
                DisplayChipKind::WorkingDirectory {
                    menu_open, menu, ..
                } => {
                    *menu_open = false;
                    menu.update(ctx, |menu, _| {
                        menu.reset_selected_index();
                    });
                    ctx.notify();
                }
                DisplayChipKind::Ssh
                | DisplayChipKind::Subshell
                | DisplayChipKind::VirtualEnvironment
                | DisplayChipKind::CondaEnvironment
                | DisplayChipKind::AgentPlanAndTodoList { .. }
                | DisplayChipKind::Text
                | DisplayChipKind::GithubPullRequest
                | DisplayChipKind::GitBranchStatus { .. }
                | DisplayChipKind::GitDiffStats { .. } => {}
                DisplayChipKind::NodeVersion { popup_open, .. } => {
                    *popup_open = false;
                    ctx.notify();
                }
            },
            DisplayChipAction::ToggleMenu => {
                // All ToggleMenu consumers (WorkingDirectory, GitBranch, NodeVersion)
                // route through shell commands (cd, git checkout, nvm use) that don't
                // work in CLI agent context, so we suppress all of them.
                if self.is_cli_agent_session_active(ctx) {
                    return;
                }
                match &mut self.display_chip_kind {
                    DisplayChipKind::GitBranch { menu, menu_open } => {
                        *menu_open = !*menu_open;
                        let is_menu_open = *menu_open;
                        if is_menu_open {
                            ctx.focus(menu);
                        } else {
                            menu.update(ctx, |menu, _| {
                                menu.reset_selected_index();
                            });
                        }
                        ctx.emit(PromptDisplayChipEvent::ToggleMenu { open: is_menu_open });
                        ctx.notify();
                    }
                    DisplayChipKind::WorkingDirectory {
                        menu,
                        menu_open,
                        directory_fetcher,
                        ..
                    } => {
                        *menu_open = !*menu_open;
                        let is_menu_open = *menu_open;
                        if is_menu_open {
                            // Explicitly refetch directory contents when menu opens
                            directory_fetcher.update(ctx, |fetcher, ctx| {
                                fetcher.refetch_directory(ctx);
                            });
                            ctx.focus(menu);
                        } else {
                            menu.update(ctx, |menu, _| {
                                menu.reset_selected_index();
                            });
                        }
                        ctx.emit(PromptDisplayChipEvent::ToggleMenu { open: is_menu_open });
                        ctx.notify();
                    }
                    DisplayChipKind::NodeVersion { popup, popup_open } => {
                        *popup_open = !*popup_open;
                        let is_open = *popup_open;
                        if is_open {
                            popup.update(ctx, |popup_view, popup_ctx| {
                                popup_view.focus_content(popup_ctx);
                            });
                        } else {
                            ctx.focus_self();
                        }
                        ctx.emit(PromptDisplayChipEvent::ToggleMenu { open: is_open });
                        ctx.notify();
                    }
                    _ => {}
                }
            }
            DisplayChipAction::ToggleCodeReview => {
                ctx.emit(PromptDisplayChipEvent::OpenCodeReview);
                ctx.notify();
            }
            DisplayChipAction::OpenBranchSelector => {
                // Delegate to the existing ToggleMenu action for branch selector
                self.handle_action(&DisplayChipAction::ToggleMenu, ctx);
            }
            DisplayChipAction::OpenGithubPullRequest(url) => {
                ctx.open_url(url);
            }
        }
    }
}

/// Button theme for hint buttons in the classic input prompt.
pub struct ClassicPromptChipHintButton;

impl ActionButtonTheme for ClassicPromptChipHintButton {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        NakedTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        // For consistency, always choose the text color based on the input background.
        appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid()
    }
}

/// Button theme for hint buttons in the UDI prompt chips.
pub struct UdiPromptChipHintButton;

impl ActionButtonTheme for UdiPromptChipHintButton {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        NakedTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        // For consistency, always choose the text color based on the input background.
        appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into_solid()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }
}

pub struct EnterAgentViewButton;

impl ActionButtonTheme for EnterAgentViewButton {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        Some(if hovered {
            internal_colors::fg_overlay_2(appearance.theme())
        } else {
            internal_colors::fg_overlay_1(appearance.theme())
        })
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance
            .theme()
            .main_text_color(appearance.theme().background())
            .into_solid()
    }

    fn border_gradient(&self, appearance: &Appearance) -> Option<(Vector2F, Vector2F, Gradient)> {
        Some((
            vec2f(0.0, 0.0),
            vec2f(3.0, 3.0),
            Gradient {
                start: appearance.theme().ansi_fg_magenta(),
                end: appearance.theme().ansi_fg_yellow(),
            },
        ))
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }
}

pub(crate) fn chip_container(
    content: Box<dyn Element>,
    border_override: Option<Border>,
    appearance: &Appearance,
) -> Container {
    let theme = appearance.theme();
    let border = border_override.unwrap_or(
        Border::all(CHIP_BORDER_WIDTH).with_border_color(internal_colors::neutral_3(theme)),
    );
    // Solid surface fill keeps the chip readable even when its parent isn't
    // `theme.background()` (for example, over an alt-screen CLI agent).
    Container::new(content)
        .with_background(theme.surface_1())
        .with_border(border)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CHIP_CORNER_RADIUS)))
        .with_vertical_padding(spacing::UDI_CHIP_VERTICAL_PADDING)
        .with_horizontal_padding(spacing::UDI_CHIP_HORIZONTAL_PADDING)
}

pub(crate) fn render_udi_chip(config: UdiChipConfig, appearance: &Appearance) -> Box<dyn Element> {
    let font_size = udi_font_size(appearance);
    let icon_size = font_size;

    let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if let Some(icon) = config.icon {
        content.add_child(
            Container::new(
                ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(config.color)).finish())
                    .with_height(icon_size)
                    .with_width(icon_size)
                    .finish(),
            )
            .with_margin_right(spacing::UDI_CHIP_ICON_GAP)
            .finish(),
        );
    }

    let display_text = if config.truncate_text {
        truncate_from_beginning(&config.text, UDI_CHIP_MAX_NUM_CHARACTERS)
    } else {
        config.text.clone()
    };

    let font_family = if config.is_in_agent_view || !FeatureFlag::AgentView.is_enabled() {
        appearance.ui_font_family()
    } else {
        appearance.monospace_font_family()
    };

    let mut rendered_text = Text::new_inline(display_text, font_family, font_size)
        .with_color(Fill::Solid(config.color).into())
        .with_line_height_ratio(appearance.line_height_ratio());

    if !config.is_in_agent_view {
        rendered_text = rendered_text.with_style(Properties::default().weight(Weight::Semibold))
    }

    content.add_child(rendered_text.finish());

    if let Some(keystroke) = config.keystroke {
        content.add_child(
            appearance
                .ui_builder()
                .keyboard_shortcut(&keystroke)
                .with_space_between_keys(4.)
                .with_style(UiComponentStyles {
                    font_size: Some(appearance.monospace_font_size() - 4.),
                    width: Some(appearance.monospace_font_size()),
                    height: Some(appearance.monospace_font_size()),
                    padding: Some(Coords::default()),
                    margin: Some(Coords::default().left(4.)),
                    ..Default::default()
                })
                .with_line_height_ratio(1.0)
                .build()
                .finish(),
        );
    }

    let mut container = chip_container(content.finish(), config.border_override, appearance);
    if config.hovered {
        container = container.with_background(appearance.theme().surface_2());
    }
    container.finish()
}

pub fn udi_font_size(appearance: &Appearance) -> f32 {
    appearance.monospace_font_size() - 1.
}

pub fn udi_icon_size(appearance: &Appearance, app: &AppContext) -> f32 {
    app.font_cache().line_height(
        appearance.monospace_font_size(),
        DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4,
    )
}

#[cfg(test)]
#[path = "display_chip_tests.rs"]
mod tests;
