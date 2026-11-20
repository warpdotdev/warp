//! Tools panel view that lists repositories and their git worktrees, along with the activity of
//! coding agents running inside each worktree.
mod model;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::Icon;
use warp_core::ui::theme::Fill as ThemeFill;
use warp_core::ui::theme::color::internal_colors;
use warp_util::git::run_git_command;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::elements::{
    ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Element, Empty, Fill, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, SavePosition, ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

pub use self::model::{AgentActivity, AgentState, TerminalActivity};
use self::model::{
    Repository, WorktreeActivity, aggregate_activity, parse_worktree_porcelain, worktree_containing,
};
use super::vertical_tabs::summary_conversation_status_for_terminal;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::appearance::Appearance;
use crate::menu::{self, Menu, MenuItem, MenuItemFields};
use crate::settings::WorktreeSettings;
use crate::terminal::TerminalView;
use crate::terminal::cli_agent::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::ui_components::buttons::icon_button_with_color;
use crate::util::git::get_pr_for_branch;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

const TICK_INTERVAL: Duration = Duration::from_secs(2);
/// A single click waits this long before acting, so it can be superseded by a double click.
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(250);
/// Git worktrees are re-listed every this many ticks.
const GIT_REFRESH_TICKS: u32 = 5;
/// Merged pull requests are checked every this many ticks (~5 minutes), since each check is a
/// network round trip per worktree.
const MERGED_PR_CHECK_TICKS: u32 = 150;

const HEADER_HEIGHT: f32 = 28.;
const ROW_CORNER_RADIUS: f32 = 4.;
const STATUS_DOT_SIZE: f32 = 8.;
const ICON_SIZE: f32 = 14.;
const WORKTREE_INDENT: f32 = 20.;

#[derive(Clone, Debug, PartialEq)]
pub enum WorktreePanelAction {
    ToggleRepository(PathBuf),
    ClickWorktree(PathBuf),
    DoubleClickWorktree(PathBuf),
    OpenWorktree(PathBuf),
    OpenWorktreeInNewTab(PathBuf),
    OpenWorktreeContextMenu {
        worktree: PathBuf,
        position: Vector2F,
    },
    DeleteWorktree {
        worktree: PathBuf,
        delete_branch: bool,
    },
    NewWorktree(PathBuf),
    AddRepository,
    Refresh,
}

pub enum WorktreePanelEvent {
    /// Asks the workspace to report its terminals via [`WorktreePanelView::set_terminal_activity`].
    RequestTerminalActivity,
    /// Moves the active tab into the worktree.
    OpenWorktree(PathBuf),
    OpenWorktreeInNewTab(PathBuf),
    NewWorktree {
        repository: PathBuf,
    },
    AddRepository,
}

#[derive(Default)]
struct RepositoryMouseStates {
    header: MouseStateHandle,
    new_worktree: MouseStateHandle,
}

#[derive(Default)]
struct HeaderMouseStates {
    refresh: MouseStateHandle,
    add_repository: MouseStateHandle,
}

pub struct WorktreePanelView {
    repositories: Vec<Repository>,
    terminals: Vec<TerminalActivity>,
    active_tab_index: usize,
    activity: HashMap<PathBuf, WorktreeActivity>,
    /// Expansion the user chose explicitly; other repositories are expanded only while one of
    /// their worktrees has an open terminal.
    expansion_overrides: HashMap<PathBuf, bool>,
    repository_mouse_states: HashMap<PathBuf, RepositoryMouseStates>,
    worktree_mouse_states: HashMap<PathBuf, MouseStateHandle>,
    header_mouse_states: HeaderMouseStates,
    scroll_state: ClippedScrollStateHandle,
    context_menu: ViewHandle<Menu<WorktreePanelAction>>,
    /// Where the open context menu is anchored, relative to the panel.
    context_menu_position: Option<Vector2F>,
    position_id: String,
    pending_single_click: Option<SpawnedFutureHandle>,
    /// Whether the Worktrees tab is the visible tools panel view.
    is_visible: bool,
    ticks: u32,
    git_refresh_in_flight: bool,
    merged_pr_check_in_flight: bool,
}

impl WorktreePanelView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let context_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&context_menu, |me, _, event, ctx| {
            if let menu::Event::Close { .. } = event {
                me.context_menu_position = None;
                ctx.notify();
            }
        });
        let view = Self {
            repositories: Vec::new(),
            terminals: Vec::new(),
            active_tab_index: 0,
            activity: HashMap::new(),
            expansion_overrides: HashMap::new(),
            repository_mouse_states: HashMap::new(),
            worktree_mouse_states: HashMap::new(),
            header_mouse_states: HeaderMouseStates::default(),
            scroll_state: ClippedScrollStateHandle::default(),
            context_menu,
            context_menu_position: None,
            position_id: format!("worktree_panel_{}", ctx.view_id()),
            pending_single_click: None,
            is_visible: false,
            ticks: 0,
            git_refresh_in_flight: false,
            merged_pr_check_in_flight: false,
        };
        view.schedule_tick(ctx);
        view
    }

    fn is_enabled(app: &AppContext) -> bool {
        *WorktreeSettings::as_ref(app).worktree_panel_visible
    }

    pub fn set_is_visible(&mut self, is_visible: bool, ctx: &mut ViewContext<Self>) {
        if self.is_visible == is_visible {
            return;
        }
        self.is_visible = is_visible;
        if is_visible {
            ctx.emit(WorktreePanelEvent::RequestTerminalActivity);
            self.refresh_repositories(ctx);
        }
    }

    fn schedule_tick(&self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async { warpui::r#async::Timer::after(TICK_INTERVAL).await },
            |me, _, ctx| {
                me.tick(ctx);
                me.schedule_tick(ctx);
            },
        );
    }

    /// While visible, keeps agent status and worktrees fresh. The merged pull request cleanup also
    /// runs while the Worktrees tab is hidden, as long as it is enabled.
    fn tick(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::is_enabled(ctx) {
            return;
        }
        let is_merged_pr_check_tick =
            self.ticks % MERGED_PR_CHECK_TICKS == MERGED_PR_CHECK_TICKS - 1;
        if self.is_visible || is_merged_pr_check_tick {
            ctx.emit(WorktreePanelEvent::RequestTerminalActivity);
        }
        if (self.is_visible && self.ticks.is_multiple_of(GIT_REFRESH_TICKS))
            || is_merged_pr_check_tick
        {
            self.refresh_repositories(ctx);
        }
        if is_merged_pr_check_tick {
            self.remove_worktrees_with_merged_prs(ctx);
        }
        self.ticks = self.ticks.wrapping_add(1);
    }

    /// Replaces the workspace's terminal snapshot used to compute per-worktree agent status.
    pub fn set_terminal_activity(
        &mut self,
        terminals: Vec<TerminalActivity>,
        active_tab_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.terminals == terminals && self.active_tab_index == active_tab_index {
            return;
        }
        let has_unknown_directory = terminals
            .iter()
            .any(|terminal| worktree_containing(&terminal.cwd, &self.repositories).is_none());
        self.terminals = terminals;
        self.active_tab_index = active_tab_index;
        self.activity = aggregate_activity(&self.terminals, &self.repositories);
        if has_unknown_directory {
            self.refresh_repositories(ctx);
        }
        ctx.notify();
    }

    /// The worktree containing `cwd`, if it belongs to a known repository.
    pub fn worktree_containing(&self, cwd: &Path) -> Option<&Path> {
        worktree_containing(cwd, &self.repositories)
    }

    /// Re-lists worktrees for every known repository plus any directory a terminal is in.
    pub fn refresh_repositories(&mut self, ctx: &mut ViewContext<Self>) {
        if self.git_refresh_in_flight {
            return;
        }
        let mut seen = HashSet::new();
        let candidates: Vec<PathBuf> = self
            .repositories
            .iter()
            .map(|repository| repository.root.clone())
            .chain(
                PersistedWorkspace::as_ref(ctx)
                    .workspaces()
                    .map(|workspace| workspace.path),
            )
            .chain(self.terminals.iter().map(|terminal| terminal.cwd.clone()))
            .map(
                |path| match worktree_containing(&path, &self.repositories) {
                    Some(_) => self
                        .repositories
                        .iter()
                        .find(|repository| {
                            repository
                                .worktrees
                                .iter()
                                .any(|worktree| path.starts_with(&worktree.path))
                        })
                        .map(|repository| repository.root.clone())
                        .unwrap_or(path),
                    None => path,
                },
            )
            .filter(|path| seen.insert(path.clone()))
            .collect();

        self.git_refresh_in_flight = true;
        ctx.spawn(
            async move {
                let lookups = candidates
                    .into_iter()
                    .map(|path| async move { list_repository(&path).await });
                futures::future::join_all(lookups).await
            },
            |me, repositories, ctx| {
                me.git_refresh_in_flight = false;
                me.set_repositories(repositories.into_iter().flatten().collect(), ctx);
            },
        );
    }

    fn set_repositories(&mut self, repositories: Vec<Repository>, ctx: &mut ViewContext<Self>) {
        let mut seen = HashSet::new();
        let mut repositories: Vec<Repository> = repositories
            .into_iter()
            .filter(|repository| seen.insert(repository.root.clone()))
            .collect();
        repositories.sort_by_key(|repository| repository.name.to_lowercase());
        if repositories == self.repositories {
            return;
        }

        self.repository_mouse_states
            .retain(|root, _| repositories.iter().any(|r| &r.root == root));
        self.worktree_mouse_states.retain(|path, _| {
            repositories
                .iter()
                .flat_map(|r| &r.worktrees)
                .any(|w| &w.path == path)
        });
        for repository in &repositories {
            self.repository_mouse_states
                .entry(repository.root.clone())
                .or_default();
            for worktree in &repository.worktrees {
                self.worktree_mouse_states
                    .entry(worktree.path.clone())
                    .or_default();
            }
        }

        self.repositories = repositories;
        self.activity = aggregate_activity(&self.terminals, &self.repositories);
        ctx.notify();
    }

    /// Removes linked worktrees whose branch has a merged pull request, when enabled in settings.
    /// Worktrees with an open terminal are skipped, and `git worktree remove` (without `--force`)
    /// refuses to delete worktrees with uncommitted changes.
    fn remove_worktrees_with_merged_prs(&mut self, ctx: &mut ViewContext<Self>) {
        if self.merged_pr_check_in_flight
            || !*WorktreeSettings::as_ref(ctx).delete_worktree_on_pr_merge
        {
            return;
        }
        let candidates: Vec<(PathBuf, PathBuf)> = self
            .repositories
            .iter()
            .flat_map(|repository| {
                repository
                    .worktrees
                    .iter()
                    .filter(|worktree| !worktree.is_main && worktree.branch.is_some())
                    .map(|worktree| (repository.root.clone(), worktree.path.clone()))
            })
            .filter(|(_, worktree)| !self.activity.contains_key(worktree))
            .collect();
        if candidates.is_empty() {
            return;
        }

        #[cfg(feature = "local_tty")]
        let path_env_future = {
            use crate::terminal::local_shell::LocalShellState;
            // `gh` is often only on the interactive shell's PATH when Warp is launched from the
            // macOS GUI.
            LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
                shell_state.get_interactive_path_env_var(ctx)
            })
        };
        #[cfg(not(feature = "local_tty"))]
        let path_env_future = futures::future::ready(None::<String>);

        self.merged_pr_check_in_flight = true;
        ctx.spawn(
            async move {
                let path_env = path_env_future.await;
                let mut merged = Vec::new();
                for (repository, worktree) in candidates {
                    match get_pr_for_branch(&worktree, path_env.as_deref()).await {
                        Ok(Some(pr)) if pr.state.eq_ignore_ascii_case("merged") => {
                            merged.push((repository, worktree, pr.number));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            log::debug!("Skipping merged PR check for worktree: {error:#}");
                        }
                    }
                }
                merged
            },
            |me, merged, ctx| {
                me.merged_pr_check_in_flight = false;
                for (repository, worktree, pr_number) in merged {
                    if me.activity.contains_key(&worktree) {
                        continue;
                    }
                    me.remove_worktree(
                        repository,
                        worktree,
                        WorktreeRemoval::PullRequestMerged(pr_number),
                        ctx,
                    );
                }
            },
        );
    }

    fn remove_worktree(
        &mut self,
        repository: PathBuf,
        worktree: PathBuf,
        reason: WorktreeRemoval,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        let worktree_name = worktree
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let branch_to_delete = match &reason {
            WorktreeRemoval::Requested { delete_branch } => delete_branch.clone(),
            WorktreeRemoval::PullRequestMerged(_) => None,
        };
        ctx.spawn(
            async move {
                let worktree_arg = worktree.to_string_lossy().into_owned();
                let worktree_result =
                    run_git_command(&repository, &["worktree", "remove", &worktree_arg]).await;
                let branch_result = match (&worktree_result, branch_to_delete) {
                    // `-d` rather than `-D`, so git refuses to drop commits that aren't merged.
                    (Ok(_), Some(branch)) => {
                        let result =
                            run_git_command(&repository, &["branch", "-d", "--", &branch]).await;
                        Some(match result {
                            Ok(_) => Ok(branch),
                            Err(error) => Err((branch, error)),
                        })
                    }
                    _ => None,
                };
                (worktree_result, branch_result)
            },
            move |me, (worktree_result, branch_result), ctx| {
                let toast = match (worktree_result, &reason, branch_result) {
                    (Ok(_), WorktreeRemoval::PullRequestMerged(pr_number), _) => {
                        DismissibleToast::success(format!(
                            "Removed worktree {worktree_name} because PR #{pr_number} was merged."
                        ))
                    }
                    (Ok(_), WorktreeRemoval::Requested { .. }, None) => {
                        DismissibleToast::success(format!("Deleted worktree {worktree_name}."))
                    }
                    (Ok(_), WorktreeRemoval::Requested { .. }, Some(Ok(branch))) => {
                        DismissibleToast::success(format!(
                            "Deleted worktree {worktree_name} and branch {branch}."
                        ))
                    }
                    (Ok(_), WorktreeRemoval::Requested { .. }, Some(Err((branch, error)))) => {
                        DismissibleToast::error(format!(
                            "Deleted worktree {worktree_name}, but kept branch {branch}: {}",
                            git_error_message(&error)
                        ))
                    }
                    (Err(error), WorktreeRemoval::PullRequestMerged(_), _) => {
                        log::info!("Did not remove worktree after its PR merged: {error:#}");
                        return;
                    }
                    (Err(error), WorktreeRemoval::Requested { .. }, _) => {
                        DismissibleToast::error(format!(
                            "Couldn't delete worktree {worktree_name}: {}",
                            git_error_message(&error)
                        ))
                    }
                };
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                me.refresh_repositories(ctx);
            },
        );
    }

    fn is_repository_expanded(&self, repository: &Repository) -> bool {
        self.expansion_overrides
            .get(&repository.root)
            .copied()
            .unwrap_or_else(|| {
                repository
                    .worktrees
                    .iter()
                    .any(|worktree| self.activity.contains_key(&worktree.path))
            })
    }

    fn worktree(&self, path: &Path) -> Option<&model::Worktree> {
        self.repositories
            .iter()
            .flat_map(|repository| &repository.worktrees)
            .find(|worktree| worktree.path == path)
    }

    fn is_worktree_selected(&self, worktree: &Path) -> bool {
        self.activity
            .get(worktree)
            .is_some_and(|activity| activity.tab_indices.contains(&self.active_tab_index))
    }
}

