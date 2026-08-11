use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::NaiveDateTime;
use pathfinder_geometry::rect::RectF;
use serde::{Deserialize, Serialize};
use warpui::platform::FullscreenState;
use warpui::{AppContext, SingletonEntity as _};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentManagementFilters;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::{InputConfig, SerializedBlockListItem};
use crate::code::editor_management::CodeSource;
use crate::drive::OpenWarpDriveObjectSettings;
use crate::root_view::quake_mode_window_id;
use crate::server::ids::{ServerId, SyncId};
use crate::settings_view::SettingsSection;
use crate::settings_view::environments_page::EnvironmentsPage;
use crate::tab::SelectedTabColor;
use crate::terminal::cli_agent_resume::RecordedFlag;
use crate::terminal::{CLIAgent, ShellLaunchData};
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::WorkspaceRegistry;
use crate::workspace::tab_group::TabGroupId;
use crate::workspace::view::left_panel::ToolPanelView;

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub windows: Vec<WindowSnapshot>,
    pub active_window_index: Option<usize>,
    pub block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
    /// Agent CLI state recorded per pane. Unlike the rest of this struct it is not written by a
    /// snapshot save; it is read from its own table, which snapshot saves leave alone.
    pub agent_sessions: Arc<HashMap<PaneUuid, RecordedAgentSession>>,
    pub running_mcp_servers: Vec<uuid::Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaneUuid(pub Vec<u8>);

/// The agent CLI a pane was last observed running, recorded so a restart can offer to resume it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedAgentSession {
    pub agent: CLIAgent,
    /// The session identifier the agent itself reported.
    pub session_id: String,
    /// The allowlisted flags of the invocation the user ran that matter when relaunching the
    /// agent. Never the command line itself: replaying what the user typed would replay secret
    /// placeholders and unexpandable aliases along with it.
    pub flags: Vec<RecordedFlag>,
    /// The directory the agent was running in. Recorded here rather than read back from the
    /// pane snapshot so that eligibility can compare it against the directory the pane
    /// actually restored into.
    pub directory: PathBuf,
    pub observed_at: NaiveDateTime,
}

/// Recorded agent sessions handed to pane restoration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentSessionRestore {
    pub sessions: Arc<HashMap<PaneUuid, RecordedAgentSession>>,
    /// The panes that own the identifier they recorded, resolved across every window before the
    /// first one is created. Panes left out of it recorded an identifier another pane won.
    pub claimed_panes: Arc<HashSet<PaneUuid>>,
    /// Mid-session restores (a tab added from a snapshot) reach the same restore path as
    /// startup, and resuming an agent there would be wrong, so the startup pass says so
    /// explicitly instead of leaving it to be inferred.
    pub is_startup_restore: bool,
}

impl AgentSessionRestore {
    /// The state recorded for `pane_uuid`, and only on the startup restore pass.
    pub fn recorded_on_startup(&self, pane_uuid: &PaneUuid) -> Option<&RecordedAgentSession> {
        self.is_startup_restore
            .then(|| self.sessions.get(pane_uuid))
            .flatten()
    }

    /// Whether `pane_uuid` is the pane that gets to resume the identifier it recorded.
    pub fn owns_recorded_identifier(&self, pane_uuid: &PaneUuid) -> bool {
        self.claimed_panes.contains(pane_uuid)
    }
}

