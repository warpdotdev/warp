#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        pub mod agent;
        mod block_list;
        mod sqlite;
        pub mod commands;
    }
}

pub use persistence::model;
#[cfg_attr(not(feature = "local_fs"), expect(unused_imports))]
pub use persistence::schema;

#[cfg(feature = "integration_tests")]
pub mod testing;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;

use ai::project_context::model::ProjectRulePath;
use ai::workspace::WorkspaceMetadata as CodeWorkspaceMetadata;
use chrono::{DateTime, Local, Utc};
use instant::Instant;
use lsp::supported_servers::LSPServerType;
#[cfg(any(feature = "local_fs", feature = "integration_tests"))]
pub use sqlite::database_file_path_for_current_scope;
// Only re-exported for integration tests (via `integration_testing::persistence`);
// in-crate code should resolve paths through `database_file_path_for_current_scope`.
#[cfg(any(feature = "local_fs", feature = "integration_tests"))]
#[cfg_attr(not(feature = "integration_tests"), expect(unused_imports))]
pub use sqlite::database_file_path_for_scope;
#[cfg(any(feature = "local_fs", feature = "integration_tests"))]
pub use sqlite::establish_ro_connection;
use uuid::Uuid;
use warp_core::command::ExitCode;
use warp_errors::report_error;
use warp_graphql::scalars::time::ServerTimestamp;
use warp_multi_agent_api as api;
use warpui::{AppContext, Entity, SingletonEntity};

use self::model::{AgentConversation, AgentConversationData, Project};
use crate::ai::blocklist::PersistedAIInput;
use crate::ai::mcp::TemplatableMCPServerInstallation;
use crate::ai::persisted_workspace::EnablementState;
use crate::app_state::AppState;
use crate::auth::auth_manager::PersistedCurrentUserInformation;
use crate::cloud_object::model::actions::ObjectAction;
use crate::cloud_object::model::generic_string_model::CloudStringObject;
use crate::cloud_object::{
    CloudObject, CloudObjectMetadata, ObjectIdType, RevisionAndLastEditor, ServerCreationInfo,
};
use crate::drive::folders::CloudFolder;
use crate::notebooks::CloudNotebook;
use crate::server::experiments::ServerExperiment;
use crate::server::ids::SyncId;
use crate::suggestions::ignored_suggestions_model::SuggestionType;
use crate::terminal::history::PersistedCommand;
use crate::terminal::model::block::{SerializedAgentViewVisibility, SerializedBlock};
use crate::terminal::model::session::SessionId;
use crate::workflows::CloudWorkflow;
use crate::workspaces::user_profiles::UserProfileWithUID;
use crate::workspaces::workspace::{Workspace as WorkspaceMetadata, WorkspaceUid};

#[derive(Clone)]
pub enum PersistenceScope {
    /// The GUI app (and other launch modes that share its database).
    App,
    /// The `warp-tui` front-end, which keeps its own database so GUI/TUI
    /// version skew can never migrate a shared database out from under the
    /// older binary. Cloud sync is the cross-front-end sharing mechanism.
    Tui,
    RemoteServerDaemon {
        identity_key: String,
    },
}

/// The [`PersistenceScope`] this process's persistence was initialized with.
///
/// Set once by [`initialize`]. Code that opens ad-hoc read-only connections
/// should resolve the database path through [`current_scope`] (or
/// `database_file_path_for_current_scope`) rather than hardcoding a scope, so
/// it reads the same database as the writer regardless of which front-end
/// this process is running.
static CURRENT_SCOPE: OnceLock<PersistenceScope> = OnceLock::new();

/// Returns the scope [`initialize`] was called with, defaulting to
/// [`PersistenceScope::App`] when persistence has not been initialized (e.g.
/// tests that construct models directly).
pub fn current_scope() -> PersistenceScope {
    CURRENT_SCOPE
        .get()
        .cloned()
        .unwrap_or(PersistenceScope::App)
}

/// Which subsets of [`PersistedData`] a launch mode actually consumes.
///
/// Loading everything unconditionally is expensive (GUI session-restore
/// payloads dominate startup on large databases), so headless launch modes
/// opt out of the data they never read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistedDataScope {
    /// The GUI app: everything, including window/tab/block session
    /// restoration and command history.
    Full,
    /// The `warp-tui` front-end: cloud objects and agent/conversation state,
    /// but no GUI session restoration, command history, user profiles, or
    /// pending object actions.
    TuiFrontend,
    /// The remote server daemon: only codebase index metadata.
    CodebaseIndicesOnly,
}

