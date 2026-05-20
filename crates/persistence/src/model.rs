//! These types are named after the database tables, and are used to represent specific queries.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};
use warp_multi_agent_api::{self as api, response_event::stream_finished};

use super::schema::{
    active_mcp_servers, agent_conversations, agent_tasks, ai_document_panes, ai_memory_panes,
    ambient_agent_panes, app, blocks, cloud_objects_refreshes, code_pane_tabs, code_panes,
    code_review_panes, commands, current_user_information, env_var_collection_panes, folders,
    generic_string_objects, ignored_suggestions, mcp_environment_variables,
    mcp_server_installations, mcp_server_panes, notebook_panes, notebooks, object_actions,
    object_metadata, object_permissions, pane_branches, pane_leaves, pane_nodes, panels,
    project_rules, projects, server_experiments, settings_panes, tabs, team_members, team_settings,
    teams, terminal_panes, user_profiles, welcome_panes, windows, workflow_panes, workflows,
    workspace_language_server, workspace_metadata, workspace_teams, workspaces,
};

#[derive(Insertable)]
#[diesel(table_name = app)]
pub struct NewApp {
    pub active_window_id: Option<i32>,
}

#[derive(Identifiable, Queryable)]
pub struct Window {
    pub id: i32,
    pub active_tab_index: i32,
    pub window_width: Option<f32>,
    pub window_height: Option<f32>,
    pub origin_x: Option<f32>,
    pub origin_y: Option<f32>,
    pub quake_mode: bool,
    pub universal_search_width: Option<f32>,
    pub warp_ai_width: Option<f32>,
    pub voltron_width: Option<f32>,
    pub warp_drive_index_width: Option<f32>,
    pub fullscreen_state: i32,
    pub agent_management_filters: Option<String>,
    pub left_panel_open: Option<bool>,
    pub vertical_tabs_panel_open: Option<bool>,
}

#[derive(Identifiable, Insertable, Queryable)]
pub struct GenericStringObject {
    pub id: i32,
    pub data: String,
}

#[derive(Insertable)]
#[diesel(table_name = generic_string_objects)]
pub struct NewGenericStringObject<'a> {
    pub data: &'a str,
}

#[derive(Insertable, Queryable)]
pub struct Workflow {
    pub id: i32,
    pub data: String,
}

/// A type representing a `Workflow` that is newly created. We purposefully
/// do not include the `id` here since it is unset.
#[derive(Insertable)]
#[diesel(table_name = workflows)]
pub struct NewWorkflow {
    pub data: String,
}

#[derive(Identifiable, Insertable, Queryable)]
pub struct Notebook {
    pub id: i32,
    pub title: Option<String>,
    pub data: Option<String>,
    pub ai_document_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = notebooks)]
pub struct NewNotebook {
    pub title: Option<String>,
    pub data: Option<String>,
    pub ai_document_id: Option<String>,
}

#[derive(Insertable, Identifiable, Queryable)]
pub struct Folder {
    pub id: i32,
    pub name: String,
    pub is_open: bool,
    pub is_warp_pack: bool,
}

#[derive(Insertable)]
#[diesel(table_name = folders)]
pub struct NewFolder {
    pub name: String,
    pub is_open: bool,
    pub is_warp_pack: bool,
}

#[derive(Identifiable, Insertable, Queryable)]
pub struct Team {
    pub id: i32,
    pub name: String,
    pub server_uid: String,
    pub billing_metadata_json: Option<String>,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = teams)]
pub struct NewTeam {
    pub name: String,
    pub server_uid: String,
    pub billing_metadata_json: Option<String>,
}

#[derive(Identifiable, Queryable)]
#[diesel(table_name = team_members)]
pub struct TeamMemberRow {
    pub id: i32,
    pub team_id: i32,
    pub user_uid: String,
    pub email: String,
    pub role: String,
}

#[derive(Insertable)]
#[diesel(table_name = team_members)]
pub struct NewTeamMember {
    pub team_id: i32,
    pub user_uid: String,
    pub email: String,
    pub role: String,
}

#[derive(Identifiable, Insertable, Queryable)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
    pub server_uid: String,
    pub is_selected: bool,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = workspaces)]
pub struct NewWorkspace {
    pub name: String,
    pub server_uid: String,
    pub is_selected: bool,
}

#[derive(Identifiable, Insertable, Queryable)]
pub struct TeamSetting {
    pub id: i32,
    pub team_id: i32,
    pub settings_json: String,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = team_settings)]
pub struct NewTeamSettings {
    pub team_id: i32,
    pub settings_json: String,
}

#[derive(Clone, Identifiable, Insertable, Queryable, AsChangeset)]
#[diesel(table_name = project_rules)]
pub struct ProjectRules {
    pub id: i32,
    pub path: String,
    pub project_root: String,
}

#[derive(Clone, Debug, Insertable, AsChangeset)]
#[diesel(table_name = project_rules)]
pub struct NewProjectRules {
    pub path: String,
    pub project_root: String,
}

#[derive(Clone, Identifiable, Queryable, AsChangeset)]
#[diesel(table_name = workspace_metadata)]
pub struct WorkspaceMetadata {
    pub id: i32,
    pub repo_path: String,
    pub navigated_ts: Option<NaiveDateTime>,
    pub modified_ts: Option<NaiveDateTime>,
    pub queried_ts: Option<NaiveDateTime>,
}

#[derive(Clone, Insertable, AsChangeset)]
#[diesel(table_name = workspace_metadata)]
pub struct NewWorkspaceMetadata {
    pub repo_path: String,
    pub navigated_ts: Option<NaiveDateTime>,
    pub modified_ts: Option<NaiveDateTime>,
    pub queried_ts: Option<NaiveDateTime>,
}

