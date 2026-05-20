use crate::ai::agent::AgentReviewCommentBatch;
use crate::code_review::code_review_header::HEADER_BUTTON_PADDING;
#[cfg(feature = "local_fs")]
use crate::code_review::code_review_view::CodeReviewAction;
use crate::code_review::code_review_view::{
    render_file_navigation_button, CodeReviewView, CONTENT_LEFT_MARGIN, CONTENT_RIGHT_MARGIN,
};
use crate::code_review::code_review_view::{CodeReviewCommentDebugState, CodeReviewViewEvent};
use crate::code_review::telemetry_event::CodeReviewContextDestination;
use crate::pane_group::pane::view::header::{components::HEADER_EDGE_PADDING, PANE_HEADER_HEIGHT};
use crate::pane_group::WorkingDirectoriesEvent;
use crate::pane_group::{Event as PaneGroupEvent, PaneGroup, WorkingDirectoriesModel};
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::input::MenuPositioning;
use crate::terminal::CLIAgent;
use crate::ui_components::{buttons::icon_button_with_color, icons};
use crate::util::bindings::{keybinding_name_to_display_string, CustomAction};
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::FileTarget;
use crate::view_components::action_button::{ActionButton, PaneHeaderTheme};
#[cfg(feature = "local_fs")]
use crate::view_components::action_button::{NakedTheme, TooltipAlignment};
use crate::view_components::{Dropdown, DropdownItem};
use crate::workspace::view::TOGGLE_RIGHT_PANEL_BINDING_NAME;
use crate::workspace::WorkspaceAction;
use crate::{
    appearance::{Appearance, AppearanceEvent},
    drive::panel::{MAX_SIDEBAR_WIDTH_RATIO, MIN_SIDEBAR_WIDTH},
    terminal::resizable_data::{ModalType, ResizableData},
};
use crate::{
    code::buffer_location::LocalOrRemotePath, code_review::diff_state::DiffStateModel,
    terminal::view::TerminalView,
};
use dunce::canonicalize;
use itertools::Itertools;
use pathfinder_color::ColorU;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use warp_core::features::FeatureFlag;
use warp_core::ui::Icon;
use warp_util::path::LineAndColumnArg;
use warpui::elements::{ChildAnchor, Empty, PositionedElementAnchor};
use warpui::keymap::EditableBinding;
use warpui::EntityId;
use warpui::{
    elements::{
        resizable_state_handle, Container, DragBarSide, Element, MainAxisSize, MouseStateHandle,
        Resizable, ResizableStateHandle,
    },
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle,
};
use warpui::{
    elements::{
        ChildView, Clipped, ConstrainedBox, CrossAxisAlignment, Flex, MainAxisAlignment,
        ParentElement, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    platform::Cursor,
    ui_components::components::UiComponent,
};

/// Describes which agent destination is available for sending review comments.
#[derive(Clone, Debug, PartialEq)]
pub enum ReviewDestination {
    /// No terminal is available to receive comments.
    None,
    /// A Warp agent terminal is available (input box visible, not executing).
    Warp,
    /// A CLI agent (e.g. Claude Code, Gemini) is running in a terminal.
    Cli(CLIAgent),
}

/// Result of attempting to submit review comments to a terminal.
pub enum ReviewSubmissionResult {
    Success {
        comment_count: usize,
        file_count: usize,
        destination: CodeReviewContextDestination,
    },
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReviewTerminalUnavailableReason {
    NoSelectedRepo,
    SessionPathUnavailable,
    SessionOutsideSelectedRepo,
    AIDisabled,
    TerminalExecuting,
    InputBoxNotVisible,
}

impl ReviewTerminalUnavailableReason {
    fn label(&self) -> &'static str {
        match self {
            Self::NoSelectedRepo => "no repo is selected for code review",
            Self::SessionPathUnavailable => "session cwd is unavailable or not local",
            Self::SessionOutsideSelectedRepo => "session cwd is not inside selected repo",
            Self::AIDisabled => "AI is disabled for Warp review destinations",
            Self::TerminalExecuting => "terminal is currently executing a command",
            Self::InputBoxNotVisible => "terminal input box is not visible",
        }
    }
}

#[derive(Debug)]
struct ReviewTerminalStatus {
    active_session_path: Option<PathBuf>,
    current_repo_path: Option<PathBuf>,
    active_cli_agent: Option<String>,
    is_executing: bool,
    is_input_box_visible: bool,
    unavailable_reasons: Vec<ReviewTerminalUnavailableReason>,
}
impl ReviewTerminalStatus {
    fn is_available(&self) -> bool {
        self.unavailable_reasons.is_empty()
    }
}

struct CodeReviewState {
    dropdown: ViewHandle<Dropdown<RightPanelAction>>,
    available_repos: Vec<PathBuf>,
    /// The repository path of the focused terminal
    focused_repo_path: Option<PathBuf>,
    /// The repository path of the repository selected in the dropdown
    selected_repo_path: Option<PathBuf>,
    /// Avoid showing the jump-to-repo button if the focused repo has not changed
    did_focused_repo_change: bool,
}

#[cfg(feature = "local_fs")]
struct CodeReviewSessionEnv {
    is_remote: bool,
    is_wsl: bool,
}

/// Resolve the repo-switcher dropdown's text color from the current theme.
/// Kept as a free function so the construction site and the
/// `AppearanceEvent::ThemeChanged` subscription compute the exact same color.
fn repo_dropdown_font_color(appearance: &Appearance) -> ColorU {
    appearance
        .theme()
        .sub_text_color(appearance.theme().background())
        .into_solid()
}

impl CodeReviewState {
    pub fn new(ctx: &mut ViewContext<RightPanelView>) -> Self {
        CodeReviewState {
            dropdown: ctx.add_typed_action_view(|ctx| {
                let (font_color, ui_font_size) = {
                    let appearance = Appearance::as_ref(ctx);
                    (
                        repo_dropdown_font_color(appearance),
                        appearance.ui_font_size(),
                    )
                };
                let mut dropdown = Dropdown::new(ctx);
                dropdown.set_menu_position(
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                    ctx,
                );
                dropdown.set_main_axis_size(MainAxisSize::Min, ctx);
                dropdown.set_font_color(font_color, ctx);
                dropdown.set_font_size(ui_font_size, ctx);
                dropdown.set_vertical_margin(0., ctx);
                dropdown.set_top_bar_height(warp_core::ui::icons::ICON_DIMENSIONS, ctx);
                dropdown.set_padding(HEADER_BUTTON_PADDING, ctx);

                // The font color above is derived from the active theme and
                // cached inside the dropdown. Without this subscription, the
                // cached value goes stale across light/dark switches and the
                // header label becomes unreadable on the new background
                // (e.g. white-on-white in light mode after starting in dark).
                ctx.subscribe_to_model(&Appearance::handle(ctx), |dropdown, _, event, ctx| {
                    if matches!(event, AppearanceEvent::ThemeChanged) {
                        let font_color = repo_dropdown_font_color(Appearance::as_ref(ctx));
                        dropdown.set_font_color(font_color, ctx);
                    }
                });

                dropdown
            }),
            available_repos: vec![],
            selected_repo_path: None,
            focused_repo_path: None,
            did_focused_repo_change: false,
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn set_available_repos(
        &mut self,
        _repos: Vec<PathBuf>,
        _ctx: &mut ViewContext<RightPanelView>,
    ) {
    }

    #[cfg(feature = "local_fs")]
    fn set_available_repos(&mut self, repos: Vec<PathBuf>, ctx: &mut ViewContext<RightPanelView>) {
        let should_clear = self
            .selected_repo_path
            .as_ref()
            .map(|p| !repos.contains(p))
            .unwrap_or(false);
        if should_clear {
            self.selected_repo_path = None;
        }
        self.available_repos = repos;

        self.update_repo_dropdown(ctx);

        // Auto-select first repo if we have one and no selection yet
        if self.selected_repo_path.is_none() {
            if let Some(first_repo) = self.available_repos.first() {
                self.set_selected_repo(first_repo.clone(), ctx);
            }
        }
    }

    #[cfg(not(feature = "local_fs"))]
    pub fn set_selected_repo(
        &mut self,
        _repo_path: PathBuf,
        _ctx: &mut ViewContext<RightPanelView>,
    ) {
    }

    #[cfg(feature = "local_fs")]
    pub fn set_selected_repo(&mut self, repo_path: PathBuf, ctx: &mut ViewContext<RightPanelView>) {
        self.set_selected_repo_internal(repo_path, true, ctx);
    }

    pub fn set_focused_repo(
        &mut self,
        repo_path: Option<PathBuf>,
        ctx: &mut ViewContext<RightPanelView>,
    ) {
        self.did_focused_repo_change = true;
        self.focused_repo_path = repo_path;
        ctx.notify();
    }

    /// Internal method to set the selected repo with control over whether to update the dropdown.
    /// When `update_dropdown` is false, we skip updating the dropdown (useful when the change
    /// is coming from the dropdown itself to avoid circular updates).
    #[cfg(feature = "local_fs")]
    fn set_selected_repo_internal(
        &mut self,
        repo_path: PathBuf,
        update_dropdown: bool,
        ctx: &mut ViewContext<RightPanelView>,
    ) {
        if self.selected_repo_path.as_ref() == Some(&repo_path) {
            return;
        }

        self.did_focused_repo_change = false;
        self.selected_repo_path = Some(repo_path.clone());

        // Only update the dropdown if requested (not when selection came from dropdown itself)
        if update_dropdown {
            self.update_repo_dropdown(ctx);
        }

        ctx.notify();
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn get_repo_display_name(&self, repo_path: &Path) -> Option<String> {
        repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn update_repo_dropdown(&mut self, ctx: &mut ViewContext<RightPanelView>) {
        // Collect data before borrowing mutably
        let (items, selected_display_name) = {
            let items: Vec<DropdownItem<RightPanelAction>> = self
                .available_repos
                .iter()
                .map(|repo_path| {
                    let display_name = self
                        .get_repo_display_name(repo_path)
                        .unwrap_or_else(|| "Unknown".to_string());
                    DropdownItem::new(
                        display_name,
                        RightPanelAction::SelectRepo {
                            repo_path: repo_path.clone(),
                            from_dropdown: true,
                        },
                    )
                })
                .collect();

            let selected_display_name = self
                .selected_repo_path
                .as_ref()
                .and_then(|selected| self.get_repo_display_name(selected));

            (items, selected_display_name)
        };

        // Now update the dropdown
        if !items.is_empty() {
            self.dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_items(items, ctx);
                if let Some(display_name) = selected_display_name {
                    dropdown.set_selected_by_name(display_name, ctx);
                }
            });
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum RightPanelAction {
    ToggleFileSidebar,
    SelectRepo {
        repo_path: PathBuf,
        from_dropdown: bool,
    },
    OpenRepository,
    ToggleMaximize,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum RightPanelEvent {
    ToggleMaximize,
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    OpenFileInNewTab {
        path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
    },
    #[cfg(not(target_family = "wasm"))]
    OpenLspLogs {
        log_path: PathBuf,
    },
}

pub struct RightPanelView {
    resizable_state_handle: ResizableStateHandle,
    close_button_mouse_state: MouseStateHandle,
    file_navigation_button_mouse_state: MouseStateHandle,
    #[cfg(feature = "local_fs")]
    open_repository_button: ViewHandle<ActionButton>,
    pub active_pane_group: Option<ViewHandle<PaneGroup>>,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    working_directories_model: ModelHandle<WorkingDirectoriesModel>,
    maximize_button: ViewHandle<ActionButton>,
    code_review_state: Option<CodeReviewState>,
    #[cfg(feature = "local_fs")]
    code_review_session_env: Option<CodeReviewSessionEnv>,
    is_agent_management_view_open: bool,
    panel_position: super::PanelPosition,
}

impl RightPanelView {
    pub fn init(app: &mut AppContext) {
        use warpui::keymap::macros::*;

        app.register_editable_bindings([EditableBinding::new(
            "workspace:toggle_maximize_code_review_panel",
            "Toggle Maximize Code Review Panel",
            RightPanelAction::ToggleMaximize,
        )
        .with_enabled(|| cfg!(feature = "local_fs"))
        .with_context_predicate(id!("RightPanelView"))
        .with_custom_action(CustomAction::ToggleMaximizePane)]);
    }

    pub fn new(
        working_directories_model: ModelHandle<WorkingDirectoriesModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let resizable_data_handle = ResizableData::handle(ctx);
        let resizable_state_handle = match resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), ModalType::RightPanelWidth)
        {
            Some(handle) => handle,
            None => {
                log::error!("Couldn't retrieve Right panel resizable state handle.");
                resizable_state_handle(600.0)
            }
        };

        let code_review_state = if cfg!(feature = "local_fs") {
            Some(CodeReviewState::new(ctx))
        } else {
            None
        };

        ctx.subscribe_to_model(&working_directories_model, move |me, _, event, ctx| {
            me.handle_working_directories_event(event, ctx)
        });

        // Recompute terminal availability when CLI agent sessions start or end.
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, _, ctx| {
            me.recompute_terminal_availability(ctx);
        });

        // Recompute terminal availability when AI is toggled on or off, so the
        // send button and tooltip update immediately.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::IsAnyAIEnabled { .. }) {
                me.recompute_terminal_availability(ctx);
            }
        });

        let maximize_button = ctx.add_typed_action_view(|ctx| {
            let mut button = ActionButton::new("", PaneHeaderTheme)
                .with_icon(Icon::Maximize)
                .with_tooltip("Maximize")
                .with_tooltip_positioning_provider(Arc::new(MenuPositioning::BelowInputBox))
                .on_click(|ctx| ctx.dispatch_typed_action(RightPanelAction::ToggleMaximize));

            if let Some(keybinding_label) = keybinding_name_to_display_string(
                "workspace:toggle_maximize_code_review_panel",
                ctx,
            ) {
                button = button.with_tooltip_sublabel(keybinding_label);
            }

            button
        });

        #[cfg(feature = "local_fs")]
        let open_repository_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Open repository", NakedTheme)
                .with_size(crate::view_components::action_button::ButtonSize::Small)
                .with_tooltip("Navigate to a repo and initialize it for coding")
                .with_tooltip_alignment(TooltipAlignment::Center)
                .on_click(|ctx| ctx.dispatch_typed_action(RightPanelAction::OpenRepository))
        });

        Self {
            resizable_state_handle,
            close_button_mouse_state: Default::default(),
            file_navigation_button_mouse_state: Default::default(),
            #[cfg(feature = "local_fs")]
            open_repository_button,
            active_pane_group: None,
            working_directories_model,
            maximize_button,
            code_review_state,
            #[cfg(feature = "local_fs")]
            code_review_session_env: None,
            is_agent_management_view_open: false,
            panel_position: super::PanelPosition::Right,
        }
    }

    pub fn set_agent_management_view_open(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        self.is_agent_management_view_open = is_open;
        ctx.notify();
    }

    pub fn set_panel_position(
        &mut self,
        position: super::PanelPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        self.panel_position = position;
        ctx.notify();
    }

    #[cfg(feature = "local_fs")]
    pub fn update_session_env(
        &mut self,
        is_remote: bool,
        is_wsl: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.code_review_session_env = Some(CodeReviewSessionEnv { is_remote, is_wsl });
        ctx.notify();
    }

    pub fn selected_repo_path(&self) -> Option<&PathBuf> {
        self.code_review_state
            .as_ref()
            .and_then(|s| s.selected_repo_path.as_ref())
    }

    #[cfg(feature = "local_fs")]
    pub fn update_selected_repo(&mut self, repo_path: PathBuf, ctx: &mut ViewContext<Self>) {
        self.handle_action(
            &RightPanelAction::SelectRepo {
                repo_path,
                from_dropdown: false,
            },
            ctx,
        );
    }

    fn handle_working_directories_event(
        &mut self,
        event: &WorkingDirectoriesEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WorkingDirectoriesEvent::RepositoriesChanged {
                pane_group_id,
                repositories,
            } => {
                let Some(active_pane_group) = &self.active_pane_group else {
                    return;
                };
                if active_pane_group.id() != *pane_group_id {
                    return;
                }
                let old_selected = self
                    .code_review_state
                    .as_ref()
                    .and_then(|s| s.selected_repo_path.clone());

                if let Some(state) = self.code_review_state.as_mut() {
                    state.set_available_repos(repositories.to_owned(), ctx);
                }

                let new_selected = self
                    .code_review_state
                    .as_ref()
                    .and_then(|s| s.selected_repo_path.clone());

                // Only close the old view if the selection actually changed.
                if old_selected != new_selected {
                    if let Some(old_path) = &old_selected {
                        self.close_code_review_view(*pane_group_id, old_path, ctx);
                    }
                }

                if let Some(path) = &new_selected {
                    self.ensure_code_review_view_exists(path, ctx);
                }

                self.recompute_terminal_availability(ctx);
                ctx.notify();
            }
            WorkingDirectoriesEvent::FocusedRepoChanged {
                pane_group_id,
                repository_terminal_map: _,
                focused_repo,
            } => {
                let Some(active_pane_group) = &self.active_pane_group else {
                    return;
                };
                if active_pane_group.id() != *pane_group_id {
                    return;
                }

                // When the focused terminal changes repos (via CD or pane focus),
                // update the dropdown to match the focused terminal's repo
                if let Some(state) = self.code_review_state.as_mut() {
                    state.set_focused_repo(focused_repo.clone(), ctx);
                }

                self.recompute_terminal_availability(ctx);
                ctx.notify();
            }
            _ => {}
        }
    }

    pub fn set_active_pane_group(
        &mut self,
        pane_group: ViewHandle<PaneGroup>,
        working_directories_model: &ModelHandle<WorkingDirectoriesModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let pane_group_id = pane_group.id();

        // Subscribe to pane group events so we can recompute terminal
        // availability when terminal state changes (e.g. command
        // starts/finishes).
        ctx.subscribe_to_view(&pane_group, |me, _, event, ctx| {
            if matches!(event, PaneGroupEvent::TerminalViewStateChanged) {
                me.recompute_terminal_availability(ctx);
            }
        });

        self.active_pane_group = Some(pane_group);

        if let Some(state) = &mut self.code_review_state {
            let (active_repositories, saved_selection) =
                working_directories_model.read(ctx, |model, _| {
                    let repos: Vec<PathBuf> = model
                        .most_recent_repositories_for_pane_group(pane_group_id)
                        .map(|repos| repos.collect())
                        .unwrap_or_default();
                    let saved = model
                        .get_selected_review_repo(pane_group_id)
                        .map(Path::to_path_buf);
                    (repos, saved)
                });

            // Replace the carried-over selection from a different pane group
            // with whatever was saved for this pane group (if anything). This
            // ensures `set_available_repos` either keeps the saved selection
            // (when it's still in the repo list) or falls back to auto-selecting
            // the first repo, instead of preserving the previous tab's repo.
            state.selected_repo_path = saved_selection;
            state.set_available_repos(active_repositories, ctx);
        }

        let selected = self
            .code_review_state
            .as_ref()
            .and_then(|s| s.selected_repo_path.clone());

        if let Some(selected) = &selected {
            self.ensure_code_review_view_exists(selected, ctx);
        }

        let is_maximized = self.is_maximized(ctx);
        self.set_maximized(is_maximized, ctx);

        ctx.notify();
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    /// Will only update repo_path if one is not already set
    pub fn open_code_review(
        &mut self,
        repo_path: Option<PathBuf>,
        diff_state_model: ModelHandle<DiffStateModel>,
        terminal_view: WeakViewHandle<TerminalView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(repo_dropdown_state) = &mut self.code_review_state else {
            return;
        };
        let (Some(repo_path), Some(active_pane_group)) = (&repo_path, &self.active_pane_group)
        else {
            return;
        };
        let pane_group_id = active_pane_group.id();

        if repo_dropdown_state.selected_repo_path.is_none() {
            repo_dropdown_state.set_selected_repo(repo_path.clone(), ctx);
        }
        // Check if we already have a cached CodeReviewView
        let working_directories_model = self.working_directories_model.clone();
        let existing_view = working_directories_model
            .as_ref(ctx)
            .get_code_review_view(pane_group_id, repo_path);
        if let Some(view) = existing_view {
            view.update(ctx, |view, ctx| {
                view.set_terminal_view(terminal_view);
                view.on_open(ctx);
            });
            self.recompute_terminal_availability(ctx);
        } else if let Some(view) = self.create_code_review_view(
            repo_path,
            diff_state_model.clone(),
            pane_group_id,
            terminal_view.clone(),
            ctx,
        ) {
            view.update(ctx, |view, ctx| {
                view.on_open(ctx);
            });
            self.recompute_terminal_availability(ctx);
        };
        ctx.notify();
    }

    /// Closes the CodeReviewView for the given pane group and repo path (if any)
    /// by calling on_close. This stops event subscriptions and background work.
    fn close_code_review_view(
        &self,
        pane_group_id: EntityId,
        repo_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(code_review_view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_code_review_view(pane_group_id, repo_path)
        {
            code_review_view.update(ctx, |view, ctx| {
                view.on_close(ctx);
            });
        }
    }

    /// Closes the currently active CodeReviewView (if any) by calling on_close.
    fn close_active_code_review_view(&self, ctx: &mut ViewContext<Self>) {
        let Some(state) = &self.code_review_state else {
            return;
        };
        let (Some(repo_path), Some(pane_group)) =
            (&state.selected_repo_path, &self.active_pane_group)
        else {
            return;
        };
        self.close_code_review_view(pane_group.id(), repo_path, ctx);
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn close_code_review(&mut self, ctx: &mut ViewContext<Self>) {
        self.close_active_code_review_view(ctx);

        // Views are cached in WorkingDirectoriesModel, so we just update the UI state
        if let Some(code_review_state) = &mut self.code_review_state {
            code_review_state.selected_repo_path = None;
        }
        ctx.notify();
    }

    fn render_repo_dropdown(&self) -> Option<Box<dyn Element>> {
        let Some(state) = &self.code_review_state else {
            return None;
        };
        if state.available_repos.len() <= 1 {
            return None;
        }
        Some(
            Container::new(
                ConstrainedBox::new(ChildView::new(&state.dropdown).finish())
                    .with_max_width(300.)
                    .finish(),
            )
            .with_margin_right(4.)
            .finish(),
        )
    }

    fn close_button(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder().clone();
        let tooltip_keybinding =
            keybinding_name_to_display_string(TOGGLE_RIGHT_PANEL_BINDING_NAME, app);

        let tooltip = if let Some(keybinding) = tooltip_keybinding {
            ui_builder
                .tool_tip_with_sublabel("Close panel".to_string(), keybinding)
                .build()
                .finish()
        } else {
            ui_builder
                .tool_tip("Close panel".to_string())
                .build()
                .finish()
        };

        let icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        icon_button_with_color(
            appearance,
            icons::Icon::X,
            false,
            self.close_button_mouse_state.clone(),
            icon_color,
        )
        .with_tooltip(move || tooltip)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleRightPanel);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_simple_header(&self, close_button: Box<dyn Element>) -> Box<dyn Element> {
        let left_spacer = Box::new(Shrinkable::new(1.0, Empty::new().finish()));
        Container::new(
            ConstrainedBox::new(
                Flex::row()
                    .with_child(left_spacer)
                    .with_children(vec![close_button])
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .with_height(PANE_HEADER_HEIGHT)
            .finish(),
        )
        .with_padding_left(16.)
        .with_padding_right(HEADER_EDGE_PADDING)
        .finish()
    }

    fn render_panel_content(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let close_button = self.close_button(appearance, app);

        let Some(state) = &self.code_review_state else {
            let simple_header = self.render_simple_header(close_button);
            return Flex::column()
                .with_child(simple_header)
                .with_child(
                    Shrinkable::new(1.0, CodeReviewView::render_loading_state(appearance)).finish(),
                )
                .finish();
        };

        let selected_repo_path = state
            .selected_repo_path
            .as_ref()
            .filter(|repo_path| state.available_repos.contains(repo_path));

        let Some(selected_repo_path) = selected_repo_path else {
            let simple_header = self.render_simple_header(close_button);

            #[cfg(feature = "local_fs")]
            let no_repo_body = {
                let button = Some(ChildView::new(&self.open_repository_button).finish());
                if let Some(env) = &self.code_review_session_env {
                    if env.is_remote {
                        CodeReviewView::render_remote_state(appearance, button)
                    } else if env.is_wsl {
                        CodeReviewView::render_wsl_state(appearance, button)
                    } else {
                        CodeReviewView::render_not_repo_state(appearance, button)
                    }
                } else {
                    CodeReviewView::render_not_repo_state(appearance, button)
                }
            };

            #[cfg(not(feature = "local_fs"))]
            let no_repo_body = CodeReviewView::render_not_repo_state(appearance, None);

            return Flex::column()
                .with_child(simple_header)
                .with_child(Shrinkable::new(1.0, no_repo_body).finish())
                .finish();
        };

        let current_code_review_view = self.active_pane_group.as_ref().and_then(|pane_group| {
            let pane_group_id = pane_group.id();
            self.working_directories_model
                .as_ref(app)
                .get_code_review_view(pane_group_id, selected_repo_path)
        });

        if let Some(code_review_view) = current_code_review_view {
            let header = if FeatureFlag::GitOperationsInCodeReview.is_enabled() {
                self.render_header(&code_review_view, appearance, app)
            } else {
                self.render_header_legacy(appearance, app)
            };
            let code_review_content =
                Shrinkable::new(1.0, ChildView::new(&code_review_view).finish()).finish();

            Flex::column()
                .with_child(header)
                .with_child(code_review_content)
                .finish()
        } else {
            let simple_header = self.render_simple_header(close_button);
            Flex::column()
                .with_child(simple_header)
                .with_child(
                    Shrinkable::new(1.0, CodeReviewView::render_loading_state(appearance)).finish(),
                )
                .finish()
        }
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn render_maximize_pane_button(&self) -> Box<dyn Element> {
        ConstrainedBox::new(ChildView::new(&self.maximize_button).finish())
            .with_height(warp_core::ui::icons::ICON_DIMENSIONS)
            .with_width(warp_core::ui::icons::ICON_DIMENSIONS)
            .finish()
    }

    fn render_header(
        &self,
        code_review_view: &ViewHandle<CodeReviewView>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_text_color = theme.sub_text_color(theme.background());

        let crv = code_review_view.as_ref(app);
        let repo_path = crv.repo_path();
        let branch_name = crv
            .diff_state_model()
            .read(app, |model, ctx| model.get_current_branch_name(ctx));
        let diff_stats = crv.loaded_diff_stats();

        let repo_path_element = repo_path.map(|repo_path| {
            let display_path = dirs::home_dir()
                .and_then(|home| repo_path.strip_prefix(&home).ok())
                .map(|relative| format!("~/{}", relative.display()))
                .unwrap_or_else(|| repo_path.display().to_string());
            Container::new(
                Text::new_inline(
                    format!("{display_path}:"),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(sub_text_color.into())
                .finish(),
            )
            .with_margin_right(4.)
            .finish()
        });

        let branch_name_element = branch_name.map(|name| {
            Container::new(
                Text::new_inline(name, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(sub_text_color.into())
                    .finish(),
            )
            .with_margin_right(8.)
            .finish()
        });

        let stats_element =
            diff_stats.map(|stats| CodeReviewView::render_diff_stats(&stats, appearance));

        let close_button = self.close_button(appearance, app);

        let mut left_section = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);
        if let Some(repo_path_el) = repo_path_element {
            left_section.add_child(repo_path_el);
        }
        if let Some(branch_el) = branch_name_element {
            left_section.add_child(Shrinkable::new(100.0, branch_el).finish());
        }
        if let Some(stats) = stats_element {
            left_section.add_child(stats);
        }

        let mut right_section = Vec::new();
        if let Some(repo_dropdown) = self.render_repo_dropdown() {
            right_section.push(repo_dropdown);
        }
        right_section.push(self.render_maximize_pane_button());
        right_section.push(close_button);

        Container::new(
            ConstrainedBox::new(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Clipped::new(Shrinkable::new(1.0, left_section.finish()).finish()).finish(),
                    )
                    .with_children(right_section)
                    .finish(),
            )
            .with_height(PANE_HEADER_HEIGHT)
            .finish(),
        )
        .with_padding_left(CONTENT_LEFT_MARGIN)
        .with_padding_right(CONTENT_RIGHT_MARGIN)
        .finish()
    }

    /// Legacy header layout: "Code review" title + file nav button.
    fn render_header_legacy(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let file_navigation_button = {
            let current_code_review_view = self
                .code_review_state
                .as_ref()
                .and_then(|state| state.selected_repo_path.as_ref())
                .and_then(|repo_path| {
                    self.active_pane_group.as_ref().and_then(|pane_group| {
                        let pane_group_id = pane_group.id();
                        self.working_directories_model
                            .as_ref(app)
                            .get_code_review_view(pane_group_id, repo_path)
                    })
                });

            let has_files = current_code_review_view
                .as_ref()
                .map(|view: &ViewHandle<CodeReviewView>| view.as_ref(app).has_file_states())
                .unwrap_or(false);

            let file_sidebar_expanded = current_code_review_view
                .as_ref()
                .map(|view| view.as_ref(app).file_sidebar_expanded())
                .unwrap_or(false);

            if has_files {
                Some(render_file_navigation_button(
                    appearance,
                    file_sidebar_expanded,
                    self.file_navigation_button_mouse_state.clone(),
                    |ctx| {
                        ctx.dispatch_typed_action(RightPanelAction::ToggleFileSidebar);
                    },
                ))
            } else {
                None
            }
        };

        let theme = appearance.theme();
        let sub_text_color = theme.sub_text_color(theme.background());

        let title = Shrinkable::new(
            1.0,
            Text::new_inline("Code review".to_string(), appearance.ui_font_family(), 12.)
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(sub_text_color.into())
                .finish(),
        )
        .finish();

        let close_button = self.close_button(appearance, app);

        let mut left_section = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);
        let has_nav_button = file_navigation_button.is_some();
        if let Some(nav_button) = file_navigation_button {
            left_section.add_child(nav_button);
        }
        left_section.add_child(title);

        let mut right_section = Vec::new();
        if let Some(repo_dropdown) = self.render_repo_dropdown() {
            right_section.push(repo_dropdown);
        }
        right_section.push(self.render_maximize_pane_button());
        right_section.push(close_button);

        let left_padding = if has_nav_button { 12. } else { 16. };

        Container::new(
            ConstrainedBox::new(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Box::new(Shrinkable::new(1.0, left_section.finish())))
                    .with_children(right_section)
                    .finish(),
            )
            .with_height(PANE_HEADER_HEIGHT)
            .finish(),
        )
        .with_padding_left(left_padding)
        .with_padding_right(HEADER_EDGE_PADDING)
        .finish()
    }

    pub fn set_maximized(&mut self, is_maximized: bool, ctx: &mut ViewContext<Self>) {
        let (icon, tooltip) = if is_maximized {
            (Icon::Minimize, "Minimize")
        } else {
            (Icon::Maximize, "Maximize")
        };

        self.maximize_button.update(ctx, |button, ctx| {
            let mut new_button = ActionButton::new("", PaneHeaderTheme)
                .with_icon(icon)
                .with_tooltip(tooltip)
                .with_tooltip_positioning_provider(Arc::new(MenuPositioning::BelowInputBox))
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(RightPanelAction::ToggleMaximize);
                });

            if let Some(keybinding_label) = keybinding_name_to_display_string(
                "workspace:toggle_maximize_code_review_panel",
                ctx,
            ) {
                new_button = new_button.with_tooltip_sublabel(keybinding_label);
            }

            *button = new_button;
            ctx.notify();
        });

        // Propagate maximize state to the active code review view's file sidebar
        if let Some(code_review_view) = self.get_active_code_review_view(ctx) {
            code_review_view.update(ctx, |view, ctx| {
                view.handle_maximization_toggle(is_maximized, ctx);
            });
        }
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub fn focus_active_code_review_view(&self, ctx: &mut ViewContext<Self>) {
        let Some(state) = &self.code_review_state else {
            return;
        };
        let Some(selected_repo_path) = &state.selected_repo_path else {
            return;
        };
        let Some(active_pane_group) = &self.active_pane_group else {
            return;
        };
        let pane_group_id = active_pane_group.id();
        if let Some(code_review_view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_code_review_view(pane_group_id, selected_repo_path)
        {
            ctx.focus(&code_review_view);
        }
    }

    fn get_active_code_review_view(&self, ctx: &AppContext) -> Option<ViewHandle<CodeReviewView>> {
        let state = self.code_review_state.as_ref()?;
        let selected_repo_path = state.selected_repo_path.as_ref()?;
        let active_pane_group = self.active_pane_group.as_ref()?;
        let pane_group_id = active_pane_group.id();
        self.working_directories_model
            .as_ref(ctx)
            .get_code_review_view(pane_group_id, selected_repo_path)
    }

    fn is_maximized(&self, app: &AppContext) -> bool {
        self.active_pane_group
            .as_ref()
            .map(|pane_group| pane_group.as_ref(app).is_right_panel_maximized)
            .unwrap_or(false)
    }

    fn create_code_review_view(
        &self,
        repo_path: &Path,
        diff_state_model: ModelHandle<DiffStateModel>,
        pane_group_id: EntityId,
        terminal_view: WeakViewHandle<TerminalView>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<ViewHandle<CodeReviewView>> {
        // Early check: if pane group has no active repositories, don't create a view
        let has_active_repos = self
            .working_directories_model
            .as_ref(ctx)
            .most_recent_repositories_for_pane_group(pane_group_id)
            .is_some_and(|repos| repos.count() > 0);

        if !has_active_repos {
            return None;
        }

        let diff_state_model_clone = diff_state_model.clone();
        let code_review_comment_batch =
            self.working_directories_model
                .update(ctx, |working_directories, ctx| {
                    working_directories.get_or_create_code_review_comments(repo_path, ctx)
                });
        let code_review_view = ctx.add_typed_action_view(|ctx| {
            CodeReviewView::new(
                Some(repo_path.to_path_buf()),
                diff_state_model_clone,
                code_review_comment_batch,
                Some(terminal_view),
                ctx,
            )
        });

        // Store in cache
        self.working_directories_model.update(ctx, |model, _ctx| {
            model.store_code_review_view(
                pane_group_id,
                repo_path.to_path_buf(),
                code_review_view.clone(),
            );
        });

        ctx.subscribe_to_model(&diff_state_model, |_me, _, _event, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_view(&code_review_view, |me, code_review, event, ctx| {
            match event {
                CodeReviewViewEvent::ReviewSubmitted => {
                    if me.is_maximized(ctx) {
                        me.handle_action(&RightPanelAction::ToggleMaximize, ctx);
                    }
                }
                CodeReviewViewEvent::SubmitReviewComments {
                    comments,
                    repo_path,
                } => {
                    Self::route_review_comments(me, &code_review, comments.clone(), repo_path, ctx);
                }
                #[cfg(feature = "local_fs")]
                CodeReviewViewEvent::OpenFileWithTarget {
                    path,
                    target,
                    line_col,
                } => {
                    ctx.emit(RightPanelEvent::OpenFileWithTarget {
                        path: path.clone(),
                        target: target.clone(),
                        line_col: *line_col,
                    });
                }
                CodeReviewViewEvent::OpenFileInNewTab {
                    path,
                    line_and_column,
                } => {
                    ctx.emit(RightPanelEvent::OpenFileInNewTab {
                        path: path.clone(),
                        line_and_column: *line_and_column,
                    });
                }
                #[cfg(not(target_family = "wasm"))]
                CodeReviewViewEvent::OpenLspLogs { log_path } => {
                    ctx.emit(RightPanelEvent::OpenLspLogs {
                        log_path: log_path.clone(),
                    });
                }
                _ => {}
            }
            ctx.notify();
        });

        Some(code_review_view)
    }

    /// Routes review comments to the best available terminal.
    /// Tries the preferred terminal first, then falls back to other terminals
    /// in the same repo working directory.
    fn route_review_comments(
        &mut self,
        code_review_view: &ViewHandle<CodeReviewView>,
        comments: AgentReviewCommentBatch,
        repo_path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_group) = &self.active_pane_group else {
            code_review_view.update(ctx, |view, ctx| {
                view.handle_review_submission_result(ReviewSubmissionResult::Error, ctx);
            });
            return;
        };

        let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let chosen = self.find_review_terminal(pane_group, repo_path, ai_enabled, ctx);

        let Some(terminal_view) = chosen else {
            log::warn!("No available terminal found for submitting review comments");
            code_review_view.update(ctx, |view, ctx| {
                view.handle_review_submission_result(ReviewSubmissionResult::Error, ctx);
            });
            return;
        };

        let comment_count = comments.comments.len();
        let file_count = comments
            .comments
            .iter()
            .filter_map(|c| {
                c.target
                    .absolute_file_path()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .collect::<std::collections::HashSet<_>>()
            .len();

        let active_cli_agent = terminal_view.read(ctx, |t, ctx| t.active_cli_agent(ctx));

        let (result, destination) = if active_cli_agent.is_some() {
            let r = terminal_view.update(ctx, |terminal, ctx| {
                terminal.send_review_to_cli_agent_or_rich_input(&comments, ctx)
            });
            let dest = if terminal_view.read(ctx, |t, ctx| t.is_cli_agent_rich_input_open(ctx)) {
                CodeReviewContextDestination::RichInput
            } else {
                CodeReviewContextDestination::Pty
            };
            (r, dest)
        } else {
            let r = terminal_view.update(ctx, |terminal, ctx| {
                terminal.send_inline_review(comments, ctx)
            });
            (r, CodeReviewContextDestination::AgentReview)
        };

        if let Err(err) = &result {
            log::error!("Failed to submit review comments to terminal: {err}");
        }

        let submission_result = if result.is_ok() {
            ReviewSubmissionResult::Success {
                comment_count,
                file_count,
                destination,
            }
        } else {
            ReviewSubmissionResult::Error
        };

        code_review_view.update(ctx, |view, ctx| {
            view.handle_review_submission_result(submission_result, ctx);
        });
    }

    fn format_optional_path(path: Option<&Path>) -> String {
        path.map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string())
    }

    fn review_terminal_status(
        tv: &ViewHandle<TerminalView>,
        repo_path: Option<&Path>,
        ai_enabled: bool,
        ctx: &AppContext,
    ) -> ReviewTerminalStatus {
        tv.read(ctx, |t, ctx| {
            let active_session_path = t.active_session_path_if_local(ctx);
            let current_repo_path = t.current_local_repo_path().map(Path::to_path_buf);
            let active_cli_agent = t.active_cli_agent(ctx).map(|agent| format!("{agent:?}"));
            let model = t.model.lock();
            let is_executing = model.block_list().active_block().is_executing();
            let is_input_box_visible = t.is_input_box_visible(&model, ctx);
            let mut unavailable_reasons = Vec::new();

            match repo_path {
                Some(repo_path) => match active_session_path.as_ref() {
                    // Canonicalize the CWD, note that repo_path has already been canonicalized.
                    Some(cwd)
                        if canonicalize(cwd)
                            .as_deref()
                            .unwrap_or(cwd)
                            .starts_with(repo_path) => {}
                    Some(_) => unavailable_reasons
                        .push(ReviewTerminalUnavailableReason::SessionOutsideSelectedRepo),
                    None => unavailable_reasons
                        .push(ReviewTerminalUnavailableReason::SessionPathUnavailable),
                },
                None => unavailable_reasons.push(ReviewTerminalUnavailableReason::NoSelectedRepo),
            }

            if active_cli_agent.is_none() {
                if !ai_enabled {
                    unavailable_reasons.push(ReviewTerminalUnavailableReason::AIDisabled);
                }
                if is_executing {
                    unavailable_reasons.push(ReviewTerminalUnavailableReason::TerminalExecuting);
                }
                if !is_input_box_visible {
                    unavailable_reasons.push(ReviewTerminalUnavailableReason::InputBoxNotVisible);
                }
            }

            ReviewTerminalStatus {
                active_session_path,
                current_repo_path,
                active_cli_agent,
                is_executing,
                is_input_box_visible,
                unavailable_reasons,
            }
        })
    }

    fn log_code_review_debug_state(debug_state: &CodeReviewCommentDebugState) {
        log::info!(
            "Active code review view: repo_path={}, has_active_comment_model={}, review_destination={:?}, total_comments={}, sendable_comments={}, is_collapsed={}, is_outdated_section_collapsed={:?}, ai_available={}, ai_enabled={}, send_button_tooltip={}",
            Self::format_optional_path(debug_state.repo_path.as_deref()),
            debug_state.has_active_comment_model,
            debug_state.comment_list.review_destination,
            debug_state.comment_list.total_comments,
            debug_state.comment_list.sendable_comments,
            debug_state.comment_list.is_collapsed,
            debug_state.comment_list.is_outdated_section_collapsed,
            debug_state.comment_list.ai_available,
            debug_state.comment_list.ai_enabled,
            debug_state.comment_list.send_button_tooltip_text,
        );
    }

    pub fn log_review_comment_send_status_for_active_tab(&self, ctx: &AppContext) {
        let selected_repo_path = self.selected_repo_path().cloned();
        let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let code_review_debug_state =
            self.get_active_code_review_view(ctx)
                .map(|code_review_view| {
                    code_review_view.read(ctx, |view, ctx| view.debug_review_comment_state(ctx))
                });

        let Some(pane_group) = &self.active_pane_group else {
            log::info!(
                "Review comment send status for active tab: no active pane group, selected_repo_path={}, ai_enabled={}",
                Self::format_optional_path(selected_repo_path.as_deref()),
                ai_enabled,
            );
            if let Some(debug_state) = &code_review_debug_state {
                Self::log_code_review_debug_state(debug_state);
            }
            return;
        };

        let pane_group_id = pane_group.id();
        let visible_pane_ids = pane_group.read(ctx, |pane_group, _| pane_group.visible_pane_ids());
        let focused_pane_id =
            pane_group.read(ctx, |pane_group, ctx| pane_group.focused_pane_id(ctx));
        let preferred_terminal_id = selected_repo_path.as_ref().and_then(|repo_path| {
            self.working_directories_model
                .as_ref(ctx)
                .get_terminal_id_for_root_path(pane_group_id, repo_path)
        });
        let chosen_terminal_id = selected_repo_path.as_ref().and_then(|repo_path| {
            self.find_review_terminal(pane_group, repo_path, ai_enabled, ctx)
                .map(|terminal_view| terminal_view.id())
        });

        log::info!(
            "Review comment send status for active tab: pane_group_id={pane_group_id}, selected_repo_path={}, ai_enabled={}, focused_pane_id={focused_pane_id}, preferred_terminal_id={preferred_terminal_id:?}, chosen_terminal_id={chosen_terminal_id:?}, visible_pane_count={}",
            Self::format_optional_path(selected_repo_path.as_deref()),
            ai_enabled,
            visible_pane_ids.len(),
        );

        if let Some(debug_state) = &code_review_debug_state {
            Self::log_code_review_debug_state(debug_state);
        } else {
            log::info!(
                "No active code review view is associated with the current tab/repo selection"
            );
        }

        for (index, pane_id) in visible_pane_ids.iter().enumerate() {
            let is_focused = *pane_id == focused_pane_id;
            if !pane_id.is_terminal_pane() {
                log::info!(
                    "Pane #{index}: pane_id={pane_id}, pane_type={}, focused={is_focused}, skipped=not a terminal pane",
                    pane_id.pane_type(),
                );
                continue;
            }

            let terminal_view = pane_group.read(ctx, |pane_group, ctx| {
                pane_group.terminal_view_from_pane_id(*pane_id, ctx)
            });
            let Some(terminal_view) = terminal_view else {
                log::info!(
                    "Pane #{index}: pane_id={pane_id}, pane_type={}, focused={is_focused}, skipped=terminal view missing",
                    pane_id.pane_type(),
                );
                continue;
            };

            let terminal_id = terminal_view.id();
            let terminal_status = Self::review_terminal_status(
                &terminal_view,
                selected_repo_path.as_deref(),
                ai_enabled,
                ctx,
            );
            let unavailable_reasons = if terminal_status.unavailable_reasons.is_empty() {
                "<none>".to_string()
            } else {
                terminal_status
                    .unavailable_reasons
                    .iter()
                    .map(ReviewTerminalUnavailableReason::label)
                    .join("; ")
            };

            log::info!(
                "Pane #{index}: pane_id={pane_id}, pane_type={}, terminal_view_id={terminal_id}, focused={is_focused}, preferred={}, chosen={}, available={}, active_session_path={}, current_repo_path={}, active_cli_agent={}, is_executing={}, is_input_box_visible={}, unavailable_reasons={}",
                pane_id.pane_type(),
                preferred_terminal_id == Some(terminal_id),
                chosen_terminal_id == Some(terminal_id),
                terminal_status.is_available(),
                Self::format_optional_path(terminal_status.active_session_path.as_deref()),
                Self::format_optional_path(terminal_status.current_repo_path.as_deref()),
                terminal_status
                    .active_cli_agent
                    .as_deref()
                    .unwrap_or("<none>"),
                terminal_status.is_executing,
                terminal_status.is_input_box_visible,
                unavailable_reasons,
            );
        }
    }

    /// Returns whether a terminal is in the given repo and available to receive
    /// review comments. A terminal is available if it is not executing a command
    /// and has its input box visible, OR if it has an active CLI agent
    /// (CLI agents are long-running commands that accept review input).
    ///
    /// When `ai_enabled` is `false`, only terminals with an active CLI agent are
    /// considered available (non-CLI Warp terminals require AI to be on).
    fn is_terminal_available_for_review(
        tv: &ViewHandle<TerminalView>,
        repo_path: &Path,
        ai_enabled: bool,
        ctx: &AppContext,
    ) -> bool {
        Self::review_terminal_status(tv, Some(repo_path), ai_enabled, ctx).is_available()
    }

    /// Finds the best terminal to send review comments to.
    /// Priority: focused terminal > preferred terminal > other terminals with
    /// matching CWD that are available.
    fn find_available_terminal_for_review(
        terminal_views: &[ViewHandle<TerminalView>],
        focused_terminal: Option<&ViewHandle<TerminalView>>,
        preferred_terminal_id: Option<EntityId>,
        repo_path: &Path,
        ai_enabled: bool,
        ctx: &AppContext,
    ) -> Option<ViewHandle<TerminalView>> {
        let is_available = |tv: &ViewHandle<TerminalView>| {
            Self::is_terminal_available_for_review(tv, repo_path, ai_enabled, ctx)
        };

        // Try the focused terminal first.
        if let Some(tv) = focused_terminal {
            if is_available(tv) {
                return Some(tv.clone());
            }
        }

        // Try the preferred (repo-mapped) terminal next.
        if let Some(preferred_id) = preferred_terminal_id {
            if let Some(tv) = terminal_views.iter().find(|tv| tv.id() == preferred_id) {
                if is_available(tv) {
                    return Some(tv.clone());
                }
            }
        }

        // Fallback: any terminal in the repo that is available.
        terminal_views.iter().find(|tv| is_available(tv)).cloned()
    }

    /// Finds the best available terminal for review in the given pane group,
    /// gathering the terminal list, focused terminal, and preferred terminal ID
    /// before delegating to `find_available_terminal_for_review`.
    fn find_review_terminal(
        &self,
        pane_group: &ViewHandle<PaneGroup>,
        repo_path: &Path,
        ai_enabled: bool,
        ctx: &AppContext,
    ) -> Option<ViewHandle<TerminalView>> {
        let terminal_views = pane_group.read(ctx, |pg, ctx| pg.terminal_views(ctx));
        let focused_terminal = pane_group.read(ctx, |pg, ctx| pg.focused_session_view(ctx));
        let pane_group_id = pane_group.id();
        let preferred_terminal_id = self
            .working_directories_model
            .as_ref(ctx)
            .get_terminal_id_for_root_path(pane_group_id, repo_path);

        Self::find_available_terminal_for_review(
            &terminal_views,
            focused_terminal.as_ref(),
            preferred_terminal_id,
            repo_path,
            ai_enabled,
            ctx,
        )
    }

    /// Checks whether any terminal in the pane group is available for input in
    /// the correct working directory and pushes the result to the active
    /// CodeReviewView.
    pub fn recompute_terminal_availability(&self, ctx: &mut ViewContext<Self>) {
        let Some(code_review_view) = self.get_active_code_review_view(ctx) else {
            return;
        };

        let repo_path = code_review_view.read(ctx, |view, _| view.repo_path().cloned());
        let Some(repo_path) = repo_path else {
            code_review_view.update(ctx, |view, ctx| {
                view.set_review_destination(ReviewDestination::None, ctx);
            });
            return;
        };

        let Some(pane_group) = &self.active_pane_group else {
            code_review_view.update(ctx, |view, ctx| {
                view.set_review_destination(ReviewDestination::None, ctx);
            });
            return;
        };

        let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let destination = self
            .find_review_terminal(pane_group, &repo_path, ai_enabled, ctx)
            .map(|tv| {
                tv.read(ctx, |t, ctx| {
                    t.active_cli_agent(ctx)
                        .map(ReviewDestination::Cli)
                        .unwrap_or(ReviewDestination::Warp)
                })
            })
            .unwrap_or(ReviewDestination::None);

        code_review_view.update(ctx, |view, ctx| {
            view.set_review_destination(destination, ctx);
        });
    }

    fn ensure_code_review_view_exists(&mut self, repo_path: &Path, ctx: &mut ViewContext<Self>) {
        let Some(pane_group) = &self.active_pane_group else {
            return;
        };
        let pane_group_id = pane_group.id();
        // Only set up subscriptions and diff loading when the panel is visible.
        // When the panel opens later, open_code_review will call on_open.
        let is_panel_open = pane_group.as_ref(ctx).right_panel_open;

        let existing_view = self
            .working_directories_model
            .as_ref(ctx)
            .get_code_review_view(pane_group_id, repo_path);

        if let Some(view) = existing_view {
            if is_panel_open {
                // on_open is idempotent (guards on is_open), so this is safe for
                // already-open views and correctly re-opens cached-but-closed ones.
                view.update(ctx, |view, ctx| {
                    view.on_open(ctx);
                });
            }
        } else {
            let diff_state_model = self.working_directories_model.update(ctx, |model, ctx| {
                model.get_or_create_diff_state_model(
                    LocalOrRemotePath::Local(repo_path.to_path_buf()),
                    ctx,
                )
            });

            let Some(diff_state_model) = diff_state_model else {
                return;
            };
            let working_directories_model = self.working_directories_model.as_ref(ctx);
            let Some(terminal_view_id) =
                working_directories_model.get_terminal_id_for_root_path(pane_group_id, repo_path)
            else {
                return;
            };

            if working_directories_model
                .most_recent_repositories_for_pane_group(pane_group_id)
                .is_some_and(|mut repos| repos.contains(repo_path))
            {
                if let Some(terminal_view) =
                    ctx.view_with_id::<TerminalView>(ctx.window_id(), terminal_view_id)
                {
                    if let Some(view) = self.create_code_review_view(
                        repo_path,
                        diff_state_model,
                        pane_group_id,
                        terminal_view.downgrade(),
                        ctx,
                    ) {
                        if is_panel_open {
                            view.update(ctx, |view, ctx| {
                                view.on_open(ctx);
                            });
                        }
                    }
                }
            }
        }
    }
}

impl Entity for RightPanelView {
    type Event = RightPanelEvent;
}

#[cfg(feature = "local_fs")]
impl TypedActionView for RightPanelView {
    type Action = RightPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RightPanelAction::ToggleFileSidebar => {
                if let Some(state) = &self.code_review_state {
                    if let Some(repo_path) = &state.selected_repo_path {
                        if let Some(pane_group) = &self.active_pane_group {
                            let pane_group_id = pane_group.id();
                            let working_directories_model = self.working_directories_model.clone();
                            if let Some(code_review_view) = working_directories_model
                                .as_ref(ctx)
                                .get_code_review_view(pane_group_id, repo_path)
                            {
                                code_review_view.update(ctx, |view, ctx| {
                                    view.handle_action(&CodeReviewAction::ToggleFileSidebar, ctx);
                                });
                            }
                        }
                    }
                }
            }
            RightPanelAction::SelectRepo {
                repo_path,
                from_dropdown,
            } => {
                // Only close the old view if we're actually switching to a different repo.
                let is_switching = self
                    .code_review_state
                    .as_ref()
                    .and_then(|s| s.selected_repo_path.as_ref())
                    .is_some_and(|old| old != repo_path);
                if is_switching {
                    self.close_active_code_review_view(ctx);
                }
                if let Some(state) = &mut self.code_review_state {
                    // Don't update dropdown when selection comes from dropdown itself
                    let should_update_dropdown = !from_dropdown;
                    state.set_selected_repo_internal(
                        repo_path.clone(),
                        should_update_dropdown,
                        ctx,
                    );
                    self.ensure_code_review_view_exists(repo_path, ctx);

                    // Persist the user's manual selection so it can be restored when
                    // they leave this pane group's session and come back. We only
                    // persist explicit `SelectRepo` actions (i.e. dropdown picks or
                    // contextual opens) so that the auto-selected default doesn't
                    // overwrite an earlier manual choice for a different pane group.
                    if let Some(pane_group) = &self.active_pane_group {
                        let pane_group_id = pane_group.id();
                        let repo_path = repo_path.clone();
                        self.working_directories_model.update(ctx, |model, _| {
                            model.set_selected_review_repo(pane_group_id, repo_path);
                        });
                    }

                    ctx.notify();
                }
            }
            RightPanelAction::ToggleMaximize => {
                ctx.emit(RightPanelEvent::ToggleMaximize);
                ctx.notify();
            }
            RightPanelAction::OpenRepository => {
                if let Some(active_pane_group) = &self.active_pane_group {
                    let terminal_view = active_pane_group.read(ctx, |pane_group, ctx| {
                        pane_group
                            .active_session_id(ctx)
                            .and_then(|id| pane_group.terminal_view_from_pane_id(id, ctx))
                    });

                    if let Some(terminal_view) = terminal_view {
                        terminal_view.update(ctx, |terminal, ctx| {
                            terminal.handle_action(
                                &crate::terminal::view::TerminalAction::PickRepoToOpen,
                                ctx,
                            );
                        });
                    }
                }
            }
        }
    }
}

#[cfg(not(feature = "local_fs"))]
impl TypedActionView for RightPanelView {
    type Action = RightPanelAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        // No actions when local_fs is disabled
    }
}

impl View for RightPanelView {
    fn ui_name() -> &'static str {
        "RightPanelView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let panel_content = self.render_panel_content(app);

        if self.is_maximized(app) {
            return Shrinkable::new(1.0, panel_content).finish();
        }

        let drag_side = match self.panel_position {
            super::PanelPosition::Left => DragBarSide::Right,
            super::PanelPosition::Right => DragBarSide::Left,
        };
        Resizable::new(self.resizable_state_handle.clone(), panel_content)
            .with_dragbar_side(drag_side)
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .with_bounds_callback(Box::new(|window_size| {
                let min_width = MIN_SIDEBAR_WIDTH;
                let max_width = window_size.x() * MAX_SIDEBAR_WIDTH_RATIO;
                (min_width, max_width.max(min_width))
            }))
            .finish()
    }
}