impl PersistedDataScope {
    /// Window/tab/pane snapshots and restored blocks.
    fn session_restoration(self) -> bool {
        matches!(self, PersistedDataScope::Full)
    }

    /// Command history, user profiles, and pending object actions, which
    /// only the GUI consumes.
    fn gui_history(self) -> bool {
        matches!(self, PersistedDataScope::Full)
    }
}

/// A conversation whose `summary` column had to be derived from its task
/// snapshot at read time (rows written before the column existed, or rows
/// whose stored summary failed to parse). Sent to the SQLite writer thread
/// so the derivation happens only once per row.
#[derive(Debug)]
pub struct ConversationSummaryBackfill {
    pub conversation_id: String,
    /// Serialized [`model::AgentConversationSummary`].
    pub summary_json: String,
    /// The `summary` column value observed at read time (`None` or invalid
    /// JSON). The backfill only applies while the column still holds this
    /// value, so it never overwrites a newer write.
    pub previous_summary: Option<String>,
    /// The row's pre-backfill `last_modified_at`, restored after the
    /// update trigger bumps it.
    pub last_modified_at: chrono::NaiveDateTime,
}

/// Initializes the persistence "subsystem".
///
/// Returns the previously-persisted data, if any, and handles for
/// writing updated data to persist, if the persistence subsystem is
/// available.
#[tracing::instrument(name = "persistence::initialize", skip_all, fields(tags.cloud_agent = true))]
#[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
pub fn initialize(
    ctx: &mut AppContext,
    scope: PersistenceScope,
    data_scope: PersistedDataScope,
) -> (Option<Box<PersistedData>>, Option<WriterHandles>) {
    // Record the scope for ad-hoc read-only connections; keep the first value
    // if this is ever called more than once in a process (e.g. tests).
    let _ = CURRENT_SCOPE.set(scope.clone());
    cfg_if::cfg_if! {
        if #[cfg(feature = "local_fs")] {
            sqlite::initialize(ctx, scope, data_scope)
        } else {
            (None, None)
        }
    }
}

// Remove sqlite database as part of Logout v0.
// TODO: Implement per user scoping of sqlite.
#[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
pub fn remove(sender: &Option<SyncSender<ModelEvent>>) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "local_fs")] {
            if let Some(sender) = sender.clone() {
                sqlite::remove(sender);
            }
        } else {
            log::info!("Local filesystem persistence is not enabled.");
        }
    }
}

// Reconstruct sqlite database as part of Logout v0.
#[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
pub fn reconstruct(sender: &Option<SyncSender<ModelEvent>>) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "local_fs")] {
            if let Some(sender) = sender.clone() {
                sqlite::reconstruct(sender);
            }
        } else {
            log::info!("Local filesystem persistence is not enabled.");
        }
    }
}

/// Holds interfaces to the writer thread.
pub struct WriterHandles {
    pub handle: JoinHandle<()>,
    pub sender: SyncSender<ModelEvent>,
}

/// Model for interacting with the writer thread.
pub struct PersistenceWriter {
    thread_handle: Option<JoinHandle<()>>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
}

impl PersistenceWriter {
    pub fn new(handle: Option<WriterHandles>) -> Self {
        let (thread_handle, model_event_sender) = match handle {
            Some(handle) => (Some(handle.handle), Some(handle.sender)),
            None => (None, None),
        };
        Self {
            thread_handle,
            model_event_sender,
        }
    }

    /// Sending half for sending model updates to the persistence writer thread.
    pub fn sender(&self) -> Option<SyncSender<ModelEvent>> {
        self.model_event_sender.clone()
    }

    /// Synchronously terminate the SQLite writer thread.
    pub fn terminate(&mut self) {
        if let Some(handle) = self.thread_handle.take() {
            let start = Instant::now();
            let Some(sender) = self.sender() else {
                report_error!("Model event sender should exist if thread handle is set");
                return;
            };
            if let Err(err) = sender.send(ModelEvent::Terminate) {
                report_error!(
                    anyhow::Error::new(err).context("Could not terminate SQLite writer thread")
                );
            }
            if handle.join().is_err() {
                // If crash reporting is enabled, Sentry will have already handled the panic.
                report_error!("SQLite writer thread panicked");
            }
            log::info!("Shut down SQLite writer in {:?}", start.elapsed());
        }
    }
}