#[derive(Clone, Identifiable, Insertable, Queryable, AsChangeset)]
#[diesel(table_name = workspace_language_server)]
pub struct WorkspaceLanguageServer {
    pub id: i32,
    pub workspace_id: i32,
    pub language_server_name: String,
    pub enabled: String,
}

#[derive(Clone, Insertable, AsChangeset)]
#[diesel(table_name = workspace_language_server)]
pub struct NewWorkspaceLanguageServer {
    pub workspace_id: i32,
    pub language_server_name: String,
    pub enabled: String,
}

#[derive(Default, Clone, Debug, Insertable, Queryable, AsChangeset)]
#[diesel(table_name = projects)]
pub struct Project {
    pub path: String,
    pub added_ts: NaiveDateTime,
    pub last_opened_ts: Option<NaiveDateTime>,
}

impl Project {
    pub fn last_used_at(&self) -> NaiveDateTime {
        self.last_opened_ts.unwrap_or(self.added_ts)
    }
}

impl PartialEq for Project {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for Project {}

#[derive(Identifiable, Insertable, Queryable)]
pub struct WorkspaceTeam {
    pub id: i32,
    pub workspace_server_uid: String,
    pub team_server_uid: String,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = workspace_teams)]
pub struct NewWorkspaceTeam {
    pub workspace_server_uid: String,
    pub team_server_uid: String,
}

#[derive(Insertable, Queryable)]
#[diesel(table_name = object_permissions)]
pub struct ObjectPermissions {
    pub id: i32,
    pub object_metadata_id: i32,
    pub subject_type: String,
    pub subject_id: Option<String>,
    pub subject_uid: String,
    pub permissions_last_updated_at: Option<i64>,
    pub object_guests: Option<Vec<u8>>,
    pub anyone_with_link_access_level: Option<String>,
    pub anyone_with_link_source: Option<Vec<u8>>,
}

#[derive(Insertable, Queryable)]
#[diesel(table_name = object_permissions)]
pub struct NewObjectPermissions {
    pub object_metadata_id: i32,
    pub subject_type: String,
    pub subject_id: Option<String>,
    pub subject_uid: String,
    pub permissions_last_updated_at: Option<i64>,
    pub object_guests: Option<Vec<u8>>,
    pub anyone_with_link_access_level: Option<&'static str>,
    pub anyone_with_link_source: Option<Vec<u8>>,
}

#[derive(Insertable, Queryable)]
#[diesel(table_name = object_metadata)]
pub struct ObjectMetadata {
    pub id: i32,
    pub is_pending: bool,
    pub object_type: String,
    pub revision_ts: Option<i64>,
    pub server_id: Option<String>,
    pub client_id: Option<String>,
    pub shareable_object_id: i32,
    pub author_id: Option<i32>,
    pub retry_count: i32,
    pub metadata_last_updated_ts: Option<i64>,
    pub trashed_ts: Option<i64>,
    pub folder_id: Option<String>,
    pub is_welcome_object: bool,
    pub creator_uid: Option<String>,
    pub last_editor_uid: Option<String>,
    pub current_editor: Option<String>,
}

#[derive(Insertable, Queryable)]
#[diesel(table_name = object_metadata)]
pub struct NewObjectMetadata {
    pub is_pending: bool,
    pub object_type: String,
    pub revision_ts: Option<i64>,
    pub server_id: Option<String>,
    pub client_id: Option<String>,
    pub shareable_object_id: i32,
    pub author_id: Option<i32>,
    pub retry_count: i32,
    pub metadata_last_updated_ts: Option<i64>,
    pub trashed_ts: Option<i64>,
    pub folder_id: Option<String>,
    pub is_welcome_object: bool,
    pub creator_uid: Option<String>,
    pub last_editor_uid: Option<String>,
    pub current_editor: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = windows)]
pub struct NewWindow {
    pub active_tab_index: i32,
    pub window_width: Option<f32>,
    pub window_height: Option<f32>,
    pub origin_x: Option<f32>,
    pub origin_y: Option<f32>,
    pub quake_mode: bool,
    pub universal_search_width: Option<f32>,
    pub warp_ai_width: Option<f32>,
    pub voltron_width: Option<f32>,
    pub warp_drive_index_width: Option<f32>,
    pub fullscreen_state: i32,
    pub agent_management_filters: Option<String>,
    pub left_panel_open: Option<bool>,
    pub vertical_tabs_panel_open: Option<bool>,
}

#[derive(Identifiable, Queryable, Associations)]
#[diesel(belongs_to(Window))]
pub struct Tab {
    pub id: i32,
    pub window_id: i32,
    pub custom_title: Option<String>,
    pub color: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = tabs)]
pub struct NewTab {
    pub window_id: i32,
    pub custom_title: Option<String>,
    pub color: Option<String>,
}

/// The panes data model includes pane_nodes, pane_leaves and pane_branches.
/// In addition, each kind of pane has a table for its specific data (i.e. the cwd for terminal panes).
/// The pane_nodes table specifically keeps the node data so it is responsible for
/// keeping track of the tree relationships.
/// The pane_leaves table keeps info about a given pane (i.e. what kind of pane it is).
/// The pane_branches table keeps info about a branch in the tree (e.g. whether
/// the branch splits horizontally or vertically).
#[derive(Identifiable, Queryable)]
#[diesel(table_name = pane_leaves)]
#[diesel(primary_key(pane_node_id, kind))]
pub struct PaneLeaf {
    pub pane_node_id: i32,
    pub kind: String,
    pub is_focused: bool,
    pub custom_vertical_tabs_title: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = terminal_panes)]