enum WorktreeRemoval {
    PullRequestMerged(u64),
    Requested {
        /// Local branch to delete along with the worktree.
        delete_branch: Option<String>,
    },
}

/// Git's own explanation (e.g. "contains modified or untracked files"), without the wrapper text.
fn git_error_message(error: &anyhow::Error) -> String {
    let message = error.to_string();
    message
        .lines()
        .map(str::trim)
        .map(|line| {
            line.strip_prefix("Git command failed:")
                .unwrap_or(line)
                .trim()
        })
        .find_map(|line| line.strip_prefix("fatal:").or(line.strip_prefix("error:")))
        .map(|line| line.trim().to_owned())
        .unwrap_or(message)
}

async fn list_repository(path: &Path) -> Option<Repository> {
    let output = run_git_command(path, &["worktree", "list", "--porcelain"])
        .await
        .ok()?;
    let worktrees = parse_worktree_porcelain(&output)
        .into_iter()
        .map(|mut worktree| {
            // Terminal working directories are canonicalized, so worktree paths must be too.
            if let Ok(canonical) = std::fs::canonicalize(&worktree.path) {
                worktree.path = canonical;
            }
            worktree
        })
        .collect();
    Repository::from_worktrees(worktrees)
}

/// Reports which coding agent, if any, is running in `terminal_view` and how busy it is.
pub fn agent_activity_for_terminal(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<AgentActivity> {
    let cli_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    let name = match cli_session.map(|session| session.agent) {
        Some(CLIAgent::Unknown) => "Agent".to_owned(),
        Some(agent) => agent.display_name().to_owned(),
        None if terminal_view.is_ambient_agent_session(app) => "Cloud agent".to_owned(),
        None => "Warp Agent".to_owned(),
    };
    let state = match summary_conversation_status_for_terminal(terminal_view, app) {
        Some(status) => agent_state(&status),
        None if cli_session.is_some() => AgentState::Running,
        None => return None,
    };
    Some(AgentActivity { name, state })
}

fn agent_state(status: &ConversationStatus) -> AgentState {
    match status {
        ConversationStatus::InProgress | ConversationStatus::TransientError => AgentState::Working,
        ConversationStatus::Blocked { .. } => AgentState::NeedsInput,
        ConversationStatus::Success
        | ConversationStatus::Error
        | ConversationStatus::Cancelled
        | ConversationStatus::WaitingForEvents => AgentState::Idle,
    }
}

impl Entity for WorktreePanelView {
    type Event = WorktreePanelEvent;
}

impl TypedActionView for WorktreePanelView {
    type Action = WorktreePanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WorktreePanelAction::ToggleRepository(root) => {
                if let Some(repository) = self.repositories.iter().find(|r| &r.root == root) {
                    let expanded = self.is_repository_expanded(repository);
                    self.expansion_overrides.insert(root.clone(), !expanded);
                    ctx.notify();
                }
            }
            WorktreePanelAction::ClickWorktree(path) => {
                if let Some(pending) = self.pending_single_click.take() {
                    pending.abort();
                }
                let path = path.clone();
                self.pending_single_click = Some(ctx.spawn(
                    async { Timer::after(DOUBLE_CLICK_INTERVAL).await },
                    move |me, _, ctx| {
                        me.pending_single_click = None;
                        ctx.emit(WorktreePanelEvent::OpenWorktree(path));
                    },
                ));
            }
            WorktreePanelAction::DoubleClickWorktree(path) => {
                if let Some(pending) = self.pending_single_click.take() {
                    pending.abort();
                }
                ctx.emit(WorktreePanelEvent::OpenWorktreeInNewTab(path.clone()));
            }
            WorktreePanelAction::OpenWorktree(path) => {
                self.context_menu_position = None;
                ctx.emit(WorktreePanelEvent::OpenWorktree(path.clone()));
            }
            WorktreePanelAction::OpenWorktreeInNewTab(path) => {
                self.context_menu_position = None;
                ctx.emit(WorktreePanelEvent::OpenWorktreeInNewTab(path.clone()));
            }
            WorktreePanelAction::OpenWorktreeContextMenu { worktree, position } => {
                let Some(worktree) = self.worktree(worktree) else {
                    return;
                };
                let mut items = vec![
                    MenuItemFields::new("Open")
                        .with_on_select_action(WorktreePanelAction::OpenWorktree(
                            worktree.path.clone(),
                        ))
                        .into_item(),
                    MenuItemFields::new("Open in new tab")
                        .with_on_select_action(WorktreePanelAction::OpenWorktreeInNewTab(
                            worktree.path.clone(),
                        ))
                        .into_item(),
                    MenuItem::Separator,
                ];
                if worktree.is_main {
                    items.push(
                        MenuItemFields::new("Delete worktree")
                            .with_disabled(true)
                            .with_tooltip("The main worktree can't be deleted")
                            .into_item(),
                    );
                } else {
                    items.push(
                        MenuItemFields::new("Delete worktree")
                            .with_on_select_action(WorktreePanelAction::DeleteWorktree {
                                worktree: worktree.path.clone(),
                                delete_branch: false,
                            })
                            .into_item(),
                    );
                    if let Some(branch) = &worktree.branch {
                        items.push(
                            MenuItemFields::new("Delete worktree and local branch")
                                .with_tooltip(format!(
                                    "Also deletes {branch} if it has been merged"
                                ))
                                .with_on_select_action(WorktreePanelAction::DeleteWorktree {
                                    worktree: worktree.path.clone(),
                                    delete_branch: true,
                                })
                                .into_item(),
                        );
                    }
                }
                self.context_menu.update(ctx, move |menu, ctx| {
                    menu.set_items(items, ctx);
                    ctx.notify();
                });
                self.context_menu_position = Some(*position);
                ctx.notify();
            }
            WorktreePanelAction::DeleteWorktree {
                worktree,
                delete_branch,
            } => {
                self.context_menu_position = None;
                let target = self.repositories.iter().find_map(|repository| {
                    repository
                        .worktrees
                        .iter()
                        .find(|candidate| &candidate.path == worktree && !candidate.is_main)
                        .map(|candidate| (repository.root.clone(), candidate.branch.clone()))
                });
                if let Some((root, branch)) = target {
                    let reason = WorktreeRemoval::Requested {
                        delete_branch: branch.filter(|_| *delete_branch),
                    };
                    self.remove_worktree(root, worktree.clone(), reason, ctx);
                }
                ctx.notify();
            }
            WorktreePanelAction::NewWorktree(repository) => {
                ctx.emit(WorktreePanelEvent::NewWorktree {
                    repository: repository.clone(),
                });
            }
            WorktreePanelAction::AddRepository => ctx.emit(WorktreePanelEvent::AddRepository),
            WorktreePanelAction::Refresh => {
                ctx.emit(WorktreePanelEvent::RequestTerminalActivity);
                self.refresh_repositories(ctx);
            }
        }
    }
}