impl Drop for PersistenceWriter {
    fn drop(&mut self) {
        self.terminate();
    }
}

impl Entity for PersistenceWriter {
    type Event = ();
}

impl SingletonEntity for PersistenceWriter {}

/// TODO: all of this data should eventually be indexed by user_id so that
/// the logged in user sees the data for their user (and if another user logs in,
/// they see their respective data). To do this, we can simply return a mapping
/// of user ID->SqliteData and get the respective AppState after the user logs in.
///
/// For now, to address the global scoping here, we clear all persisted data on logout.
pub struct PersistedData {
    /// Session restoration data. `None` when the launch mode's
    /// [`PersistedDataScope`] excludes it entirely (the daemon).
    pub app_state: Option<AppState>,

    /// Shareable objects.
    pub cloud_objects: Vec<Box<dyn CloudObject>>,
    pub workspaces: Vec<WorkspaceMetadata>,
    pub current_workspace_uid: Option<WorkspaceUid>,
    pub command_history: Vec<PersistedCommand>,
    pub user_profiles: Vec<UserProfileWithUID>,
    pub time_of_next_force_object_refresh: Option<DateTime<Utc>>,
    pub object_actions: Vec<ObjectAction>,
    pub experiments: Vec<ServerExperiment>,
    pub ai_queries: Vec<PersistedAIInput>,
    pub nld_prompts: Vec<(String, DateTime<Local>)>,
    pub codebase_indices: Vec<CodeWorkspaceMetadata>,
    pub workspace_language_servers: HashMap<PathBuf, HashMap<LSPServerType, EnablementState>>,
    pub multi_agent_conversations: Vec<AgentConversation>,
    pub projects: Vec<Project>,
    pub project_rules: Vec<ProjectRulePath>,
    pub ignored_suggestions: Vec<(String, SuggestionType)>,
    pub mcp_server_installations: HashMap<Uuid, TemplatableMCPServerInstallation>,
    pub mcp_servers_to_restore: Vec<Uuid>,
    /// Conversation summaries derived at read time for pre-`summary`-column
    /// rows. Drained by `sqlite::initialize`, which hands them to the writer
    /// thread for persistence; not intended for other consumers.
    pub conversation_summary_backfills: Vec<ConversationSummaryBackfill>,
}

#[derive(Clone, Debug)]
pub struct BlockCompleted {
    pub pane_id: Vec<u8>,
    /// Indicates if the block was created locally (e.g. not in a remote session)
    pub is_local: bool,
    pub block: Arc<SerializedBlock>,
}

#[derive(Debug)]
pub struct StartedCommandMetadata {
    pub command: String,
    pub start_ts: Option<DateTime<Local>>,
    pub pwd: Option<String>,
    pub shell: Option<String>,
    pub username: Option<String>,
    pub hostname: Option<String>,
    pub session_id: Option<SessionId>,
    pub git_branch: Option<String>,
    pub cloud_workflow_id: Option<SyncId>,
    pub workflow_command: Option<String>,
    pub is_agent_executed: bool,
}

#[derive(Debug)]
pub struct FinishedCommandMetadata {
    pub exit_code: ExitCode,
    pub start_ts: DateTime<Local>,
    pub completed_ts: DateTime<Local>,
    pub session_id: SessionId,
}