#[diesel(primary_key(id))]
pub struct TerminalPane {
    pub id: i32,
    // This is hardcoded in the database, and not used in the app, but Diesel requires it so that
    // fields are in the same order as the table's columns.
    pub kind: String,
    pub uuid: Vec<u8>,
    pub cwd: Option<String>,
    pub is_active: bool,
    /// This is serialized JSON data for a ShellLaunchData struct.
    pub shell_launch_data: Option<String>,
    /// This is serialized JSON data for an InputConfig struct.
    pub input_config: Option<String>,
    pub llm_model_override: Option<String>,
    pub active_profile_id: Option<String>,
    /// This is serialized JSON data for a Vec<AIConversationId>.
    pub conversation_ids: Option<String>,
    /// The active conversation ID if the agent view was open in fullscreen mode.
    pub active_conversation_id: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = notebook_panes)]
#[diesel(primary_key(id))]
pub struct NotebookPane {
    pub id: i32,
    // This is hardcoded in the database, and not used in the app, but Diesel requires it so that
    // fields are in the same order as the table's columns.
    pub kind: String,
    pub notebook_id: Option<String>,
    pub local_path: Option<Vec<u8>>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = env_var_collection_panes)]
#[diesel(primary_key(id))]
pub struct EnvVarCollectionPane {
    pub id: i32,
    pub kind: String,
    pub env_var_collection_id: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = workflow_panes)]
#[diesel(primary_key(id))]
pub struct WorkflowPane {
    pub id: i32,
    pub kind: String,
    pub workflow_id: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = code_panes)]
#[diesel(primary_key(id))]
pub struct CodePane {
    pub id: i32,
    pub active_tab_index: i32,
    pub source_data: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = code_pane_tabs)]
#[diesel(primary_key(id))]
pub struct CodePaneTab {
    pub id: i32,
    pub code_pane_id: i32,
    pub tab_index: i32,
    pub local_path: Option<Vec<u8>>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = code_review_panes)]
#[diesel(primary_key(id))]
pub struct CodeReviewPane {
    pub id: i32,
    pub kind: String,
    pub terminal_uuid: Vec<u8>,
    pub repo_path: String,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = settings_panes)]
#[diesel(primary_key(id))]
pub struct SettingsPane {
    pub id: i32,
    pub kind: String,
    pub current_page: String,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = welcome_panes)]
#[diesel(primary_key(id))]
pub struct WelcomePane {
    pub id: i32,
    pub kind: String,
    pub startup_directory: Option<String>,
}

/// Maps to the `ai_memory_panes` table
/// (where table name is historical and not worth a migration to change).
#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = ai_memory_panes)]
#[diesel(primary_key(id))]
pub struct AIFactPane {
    pub id: i32,
    pub kind: String,
}

/// Subset of the [`terminal_panes`] table needed to retrieve per-session block lists.
///
/// The true primary key of the table is [`terminal_panes::id`]. However, Diesel's associations API
/// requires matching on the primary key, so this view pretends that [`terminal_panes::uuid`] is
/// the primary key. This is safe because the UUID is _also_ unique across all panes.
#[derive(Identifiable, Selectable, Queryable)]
#[diesel(table_name = terminal_panes)]
#[diesel(primary_key(uuid))]
pub struct TerminalSession {
    pub uuid: Vec<u8>,
}

#[derive(Queryable)]
pub struct PaneBranch {
    #[allow(dead_code)]
    pub id: i32,
    #[allow(dead_code)]
    pub pane_node_id: i32,
    pub horizontal: bool,
}

#[derive(Queryable)]
pub struct PaneNode {
    pub id: i32,
    #[allow(dead_code)]
    pub tab_id: i32,
    #[allow(dead_code)]
    pub parent_pane_node_id: Option<i32>,
    pub flex: Option<f32>,
    pub is_leaf: bool,
}

#[derive(Insertable)]
#[diesel(table_name = pane_leaves)]
pub struct NewPane {
    pub pane_node_id: i32,
    pub kind: String,
    pub is_focused: bool,
    pub custom_vertical_tabs_title: Option<String>,
}

/// The [`pane_leaves::kind`] value for terminal panes.
pub const TERMINAL_PANE_KIND: &str = "terminal";

/// The [`pane_leaves::kind`] value for notebook panes.
pub const NOTEBOOK_PANE_KIND: &str = "notebook";

/// The [`pane_leaves::kind`] value for EVC panes.
pub const ENV_VAR_COLLECTION_PANE_KIND: &str = "env_var_collection";

/// The [`pane_leaves::kind`] value for code panes.
pub const CODE_PANE_KIND: &str = "code";

/// The [`pane_leaves::kind`] value for workflow panes.
pub const WORKFLOW_PANE_KIND: &str = "workflow";

/// The [`pane_leaves::kind`] value for settings panes.
pub const SETTINGS_PANE_KIND: &str = "settings";

/// The [`pane_leaves::kind`] value for AI fact panes
/// (where kind name is historical and not worth a migration to change).
pub const AI_FACT_PANE_KIND: &str = "ai_memory";

/// The [`pane_leaves::kind`] value for MCP server panes
pub const MCP_SERVER_PANE_KIND: &str = "mcp_server";

/// The [`pane_leaves::kind`] value for code review panes.
pub const CODE_REVIEW_PANE_KIND: &str = "code_review";