/// Wrapper for persisting agent management filters to restore.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAgentManagementFilters {
    pub filters: AgentManagementFilters,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub tabs: Vec<TabSnapshot>,
    pub active_tab_index: usize,
    pub team_uid: Option<ServerId>,
    pub bounds: Option<RectF>,
    pub fullscreen_state: FullscreenState,
    pub quake_mode: bool,
    pub universal_search_width: Option<f32>,
    pub warp_ai_width: Option<f32>,
    pub voltron_width: Option<f32>,
    pub warp_drive_index_width: Option<f32>,
    pub left_panel_open: bool,
    pub vertical_tabs_panel_open: bool,
    pub left_panel_width: Option<f32>,
    pub right_panel_width: Option<f32>,
    pub agent_management_filters: Option<PersistedAgentManagementFilters>,
    /// Tab groups defined in this window. Group order is implicit from
    /// member tabs' positions, so no explicit ordering is persisted.
    pub tab_groups: Vec<TabGroupSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabGroupSnapshot {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub color: SelectedTabColor,
    pub collapsed: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabSnapshot {
    pub custom_title: Option<String>,
    pub root: PaneNodeSnapshot,
    pub default_directory_color: Option<AnsiColorIdentifier>,
    pub selected_color: SelectedTabColor,
    pub left_panel: Option<LeftPanelSnapshot>,
    pub right_panel: Option<RightPanelSnapshot>,
    /// Tab group this tab belongs to, if any.
    pub group_id: Option<TabGroupId>,
    /// True when this tab is pinned to the front of the tab list.
    pub pinned: bool,
}

impl TabSnapshot {
    pub(crate) fn color(&self) -> Option<AnsiColorIdentifier> {
        self.selected_color.resolve(self.default_directory_color)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "LeafSnapshot is significantly larger than BranchSnapshot due to nested snapshot types."
)]
pub enum PaneNodeSnapshot {
    Branch(BranchSnapshot),
    Leaf(LeafSnapshot),
}

impl PaneNodeSnapshot {
    pub fn has_horizontal_split(&self) -> bool {
        match self {
            PaneNodeSnapshot::Leaf(_) => false,
            PaneNodeSnapshot::Branch(BranchSnapshot {
                direction,
                children,
            }) => {
                let self_has_split = *direction == SplitDirection::Horizontal && children.len() > 1;
                self_has_split
                    || children
                        .iter()
                        .any(|(_, child)| child.has_horizontal_split())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BranchSnapshot {
    pub direction: SplitDirection,
    pub children: Vec<(PaneFlex, PaneNodeSnapshot)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeafSnapshot {
    pub is_focused: bool,
    pub custom_vertical_tabs_title: Option<String>,
    pub contents: LeafContents,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LeafContents {
    Terminal(TerminalPaneSnapshot),
    Notebook(NotebookPaneSnapshot),
    AIDocument(AIDocumentPaneSnapshot),
    Code(CodePaneSnapShot),
    EnvVarCollection(EnvVarCollectionPaneSnapshot),
    EnvironmentManagement(EnvironmentManagementPaneSnapshot),
    Workflow(WorkflowPaneSnapshot),
    Settings(SettingsPaneSnapshot),
    AIFact(AIFactPaneSnapshot),
    CustomRouterEditor,
    ExecutionProfileEditor,
    CodeReview(CodeReviewPaneSnapshot),
    AmbientAgent(AmbientAgentPaneSnapshot),
    /// The in-app network log pane. Not persisted across restarts because the
    /// backing log is an in-memory ring buffer that starts empty on launch.
    NetworkLog,
    /// A new first-time user experience which prioritizes choosing a coding repository.
    GetStarted,
}

#[cfg(feature = "local_fs")]
impl LeafContents {
    /// Whether this pane content should be written to (and later restored
    /// from) the SQLite app-state database.
    ///
    /// Non-persisted pane types are skipped entirely during the pane tree
    /// traversal in `save_app_state`, so no `pane_nodes` row is inserted for
    /// them. This is important: inserting a `pane_nodes` row with
    /// `is_leaf = true` but no matching `pane_leaves` row leaves an orphan
    /// that `read_node` cannot resolve, which causes the surrounding tab's
    /// restoration to fail and the whole tab to disappear on restart.
    pub(crate) fn is_persisted(&self) -> bool {
        match self {
            // Network log: the backing log is an in-memory ring buffer that
            // starts empty on launch; persisting would also regress back to
            // an on-disk log via the app-state database.
            LeafContents::NetworkLog
            // Environment management panes are opened on-demand via workspace
            // actions and have no persistable state.
            | LeafContents::EnvironmentManagement(_) => false,
            LeafContents::Terminal(_)
            | LeafContents::Notebook(_)
            | LeafContents::AIDocument(_)
            | LeafContents::Code(_)
            | LeafContents::EnvVarCollection(_)
            | LeafContents::Workflow(_)
            | LeafContents::Settings(_)
            | LeafContents::AIFact(_)
            | LeafContents::CustomRouterEditor
            | LeafContents::ExecutionProfileEditor
            | LeafContents::CodeReview(_)
            | LeafContents::AmbientAgent(_)
            | LeafContents::GetStarted => true,
        }
    }
}

/// Snapshot of an ambient agent pane.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientAgentPaneSnapshot {
    pub uuid: Vec<u8>,
    // `task_id` is purposefully optional,
    // as you can have a valid state (i.e. an empty cloud mode pane) where it is None.
    pub task_id: Option<AmbientAgentTaskId>,
}

/// Snapshot of the contents of a terminal pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPaneSnapshot {
    pub uuid: Vec<u8>,
    pub cwd: Option<String>,
    pub shell_launch_data: Option<ShellLaunchData>,
    pub is_active: bool,
    pub is_read_only: bool,
    pub input_config: Option<InputConfig>,
    pub llm_model_override: Option<String>,
    pub active_profile_id: Option<SyncId>,
    pub conversation_ids_to_restore: Vec<AIConversationId>,
    /// The active conversation ID if the agent view was open in fullscreen mode.
    /// When `Some`, the agent view should be restored to fullscreen for this conversation.
    pub active_conversation_id: Option<AIConversationId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotebookPaneSnapshot {
    CloudNotebook {
        /// The ID of the notebook that was open in this pane. There are 3 possibilities:
        /// 1. The pane contains a newly-created notebook that has not been edited yet. It might not
        ///    have an ID yet (client or server), so this will be `None`.
        /// 2. The pane contains a notebook that hasn't been synced to the server yet, so this will
        ///    contain a client ID that should exist in SQLite.
        /// 3. The pane contains a notebook that's known to the server, so this will contain the
        ///    server ID.
        notebook_id: Option<SyncId>,
        // Settings for the notebook pane when it's opened (such as a folder to focus upon opening)
        settings: OpenWarpDriveObjectSettings,
    },
    LocalFileNotebook {
        /// The path to the local file that was open in this pane. This may be `None` if
        /// the pane contained an unreadable file.
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIDocumentPaneSnapshot {
    Local {
        document_id: String,
        version: i32,
        content: Option<String>,
        title: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodePaneTabSnapshot {
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodePaneSnapShot {
    Local {
        tabs: Vec<CodePaneTabSnapshot>,
        active_tab_index: usize,
        /// The full `CodeSource` for this pane, serialized as JSON in the DB.
        source: Option<CodeSource>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowPaneSnapshot {
    CloudWorkflow {
        workflow_id: Option<SyncId>,
        // Settings for the workflow pane when it's opened (such as a folder to focus upon opening)
        settings: OpenWarpDriveObjectSettings,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EnvVarCollectionPaneSnapshot {
    // CloudEnvVarCollection snapshots operate under the same heuristics
    // as NotebookPaneSnapshot::CloudNotebook
    CloudEnvVarCollection {
        env_var_collection_id: Option<SyncId>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentManagementPaneSnapshot {
    pub mode: EnvironmentsPage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsPaneSnapshot {
    Local {
        current_page: SettingsSection,
        search_query: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AIFactPaneSnapshot {
    Personal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeReviewPaneSnapshot {
    Local {
        terminal_uuid: Vec<u8>,
        repo_path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LeftPanelDisplayedTab {
    FileTree,
    GlobalSearch,
    WarpDrive,
    ConversationListView,
}

impl From<ToolPanelView> for LeftPanelDisplayedTab {
    fn from(view: ToolPanelView) -> Self {
        match view {
            ToolPanelView::ProjectExplorer => LeftPanelDisplayedTab::FileTree,
            ToolPanelView::GlobalSearch { .. } => LeftPanelDisplayedTab::GlobalSearch,
            ToolPanelView::WarpDrive => LeftPanelDisplayedTab::WarpDrive,
            ToolPanelView::ConversationListView => LeftPanelDisplayedTab::ConversationListView,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeftPanelSnapshot {
    pub left_panel_displayed_tab: LeftPanelDisplayedTab,
    pub pane_group_id: String,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RightPanelSnapshot {
    pub pane_group_id: String,
    pub width: usize,
    pub is_maximized: bool,
}

/// Copied from pane group model, which should be private to pane group.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneFlex(pub f32);

pub fn get_app_state(app: &AppContext) -> AppState {
    let active_window_id = app.windows().active_window();
    let quake_mode_id = quake_mode_window_id();

    let mut active_window_index = None;

    let mut windows = vec![];

    for (index, window_id) in app.window_ids().enumerate() {
        // Determine index of active window
        if let Some(active_window_id) = active_window_id
            && active_window_id == window_id
        {
            active_window_index = Some(index);
        }

        if let Some(workspace) = WorkspaceRegistry::as_ref(app).get(window_id, app) {
            let ws = workspace.as_ref(app);
            // Transient drag-preview windows are not real user-visible
            // workspaces; skip them so they never end up in the persisted
            // session. (Persistence is also short-circuited entirely while a
            // cross-window drag is active; see `save_app` in
            // `workspace/global_actions.rs`.)
            if ws.is_tab_drag_preview() {
                continue;
            }
            let snapshot = ws.snapshot(
                window_id,
                quake_mode_id.map(|id| id == window_id).unwrap_or(false),
                app,
            );
            if !snapshot.tabs.is_empty() {
                windows.push(snapshot);
            }
        }
    }

    AppState {
        windows,
        active_window_index,
        block_lists: Default::default(),
        agent_sessions: Default::default(),
        running_mcp_servers: Vec::new(),
    }
}

#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;