impl View for WorktreePanelView {
    fn ui_name() -> &'static str {
        "WorktreePanelView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut list = Flex::column().with_main_axis_size(MainAxisSize::Min);
        if self.repositories.is_empty() {
            list.add_child(render_empty_state(appearance));
        }
        for repository in &self.repositories {
            list.add_child(self.render_repository_header(repository, appearance));
            if self.is_repository_expanded(repository) {
                for worktree in &repository.worktrees {
                    list.add_child(self.render_worktree_row(worktree, appearance));
                }
            }
        }

        let theme = appearance.theme();
        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            Container::new(list.finish())
                .with_padding_left(6.)
                .with_padding_right(6.)
                .with_padding_bottom(8.)
                .finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let panel = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_header(appearance))
            .with_child(Shrinkable::new(1., scrollable).finish())
            .finish();

        let mut stack = Stack::new();
        stack.add_child(SavePosition::new(panel, &self.position_id).finish());
        if let Some(position) = self.context_menu_position {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    position,
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }
        stack.finish()
    }
}

impl WorktreePanelView {
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title = Text::new_inline(
            "Worktrees".to_owned(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .finish();

        let buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.)
            .with_child(header_button(
                appearance,
                Icon::Plus,
                "Add repository",
                self.header_mouse_states.add_repository.clone(),
                WorktreePanelAction::AddRepository,
            ))
            .with_child(header_button(
                appearance,
                Icon::Refresh,
                "Refresh",
                self.header_mouse_states.refresh.clone(),
                WorktreePanelAction::Refresh,
            ))
            .finish();

        Container::new(
            ConstrainedBox::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Shrinkable::new(1., title).finish())
                    .with_child(buttons)
                    .finish(),
            )
            .with_height(HEADER_HEIGHT)
            .finish(),
        )
        .with_padding_left(12.)
        .with_padding_right(6.)
        .finish()
    }

    fn render_repository_header(
        &self,
        repository: &Repository,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let Some(mouse_states) = self.repository_mouse_states.get(&repository.root) else {
            return Empty::new().finish();
        };
        let theme = appearance.theme();
        let sub_text = theme.sub_text_color(theme.background());
        let expanded = self.is_repository_expanded(repository);
        let chevron = sized_icon(
            if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            sub_text,
        );
        let name = Text::new_inline(
            repository.name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .finish();
        let count = Text::new_inline(
            repository.worktrees.len().to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size() - 2.,
        )
        .with_color(sub_text.into_solid())
        .finish();
        let new_worktree_button = header_button(
            appearance,
            Icon::Plus,
            "New worktree",
            mouse_states.new_worktree.clone(),
            WorktreePanelAction::NewWorktree(repository.root.clone()),
        );

        let content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(chevron)
            .with_child(Shrinkable::new(1., name).finish())
            .with_child(count)
            .with_child(new_worktree_button)
            .finish();
        let root = repository.root.clone();
        Hoverable::new(mouse_states.header.clone(), move |state| {
            let mut container = Container::new(content)
                .with_padding_left(6.)
                .with_padding_right(2.)
                .with_padding_top(3.)
                .with_padding_bottom(3.)
                .with_margin_top(6.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_CORNER_RADIUS)));
            if state.is_hovered() {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorktreePanelAction::ToggleRepository(root.clone()));
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_worktree_row(
        &self,
        worktree: &model::Worktree,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let Some(mouse_state) = self.worktree_mouse_states.get(&worktree.path) else {
            return Empty::new().finish();
        };
        let theme = appearance.theme();
        let sub_text = theme.sub_text_color(theme.background());
        let activity = self.activity.get(&worktree.path);
        let state = activity.and_then(WorktreeActivity::state);
        let is_selected = self.is_worktree_selected(&worktree.path);

        let name = Text::new_inline(
            worktree.display_name(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .finish();
        let subtitle = activity
            .and_then(WorktreeActivity::summary)
            .or_else(|| worktree.is_main.then(|| "Main worktree".to_owned()))
            .or_else(|| activity.map(|_| "Open".to_owned()));

        let mut text_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(name);
        if let Some(subtitle) = subtitle {
            text_column.add_child(
                Text::new_inline(
                    subtitle,
                    appearance.ui_font_family(),
                    appearance.ui_font_size() - 2.,
                )
                .with_color(sub_text.into_solid())
                .finish(),
            );
        }

        let leading_icon = sized_icon(
            if worktree.is_main {
                Icon::Folder
            } else {
                Icon::GitBranch
            },
            sub_text,
        );
        let content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(leading_icon)
            .with_child(Shrinkable::new(1., text_column.finish()).finish())
            .with_child(status_dot(state, activity.is_some(), appearance))
            .finish();

        let path = worktree.path.clone();
        let double_click_path = worktree.path.clone();
        let context_menu_path = worktree.path.clone();
        let position_id = self.position_id.clone();
        Hoverable::new(mouse_state.clone(), move |hover| {
            let mut container = Container::new(content)
                .with_padding_left(WORKTREE_INDENT)
                .with_padding_right(8.)
                .with_padding_top(4.)
                .with_padding_bottom(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_CORNER_RADIUS)));
            if is_selected {
                container = container.with_background(internal_colors::fg_overlay_3(theme));
            } else if hover.is_hovered() {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }
            container.finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorktreePanelAction::ClickWorktree(path.clone()));
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorktreePanelAction::DoubleClickWorktree(
                double_click_path.clone(),
            ));
        })
        .on_right_click(move |ctx, _, position| {
            let Some(panel_bounds) = ctx.element_position_by_id(&position_id) else {
                return;
            };
            ctx.dispatch_typed_action(WorktreePanelAction::OpenWorktreeContextMenu {
                worktree: context_menu_path.clone(),
                position: position - panel_bounds.origin(),
            });
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

fn render_empty_state(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        Text::new(
            "No repositories yet. Open a terminal in a git repository or add one with +."
                .to_owned(),
            appearance.ui_font_family(),
            appearance.ui_font_size() - 1.,
        )
        .with_color(theme.sub_text_color(theme.background()).into_solid())
        .finish(),
    )
    .with_padding_left(6.)
    .with_padding_right(6.)
    .with_padding_top(8.)
    .finish()
}

fn sized_icon(icon: Icon, color: ThemeFill) -> Box<dyn Element> {
    ConstrainedBox::new(icon.to_warpui_icon(color).finish())
        .with_width(ICON_SIZE)
        .with_height(ICON_SIZE)
        .finish()
}

fn header_button(
    appearance: &Appearance,
    icon: Icon,
    tooltip: &'static str,
    mouse_state: MouseStateHandle,
    action: WorktreePanelAction,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder().clone();
    icon_button_with_color(
        appearance,
        icon,
        false,
        mouse_state,
        theme.sub_text_color(theme.background()),
    )
    .with_tooltip(move || ui_builder.tool_tip(tooltip.to_owned()).build().finish())
    .build()
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(action.clone());
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

/// Colored dot summarizing agent state. Worktrees with an open terminal but no agent get a
/// hollow-looking neutral dot; worktrees with nothing open get no dot.
fn status_dot(
    state: Option<AgentState>,
    has_open_terminal: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let color: Option<ColorU> = match state {
        Some(AgentState::Working) => Some(theme.ansi_fg_magenta()),
        Some(AgentState::NeedsInput) => Some(theme.ansi_fg_yellow()),
        Some(AgentState::Running) => Some(theme.ansi_fg_blue()),
        Some(AgentState::Idle) => Some(theme.ansi_fg_green()),
        None if has_open_terminal => Some(internal_colors::neutral_5(theme)),
        None => None,
    };
    let Some(color) = color else {
        return ConstrainedBox::new(Empty::new().finish())
            .with_width(STATUS_DOT_SIZE)
            .with_height(STATUS_DOT_SIZE)
            .finish();
    };
    ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background_color(color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(STATUS_DOT_SIZE / 2.)))
            .finish(),
    )
    .with_width(STATUS_DOT_SIZE)
    .with_height(STATUS_DOT_SIZE)
    .finish()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