/// The [`pane_leaves::kind`] value for execution profile editor panes.
pub const EXECUTION_PROFILE_EDITOR_PANE_KIND: &str = "execution_profile_editor";

/// The [`pane_leaves::kind`] value for the welcome pane.
pub const WELCOME_PANE_KIND: &str = "welcome";

/// The [`pane_leaves::kind`] value for the get-started pane.
pub const GET_STARTED_PANE_KIND: &str = "get_started";

/// The [`pane_leaves::kind`] value for AI document panes.
pub const AI_DOCUMENT_PANE_KIND: &str = "ai_document";

/// The [`pane_leaves::kind`] value for ambient agent (cloud mode) panes.
pub const AMBIENT_AGENT_PANE_KIND: &str = "ambient_agent";

#[derive(Insertable)]
#[diesel(table_name = terminal_panes)]
pub struct NewTerminalPane {
    pub id: i32,
    pub uuid: Vec<u8>,
    pub cwd: Option<String>,
    pub is_active: bool,
    /// This is serialized JSON data for a ShellLaunchData struct.
    pub shell_launch_data: Option<String>,
    /// This is serialized JSON data for an InputConfig struct.
    pub input_config: Option<String>,
    pub llm_model_override: Option<String>,
    pub active_profile_id: Option<String>,
    /// This is serialized JSON data for a Vec<AIConversationId>.
    pub conversation_ids: Option<String>,
    /// The active conversation ID if the agent view was open in fullscreen mode.
    pub active_conversation_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = notebook_panes)]
pub struct NewNotebookPane {
    pub id: i32,
    pub notebook_id: Option<String>,
    pub local_path: Option<Vec<u8>>,
}

#[derive(Insertable)]
#[diesel(table_name = env_var_collection_panes)]
pub struct NewEnvVarCollectionPane {
    pub id: i32,
    pub env_var_collection_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = workflow_panes)]
pub struct NewWorkflowPane {
    pub id: i32,
    pub workflow_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = code_panes)]
pub struct NewCodePane {
    pub id: i32,
    pub active_tab_index: i32,
    pub source_data: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = code_pane_tabs)]
pub struct NewCodePaneTab {
    pub code_pane_id: i32,
    pub tab_index: i32,
    pub local_path: Option<Vec<u8>>,
}

#[derive(Insertable)]
#[diesel(table_name = code_review_panes)]
pub struct NewCodeReviewPane {
    pub id: i32,
    pub terminal_uuid: Vec<u8>,
    pub repo_path: String,
}

#[derive(Insertable)]
#[diesel(table_name = settings_panes)]
pub struct NewSettingsPane {
    pub id: i32,
    pub current_page: String,
}

#[derive(Insertable)]
#[diesel(table_name = ai_memory_panes)]
pub struct NewAIFactPane {
    pub id: i32,
}

#[derive(Insertable)]
#[diesel(table_name = mcp_server_panes)]
pub struct NewMCPServerPane {
    pub id: i32,
}

#[derive(Insertable)]
#[diesel(table_name = welcome_panes)]
pub struct NewWelcomePane {
    pub id: i32,
    pub startup_directory: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = ambient_agent_panes)]
#[diesel(primary_key(id))]
pub struct AmbientAgentPane {
    pub id: i32,
    pub kind: String,
    pub uuid: Vec<u8>,
    pub task_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = ambient_agent_panes)]
pub struct NewAmbientAgentPane {
    pub id: i32,
    pub uuid: Vec<u8>,
    pub task_id: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = pane_branches)]
pub struct NewPaneBranch {
    pub pane_node_id: i32,
    pub horizontal: bool,
}

#[derive(Insertable)]
#[diesel(table_name = pane_nodes)]
pub struct NewPaneNode {
    pub tab_id: i32,
    pub parent_pane_node_id: Option<i32>,
    pub flex: Option<f32>,
    pub is_leaf: bool,
}

#[derive(Insertable)]
#[diesel(table_name = blocks)]
pub struct NewBlock<'a> {
    pub block_id: &'a str,
    // Note that there is no pane leaf UUID foreign key relationship because there's no good way to
    // enforce it: when we remove a pane and subsequently create a new snapshot, the old blocks
    // will now violate the constraint. While sqlite does have deferred constraints, it doesn't
    // work well with ON DELETE CASCADE (i.e. the cascade happens on the delete, not after the
    // transaction commit).
    pub pane_leaf_uuid: Vec<u8>,
    pub stylized_command: &'a Vec<u8>,
    pub stylized_output: &'a Vec<u8>,
    pub pwd: Option<&'a String>,
    pub git_branch: Option<&'a String>,
    pub git_branch_name: Option<&'a String>,
    pub virtual_env: Option<&'a String>,
    pub conda_env: Option<&'a String>,
    pub exit_code: i32,
    pub did_execute: bool,
    pub is_background: bool,
    pub completed_ts: Option<NaiveDateTime>,
    pub start_ts: Option<NaiveDateTime>,
    pub ps1: Option<&'a String>,
    pub rprompt: Option<&'a String>,
    pub honor_ps1: bool,
    pub shell: Option<&'a str>,
    pub user: Option<&'a str>,
    pub host: Option<&'a str>,
    pub prompt_snapshot: Option<&'a String>,
    pub ai_metadata: Option<&'a String>,
    pub is_local: Option<bool>,
    pub agent_view_visibility: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable, Associations)]