#[derive(Debug)]
pub enum ModelEvent {
    SaveBlock(BlockCompleted),
    DeleteBlocks(Vec<u8>),
    Snapshot(AppState),
    UpsertWorkflows(Vec<CloudWorkflow>),
    UpsertNotebooks(Vec<CloudNotebook>),
    UpsertFolders(Vec<CloudFolder>),
    MarkObjectAsSynced {
        hashed_sqlite_id: String,
        revision_and_editor: RevisionAndLastEditor,
        metadata_ts: Option<ServerTimestamp>,
    },
    IncrementRetryCount(String),
    UpsertGenericStringObject {
        object: Box<dyn CloudStringObject>,
    },
    UpsertGenericStringObjects(Vec<Box<dyn CloudStringObject>>),
    UpsertNotebook {
        notebook: CloudNotebook,
    },
    UpsertWorkflow {
        workflow: CloudWorkflow,
    },
    UpsertFolder {
        folder: CloudFolder,
    },
    UpdateObjectAfterServerCreation {
        client_id: String,
        server_creation_info: ServerCreationInfo,
    },
    DeleteObjects {
        ids: Vec<(SyncId, ObjectIdType)>,
    },
    UpsertWorkspace {
        workspace: Box<WorkspaceMetadata>,
    },
    UpsertWorkspaces {
        workspaces: Vec<WorkspaceMetadata>,
    },
    SetCurrentWorkspace {
        workspace_uid: WorkspaceUid,
    },
    UpdateObjectMetadata {
        id: String,
        metadata: CloudObjectMetadata,
    },
    InsertCommand {
        metadata: StartedCommandMetadata,
    },
    UpdateFinishedCommand {
        metadata: FinishedCommandMetadata,
    },
    UpsertUserProfiles {
        profiles: Vec<UserProfileWithUID>,
    },
    ClearUserProfiles,
    RecordTimeOfNextRefresh {
        timestamp: DateTime<Utc>,
    },
    SaveExperiments {
        experiments: Vec<ServerExperiment>,
    },
    // `PauseAndRemoveDatabase` and `ReconstructAndResume` are used to pause and resume the writer thread.
    // These are employed as part of Logout v0 to ensure that the writer thread
    // does not continue writing to the DB after the user has logged out and the DB is deleted.
    PauseAndRemoveDatabase,
    #[cfg(feature = "local_fs")]
    ReconstructAndResume,
    InsertObjectAction {
        object_action: ObjectAction,
    },
    SyncObjectActions {
        actions_to_sync: Vec<ObjectAction>,
    },
    /// Close the SQLite writer thread when the app is about to quit.
    Terminate,
    UpsertAIQuery {
        query: Arc<PersistedAIInput>,
    },
    /// Delete the AI query and related data for a given conversation.
    DeleteAIConversation {
        conversation_id: String,
    },
    UpdateMultiAgentConversation {
        conversation_id: String,
        updated_tasks: Vec<api::Task>,
        conversation_data: AgentConversationData,
    },
    /// Persists read-time-derived conversation summaries for rows written
    /// before the `summary` column existed.
    BackfillConversationSummaries {
        backfills: Vec<ConversationSummaryBackfill>,
    },
    DeleteMultiAgentConversations {
        conversation_ids: Vec<String>,
    },

    UpsertCurrentUserInformation {
        user_information: PersistedCurrentUserInformation,
    },
    UpsertCodebaseIndexMetadata {
        index_metadata: Box<CodeWorkspaceMetadata>,
    },
    DeleteCodebaseIndexMetadata {
        repo_path: PathBuf,
    },
    UpsertProject {
        project: Project,
    },
    DeleteProject {
        path: String,
    },
    UpsertMCPServerEnvironmentVariables {
        mcp_server_uuid: Vec<u8>,
        environment_variables: String,
    },
    UpsertProjectRules {
        project_rule_paths: Vec<ProjectRulePath>,
    },
    DeleteProjectRules {
        path: Vec<PathBuf>,
    },
    AddIgnoredSuggestion {
        suggestion: String,
        suggestion_type: SuggestionType,
    },
    RemoveIgnoredSuggestion {
        suggestion: String,
        suggestion_type: SuggestionType,
    },
    UpsertMCPServerInstallation {
        mcp_server_installation: TemplatableMCPServerInstallation,
    },
    DeleteMCPServerInstallations {
        installation_uuids: Vec<Uuid>,
    },
    DeleteMCPServerInstallationsByTemplateUuid {
        template_uuid: Uuid,
    },
    UpdateMCPInstallationRunning {
        installation_uuid: Uuid,
        running: bool,
    },
    UpsertWorkspaceLanguageServer {
        workspace_path: PathBuf,
        lsp_type: LSPServerType,
        enabled: EnablementState,
    },
    UpdateBlockAgentViewVisibility {
        block_id: String,
        agent_view_visibility: SerializedAgentViewVisibility,
    },
    SaveAIDocumentContent {
        document_id: String,
        content: String,
        version: i32,
        title: String,
    },
}
