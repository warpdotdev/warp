use std::collections::HashSet;

use chrono::NaiveDateTime;

use warpui::{AppContext, Entity, EntityId, WindowId};

use crate::context_chips::prompt_snapshot::PromptSnapshot;
use crate::pane_group::PaneGroup;
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::{
    pane_group::PaneId,
    workspace::{PaneViewLocator, Workspace},
};

/// Contains session metadata, including a prompt and running command (if there is one).
#[derive(Clone)]
pub struct SessionNavigationData {
    /// The prompt of the session.
    prompt: String,
    /// The various parts of the prompt, like virtual environment and working directory.
    prompt_elements: SessionNavigationPromptElements,
    /// A running command, if there is one.
    command_context: CommandContext,
    /// A `PaneViewLocator` to navigate to the session.
    pane_view_locator: PaneViewLocator,
    /// The id of the window the session is located in.
    window_id: WindowId,
    /// The timestamp of the last interaction.
    last_focus_ts: Option<NaiveDateTime>,
    /// Whether or not the session is in a read-only state.
    is_read_only: bool,
    /// The sharing status of the session.
    shared_session_status: SharedSessionStatus,
}

impl SessionNavigationData {
    /// Returns whether the session is the session identified by `session_id`.
    pub fn is_for_session(&self, session_id: PaneId) -> bool {
        session_id == self.pane_view_locator().pane_id
    }
}

/// Contains prompt data for rendering in the command palette.
#[derive(Clone)]
pub struct SessionNavigationPromptElements {
    /// The raw terminal grid of the PS1 prompt, populated when `honor_ps1` is
    /// active. When present, the command palette renders this grid directly.
    pub ps1_prompt_grid: Option<BlockGrid>,
    /// A snapshot of the user's configured prompt chips and their current
    /// values. Used as the default prompt representation in the command palette.
    pub prompt_chip_snapshot: Option<PromptSnapshot>,
}

/// Represents the execution context of a session - what command or AI interaction
/// was last run or is currently running.
#[derive(Clone, Debug)]
pub enum CommandContext {
    /// The last executed terminal command
    LastRunCommand {
        last_run_command: String,
        mins_since_completion: Option<i64>,
    },
    /// The last completed AI interaction
    LastRunAIBlock {
        prompt: String, // The prompt that initiated the AI interaction
    },
    /// Currently running terminal command
    RunningCommand { running_command: String },
    /// Currently running AI interaction
    RunningAIBlock {
        prompt: String, // The prompt for the active AI conversation
    },
    /// No command context (e.g. just launched terminal)
    None,
}

impl CommandContext {
    pub fn a11y_description(&self) -> Option<String> {
        match self {
            Self::None => None,
            Self::LastRunCommand {
                last_run_command, ..
            } => Some(format!("Last run command {}", last_run_command.clone())),
            Self::LastRunAIBlock { prompt } => Some(format!("Last AI interaction: {prompt}")),
            Self::RunningCommand { running_command } => {
                Some(format!("Currently running {running_command}"))
            }
            Self::RunningAIBlock { prompt } => {
                Some(format!("Currently running AI interaction: {prompt}"))
            }
        }
    }
}

impl SessionNavigationData {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt: String,
        prompt_elements: SessionNavigationPromptElements,
        command_context: CommandContext,
        pane_view_locator: PaneViewLocator,
        last_focus_ts: Option<NaiveDateTime>,
        is_read_only: bool,
        window_id: WindowId,
        shared_session_status: SharedSessionStatus,
    ) -> Self {
        SessionNavigationData {
            prompt,
            prompt_elements,
            command_context,
            pane_view_locator,
            last_focus_ts,
            is_read_only,
            window_id,
            shared_session_status,
        }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn prompt_elements(&self) -> &SessionNavigationPromptElements {
        &self.prompt_elements
    }

    pub fn command_context(&self) -> CommandContext {
        self.command_context.clone()
    }

    pub fn pane_view_locator(&self) -> PaneViewLocator {
        self.pane_view_locator
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn last_focus_ts(&self) -> Option<NaiveDateTime> {
        self.last_focus_ts
    }

    pub fn is_read_only(&self) -> bool {
        self.is_read_only
    }

    pub fn shared_session_status(&self) -> SharedSessionStatus {
        self.shared_session_status.clone()
    }

    /// Fetches all sessions currently open in the app.
    pub fn all_sessions(app: &AppContext) -> impl Iterator<Item = SessionNavigationData> + '_ {
        app.window_ids()
            .filter_map(move |window_id| {
                let workspaces = app.views_of_type::<Workspace>(window_id)?;

                Some(workspaces.into_iter().flat_map(move |workspace| {
                    workspace.as_ref(app).workspace_sessions(window_id, app)
                }))
            })
            .flatten()
    }
}

pub struct RunningSessionSummary<'a> {
    /// Does not include long running blocks for viewer of a shared session.
    pub long_running_cmds: Vec<&'a SessionNavigationData>,
}

impl<'a> RunningSessionSummary<'a> {
    pub fn new(sessions: &'a [SessionNavigationData]) -> Self {
        let long_running_cmds: Vec<_> = sessions
            .iter()
            .filter(|session| {
                matches!(
                    session.command_context(),
                    CommandContext::RunningCommand { .. } | CommandContext::RunningAIBlock { .. }
                ) && !session.shared_session_status().is_viewer()
                    && !session.is_read_only()
            })
            .collect();
        Self { long_running_cmds }
    }

    pub fn windows_running(&self) -> HashSet<WindowId> {
        self.long_running_cmds
            .iter()
            .map(|session| session.window_id())
            .collect()
    }

    pub fn tabs_running(&self) -> HashSet<EntityId> {
        self.long_running_cmds
            .iter()
            .map(|session| session.pane_view_locator().pane_group_id)
            .collect()
    }

    pub fn processes_in_window(&self, window_id: &WindowId) -> Vec<&SessionNavigationData> {
        self.long_running_cmds
            .iter()
            .filter(|&session| session.window_id() == *window_id)
            .cloned()
            .collect()
    }
}

pub enum SessionSource {
    None,
    Set {
        active_pane_id: PaneId,
        active_tab_id: EntityId,
        active_window_id: WindowId,
    },
}

impl Entity for SessionSource {
    type Event = ();
}

pub fn num_shared_sessions(ctx: &AppContext) -> usize {
    let mut num_shared_sessions = 0;
    let window_ids: Vec<WindowId> = ctx.window_ids().collect();
    for window_id in window_ids {
        let Some(pane_group_views) = ctx.views_of_type::<PaneGroup>(window_id) else {
            continue;
        };
        for pane_group_view in pane_group_views {
            pane_group_view.read(ctx, |pane_group, ctx| {
                num_shared_sessions += pane_group.number_of_shared_sessions(ctx);
            })
        }
    }
    num_shared_sessions
}

/// Metadata for a single tab, used by the Ctrl+Tab MRU switcher.
#[derive(Clone)]
pub struct TabNavigationData {
    pub pane_group_id: EntityId,
    pub title: String,
    pub subtitle: Option<String>,
    pub window_id: WindowId,
    /// 1-based left-to-right tab index for display disambiguation.
    pub tab_index: usize,
}