#[diesel(table_name = blocks)]
#[diesel(belongs_to(TerminalSession, foreign_key = pane_leaf_uuid))]
pub struct Block {
    pub id: Option<i32>,
    pub pane_leaf_uuid: Vec<u8>,
    pub stylized_command: Vec<u8>,
    pub stylized_output: Vec<u8>,
    pub pwd: Option<String>,
    pub git_branch: Option<String>,
    pub git_branch_name: Option<String>,
    pub virtual_env: Option<String>,
    pub conda_env: Option<String>,
    pub exit_code: i32,
    pub did_execute: bool,
    pub completed_ts: Option<NaiveDateTime>,
    pub start_ts: Option<NaiveDateTime>,
    pub ps1: Option<String>,
    pub honor_ps1: bool,
    pub shell: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
    pub is_background: bool,
    pub rprompt: Option<String>,
    /// JSON-serialized representation of the Warp prompt snapshot (Context Chips). Note that this
    /// is different from PS1 and RPROMPT1
    pub prompt_snapshot: Option<String>,
    pub block_id: String,
    pub ai_metadata: Option<String>,
    pub is_local: Option<bool>,
    pub agent_view_visibility: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = commands)]
pub struct NewCommand {
    pub command: String,
    pub exit_code: Option<i32>,
    pub start_ts: Option<NaiveDateTime>,
    pub completed_ts: Option<NaiveDateTime>,
    pub pwd: Option<String>,
    pub shell: Option<String>,
    pub username: Option<String>,
    pub hostname: Option<String>,
    pub session_id: Option<i64>,
    pub git_branch: Option<String>,
    pub cloud_workflow_id: Option<String>,
    pub workflow_command: Option<String>,
    pub is_agent_executed: Option<bool>,
}

#[derive(Identifiable, Queryable, Default, Clone)]
#[diesel(table_name = commands)]
pub struct Command {
    pub id: i32,
    pub command: String,
    pub exit_code: Option<i32>,
    pub start_ts: Option<NaiveDateTime>,
    pub completed_ts: Option<NaiveDateTime>,
    pub pwd: Option<String>,
    pub shell: Option<String>,
    pub username: Option<String>,
    pub hostname: Option<String>,
    pub session_id: Option<i64>,
    pub git_branch: Option<String>,
    pub cloud_workflow_id: Option<String>,
    pub workflow_command: Option<String>,
    pub is_agent_executed: Option<bool>,
}

#[derive(Identifiable, Queryable, Insertable)]
#[diesel(table_name = user_profiles)]
#[diesel(primary_key(firebase_uid))]
pub struct UserProfile {
    pub firebase_uid: String,
    pub photo_url: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name = cloud_objects_refreshes)]
pub struct NewCloudObjectsRefresh {
    pub time_of_next_refresh: NaiveDateTime,
}

#[derive(Identifiable, Queryable)]
#[diesel(table_name = cloud_objects_refreshes)]
pub struct CloudObjectsRefresh {
    pub id: i32,
    pub time_of_next_refresh: NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = object_actions)]
pub struct NewPersistedObjectAction {
    pub hashed_object_id: String,
    pub timestamp: Option<NaiveDateTime>,
    pub action: String,
    pub data: Option<String>,
    pub count: Option<i32>,
    pub oldest_timestamp: Option<NaiveDateTime>,
    pub latest_timestamp: Option<NaiveDateTime>,
    pub pending: Option<bool>,
    pub processed_at_timestamp: Option<NaiveDateTime>,
}

#[derive(Identifiable, Queryable, Insertable, Debug)]
#[diesel(table_name = object_actions)]
pub struct PersistedObjectAction {
    pub id: i32,
    pub hashed_object_id: String,
    pub timestamp: Option<NaiveDateTime>,
    pub action: String,
    pub data: Option<String>,
    pub count: Option<i32>,
    pub oldest_timestamp: Option<NaiveDateTime>,
    pub latest_timestamp: Option<NaiveDateTime>,
    pub pending: Option<bool>,
    pub processed_at_timestamp: Option<NaiveDateTime>,
}

#[derive(Insertable, Queryable)]
pub struct ServerExperiment {
    pub experiment: String,
}

#[derive(Insertable)]
#[diesel(table_name = server_experiments)]
pub struct NewServerExperiment {
    pub experiment: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = current_user_information)]
pub struct CurrentUserInformation {
    pub email: String,
}

#[derive(Debug, Insertable, Queryable, AsChangeset)]
#[diesel(table_name = mcp_environment_variables)]
pub struct MCPEnvironmentVariables {
    pub mcp_server_uuid: Vec<u8>,
    pub environment_variables: String,
}

#[derive(Debug, Insertable, Queryable)]
#[diesel(table_name = active_mcp_servers)]
pub struct ActiveMCPServer {
    pub id: i32,
    pub mcp_server_uuid: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = active_mcp_servers)]
pub struct NewActiveMCPServer {
    pub mcp_server_uuid: String,
}

// Queryable structs for reading from the database
#[derive(Debug, PartialEq, Default, Queryable, Selectable, Clone)]
#[diesel(table_name = agent_conversations)]
#[diesel(primary_key(id))]
pub struct AgentConversationRecord {
    pub id: i32,
    pub conversation_id: String,
    pub conversation_data: String,
    pub last_modified_at: NaiveDateTime,
}

#[derive(Debug, PartialEq, Queryable, Selectable)]
#[diesel(table_name = agent_tasks)]
#[diesel(primary_key(id))]
pub struct AgentTaskRecord {
    pub id: i32,
    pub conversation_id: String,
    pub task_id: String,
    pub task: Vec<u8>,
    pub last_modified_at: NaiveDateTime,
}

#[derive(Debug, PartialEq, Queryable, Selectable, Clone)]
#[diesel(table_name = ai_document_panes)]
#[diesel(primary_key(id))]
pub struct AIDocumentPane {
    pub id: i32,
    pub kind: String,
    pub document_id: String,
    pub version: i32,
    pub content: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = ai_document_panes)]
pub struct NewAIDocumentPane {
    pub id: i32,
    pub document_id: String,
    pub version: i32,
    pub content: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, PartialEq, Default, Clone)]
pub struct AgentConversation {
    pub conversation: AgentConversationRecord,
    pub tasks: Vec<api::Task>,
}

impl AgentConversation {
    /// Returns `true` if the conversation is restorable.
    ///
    /// A conversation is restorable if:
    /// - It contains a single task or fewer, OR
    /// - It contains multiple tasks where every task other than the root task has a parent task ID.
    pub fn is_restorable(&self) -> bool {
        if self.tasks.len() <= 1 {
            return true;
        }

        // Find the root task(s) - tasks with no parent_task_id or empty parent_task_id
        let root_tasks: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| {
                task.dependencies
                    .as_ref()
                    .map(|deps| deps.parent_task_id.is_empty())
                    .unwrap_or(true)
            })
            .collect();

        // Must have exactly one root task
        if root_tasks.len() != 1 {
            return false;
        }

        // All non-root tasks must have a non-empty parent_task_id
        self.tasks.iter().all(|task| {
            // Root task is always valid
            if task
                .dependencies
                .as_ref()
                .map(|deps| deps.parent_task_id.is_empty())
                .unwrap_or(true)
            {
                return true;
            }

            // Non-root tasks must have a non-empty parent_task_id
            task.dependencies
                .as_ref()
                .is_some_and(|deps| !deps.parent_task_id.is_empty())
        })
    }
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, Default)]
pub enum PersistedAutoexecuteMode {
    #[default]
    RespectUserSettings,
    RunToCompletion,
}

impl<'de> Deserialize<'de> for PersistedAutoexecuteMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "RespectUserSettings" => Self::RespectUserSettings,
            "RunToCompletion" => Self::RunToCompletion,
            _ => Self::default(),
        })
    }
}
fn is_false(value: &bool) -> bool {
    !*value
}

// Serializes to `conversation_data` column in `agent_conversations`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConversationData {
    pub server_conversation_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_usage_metadata: Option<ConversationUsageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverted_action_ids: Option<HashSet<AIAgentActionId>>,
    /// The server conversation ID of the source conversation if this conversation was forked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_server_conversation_token: Option<String>,
    /// Serialized Vec<Artifact> for local artifact tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts_json: Option<String>,
    /// Server-side identifier of the parent agent that spawned this child.
    /// In v1 this is the parent's conversation token; in v2 it is the parent's run_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    /// The display name for this agent, assigned by the orchestrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Harness type used to render the child agent's shared icon in orchestration UI.
    #[serde(
        default,
        alias = "orchestration_avatar_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub orchestration_harness_type: Option<String>,
    /// The local conversation ID of the parent conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_conversation_id: Option<String>,
    /// True when this conversation is a parent-side placeholder for a child
    /// agent executing on a remote worker.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_remote_child: bool,
    /// The server-assigned run identifier (`ai_tasks.id`) for v2 orchestration.
    /// For local agents this arrives via StreamInit; for cloud agents it will
    /// come from SpawnAgentResponse once the local→cloud spawn path is wired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoexecute_override: Option<PersistedAutoexecuteMode>,
    /// The last event sequence number from the v2 orchestration event log
    /// that this conversation has observed. Used on restore to resume event
    /// delivery without re-delivering already-processed events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_sequence: Option<i64>,
    /// Whether the user has pinned this child agent in the orchestration
    /// pill bar. Orchestrator conversations always serialize as `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AIAgentActionId(pub String);

pub type TokenUsageCategory = String;

pub const PRIMARY_AGENT_CATEGORY: &str = "primary_agent";
pub const FULL_TERMINAL_USE_CATEGORY: &str = "full_terminal_use";

pub fn token_usage_category_display_name(category: &str) -> String {
    category
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ModelTokenUsage {
    pub model_id: String,
    /// Alias for backward compat: old persisted data used `total_tokens` for warp usage.
    #[serde(default, alias = "total_tokens")]
    pub warp_tokens: u32,
    #[serde(default)]
    pub byok_tokens: u32,
    #[serde(default)]
    pub warp_token_usage_by_category: HashMap<TokenUsageCategory, u32>,
    #[serde(default)]
    pub byok_token_usage_by_category: HashMap<TokenUsageCategory, u32>,
}

impl ModelTokenUsage {
    #[allow(deprecated)]
    fn to_proto_usage(
        &self,
        total_tokens: u32,
        usage_by_category: &HashMap<TokenUsageCategory, u32>,
    ) -> Option<(String, stream_finished::ModelTokenUsage)> {
        if total_tokens == 0 {
            return None;
        }
        Some((
            self.model_id.clone(),
            stream_finished::ModelTokenUsage {
                model_id: self.model_id.clone(),
                total_tokens,
                token_usage_by_category: usage_by_category
                    .iter()
                    .map(|(cat, tokens)| (cat.clone(), *tokens))
                    .collect(),
            },
        ))
    }

    pub fn to_proto_warp_usage(&self) -> Option<(String, stream_finished::ModelTokenUsage)> {
        self.to_proto_usage(self.warp_tokens, &self.warp_token_usage_by_category)
    }

    pub fn to_proto_byok_usage(&self) -> Option<(String, stream_finished::ModelTokenUsage)> {
        self.to_proto_usage(self.byok_tokens, &self.byok_token_usage_by_category)
    }

    #[allow(deprecated)]
    pub fn to_proto_combined(&self) -> stream_finished::ModelTokenUsage {
        stream_finished::ModelTokenUsage {
            model_id: self.model_id.clone(),
            total_tokens: self.warp_tokens + self.byok_tokens,
            token_usage_by_category: self
                .warp_token_usage_by_category
                .iter()
                .chain(self.byok_token_usage_by_category.iter())
                .fold(HashMap::new(), |mut acc, (cat, tokens)| {
                    *acc.entry(cat.clone()).or_insert(0) += tokens;
                    acc
                }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ToolCallStats {
    pub count: i32,
}

impl From<&ToolCallStats> for stream_finished::ToolCallStats {
    fn from(stats: &ToolCallStats) -> Self {
        Self { count: stats.count }
    }
}

impl From<&stream_finished::ToolCallStats> for ToolCallStats {
    fn from(tool_call_stats: &stream_finished::ToolCallStats) -> Self {
        Self {
            count: tool_call_stats.count,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RunCommandStats {
    pub count: i32,
    pub commands_executed: i32,
}

impl From<&RunCommandStats> for stream_finished::RunCommandStats {
    fn from(stats: &RunCommandStats) -> Self {
        Self {
            count: stats.count,
            command_executed: stats.commands_executed,
        }
    }
}

impl From<&stream_finished::RunCommandStats> for RunCommandStats {
    fn from(stats: &stream_finished::RunCommandStats) -> Self {
        Self {
            count: stats.count,
            commands_executed: stats.command_executed,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ApplyFileDiffStats {
    pub count: i32,
    pub lines_added: i32,
    pub lines_removed: i32,
    pub files_changed: i32,
}

impl From<&ApplyFileDiffStats> for stream_finished::ApplyFileDiffStats {
    fn from(stats: &ApplyFileDiffStats) -> Self {
        Self {
            count: stats.count,
            lines_added: stats.lines_added,
            lines_removed: stats.lines_removed,
            files_changed: stats.files_changed,
        }
    }
}

impl From<&stream_finished::ApplyFileDiffStats> for ApplyFileDiffStats {
    fn from(file_diff_stats: &stream_finished::ApplyFileDiffStats) -> Self {
        Self {
            count: file_diff_stats.count,
            lines_added: file_diff_stats.lines_added,
            lines_removed: file_diff_stats.lines_removed,
            files_changed: file_diff_stats.files_changed,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ToolUsageMetadata {
    pub run_command_stats: RunCommandStats,
    pub read_files_stats: ToolCallStats,
    pub search_codebase_stats: ToolCallStats,
    pub grep_stats: ToolCallStats,
    pub file_glob_stats: ToolCallStats,
    pub apply_file_diff_stats: ApplyFileDiffStats,
    pub write_to_long_running_shell_command_stats: ToolCallStats,
    pub read_mcp_resource_stats: ToolCallStats,
    pub call_mcp_tool_stats: ToolCallStats,
    pub suggest_plan_stats: ToolCallStats,
    pub suggest_create_plan_stats: ToolCallStats,
    pub read_shell_command_output_stats: ToolCallStats,
    pub use_computer_stats: ToolCallStats,
}

impl ToolUsageMetadata {
    pub fn total_tool_calls(&self) -> i32 {
        self.run_command_stats.count
            + self.read_files_stats.count
            + self.search_codebase_stats.count
            + self.grep_stats.count
            + self.file_glob_stats.count
            + self.write_to_long_running_shell_command_stats.count
            + self.read_mcp_resource_stats.count
            + self.call_mcp_tool_stats.count
            + self.suggest_plan_stats.count
            + self.suggest_create_plan_stats.count
            + self.apply_file_diff_stats.count
            + self.read_shell_command_output_stats.count
            + self.use_computer_stats.count
    }
}

impl From<&ToolUsageMetadata> for stream_finished::ToolUsageMetadata {
    fn from(metadata: &ToolUsageMetadata) -> Self {
        Self {
            run_command_stats: Some((&metadata.run_command_stats).into()),
            read_files_stats: Some((&metadata.read_files_stats).into()),
            search_codebase_stats: Some((&metadata.search_codebase_stats).into()),
            grep_stats: Some((&metadata.grep_stats).into()),
            file_glob_stats: Some((&metadata.file_glob_stats).into()),
            apply_file_diff_stats: Some((&metadata.apply_file_diff_stats).into()),
            write_to_long_running_shell_command_stats: Some(
                (&metadata.write_to_long_running_shell_command_stats).into(),
            ),
            read_mcp_resource_stats: Some((&metadata.read_mcp_resource_stats).into()),
            call_mcp_tool_stats: Some((&metadata.call_mcp_tool_stats).into()),
            suggest_plan_stats: Some((&metadata.suggest_plan_stats).into()),
            suggest_create_plan_stats: Some((&metadata.suggest_create_plan_stats).into()),
            read_shell_command_output_stats: Some(
                (&metadata.read_shell_command_output_stats).into(),
            ),
            use_computer_stats: Some((&metadata.use_computer_stats).into()),
        }
    }
}

impl From<&stream_finished::ToolUsageMetadata> for ToolUsageMetadata {
    fn from(tool_usage_metadata: &stream_finished::ToolUsageMetadata) -> Self {
        let convert = |opt: &Option<_>| opt.as_ref().map(Into::into).unwrap_or_default();

        Self {
            run_command_stats: tool_usage_metadata
                .run_command_stats
                .as_ref()
                .map(Into::into)
                .unwrap_or_default(),
            read_files_stats: convert(&tool_usage_metadata.read_files_stats),
            search_codebase_stats: convert(&tool_usage_metadata.search_codebase_stats),
            grep_stats: convert(&tool_usage_metadata.grep_stats),
            file_glob_stats: convert(&tool_usage_metadata.file_glob_stats),
            apply_file_diff_stats: tool_usage_metadata
                .apply_file_diff_stats
                .as_ref()
                .map(Into::into)
                .unwrap_or_default(),
            write_to_long_running_shell_command_stats: convert(
                &tool_usage_metadata.write_to_long_running_shell_command_stats,
            ),
            read_mcp_resource_stats: convert(&tool_usage_metadata.read_mcp_resource_stats),
            call_mcp_tool_stats: convert(&tool_usage_metadata.call_mcp_tool_stats),
            suggest_plan_stats: convert(&tool_usage_metadata.suggest_plan_stats),
            suggest_create_plan_stats: convert(&tool_usage_metadata.suggest_create_plan_stats),
            read_shell_command_output_stats: convert(
                &tool_usage_metadata.read_shell_command_output_stats,
            ),
            use_computer_stats: convert(&tool_usage_metadata.use_computer_stats),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ConversationUsageMetadata {
    pub was_summarized: bool,
    pub context_window_usage: f32,
    pub credits_spent: f32,
    #[serde(default)]
    pub credits_spent_for_last_block: Option<f32>,
    #[serde(default)]
    pub token_usage: Vec<ModelTokenUsage>,
    #[serde(default)]
    pub tool_usage_metadata: ToolUsageMetadata,
}

impl ConversationUsageMetadata {
    pub fn total_tool_calls(&self) -> i32 {
        self.tool_usage_metadata.total_tool_calls()
    }
}

#[derive(Debug, Insertable)]
#[diesel(table_name = ignored_suggestions)]
pub struct NewIgnoredSuggestion {
    pub suggestion: String,
    pub suggestion_type: String,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = mcp_server_installations)]
#[diesel(treat_none_as_null = true)]
#[diesel(primary_key(id))]
pub struct NewMCPServerInstallation {
    pub id: String,
    pub templatable_mcp_server: String,
    pub template_version_ts: NaiveDateTime,
    pub variable_values: String,
    pub restore_running: bool,
    pub last_modified_at: NaiveDateTime,
}

#[cfg(test)]
mod tests {
    use super::AgentConversationData;

    #[test]
    fn agent_conversation_data_roundtrips_last_event_sequence() {
        let data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: Some("claude".to_string()),
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: Some(42),
            pinned: false,
        };
        let json = serde_json::to_string(&data).expect("serialize");
        let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtripped.last_event_sequence, Some(42));
        assert_eq!(
            roundtripped.orchestration_harness_type.as_deref(),
            Some("claude")
        );
    }

    #[test]
    fn agent_conversation_data_accepts_legacy_orchestration_avatar_id() {
        let legacy_json = r#"{"orchestration_avatar_id":"orbit"}"#;
        let data: AgentConversationData =
            serde_json::from_str(legacy_json).expect("legacy rows must deserialize");

        assert_eq!(data.orchestration_harness_type.as_deref(), Some("orbit"));
    }

    #[test]
    fn agent_conversation_data_roundtrips_remote_child_marker() {
        let data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: true,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };
        let json = serde_json::to_string(&data).expect("serialize");
        let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
        assert!(roundtripped.is_remote_child);
    }

    #[test]
    fn agent_conversation_data_deserializes_legacy_payload_without_last_event_sequence() {
        // Legacy rows persisted before this feature landed omit the field
        // entirely. `#[serde(default)]` must accept them as `None`.
        let legacy_json = r#"{"server_conversation_token":null}"#;
        let data: AgentConversationData =
            serde_json::from_str(legacy_json).expect("legacy rows must deserialize");
        assert_eq!(data.last_event_sequence, None);
        assert_eq!(data.orchestration_harness_type, None);
        assert!(!data.is_remote_child);
    }

    #[test]
    fn agent_conversation_data_skips_serializing_none_last_event_sequence() {
        let data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };
        let json = serde_json::to_string(&data).expect("serialize");
        assert!(
            !json.contains("last_event_sequence"),
            "None should be skipped in serialized output: {json}"
        );
    }

    #[test]
    fn agent_conversation_data_roundtrips_pinned() {
        let data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: true,
        };
        let json = serde_json::to_string(&data).expect("serialize");
        let roundtripped: AgentConversationData = serde_json::from_str(&json).expect("deserialize");
        assert!(roundtripped.pinned);
    }

    #[test]
    fn agent_conversation_data_skips_serializing_unpinned() {
        let data = AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        };
        let json = serde_json::to_string(&data).expect("serialize");
        assert!(
            !json.contains("pinned"),
            "Unpinned default should be skipped: {json}"
        );
    }

    #[test]
    fn agent_conversation_data_legacy_rows_default_to_unpinned() {
        let legacy_json = r#"{"server_conversation_token":null}"#;
        let data: AgentConversationData =
            serde_json::from_str(legacy_json).expect("legacy rows must deserialize");
        assert!(!data.pinned);
    }
}

#[derive(Insertable)]
#[diesel(table_name = panels)]
pub struct NewPanel {
    pub tab_id: i32,
    pub left_panel: Option<String>,
    pub right_panel: Option<String>,
}

#[derive(Identifiable, Queryable, Selectable)]
#[diesel(table_name = panels)]
pub struct Panel {
    pub id: i32,
    pub tab_id: i32,
    pub left_panel: Option<String>,
    pub right_panel: Option<String>,
}
