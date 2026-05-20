use std::collections::HashSet;
use std::ffi::OsString;
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc::SyncSender;
use std::sync::Once;
use std::{
    collections::{HashMap, VecDeque},
    convert::TryInto,
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
};

use ai::project_context::model::ProjectRulePath;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use diesel::{
    connection::{DefaultLoadingMode, SimpleConnection},
    result::Error,
    sqlite::SqliteConnection,
    BelongingToDsl, BoolExpressionMethods, Connection, ExpressionMethods, GroupedBy,
    OptionalExtension, QueryDsl, RunQueryDsl, SelectableHelper,
};
use diesel_migrations::MigrationHarness;
use itertools::Itertools;
use libsqlite3_sys as sqlite3;
use num_traits::FromPrimitive;
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use persistence::model::AMBIENT_AGENT_PANE_KIND;
use uuid::Uuid;
use warp_graphql::scalars::time::ServerTimestamp;
use warpui::platform::FullscreenState;
use warpui::{AppContext, SingletonEntity};

use super::agent::{delete_agent_conversations, upsert_agent_conversation};
use super::block_list::{
    delete_ai_conversation, delete_blocks, save_block, update_block_agent_view_visibility,
    upsert_ai_query,
};
use super::model::{
    self, ActiveMCPServer, CurrentUserInformation, MCPEnvironmentVariables, NewActiveMCPServer,
    NewApp, NewCommand, NewFolder, NewNotebook, NewServerExperiment, NewTab, NewTeam, NewWindow,
    NewWorkspace, NewWorkspaceMetadata, NewWorkspaceTeam, ObjectMetadata, ObjectPermissions,
    Project, Tab, Window, WorkspaceMetadata as WorkspaceMetadataModel, AI_DOCUMENT_PANE_KIND,
    AI_FACT_PANE_KIND, CODE_PANE_KIND, ENV_VAR_COLLECTION_PANE_KIND,
    EXECUTION_PROFILE_EDITOR_PANE_KIND, MCP_SERVER_PANE_KIND, NOTEBOOK_PANE_KIND,
    SETTINGS_PANE_KIND, TERMINAL_PANE_KIND, WELCOME_PANE_KIND, WORKFLOW_PANE_KIND,
};
use super::schema;
use super::{
    BlockCompleted, FinishedCommandMetadata, ModelEvent, PersistedData, PersistenceScope,
    StartedCommandMetadata, WriterHandles,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::scheduled::{
    CloudScheduledAmbientAgent, CloudScheduledAmbientAgentModel,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::cloud_environments::{
    CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
};
use crate::ai::document::ai_document_model::AIDocumentId;
use crate::ai::execution_profiles::{CloudAIExecutionProfile, CloudAIExecutionProfileModel};
use crate::ai::facts::{CloudAIFact, CloudAIFactModel};
use crate::ai::mcp::templatable::{CloudTemplatableMCPServer, CloudTemplatableMCPServerModel};
use crate::ai::mcp::templatable_installation::VariableValue;
use crate::ai::mcp::{
    CloudMCPServer, CloudMCPServerModel, TemplatableMCPServer, TemplatableMCPServerInstallation,
};
use crate::ai::persisted_workspace::EnablementState;
use crate::app_state::{
    AIFactPaneSnapshot, AmbientAgentPaneSnapshot, CodeReviewPaneSnapshot,
    EnvVarCollectionPaneSnapshot, LeftPanelSnapshot, RightPanelSnapshot, SettingsPaneSnapshot,
    WorkflowPaneSnapshot,
};
use crate::auth::auth_manager::PersistedCurrentUserInformation;
use crate::auth::auth_state::AuthStateProvider;
use crate::auth::UserUid;
use crate::cloud_object::model::actions::{ObjectAction, ObjectActionSubtype};
use crate::cloud_object::model::generic_string_model::{CloudStringObject, GenericStringObjectId};
use crate::cloud_object::{
    CloudObject, JsonObjectType, ObjectIdType, ObjectType, Owner, RevisionAndLastEditor,
    GENERIC_STRING_OBJECT_PREFIX, JSON_OBJECT_PREFIX,
};
use crate::code::editor_management::CodeSource;
use crate::drive::folders::{CloudFolder, CloudFolderModel, FolderId};
use crate::drive::OpenWarpDriveObjectSettings;
use crate::env_vars::{CloudEnvVarCollection, CloudEnvVarCollectionModel};
use crate::features::FeatureFlag;
use crate::notebooks::{CloudNotebook, NotebookId};
use crate::persistence::agent::read_agent_conversations;
use crate::persistence::block_list::{get_all_restored_blocks, read_ai_queries};
use crate::persistence::model::{
    NewCloudObjectsRefresh, NewGenericStringObject, NewPersistedObjectAction, NewTeamSettings,
    ProjectRules, UserProfile, CODE_REVIEW_PANE_KIND, GET_STARTED_PANE_KIND,
};
use crate::server::experiments::ServerExperiment;
use crate::server::ids::{ClientId, HashableId, ServerId, SyncId, ToServerId};
use crate::server::telemetry::TelemetryEvent;
use crate::settings::cloud_preferences::{CloudPreference, CloudPreferenceModel};
use crate::settings_view::SettingsSection;
use crate::suggestions::ignored_suggestions_model::SuggestionType;
use crate::tab::SelectedTabColor;
use crate::terminal::history::PersistedCommand;
use crate::terminal::ShellLaunchData;
use crate::themes::theme::AnsiColorIdentifier;
use crate::workflows::workflow_enum::{CloudWorkflowEnum, CloudWorkflowEnumModel};
use crate::workflows::{CloudWorkflow, WorkflowId};
use crate::workspaces::team::Team as TeamMetadata;
use crate::workspaces::workspace::Workspace as WorkspaceMetadata;
use crate::workspaces::workspace::WorkspaceUid;
use crate::{
    app_state::{
        AppState, BranchSnapshot, CodePaneSnapShot, CodePaneTabSnapshot, LeafContents,
        LeafSnapshot, NotebookPaneSnapshot, PaneFlex, PaneNodeSnapshot, SplitDirection,
        TabSnapshot, TerminalPaneSnapshot, WindowSnapshot,
    },
    workspaces::user_profiles::UserProfileWithUID,
};
use crate::{
    cloud_object::{CloudObjectMetadata, NumInFlightRequests, Revision, ServerCreationInfo},
    notebooks::CloudNotebookModel,
};
use crate::{
    cloud_object::{CloudObjectPermissions, CloudObjectStatuses, CloudObjectSyncStatus},
    workflows::CloudWorkflowModel,
};
use crate::{report_error, report_if_error, safe_info, send_telemetry_from_app_ctx};
use lsp::supported_servers::LSPServerType;

diesel::define_sql_function! {
    fn json_extract(target: diesel::sql_types::Text, path: diesel::sql_types::Text) -> diesel::sql_types::Text;
}

// Choose a power of 2 that seems to be a reasonable upper bound for how many
// events to queue.
const CHANNEL_SIZE: usize = 1024;
const COMMANDS_COUNT_LIMIT: i64 = 10000;

use warp_server_client::persistence::{upsert_cloud_object, CloudObjectId};

const WARP_SQLITE_FILE_NAME: &str = "warp.sqlite";

/// When delete a cloud object, this callback is used to delete the cloud
/// object. It takes the id of the cloud object to delete as a parameter.
/// The supplied conn has already started a transaction.
type DeleteCloudObjectFn =
    Box<dyn FnOnce(&mut SqliteConnection, CloudObjectId) -> Result<(), Error>>;

/// Runs any migrations and creates the Sqlite database if it doesn't exist.
/// Reads from the sqlite database to get the app state for session restoration.
/// Starts a writer thread that listens for ModelEvents and processes them.
pub fn initialize(
    ctx: &mut AppContext,
    scope: PersistenceScope,
) -> (Option<Box<PersistedData>>, Option<WriterHandles>) {
    unsafe {
        // Set up logging before any SQLite calls.
        init_logging();
    }
    let database_path = database_file_path_for_scope(&scope);
    match init_db(&scope) {
        Ok(mut conn) => {
            let persisted_data = read_persisted_data(&mut conn, ctx);

            let writer_handles = match start_writer(conn, database_path.clone()) {
                Ok(writer_handles) => Some(writer_handles),
                Err(err) => {
                    send_telemetry_from_app_ctx!(
                        TelemetryEvent::DatabaseWriteError(err.to_string()),
                        ctx
                    );
                    report_db_error("starting writer", err, &database_path);
                    None
                }
            };
            (persisted_data, writer_handles)
        }
        Err(err) => {
            send_telemetry_from_app_ctx!(
                TelemetryEvent::DatabaseStartUpError(err.to_string()),
                ctx
            );
            report_db_error("initialization", err, &database_path);
            (None, None)
        }
    }
}

fn read_persisted_data(
    conn: &mut SqliteConnection,
    ctx: &mut AppContext,
) -> Option<Box<PersistedData>> {
    let user_uid = AuthStateProvider::as_ref(ctx).get().user_id();
    match read_sqlite_data(conn, user_uid) {
        Ok(app_state) => Some(Box::new(app_state)),
        Err(err) => {
            send_telemetry_from_app_ctx!(TelemetryEvent::DatabaseReadError(err.to_string()), ctx);
            report_error!(anyhow::Error::new(err).context("Failed to read persisted data"));
            None
        }
    }
}

/// Returns a read-only connection to the sqlite database.
/// We want only one write connection to exist and use event processing to write any data needed.
pub fn establish_ro_connection(database_url: &str) -> Result<SqliteConnection> {
    establish_connection(database_url, true)
}

fn establish_connection(database_url: &str, read_only: bool) -> Result<SqliteConnection> {
    let full_database_url = if read_only {
        &format!("file:{database_url}?mode=ro")
    } else {
        database_url
    };
    let mut conn = SqliteConnection::establish(full_database_url)?;
    conn.batch_execute(
        r#"
        PRAGMA foreign_keys = ON;           -- enforce foreign key constraints
        PRAGMA busy_timeout = 1000;         -- sleep for up to 1s if the database is busy
    "#,
    )?;

    // Enable WAL mode, checkpointing whenever the log is at least 500 pages long (in theory,
    // around 2MB). In addition, SQLite will automatically checkpoint when the app closes its
    // database connection.
    // The auto-checkpoint interval is lowered from the default of 1000 because all writes
    // already run in a background thread and can afford to checkpoint slightly more often.
    // At the default value, the WAL can grow larger than a typical database (for our usage).
    conn.batch_execute(
        r#"
        PRAGMA journal_mode=WAL;
        PRAGMA wal_autocheckpoint=500;
    "#,
    )
    .context("Failed to enable WAL")?;

    Ok(conn)
}

/// Set up SQLite [error logging](https://www.sqlite.org/errlog.html)
///
/// ## Safety
/// Setting up SQLite logging is not thread-safe. No other SQLite calls may be made while this
/// function is running.
unsafe fn init_logging() {
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::panic;
    use std::ptr;

    extern "C-unwind" fn log_callback(_data: *mut c_void, err_code: c_int, msg: *const c_char) {
        // `err_code` is an extended error code (https://www.sqlite.org/rescode.html#primary_result_codes_versus_extended_result_codes).
        // In general, the least-significant byte of an extended error code is the primary error
        // code it belongs to. Each primary error code can also be used where an extended error
        // code is expected (for example, `SQLITE_SCHEMA` has no extended error codes).
        let primary_error_code = err_code & 0xFF;
        let level = match (primary_error_code, err_code) {
            // This usually means that a schema change invalidated a prepared statement.
            (sqlite3::SQLITE_SCHEMA, _) => log::Level::Debug,
            // These are used with sqlite3_log, in extensions.
            (sqlite3::SQLITE_NOTICE | sqlite3::SQLITE_WARNING, _) => log::Level::Warn,
            // According to the docs, this error means that the database file was moved (or deleted),
            // so SQLite can't safely modify it and the rollback journal:
            //     https://www.sqlite.org/rescode.html#readonly_dbmoved
            // This is mostly outside of Warp's control (e.g. the user or some system program is
            // moving around files in the user data directory), so downgrade to a warning.
            (_, sqlite3::SQLITE_READONLY_DBMOVED) => log::Level::Warn,
            _ => log::Level::Error,
        };

        // Safety: the message pointer came from the SQLite library, which promises that it's a
        // valid C string pointer.
        let msg = unsafe { CStr::from_ptr(msg) };
        let err_message = String::from_utf8_lossy(msg.to_bytes());
        // Sentry shouldn't panic, but to be safe, make sure we don't unwind across the FFI
        // boundary.
        let _ = panic::catch_unwind(|| {
            // We report SQLite errors to Sentry in a more-structured format so that they have
            // better grouping (all are under the same Sentry issue, with details for the specific
            // error kind). Warning and debug SQLite messages are logged - with the default
            // sentry_log configuration, warnings are added as breadcrumbs to other events and
            // debug messages are ignored.
            // In local builds without crash reporting, all SQLite messages get logged locally.

            #[cfg(feature = "crash_reporting")]
            if level == log::Level::Error {
                sentry::with_scope(
                    |scope| {
                        let mut context = std::collections::BTreeMap::new();
                        context.insert("message".to_string(), err_message.into());
                        context.insert("code".to_string(), err_code.into());
                        context.insert(
                            "code_description".to_string(),
                            sqlite3::code_to_str(err_code).into(),
                        );
                        scope.set_context("sqlite", sentry::protocol::Context::Other(context));
                    },
                    || {
                        sentry::capture_message(
                            "Sqlite Error",
                            sentry_log::convert_log_level(level),
                        )
                    },
                );
                return;
            }

            log::log!(
                level,
                "SQLite error {} ({}): {}",
                err_code,
                sqlite3::code_to_str(err_code),
                err_message
            );
        });
    }

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let null: *const c_void = ptr::null();
        // Diesel doesn't expose SQLite's logging/tracing APIs, but the FFI bindings do.
        let status = sqlite3::sqlite3_config(
            sqlite3::SQLITE_CONFIG_LOG,
            log_callback as extern "C-unwind" fn(_, _, _),
            null,
        );

        if status != sqlite3::SQLITE_OK {
            log::error!(
                "Error setting up SQLite logging: {}",
                sqlite3::code_to_str(status)
            );
        }
    });
}

/// Determines the db path, establishes a connection and runs any migrations.
pub(super) fn init_db(scope: &PersistenceScope) -> Result<SqliteConnection> {
    // First, make sure the parent directory of the file exists, otherwise
    // we'll get an error if the file doesn't already exist.
    let db_path = database_file_path_for_scope(scope);
    // If we fail to create the necessary directories, log a warning and
    // continue; we'll return a sqlite error if it actually fails to initialize
    // a database connection.
    let db_parent = db_path
        .parent()
        .expect("database file path should be absolute");
    if let Err(err) = std::fs::create_dir_all(db_parent) {
        log::warn!(
            "Encountered an error while creating parent directories for sqlite database: {err:#}"
        );
    }
    if matches!(scope, PersistenceScope::RemoteServerDaemon { .. }) {
        ensure_owner_only_dir(db_parent)?;
    }

    if matches!(scope, PersistenceScope::App) {
        migrate_old_sqlite_into_secure_container_if_needed(&db_path);
    }

    let conn = setup_database(&db_path)?;
    if matches!(scope, PersistenceScope::RemoteServerDaemon { .. }) {
        ensure_owner_only_file(&db_path)?;
    }
    Ok(conn)
}

fn migrate_old_sqlite_into_secure_container_if_needed(db_path: &Path) {
    let old_db_path = warp_core::paths::state_dir().join(WARP_SQLITE_FILE_NAME);
    if old_db_path == db_path || !old_db_path.exists() || db_path.exists() {
        return;
    }

    match std::fs::rename(&old_db_path, db_path) {
        Ok(_) => {
            safe_info!(
                safe: ("Migrated SQLite database into application container"),
                full: ("Migrated SQLite database from `{}` to `{}`", old_db_path.display(), db_path.display())
            );

            // Also migrate the associated WAL and SHM files.
            let old_wal = old_db_path.with_extension("sqlite-wal");
            let old_shm = old_db_path.with_extension("sqlite-shm");
            let new_wal = db_path.with_extension("sqlite-wal");
            let new_shm = db_path.with_extension("sqlite-shm");

            if let Err(err) = std::fs::rename(&old_wal, &new_wal) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    report_error!(anyhow::Error::new(err)
                        .context("Failed to migrate SQLite WAL into application container"));
                }
            } else {
                log::info!("Migrated SQLite WAL into application container");
            }

            if let Err(err) = std::fs::rename(&old_shm, &new_shm) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    report_error!(anyhow::Error::new(err)
                        .context("Failed to migrate SQLite SHM into application container"));
                }
            } else {
                log::info!("Migrated SQLite shared memory file into application container");
            }
        }
        Err(err) => {
            report_error!(anyhow::Error::new(err)
                .context("Failed to migrate SQLite database into application container"));
        }
    }
}

/// Creates or connects to the database at `database_path` and runs any migrations.
fn setup_database(database_path: &Path) -> Result<SqliteConnection> {
    let db_url = database_path
        .to_str()
        .ok_or_else(|| anyhow!("Failed to convert db path to a string"))?;
    let mut conn = establish_connection(db_url, false)?;

    safe_info!(
        safe: ("Connecting to SQLite database"),
        full: ("Connecting to SQLite database at {db_url}")
    );
    conn.run_pending_migrations(persistence::MIGRATIONS)
        .map_err(|e| anyhow!(e))
        .context("Failed to perform migrations")?;
    Ok(conn)
}

/// The path at which the sqlite database is located for the given scope.
///
/// Integration tests that initialize the database with known data should use
/// this function to determine where to create the database file.
pub fn database_file_path_for_scope(scope: &PersistenceScope) -> PathBuf {
    match scope {
        PersistenceScope::App => app_database_file_path(),
        PersistenceScope::RemoteServerDaemon { identity_key } => {
            remote_server_daemon_database_file_path(identity_key)
        }
    }
}

fn app_database_file_path() -> PathBuf {
    warp_core::paths::secure_state_dir()
        .unwrap_or_else(warp_core::paths::state_dir)
        .join(WARP_SQLITE_FILE_NAME)
}

fn remote_server_daemon_database_file_path(identity_key: &str) -> PathBuf {
    let data_dir = remote_server::setup::remote_server_daemon_data_dir(identity_key);
    let expanded_data_dir = shellexpand::tilde(&data_dir).into_owned();
    PathBuf::from(expanded_data_dir).join(WARP_SQLITE_FILE_NAME)
}

#[cfg(unix)]
fn ensure_owner_only_dir(path: &Path) -> Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, Permissions::from_mode(0o700))
        .with_context(|| format!("setting permissions on directory {}", path.display()))
}

#[cfg(not(unix))]
fn ensure_owner_only_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn ensure_owner_only_file(path: &Path) -> Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    if path.exists() {
        std::fs::set_permissions(path, Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on file {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only_file(_path: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn remove(sender: SyncSender<ModelEvent>) {
    // Instruct the writer thread to remove the database and pause processing
    // events.
    // Ideally, we'd drop any other events in the channel, but it's not worth the complexity right
    // now. Having the writer thread remove the database file prevents race conditions if the
    // thread is in the middle of another update.
    report_if_error!(sender
        .send(ModelEvent::PauseAndRemoveDatabase)
        .context("Error requesting database deletion"));
}

pub(super) fn reconstruct(sender: SyncSender<ModelEvent>) {
    report_if_error!(sender
        .send(ModelEvent::ReconstructAndResume)
        .context("Error resuming SQLite thread"));
}

fn reconstruct_database(path: &Path) -> Result<SqliteConnection> {
    // If the DB still exists, logout might have failed. However, it's more likely that something
    // else wrote to it before the user logged back in.
    if std::fs::metadata(path).is_ok() {
        log::info!("Reconstructing database, but it already exists");
    }

    // Always reinitialize DB - setup_database will only create it if it doesn't exist.
    setup_database(path)
}

fn start_writer(conn: SqliteConnection, database_path: PathBuf) -> Result<WriterHandles> {
    let (tx, rx) = std::sync::mpsc::sync_channel(CHANNEL_SIZE);
    let mut current_conn = conn;
    let handle = thread::Builder::new()
        .name("SQLite Writer".into())
        .spawn(move || {
            let mut paused = false;
            loop {
                let events = match rx.recv() {
                    Ok(event) => {
                        // Wait for there to be at least one event, but collect any other pending
                        // events too. This way, we can start dropping redundant events if the
                        // writer thread is falling behind.
                        let mut events = vec![event];
                        events.extend(rx.try_iter());
                        deduplicate_events(events)
                    }
                    Err(_) => {
                        log::warn!(
                            "SQLite event sender has closed; terminating SQLite writer thread."
                        );
                        break;
                    }
                };

                for event in events {
                    match event {
                        ModelEvent::ReconstructAndResume => {
                            match reconstruct_database(&database_path) {
                                Ok(conn) => {
                                    current_conn = conn;
                                    paused = false;
                                    log::info!("SQLite Writer is resumed");
                                }
                                Err(err) => {
                                    report_db_error("reconstruction", err, &database_path);
                                }
                            }
                        }
                        ModelEvent::PauseAndRemoveDatabase => {
                            paused = true;
                            log::info!("SQLite Writer is paused");

                            if let Err(err) = std::fs::remove_file(&database_path) {
                                report_error!(anyhow::Error::new(err)
                                    .context("Error removing SQLite database"));
                            } else {
                                log::info!("Removed SQLite database");
                            }
                        }
                        ModelEvent::Terminate => {
                            log::info!("Shutting down SQLite writer thread");
                            return;
                        }
                        event => {
                            if paused {
                                log::info!("Ignoring event as SQLite Writer is on pause");
                                continue;
                            }
                            if let Err(err) = handle_model_event(event, &mut current_conn) {
                                report_db_error("Model", err, &database_path);
                            }
                        }
                    }
                }
            }
        })?;
    Ok(WriterHandles { handle, sender: tx })
}

/// Handles a single [`ModelEvent`] by dispatching to an event-specific function.
/// Events which affect the SQLite writer event loop _must_ instead be handled by the event loop itself:
/// * [`ModelEvent::PauseAndRemoveDatabase`]
/// * [`ModelEvent::ReconstructAndResume`]
/// * [`ModelEvent::Terminate`]
fn handle_model_event(event: ModelEvent, connection: &mut SqliteConnection) -> anyhow::Result<()> {
    match event {
        ModelEvent::PauseAndRemoveDatabase
        | ModelEvent::ReconstructAndResume
        | ModelEvent::Terminate => {
            panic!("Unhandled control-flow event {event:?}");
        }
        ModelEvent::SaveBlock(BlockCompleted {
            pane_id,
            block,
            is_local,
        }) => save_block(connection, pane_id, &block, is_local).context("error saving block"),
        ModelEvent::DeleteBlocks(pane_id) => {
            // Delete the blocks even if the setting is off so users can still remove
            // panes and have their data deleted locally.
            delete_blocks(connection, pane_id).context("error deleting blocks")
        }
        ModelEvent::Snapshot(app_state) => {
            save_app_state(connection, &app_state).context("error saving app state")
        }
        ModelEvent::UpsertWorkflows(workflows) => {
            upsert_workflows(connection, workflows).context("error saving workflows")
        }
        ModelEvent::UpsertNotebooks(notebooks) => {
            upsert_notebooks(connection, notebooks).context("error saving notebooks")
        }
        ModelEvent::UpsertFolders(folders) => {
            upsert_folders(connection, folders).context("error saving folders")
        }
        ModelEvent::UpsertGenericStringObject { object } => {
            upsert_generic_string_objects(connection, vec![object])
                .context("error upserting generic object")
        }
        ModelEvent::UpsertGenericStringObjects(objects) => {
            upsert_generic_string_objects(connection, objects)
                .context("error upserting generic objects")
        }
        ModelEvent::UpsertNotebook { notebook } => {
            upsert_notebooks(connection, vec![notebook]).context("error upserting notebook")
        }
        ModelEvent::UpsertWorkflow { workflow } => {
            upsert_workflows(connection, vec![workflow]).context("error upserting workflow")
        }
        ModelEvent::UpsertFolder { folder } => {
            upsert_folders(connection, vec![folder]).context("error upserting folder")
        }
        ModelEvent::MarkObjectAsSynced {
            revision_and_editor,
            metadata_ts,
            hashed_sqlite_id,
        } => mark_object_as_synced(
            connection,
            hashed_sqlite_id,
            revision_and_editor,
            metadata_ts,
        )
        .context("error marking object as synced"),
        ModelEvent::IncrementRetryCount(id) => {
            increment_retry_count(connection, id).context("error incrementing retry count")
        }
        ModelEvent::DeleteObjects { ids } => {
            delete_objects(connection, ids).context("error deleting objects")
        }
        ModelEvent::UpdateObjectAfterServerCreation {
            client_id,
            server_creation_info,
        } => update_object_after_server_creation(connection, client_id, server_creation_info)
            .context("error executing object creation succeeded callback"),
        ModelEvent::UpsertCodebaseIndexMetadata { index_metadata } => {
            save_codebase_index_metadata(connection, *index_metadata)
                .context("error upserting codebase index metadata")
        }
        ModelEvent::DeleteCodebaseIndexMetadata { repo_path } => {
            delete_codebase_index_metadata(connection, &repo_path)
                .context("error deleting codebase index metadata")
        }
        ModelEvent::UpsertProject { project } => {
            save_project(connection, project).context("error upserting project")
        }
        ModelEvent::DeleteProject { path } => {
            delete_project(connection, &path).context("error deleting project")
        }
        ModelEvent::UpsertWorkspace { workspace } => {
            save_workspace(connection, *workspace).context("error upserting workspace")
        }
        ModelEvent::UpsertWorkspaces { workspaces } => {
            save_workspaces(connection, workspaces).context("error upserting workspaces")
        }
        ModelEvent::SetCurrentWorkspace { workspace_uid } => {
            set_current_workspace(connection, workspace_uid)
                .context("error setting current workspace")
        }
        ModelEvent::UpdateObjectMetadata { id, metadata } => {
            update_object_metadata(connection, id, metadata).context("error updating metadata")
        }
        ModelEvent::InsertCommand { metadata } => {
            insert_command(connection, metadata).context("error inserting command")
        }
        ModelEvent::UpdateFinishedCommand { metadata } => {
            update_finished_command(connection, metadata).context("error updating finished command")
        }
        ModelEvent::UpsertUserProfiles { profiles } => {
            upsert_user_profiles(connection, profiles).context("error updating user profiles")
        }
        ModelEvent::ClearUserProfiles => {
            clear_user_profiles(connection).context("error clearing user profiles")
        }
        ModelEvent::RecordTimeOfNextRefresh { timestamp } => {
            record_time_of_next_refresh(connection, timestamp)
                .context("error marking object refresh as completed")
        }
        ModelEvent::InsertObjectAction { object_action } => {
            insert_object_action(connection, object_action).context("error inserting object action")
        }
        ModelEvent::SyncObjectActions {
            actions_to_sync: objects_to_sync,
        } => {
            sync_object_actions(connection, objects_to_sync).context("error syncing object actions")
        }
        ModelEvent::SaveExperiments { experiments } => {
            save_experiments(connection, experiments).context("error saving experiments")
        }
        ModelEvent::UpsertAIQuery { query } => {
            upsert_ai_query(connection, query).context("error upserting AI query")
        }
        ModelEvent::DeleteAIConversation { conversation_id } => {
            delete_ai_conversation(connection, &conversation_id)
                .context("error deleting AI conversation")
        }
        ModelEvent::UpdateMultiAgentConversation {
            conversation_id,
            updated_tasks,
            conversation_data,
        } => upsert_agent_conversation(
            connection,
            &conversation_id,
            &updated_tasks,
            conversation_data,
        )
        .map_err(anyhow::Error::from),
        ModelEvent::DeleteMultiAgentConversations { conversation_ids } => {
            delete_agent_conversations(connection, conversation_ids)
                .map_err(anyhow::Error::from)
                .context("error deleting multi-agent conversation")
        }
        ModelEvent::UpsertCurrentUserInformation { user_information } => {
            upsert_current_user_information(connection, user_information)
                .context("error upserting user information")
        }
        ModelEvent::UpsertMCPServerEnvironmentVariables {
            mcp_server_uuid,
            environment_variables,
        } => upsert_mcp_server_environment_variables(
            connection,
            mcp_server_uuid,
            environment_variables,
        )
        .context("error upserting mcp server mcp_environment variables"),
        ModelEvent::UpsertProjectRules { project_rule_paths } => {
            upsert_project_rules(connection, project_rule_paths)
                .context("error upserting project rules")
        }
        ModelEvent::DeleteProjectRules { path } => {
            delete_project_rules(connection, path).context("error deleting project rules")
        }
        ModelEvent::AddIgnoredSuggestion {
            suggestion,
            suggestion_type,
        } => add_ignored_suggestion(connection, suggestion, suggestion_type)
            .context("error adding ignored suggestion"),
        ModelEvent::RemoveIgnoredSuggestion {
            suggestion,
            suggestion_type,
        } => remove_ignored_suggestion(connection, suggestion, suggestion_type)
            .context("error removing ignored suggestion"),
        ModelEvent::UpsertMCPServerInstallation {
            mcp_server_installation,
        } => upsert_mcp_server_installation(connection, mcp_server_installation),
        ModelEvent::DeleteMCPServerInstallations { installation_uuids } => {
            delete_mcp_server_installations(connection, installation_uuids)
        }
        ModelEvent::DeleteMCPServerInstallationsByTemplateUuid { template_uuid } => {
            delete_mcp_server_installations_by_template_uuid(connection, template_uuid)
        }
        ModelEvent::UpdateMCPInstallationRunning {
            installation_uuid,
            running,
        } => update_mcp_server_running(connection, installation_uuid, running)
            .context("Error updating running field for MCP installation"),
        ModelEvent::UpsertWorkspaceLanguageServer {
            workspace_path,
            lsp_type,
            enabled,
        } => upsert_workspace_language_server(connection, &workspace_path, lsp_type, enabled)
            .context("error upserting workspace language server"),
        ModelEvent::UpdateBlockAgentViewVisibility {
            block_id,
            agent_view_visibility,
        } => update_block_agent_view_visibility(connection, &block_id, &agent_view_visibility)
            .context("error updating block agent view visibility"),
        ModelEvent::SaveAIDocumentContent {
            document_id,
            content,
            version,
            title,
        } => save_ai_document_content(connection, &document_id, &content, version, &title)
            .context("error saving AI document content"),
    }
}

/// Report a database error and additional context for debugging.
fn report_db_error(err_kind: &str, err: anyhow::Error, database_path: &Path) {
    // Sentry reports indicate that the database is sometimes missing/inaccessible, so check its
    // permissions and whether or not it exists.
    fn log_access(prefix: &str, path: &Path) {
        match fs::metadata(path) {
            Ok(metadata) => {
                cfg_if::cfg_if! {
                    if #[cfg(windows)] {
                        use async_fs::windows::MetadataExt;
                        // Windows does not have the same notion of permissions as Unix-based file systems.
                        // See more about what File Attributes contain [here](https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants).
                        let attributes = metadata.file_attributes();
                        safe_info!(
                            safe: ("{prefix} attributes: {attributes}"),
                            full: ("{prefix} {} attributes: {attributes}", path.display())
                        );
                    } else {
                        use async_fs::unix::PermissionsExt;
                        let mode = metadata.permissions().mode();
                        safe_info!(
                            safe: ("{prefix} permissions: {mode:o}"),
                            full: ("{prefix} {} permissions: {mode:o}", path.display())
                        );
                    }
                }
            }
            Err(err) => {
                safe_info!(
                    safe: ("{prefix} is inaccessible: {err}"),
                    full: ("{prefix} {} is inaccessible: {err}", path.display())
                );
            }
        }
    }

    if let Some(parent) = database_path.parent() {
        log_access("Database directory", parent);
    }
    log_access("Database", database_path);

    report_error!(err.context(format!("SQLite {err_kind} error")));
}

/// Filter a collection of model events to remove skippable events:
/// * [`ModelEvent::Snapshot`] includes the entire app state, so we only need the latest one.
fn deduplicate_events(events: Vec<ModelEvent>) -> Vec<ModelEvent> {
    let last_snapshot = events
        .iter()
        .enumerate()
        .rfind(|(_, event)| matches!(event, ModelEvent::Snapshot(_)));
    match last_snapshot {
        Some((last_snapshot_index, _)) => events
            .into_iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                ModelEvent::Snapshot(_) if index < last_snapshot_index => None,
                event => Some(event),
            })
            .collect(),
        None => events,
    }
}

// Used in the save_app_state function to help make the code more readable.
struct SaveAppStateNodeTraversal<'a> {
    node: &'a PaneNodeSnapshot,
    flex: Option<f32>,
    parent_pane_node_id: Option<i32>,
}

// Saves the app state snapshot in the sqlite database. Removes any old app state.
// Does so in a transaction so we're never in a partial state.
fn save_app_state(conn: &mut SqliteConnection, app_state: &AppState) -> Result<()> {
    conn.transaction::<(), Error, _>(|conn| {
        // Remove old app state
        diesel::delete(schema::app::dsl::app).execute(conn)?;
        diesel::delete(schema::terminal_panes::dsl::terminal_panes).execute(conn)?;
        diesel::delete(schema::notebook_panes::dsl::notebook_panes).execute(conn)?;
        diesel::delete(schema::code_panes::dsl::code_panes).execute(conn)?;
        diesel::delete(schema::env_var_collection_panes::dsl::env_var_collection_panes)
            .execute(conn)?;
        diesel::delete(schema::workflow_panes::dsl::workflow_panes).execute(conn)?;
        diesel::delete(schema::settings_panes::dsl::settings_panes).execute(conn)?;
        diesel::delete(schema::ai_memory_panes::dsl::ai_memory_panes).execute(conn)?;
        diesel::delete(schema::ai_document_panes::dsl::ai_document_panes).execute(conn)?;
        diesel::delete(schema::mcp_server_panes::dsl::mcp_server_panes).execute(conn)?;
        diesel::delete(schema::code_review_panes::dsl::code_review_panes).execute(conn)?;
        diesel::delete(schema::ambient_agent_panes::dsl::ambient_agent_panes).execute(conn)?;
        diesel::delete(schema::welcome_panes::dsl::welcome_panes).execute(conn)?;
        diesel::delete(schema::pane_leaves::dsl::pane_leaves).execute(conn)?;
        diesel::delete(schema::pane_branches::dsl::pane_branches).execute(conn)?;
        diesel::delete(schema::pane_nodes::dsl::pane_nodes).execute(conn)?;
        diesel::delete(schema::tabs::dsl::tabs).execute(conn)?;
        diesel::delete(schema::windows::dsl::windows).execute(conn)?;
        diesel::delete(schema::active_mcp_servers::dsl::active_mcp_servers).execute(conn)?;
        diesel::delete(schema::panels::dsl::panels).execute(conn)?;

        let mut active_window_id = None;

        for (idx, window) in app_state.windows.iter().enumerate() {
            // Just save zero as the tab index, if we overflow when converting
            // unsigned to signed.
            let active_tab_index: i32 = window.active_tab_index.try_into().unwrap_or(0);

            // In the database each individual field is nullable but in practice these
            // fields are either all null or all non-null as they together represent
            // the stored window bound.
            let (window_width, window_height, origin_x, origin_y) = match window.bounds {
                Some(rect) => (
                    Some(rect.size().x()),
                    Some(rect.size().y()),
                    Some(rect.origin().x()),
                    Some(rect.origin().y()),
                ),
                _ => (None, None, None, None),
            };

            let new_window = NewWindow {
                active_tab_index,
                window_width,
                window_height,
                origin_x,
                origin_y,
                quake_mode: window.quake_mode,
                universal_search_width: window.universal_search_width,
                warp_ai_width: window.warp_ai_width,
                voltron_width: window.voltron_width,
                warp_drive_index_width: window.warp_drive_index_width,
                left_panel_open: Some(window.left_panel_open),
                vertical_tabs_panel_open: Some(window.vertical_tabs_panel_open),
                fullscreen_state: window.fullscreen_state as i32,
                agent_management_filters: window
                    .agent_management_filters
                    .as_ref()
                    .and_then(|f| serde_json::to_string(f).ok()),
            };
            diesel::insert_into(schema::windows::dsl::windows)
                .values(new_window)
                .execute(conn)?;

            // We cannot directly return the id from the insert so perform
            // a second query for the id https://github.com/diesel-rs/diesel/issues/771.
            let window_id: i32 = schema::windows::dsl::windows
                .select(schema::windows::columns::id)
                .order(schema::windows::columns::id.desc())
                .first(conn)?;

            if app_state
                .active_window_index
                .map(|id| id == idx)
                .unwrap_or(false)
            {
                active_window_id = Some(window_id)
            }

            let tabs: Vec<NewTab> = window
                .tabs
                .iter()
                .map(|tab| NewTab {
                    window_id,
                    custom_title: tab.custom_title.clone(),
                    // We only persist and restore the selected color here
                    // (the default color based on the pwd is separately persisted and then applied on-restore)
                    color: match tab.selected_color {
                        // Keep the column NULL for the common no-override case
                        SelectedTabColor::Unset => None,
                        _ => serde_yaml::to_string(&tab.selected_color).ok(),
                    },
                })
                .collect();

            diesel::insert_into(schema::tabs::dsl::tabs)
                .values(tabs)
                .execute(conn)?;

            // Same ID issue as above.
            let tab_ids: Vec<i32> = schema::tabs::dsl::tabs
                .filter(schema::tabs::columns::window_id.eq(window_id))
                .select(schema::tabs::columns::id)
                .order(schema::tabs::columns::id.desc())
                .load(conn)?;

            // Since we retrieved the tab ids in descending order, we need to reverse them when we
            // iterate to restore the correct order.
            for (tab_id, tab) in tab_ids.iter().rev().zip(window.tabs.iter()) {
                let mut pane_nodes = VecDeque::new();
                pane_nodes.push_back(SaveAppStateNodeTraversal {
                    node: &tab.root,
                    flex: None,
                    parent_pane_node_id: None,
                });

                if tab.left_panel.is_some() || tab.right_panel.is_some() {
                    let new_panel = model::NewPanel {
                        tab_id: *tab_id,
                        left_panel: tab
                            .left_panel
                            .as_ref()
                            .and_then(|p| serde_json::to_string(p).ok()),
                        right_panel: tab
                            .right_panel
                            .as_ref()
                            .and_then(|p| serde_json::to_string(p).ok()),
                    };
                    diesel::insert_into(schema::panels::dsl::panels)
                        .values(new_panel)
                        .execute(conn)?;
                }

                while !pane_nodes.is_empty() {
                    let SaveAppStateNodeTraversal {
                        node: pane_node,
                        flex,
                        parent_pane_node_id,
                    } = pane_nodes.pop_front().expect("Should have node");

                    // Skip leaves whose content types don't get a
                    // corresponding `pane_leaves` row on save. Otherwise the
                    // `pane_nodes` insert below would create an orphan row
                    // (is_leaf=true, but no matching row in `pane_leaves`),
                    // and `read_node` would fail to resolve the leaf on
                    // restore, causing the entire surrounding tab to be
                    // dropped. See `LeafContents::is_persisted`.
                    if let PaneNodeSnapshot::Leaf(leaf) = pane_node {
                        if !leaf.contents.is_persisted() {
                            continue;
                        }
                    }

                    let is_leaf = matches!(pane_node, PaneNodeSnapshot::Leaf(_));
                    let new_pane_node = model::NewPaneNode {
                        tab_id: *tab_id,
                        parent_pane_node_id,
                        flex,
                        is_leaf,
                    };

                    diesel::insert_into(schema::pane_nodes::dsl::pane_nodes)
                        .values(new_pane_node)
                        .execute(conn)?;

                    // Same ID issue as above.
                    let pane_node_id = schema::pane_nodes::dsl::pane_nodes
                        .select(schema::pane_nodes::columns::id)
                        .order(schema::pane_nodes::columns::id.desc())
                        .first(conn)?;
                    match pane_node {
                        PaneNodeSnapshot::Branch(pane_group) => {
                            let new_pane_branch = model::NewPaneBranch {
                                pane_node_id,
                                horizontal: pane_group.direction == SplitDirection::Horizontal,
                            };
                            diesel::insert_into(schema::pane_branches::dsl::pane_branches)
                                .values(new_pane_branch)
                                .execute(conn)?;

                            for (flex, child_pane_node) in &pane_group.children {
                                pane_nodes.push_back(SaveAppStateNodeTraversal {
                                    node: child_pane_node,
                                    flex: Some(flex.0),
                                    parent_pane_node_id: Some(pane_node_id),
                                });
                            }
                        }
                        PaneNodeSnapshot::Leaf(pane) => {
                            save_pane_state(conn, pane_node_id, pane)?;
                        }
                    }
                }
            }
        }

        let new_app = NewApp { active_window_id };

        diesel::insert_into(schema::app::dsl::app)
            .values(new_app)
            .execute(conn)?;

        // Save active MCP servers
        let active_mcp_servers: Vec<NewActiveMCPServer> = app_state
            .running_mcp_servers
            .iter()
            .map(|uuid| NewActiveMCPServer {
                mcp_server_uuid: uuid.to_string(),
            })
            .collect();

        if !active_mcp_servers.is_empty() {
            diesel::insert_into(schema::active_mcp_servers::dsl::active_mcp_servers)
                .values(active_mcp_servers)
                .execute(conn)?;
        }

        Ok(())
    })?;
    Ok(())
}

/// Saves the state of an individual pane, after the corresponding `pane_nodes` entry
/// has been written.
fn save_pane_state(
    conn: &mut SqliteConnection,
    id: i32,
    snapshot: &LeafSnapshot,
) -> Result<(), Error> {
    // The pane_leaves row must be inserted first to satisfy foreign key constraints on the
    // kind-specific tables.
    let kind = match &snapshot.contents {
        LeafContents::Terminal(_) => TERMINAL_PANE_KIND,
        LeafContents::Notebook(_) => NOTEBOOK_PANE_KIND,
        LeafContents::EnvVarCollection(_) => ENV_VAR_COLLECTION_PANE_KIND,
        LeafContents::Code(_) => CODE_PANE_KIND,
        LeafContents::Workflow(_) => WORKFLOW_PANE_KIND,
        LeafContents::Settings(_) => SETTINGS_PANE_KIND,
        LeafContents::AIFact(_) => AI_FACT_PANE_KIND,
        LeafContents::CodeReview(_) => CODE_REVIEW_PANE_KIND,
        LeafContents::AmbientAgent(_) => AMBIENT_AGENT_PANE_KIND,
        LeafContents::ExecutionProfileEditor => EXECUTION_PROFILE_EDITOR_PANE_KIND,
        LeafContents::GetStarted => GET_STARTED_PANE_KIND,
        LeafContents::Welcome { .. } => WELCOME_PANE_KIND,
        LeafContents::AIDocument(_) => AI_DOCUMENT_PANE_KIND,
        LeafContents::EnvironmentManagement(_) | LeafContents::NetworkLog => {
            // These pane types are filtered out before this function is
            // called; see `LeafContents::is_persisted` and the skip in
            // `save_app_state`. Reaching this arm would mean a `pane_nodes`
            // row had already been inserted with no corresponding
            // `pane_leaves` row, which would break restoration.
            debug_assert!(
                false,
                "save_pane_state called for non-persisted LeafContents variant"
            );
            return Ok(());
        }
    };

    let leaf = model::NewPane {
        pane_node_id: id,
        kind: kind.into(),
        is_focused: snapshot.is_focused,
        custom_vertical_tabs_title: snapshot.custom_vertical_tabs_title.clone(),
    };

    diesel::insert_into(schema::pane_leaves::dsl::pane_leaves)
        .values(leaf)
        .execute(conn)?;

    match &snapshot.contents {
        LeafContents::Terminal(terminal_snapshot) => {
            let conversation_ids = if terminal_snapshot.conversation_ids_to_restore.is_empty() {
                None
            } else {
                let ids: Vec<String> = terminal_snapshot
                    .conversation_ids_to_restore
                    .iter()
                    .map(|id| id.to_string())
                    .collect();
                serde_json::to_string(&ids).ok()
            };

            let terminal = model::NewTerminalPane {
                id,
                uuid: terminal_snapshot.uuid.clone(),
                cwd: terminal_snapshot.cwd.clone(),
                is_active: terminal_snapshot.is_active,
                shell_launch_data: terminal_snapshot
                    .shell_launch_data
                    .as_ref()
                    .and_then(|shell| serde_json::to_string(shell).ok()),
                input_config: terminal_snapshot
                    .input_config
                    .as_ref()
                    .and_then(|config| serde_json::to_string(config).ok()),
                llm_model_override: terminal_snapshot.llm_model_override.clone(),
                active_profile_id: terminal_snapshot
                    .active_profile_id
                    .as_ref()
                    .and_then(|sync_id| serde_json::to_string(sync_id).ok()),
                conversation_ids,
                active_conversation_id: terminal_snapshot
                    .active_conversation_id
                    .map(|id| id.to_string()),
            };

            diesel::insert_into(schema::terminal_panes::dsl::terminal_panes)
                .values(terminal)
                .execute(conn)?;
        }
        LeafContents::Notebook(notebook_snapshot) => {
            let (notebook_id, local_path) = match notebook_snapshot {
                NotebookPaneSnapshot::CloudNotebook {
                    notebook_id,
                    settings: _,
                } => (
                    notebook_id.map(|id| id.sqlite_uid_hash(ObjectIdType::Notebook)),
                    None,
                ),
                NotebookPaneSnapshot::LocalFileNotebook { path } => {
                    (None, path.clone().map(encode_path))
                }
            };

            let notebook = model::NewNotebookPane {
                id,
                notebook_id,
                local_path,
            };

            diesel::insert_into(schema::notebook_panes::dsl::notebook_panes)
                .values(notebook)
                .execute(conn)?;
        }
        LeafContents::Code(code_snapshot) => {
            let CodePaneSnapShot::Local {
                tabs,
                active_tab_index,
                source,
            } = code_snapshot;

            let serialized_source = source.as_ref().and_then(|s| serde_json::to_string(s).ok());

            let code = model::NewCodePane {
                id,
                active_tab_index: *active_tab_index as i32,
                source_data: serialized_source,
            };

            diesel::insert_into(schema::code_panes::dsl::code_panes)
                .values(code)
                .execute(conn)?;

            // Write ordered tab rows.
            for (tab_idx, tab) in tabs.iter().enumerate() {
                let tab_row = model::NewCodePaneTab {
                    code_pane_id: id,
                    tab_index: tab_idx as i32,
                    local_path: tab.path.clone().map(encode_path),
                };
                diesel::insert_into(schema::code_pane_tabs::dsl::code_pane_tabs)
                    .values(tab_row)
                    .execute(conn)?;
            }
        }
        LeafContents::EnvVarCollection(env_var_collection_snapshot) => {
            let env_var_collection_id = match env_var_collection_snapshot {
                EnvVarCollectionPaneSnapshot::CloudEnvVarCollection {
                    env_var_collection_id,
                } => env_var_collection_id
                    .map(|id| id.sqlite_uid_hash(ObjectIdType::GenericStringObject)),
            };

            let env_var_collection = model::NewEnvVarCollectionPane {
                id,
                env_var_collection_id,
            };

            diesel::insert_into(schema::env_var_collection_panes::dsl::env_var_collection_panes)
                .values(env_var_collection)
                .execute(conn)?;
        }
        LeafContents::Workflow(workflow_pane_snapshot) => {
            let workflow_id = match workflow_pane_snapshot {
                WorkflowPaneSnapshot::CloudWorkflow {
                    workflow_id,
                    settings: _,
                } => workflow_id.map(|id| id.sqlite_uid_hash(ObjectIdType::Workflow)),
            };

            let workflow = model::NewWorkflowPane { id, workflow_id };

            diesel::insert_into(schema::workflow_panes::dsl::workflow_panes)
                .values(workflow)
                .execute(conn)?;
        }
        LeafContents::EnvironmentManagement(_) => {
            // Unreachable: filtered by `is_persisted` in `save_app_state`.
        }
        LeafContents::Settings(settings_pane_snapshot) => {
            let current_page = match settings_pane_snapshot {
                SettingsPaneSnapshot::Local { current_page, .. } => current_page,
            };

            let settings_pane = model::NewSettingsPane {
                id,
                current_page: current_page.to_string(),
            };

            diesel::insert_into(schema::settings_panes::dsl::settings_panes)
                .values(settings_pane)
                .execute(conn)?;
        }
        LeafContents::AIFact(_ai_fact_pane_snapshot) => {
            let ai_fact = model::NewAIFactPane { id };

            diesel::insert_into(schema::ai_memory_panes::dsl::ai_memory_panes)
                .values(ai_fact)
                .execute(conn)?;
        }
        LeafContents::CodeReview(code_review_pane_snapshot) => {
            let CodeReviewPaneSnapshot::Local {
                terminal_uuid,
                repo_path,
            } = code_review_pane_snapshot;
            let code_review = model::NewCodeReviewPane {
                id,
                terminal_uuid: terminal_uuid.clone(),
                repo_path: repo_path.to_string_lossy().into_owned(),
            };

            diesel::insert_into(schema::code_review_panes::dsl::code_review_panes)
                .values(code_review)
                .execute(conn)?;
        }
        LeafContents::ExecutionProfileEditor => {
            // TODO: Implement execution profile editor pane saving.
        }
        LeafContents::GetStarted => {
            // Stateless
        }
        LeafContents::Welcome { startup_directory } => {
            let welcome_pane = model::NewWelcomePane {
                id,
                startup_directory: startup_directory
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
            };
            diesel::insert_into(schema::welcome_panes::dsl::welcome_panes)
                .values(welcome_pane)
                .execute(conn)?;
        }
        LeafContents::AIDocument(ai_document_snapshot) => match ai_document_snapshot {
            crate::app_state::AIDocumentPaneSnapshot::Local {
                document_id,
                version,
                content,
                title,
            } => {
                let ai_document_pane = model::NewAIDocumentPane {
                    id,
                    document_id: document_id.clone(),
                    version: *version,
                    content: content.clone(),
                    title: title.clone(),
                };

                diesel::insert_into(schema::ai_document_panes::dsl::ai_document_panes)
                    .values(ai_document_pane)
                    .execute(conn)?;
            }
        },
        LeafContents::AmbientAgent(snapshot) => {
            let ambient_agent_pane = model::NewAmbientAgentPane {
                id,
                uuid: snapshot.uuid.clone(),
                task_id: snapshot.task_id.map(|t| t.to_string()),
            };

            diesel::insert_into(schema::ambient_agent_panes::dsl::ambient_agent_panes)
                .values(ambient_agent_pane)
                .execute(conn)?;
        }
        LeafContents::NetworkLog => {
            // Unreachable: filtered by `is_persisted` in `save_app_state`.
        }
    }

    Ok(())
}

/// Update the content, version, and title of an AI document pane in SQLite.
fn save_ai_document_content(
    conn: &mut SqliteConnection,
    doc_id: &str,
    doc_content: &str,
    doc_version: i32,
    doc_title: &str,
) -> Result<()> {
    use schema::ai_document_panes::dsl::*;

    diesel::update(ai_document_panes.filter(document_id.eq(doc_id)))
        .set((
            content.eq(Some(doc_content)),
            version.eq(doc_version),
            title.eq(Some(doc_title)),
        ))
        .execute(conn)?;

    Ok(())
}

/// Encode a path into a platform-specific byte representation for persistence.
fn encode_path(path: PathBuf) -> Vec<u8> {
    if path == PathBuf::new() {
        // bytemuck will throw a TargetAlignmentGreaterAndInputNotAligned error
        // if we don't special-case the empty path.
        return Vec::new();
    }

    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::ffi::OsStringExt;
            path.into_os_string().into_vec()
        } else if #[cfg(windows)] {
            use std::os::windows::ffi::OsStrExt;
            let wide_char_sequence: Vec<u16> = path.into_os_string().encode_wide().collect();
            // We need to deal with slices (not Vec) because otherwise we will get a PodCastError::AlignmentMismatch.
            let slice: &[u8] = bytemuck::cast_slice(wide_char_sequence.as_slice());
            slice.to_vec()
        }
    }
}

/// Decode a path from its platform-specific byte representation.
fn decode_path(bytes: Vec<u8>) -> PathBuf {
    if bytes.is_empty() {
        // bytemuck will throw a TargetAlignmentGreaterAndInputNotAligned error
        // if we don't special-case the empty path.
        return PathBuf::new();
    }

    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(bytes).into()
        } else if #[cfg(windows)] {
            use std::os::windows::ffi::OsStringExt;
            // We need to deal with slices (not Vec) because otherwise we will get a PodCastError::AlignmentMismatch.
            let wide_char_sequence: &[u16] = bytemuck::cast_slice(bytes.as_slice());
            OsString::from_wide(wide_char_sequence).into()
        }
    }
}

fn save_codebase_index_metadata(
    conn: &mut SqliteConnection,
    index_metadata: ai::workspace::WorkspaceMetadata,
) -> Result<()> {
    use schema::workspace_metadata::dsl::*;

    let new_metadata: NewWorkspaceMetadata = index_metadata.into();

    diesel::insert_into(workspace_metadata)
        .values(new_metadata.clone())
        .on_conflict(repo_path)
        .do_update()
        .set(&new_metadata)
        .execute(conn)?;

    Ok(())
}

fn get_all_codebase_index_metadata(
    conn: &mut SqliteConnection,
) -> Result<Vec<ai::workspace::WorkspaceMetadata>, diesel::result::Error> {
    use schema::workspace_metadata::dsl::*;

    Ok(workspace_metadata
        .load_iter::<WorkspaceMetadataModel, DefaultLoadingMode>(conn)?
        .filter_map(|item| item.ok().map(ai::workspace::WorkspaceMetadata::from))
        .collect_vec())
}

fn get_all_workspace_language_servers_by_workspace(
    conn: &mut SqliteConnection,
) -> Result<HashMap<PathBuf, HashMap<LSPServerType, EnablementState>>, diesel::result::Error> {
    use schema::workspace_language_server::dsl::*;
    use schema::workspace_metadata;

    let results = workspace_language_server
        .inner_join(workspace_metadata::table)
        .select((workspace_metadata::repo_path, language_server_name, enabled))
        .load::<(String, String, String)>(conn)?;

    let mut grouped: HashMap<PathBuf, HashMap<LSPServerType, EnablementState>> = HashMap::new();
    for (path_str, server_name, enablement_str) in results {
        let path = PathBuf::from(path_str);
        let Some(server_type) = serde_json::from_str(&server_name).ok() else {
            continue;
        };

        let Some(enablement) = serde_json::from_str(&enablement_str).ok() else {
            continue;
        };

        grouped
            .entry(path)
            .or_default()
            .insert(server_type, enablement);
    }

    Ok(grouped)
}

fn upsert_workspace_language_server(
    conn: &mut SqliteConnection,
    workspace_path: &Path,
    server_type: LSPServerType,
    enablement: EnablementState,
) -> Result<()> {
    use schema::workspace_language_server::dsl::*;
    use schema::workspace_metadata::dsl::*;
    let path_string = workspace_path.to_string_lossy().to_string();

    // Try to find existing workspace
    let metadata = workspace_metadata
        .filter(repo_path.eq(&path_string))
        .first::<WorkspaceMetadataModel>(conn)
        .optional()?
        .ok_or(anyhow::anyhow!("Can't find workspace for path"))?;

    let ws_id = metadata.id;
    let server_name = serde_json::to_string(&server_type)?;

    // Now upsert the language server setting
    // Check if record already exists
    let existing = workspace_language_server
        .filter(workspace_id.eq(ws_id))
        .filter(language_server_name.eq(server_name.clone()))
        .first::<model::WorkspaceLanguageServer>(conn)
        .optional()?;

    let enablement_str = serde_json::to_string(&enablement)?;

    if let Some(existing_record) = existing {
        // Update existing record
        diesel::update(workspace_language_server.find(existing_record.id))
            .set(enabled.eq(enablement_str))
            .execute(conn)?;
    } else {
        // Insert new record
        let new_language_server = model::NewWorkspaceLanguageServer {
            workspace_id: ws_id,
            language_server_name: server_name,
            enabled: enablement_str.to_string(),
        };

        diesel::insert_into(workspace_language_server)
            .values(&new_language_server)
            .execute(conn)?;
    }

    Ok(())
}

fn delete_codebase_index_metadata(conn: &mut SqliteConnection, index_path: &Path) -> Result<()> {
    use schema::workspace_metadata::dsl::*;

    let target_path = index_path.to_string_lossy().to_string();
    diesel::delete(workspace_metadata.filter(repo_path.eq(target_path))).execute(conn)?;

    Ok(())
}

fn save_project(conn: &mut SqliteConnection, project: Project) -> Result<()> {
    use schema::projects::dsl::*;

    diesel::insert_into(projects)
        .values(project.clone())
        .on_conflict(path)
        .do_update()
        .set(&project)
        .execute(conn)?;

    Ok(())
}

fn get_all_projects(conn: &mut SqliteConnection) -> Result<Vec<Project>, diesel::result::Error> {
    use schema::projects::dsl::*;

    Ok(projects
        .load_iter::<Project, DefaultLoadingMode>(conn)?
        .filter_map(|item| item.ok())
        .collect_vec())
}

fn delete_project(conn: &mut SqliteConnection, project_path: &str) -> Result<()> {
    use schema::projects::dsl::*;

    diesel::delete(projects.filter(path.eq(project_path))).execute(conn)?;

    Ok(())
}

fn get_all_project_rules(
    conn: &mut SqliteConnection,
) -> Result<Vec<ProjectRulePath>, diesel::result::Error> {
    use schema::project_rules::dsl::*;

    Ok(project_rules
        .load_iter::<ProjectRules, DefaultLoadingMode>(conn)?
        .filter_map(|item| match item {
            Ok(rule) => Some(ProjectRulePath {
                path: PathBuf::from(rule.path),
                project_root: PathBuf::from(rule.project_root),
            }),
            Err(_) => None,
        })
        .collect_vec())
}

fn upsert_project_rules(
    conn: &mut SqliteConnection,
    new_project_rules: Vec<ProjectRulePath>,
) -> Result<()> {
    use schema::project_rules::dsl::*;

    // SQLite doesn't support batch upserts, so we need to iterate
    for rule in new_project_rules {
        let new_rule = model::NewProjectRules {
            path: rule.path.to_string_lossy().to_string(),
            project_root: rule.project_root.to_string_lossy().to_string(),
        };

        diesel::insert_into(project_rules)
            .values(&new_rule)
            .on_conflict(path)
            .do_update()
            .set(&new_rule)
            .execute(conn)?;
    }

    Ok(())
}

fn delete_project_rules(conn: &mut SqliteConnection, rules_paths: Vec<PathBuf>) -> Result<()> {
    use schema::project_rules::dsl::*;

    // Convert PathBuf to String for comparison
    let path_strings: Vec<String> = rules_paths
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    diesel::delete(project_rules.filter(path.eq_any(path_strings))).execute(conn)?;

    Ok(())
}

fn get_all_ignored_suggestions(
    conn: &mut SqliteConnection,
) -> Result<Vec<(String, SuggestionType)>, diesel::result::Error> {
    use schema::ignored_suggestions::dsl::*;

    Ok(ignored_suggestions
        .select((suggestion, suggestion_type))
        .load::<(String, String)>(conn)?
        .into_iter()
        .filter_map(|(suggestion_text, suggestion_type_str)| {
            SuggestionType::from_str(&suggestion_type_str)
                .map(|parsed_suggestion_type| (suggestion_text, parsed_suggestion_type))
        })
        .collect())
}

fn get_all_mcp_server_installations(
    conn: &mut SqliteConnection,
) -> Result<HashMap<Uuid, TemplatableMCPServerInstallation>, diesel::result::Error> {
    use schema::mcp_server_installations::dsl::*;

    let rows: Vec<(String, String, String)> = mcp_server_installations
        .select((id, templatable_mcp_server, variable_values))
        .load::<(String, String, String)>(conn)?;
    let rows_len = rows.len();

    let result: HashMap<Uuid, TemplatableMCPServerInstallation> = rows
        .into_iter()
        .filter_map(|(id_str, templ_mcp, vars_json)| {
            let uuid = uuid::Uuid::parse_str(&id_str).ok()?;

            // Parse variable_values JSON into a flat HashMap<String, String>
            let vars: HashMap<String, VariableValue> =
                match serde_json::from_str::<HashMap<String, VariableValue>>(&vars_json) {
                    Ok(map) => map,
                    Err(_) => return None,
                };

            let mcp_server = match serde_json::from_str::<TemplatableMCPServer>(&templ_mcp) {
                Ok(map) => map,
                Err(_) => return None,
            };

            Some((
                uuid,
                TemplatableMCPServerInstallation::new(uuid, mcp_server, vars),
            ))
        })
        .collect();

    let improper_rows = rows_len - result.len();
    if improper_rows > 0 {
        log::warn!(
            "Skipping {improper_rows} rows from mcp_server_installations table due to malformation."
        );
    }

    Ok(result)
}

fn upsert_mcp_server_installation(
    conn: &mut SqliteConnection,
    mcp_server_installation: TemplatableMCPServerInstallation,
) -> Result<()> {
    use schema::mcp_server_installations::dsl::*;

    let new_installation = model::NewMCPServerInstallation {
        id: mcp_server_installation.uuid().to_string(),
        templatable_mcp_server: serde_json::to_string(
            mcp_server_installation.templatable_mcp_server(),
        )?,
        // TODO(pei): Change this to be the timestamp of the Cloud object
        template_version_ts: Utc::now().naive_utc(),
        variable_values: serde_json::to_string(mcp_server_installation.variable_values())?,
        restore_running: false,
        last_modified_at: Utc::now().naive_utc(),
    };

    conn.transaction::<_, Error, _>(|conn| {
        diesel::insert_into(mcp_server_installations)
            .values(&new_installation)
            .on_conflict(id)
            .do_update()
            .set(&new_installation)
            .execute(conn)?;

        Ok(())
    })?;

    Ok(())
}

fn delete_mcp_server_installations(conn: &mut SqliteConnection, uuids: Vec<Uuid>) -> Result<()> {
    use schema::mcp_server_installations::dsl::*;

    let id_strings: Vec<String> = uuids.iter().map(|uuid| uuid.to_string()).collect();
    diesel::delete(mcp_server_installations.filter(id.eq_any(id_strings))).execute(conn)?;

    Ok(())
}

fn delete_mcp_server_installations_by_template_uuid(
    conn: &mut SqliteConnection,
    target_template_uuid: Uuid,
) -> Result<()> {
    use schema::mcp_server_installations::dsl::*;

    diesel::delete(mcp_server_installations.filter(
        json_extract(templatable_mcp_server, "$.uuid").eq(target_template_uuid.to_string()),
    ))
    .execute(conn)?;

    Ok(())
}

fn get_mcp_servers_to_restore(
    conn: &mut SqliteConnection,
) -> Result<Vec<Uuid>, diesel::result::Error> {
    use schema::mcp_server_installations::dsl::*;

    let rows = mcp_server_installations
        .filter(restore_running.eq(true))
        .select(id)
        .load::<String>(conn)?;

    let installation_uuid = rows
        .iter()
        .filter_map(|uuid| uuid::Uuid::parse_str(uuid).ok())
        .collect();

    Ok(installation_uuid)
}

fn update_mcp_server_running(
    conn: &mut SqliteConnection,
    installation_uuid: Uuid,
    running: bool,
) -> Result<(), diesel::result::Error> {
    use schema::mcp_server_installations::dsl::*;

    diesel::update(mcp_server_installations.find(installation_uuid.to_string()))
        .set((
            restore_running.eq(running),
            last_modified_at.eq(Utc::now().naive_utc()),
        ))
        .execute(conn)?;

    Ok(())
}

fn add_ignored_suggestion(
    conn: &mut SqliteConnection,
    suggestion_text: String,
    suggestion_type_param: SuggestionType,
) -> Result<()> {
    use schema::ignored_suggestions::dsl::*;

    let new_suggestion = model::NewIgnoredSuggestion {
        suggestion: suggestion_text,
        suggestion_type: suggestion_type_param.as_str().to_string(),
    };

    diesel::insert_into(ignored_suggestions)
        .values(&new_suggestion)
        .on_conflict((suggestion, suggestion_type))
        .do_nothing()
        .execute(conn)?;

    Ok(())
}

fn remove_ignored_suggestion(
    conn: &mut SqliteConnection,
    suggestion_text: String,
    suggestion_type_param: SuggestionType,
) -> Result<()> {
    use schema::ignored_suggestions::dsl::*;

    diesel::delete(
        ignored_suggestions.filter(
            suggestion
                .eq(suggestion_text)
                .and(suggestion_type.eq(suggestion_type_param.as_str())),
        ),
    )
    .execute(conn)?;

    Ok(())
}

fn save_workspace(conn: &mut SqliteConnection, workspace: WorkspaceMetadata) -> Result<()> {
    // Set all existing workspaces as not selected
    diesel::update(workspaces)
        .set(is_selected.eq(false))
        .execute(conn)?;

    // Save new workspace and set it as current workspace
    use schema::workspaces::dsl::*;
    let new_workspace = NewWorkspace {
        name: workspace.name,
        server_uid: workspace.uid.into(),
        is_selected: true,
    };

    diesel::insert_into(workspaces)
        .values(&new_workspace)
        .on_conflict(schema::workspaces::dsl::server_uid)
        .do_update()
        // If there's already a workspace with this server_uid, then lets just update the other values
        .set(&new_workspace)
        .execute(conn)?;

    // Save teams for workspace
    for team in workspace.teams {
        use schema::teams::dsl::*;
        use schema::workspace_teams::dsl::*;
        let new_team = NewTeam {
            name: team.name,
            server_uid: team.uid.into(),
            billing_metadata_json: serde_json::to_string(&team.billing_metadata).ok(),
        };
        diesel::insert_into(teams)
            .values(&new_team)
            .on_conflict(server_uid)
            .do_update()
            // If there's already a team with this server_uid, then lets just update the other values
            .set(&new_team)
            .execute(conn)?;

        let team_db_id: i32 = schema::teams::dsl::teams
            .filter(schema::teams::dsl::server_uid.eq::<String>(team.uid.into()))
            .select(schema::teams::dsl::id)
            .first(conn)?;

        diesel::delete(
            schema::team_members::dsl::team_members
                .filter(schema::team_members::dsl::team_id.eq(team_db_id)),
        )
        .execute(conn)?;

        for member in &team.members {
            let new_member = model::NewTeamMember {
                team_id: team_db_id,
                user_uid: member.uid.as_string(),
                email: member.email.clone(),
                role: serde_json::to_string(&member.role).unwrap_or_default(),
            };
            diesel::insert_into(schema::team_members::dsl::team_members)
                .values(&new_member)
                .execute(conn)?;
        }

        let new_workspace_team = NewWorkspaceTeam {
            workspace_server_uid: workspace.uid.into(),
            team_server_uid: team.uid.into(),
        };
        diesel::insert_into(workspace_teams)
            .values(&new_workspace_team)
            .on_conflict((workspace_server_uid, team_server_uid))
            .do_update()
            .set(&new_workspace_team)
            .execute(conn)?;
    }

    Ok(())
}

fn save_workspaces(
    conn: &mut SqliteConnection,
    workspaces_to_insert: Vec<WorkspaceMetadata>,
) -> Result<()> {
    use schema::team_settings::dsl::*;
    use schema::teams::dsl::*;
    use schema::workspace_teams::dsl::*;
    use schema::workspaces::dsl::*;

    // Get currently selected workspace uid if there is one
    let current_workspace_uid: Option<WorkspaceUid> = workspaces
        .filter(is_selected.eq(true))
        .select(schema::workspaces::dsl::server_uid)
        .first::<String>(conn)
        .optional()?
        .map(|uid| uid.into());

    // Remove all team_members/team_settings/workspaces/teams/workspace_teams stored locally.
    diesel::delete(schema::team_members::dsl::team_members).execute(conn)?;
    diesel::delete(team_settings).execute(conn)?;
    diesel::delete(workspace_teams).execute(conn)?;
    diesel::delete(teams).execute(conn)?;
    diesel::delete(workspaces).execute(conn)?;

    // Insert workspaces returned by server (doing nothing on conflict), set is_selected
    // to true for the current_workspace_uid if it is in the list of workspaces.
    let new_workspace_values: Vec<NewWorkspace> = workspaces_to_insert
        .clone()
        .into_iter()
        .map(|workspace| NewWorkspace {
            server_uid: workspace.uid.into(),
            name: workspace.name,
            is_selected: current_workspace_uid
                .map(|current_uid| workspace.uid == current_uid)
                .unwrap_or(false),
        })
        .collect();
    diesel::insert_or_ignore_into(workspaces)
        .values(&new_workspace_values)
        .execute(conn)?;

    // Insert teams returned by server (doing nothing on conflict)
    let new_team_values: Vec<NewTeam> = workspaces_to_insert
        .clone()
        .into_iter()
        .flat_map(|workspace| {
            workspace
                .teams
                .into_iter()
                .map(|team| NewTeam {
                    server_uid: team.uid.into(),
                    name: team.name.clone(),
                    billing_metadata_json: serde_json::to_string(&team.billing_metadata).ok(),
                })
                .collect::<Vec<NewTeam>>()
        })
        .collect();
    diesel::insert_or_ignore_into(teams)
        .values(&new_team_values)
        .execute(conn)?;

    // We cannot directly return the id from the insert so perform
    // a second query for the id https://github.com/diesel-rs/diesel/issues/771.
    let teams_with_id: Vec<(i32, String)> = schema::teams::dsl::teams
        .select((schema::teams::dsl::id, schema::teams::dsl::server_uid))
        .load(conn)?;
    let teams_by_server_uid: HashMap<&String, i32> = HashMap::from_iter(
        teams_with_id
            .iter()
            .map(|(table_id, table_server_uid)| (table_server_uid, *table_id)),
    );

    // Insert workspace_teams returned by server (doing nothing on conflict)
    let workspace_teams_values: Vec<NewWorkspaceTeam> = workspaces_to_insert
        .clone()
        .into_iter()
        .flat_map(|workspace| {
            workspace
                .teams
                .into_iter()
                .map(|team| NewWorkspaceTeam {
                    workspace_server_uid: workspace.uid.into(),
                    team_server_uid: team.uid.into(),
                })
                .collect::<Vec<NewWorkspaceTeam>>()
        })
        .collect();
    diesel::insert_or_ignore_into(workspace_teams)
        .values(&workspace_teams_values)
        .execute(conn)?;

    // Cache workspace settings returned by the server (overwriting any existing settings)
    let team_settings_values: Vec<NewTeamSettings> = workspaces_to_insert
        .clone()
        .into_iter()
        .flat_map(|workspace| {
            workspace.teams.into_iter().filter_map(|team| {
                let serialized_settings_json =
                    serde_json::to_string(&team.organization_settings).ok()?;
                let team_id_match = teams_by_server_uid.get(&team.uid.uid())?;
                Some(NewTeamSettings {
                    team_id: *team_id_match,
                    settings_json: serialized_settings_json,
                })
            })
        })
        .collect();
    diesel::insert_into(schema::team_settings::dsl::team_settings)
        .values(&team_settings_values)
        .execute(conn)?;

    // Cache team members
    let team_member_values: Vec<model::NewTeamMember> = workspaces_to_insert
        .clone()
        .into_iter()
        .flat_map(|workspace| {
            workspace.teams.into_iter().flat_map(|team| {
                let team_id_match = teams_by_server_uid.get(&team.uid.uid()).copied();
                team.members.into_iter().filter_map(move |member| {
                    Some(model::NewTeamMember {
                        team_id: team_id_match?,
                        user_uid: member.uid.as_string(),
                        email: member.email,
                        role: serde_json::to_string(&member.role).unwrap_or_default(),
                    })
                })
            })
        })
        .collect();
    if !team_member_values.is_empty() {
        diesel::insert_into(schema::team_members::dsl::team_members)
            .values(&team_member_values)
            .execute(conn)?;
    }

    if let Some(current_workspace_uid) = current_workspace_uid {
        if !workspaces_to_insert
            .iter()
            .any(|workspace| workspace.uid == current_workspace_uid)
        {
            // If the currently selected workspace is not in the list of workspaces, set
            // the first workspace as the current workspace.
            if let Some(first_workspace) = workspaces_to_insert.first() {
                diesel::update(workspaces.filter(
                    schema::workspaces::dsl::server_uid.eq::<String>(first_workspace.uid.into()),
                ))
                .set(is_selected.eq(true))
                .execute(conn)?;
            }
        }
    }

    Ok(())
}

fn set_current_workspace(conn: &mut SqliteConnection, workspace_uid: WorkspaceUid) -> Result<()> {
    use schema::workspaces::dsl::*;

    // Set all existing workspaces as not selected
    diesel::update(workspaces)
        .set(is_selected.eq(false))
        .execute(conn)?;

    diesel::update(
        workspaces.filter(schema::workspaces::dsl::server_uid.eq::<String>(workspace_uid.into())),
    )
    .set(is_selected.eq(true))
    .execute(conn)?;

    Ok(())
}

/// Mark a shareable object as no longer having pending changes.
fn mark_object_as_synced(
    conn: &mut SqliteConnection,
    hashed_sqlite_id: String,
    new_revision_and_editor: RevisionAndLastEditor,
    new_metadata_ts: Option<ServerTimestamp>,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(server_id.eq(Some(hashed_sqlite_id.as_str()))))
            .set(is_pending.eq(false))
            .execute(conn)?;
        diesel::update(object_metadata.filter(server_id.eq(Some(hashed_sqlite_id.clone()))))
            .set((
                revision_ts.eq(new_revision_and_editor.revision.timestamp_micros()),
                last_editor_uid.eq(new_revision_and_editor.last_editor_uid),
            ))
            .execute(conn)?;

        if let Some(metadata_ts) = new_metadata_ts {
            diesel::update(object_metadata.filter(server_id.eq(Some(hashed_sqlite_id))))
                .set((metadata_last_updated_ts.eq(metadata_ts.timestamp_micros()),))
                .execute(conn)?;
        }
        Ok(())
    })
}

fn increment_retry_count(
    conn: &mut SqliteConnection,
    server_id_string: String,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(server_id.eq(Some(server_id_string))))
            .set(retry_count.eq(retry_count + 1))
            .execute(conn)?;
        Ok(())
    })
}

fn update_object_after_server_creation(
    conn: &mut SqliteConnection,
    client_id_string: String,
    server_creation_info: ServerCreationInfo,
) -> Result<(), Error> {
    use schema::commands::dsl::*;
    use schema::object_metadata::dsl::*;

    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(client_id.eq(Some(client_id_string.clone()))))
            .set((
                server_id.eq(Some(
                    server_creation_info
                        .server_id_and_type
                        .sqlite_type_and_uid_hash(),
                )),
                creator_uid.eq(server_creation_info.creator_uid),
            ))
            .execute(conn)?;

        diesel::update(commands.filter(cloud_workflow_id.eq(Some(client_id_string))))
            .set(
                cloud_workflow_id.eq(Some(
                    server_creation_info
                        .server_id_and_type
                        .sqlite_type_and_uid_hash(),
                )),
            )
            .execute(conn)?;

        Ok(())
    })
}

/// Helper function to delete a cloud object identified by `sync_id`. If a valid object metadata row
/// for the object is found, `delete_object_fn` is called to delete the actual object.
fn delete_cloud_object(
    conn: &mut SqliteConnection,
    sync_id: SyncId,
    object_id_type: ObjectIdType,
    delete_object_fn: DeleteCloudObjectFn,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;

    // Filter to find metadata row.
    // The diesel types for `filter`s are dependent on the columns being filtered
    // so while the `hashed_sync_id` will only match one of `client_id` and `server_id`,
    // we filter on both here for ergonomics.
    let hashed_sync_id = sync_id.sqlite_uid_hash(object_id_type);
    let metadata_filter = object_metadata
        .filter(client_id.eq(Some(hashed_sync_id.as_str())))
        .or_filter(server_id.eq(Some(hashed_sync_id.as_str())));

    let metadata: ObjectMetadata = metadata_filter.first(conn)?;
    let object_id = metadata.shareable_object_id;
    diesel::delete(object_metadata.filter(id.eq(metadata.id))).execute(conn)?;
    diesel::delete(
        schema::object_permissions::dsl::object_permissions
            .filter(schema::object_permissions::object_metadata_id.eq(metadata.id)),
    )
    .execute(conn)?;
    diesel::delete(
        schema::object_actions::dsl::object_actions
            .filter(schema::object_actions::hashed_object_id.eq(hashed_sync_id)),
    )
    .execute(conn)?;
    delete_object_fn(conn, object_id)?;
    Ok(())
}

/// SQLite endpoint for the ObjectMetadataUpdated RTC message that updates the metadata ts and other
/// metadata like current team_id of the object.
fn update_object_metadata(
    conn: &mut SqliteConnection,
    hashed_id: String,
    metadata: CloudObjectMetadata,
) -> Result<(), Error> {
    use schema::object_metadata::dsl::*;
    let metadata_last_updated_at = metadata
        .metadata_last_updated_ts
        .map(|ts| ts.timestamp_micros());

    let trashed_timestamp = metadata.trashed_ts.map(|ts| ts.timestamp_micros());
    let folder_id_str = metadata
        .folder_id
        .map(|folder_sync_id| folder_sync_id.sqlite_uid_hash(ObjectIdType::Folder));

    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(object_metadata.filter(server_id.eq(Some(hashed_id.as_str()))))
            .set((
                metadata_last_updated_ts.eq(metadata_last_updated_at),
                trashed_ts.eq(trashed_timestamp),
                folder_id.eq(folder_id_str),
                current_editor.eq(metadata.current_editor_uid),
            ))
            .execute(conn)?;

        Ok(())
    })
}

fn upsert_generic_string_objects(
    conn: &mut SqliteConnection,
    cloud_generic_string_objects: Vec<Box<dyn CloudStringObject>>,
) -> Result<(), Error> {
    use schema::generic_string_objects::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for object in cloud_generic_string_objects {
            let serialized_data = Arc::new(object.serialized().take());
            let serialized_data_clone = serialized_data.clone();
            upsert_cloud_object(
                conn,
                ObjectType::GenericStringObject(object.generic_string_object_format()),
                object.id(),
                object.metadata().clone(),
                object.permissions().clone(),
                Box::new(move |conn| {
                    let new_object = NewGenericStringObject {
                        data: serialized_data.as_ref(),
                    };
                    diesel::insert_into(
                        schema::generic_string_objects::dsl::generic_string_objects,
                    )
                    .values(new_object)
                    .execute(conn)?;
                    let object_id: i32 =
                        schema::generic_string_objects::dsl::generic_string_objects
                            .select(schema::generic_string_objects::columns::id)
                            .order(schema::generic_string_objects::columns::id.desc())
                            .first(conn)?;
                    Ok(object_id)
                }),
                Box::new(move |conn, object_id| {
                    diesel::update(
                        generic_string_objects
                            .filter(schema::generic_string_objects::dsl::id.eq(object_id)),
                    )
                    .set((data.eq(serialized_data_clone.as_ref()),))
                    .execute(conn)?;
                    Ok(())
                }),
            )?
        }
        Ok(())
    })
}

fn upsert_workflows(
    conn: &mut SqliteConnection,
    cloud_workflows: Vec<CloudWorkflow>,
) -> Result<(), Error> {
    use schema::workflows::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        // todo: wrap in an arc to avoid unnecessary cloning.
        for cloud_workflow in cloud_workflows {
            let workflow_id = cloud_workflow.id;
            if let Ok(serialized_workflow) = serde_json::to_string(&cloud_workflow.model().data) {
                let serialized_workflow_clone = serialized_workflow.clone();
                upsert_cloud_object(
                    conn,
                    ObjectType::Workflow,
                    workflow_id,
                    cloud_workflow.metadata,
                    cloud_workflow.permissions,
                    Box::new(move |conn| {
                        let workflow = model::NewWorkflow {
                            data: serialized_workflow.clone(),
                        };
                        diesel::insert_into(schema::workflows::dsl::workflows)
                            .values(workflow)
                            .execute(conn)?;
                        let workflow_id: i32 = schema::workflows::dsl::workflows
                            .select(schema::workflows::columns::id)
                            .order(schema::workflows::columns::id.desc())
                            .first(conn)?;
                        Ok(workflow_id)
                    }),
                    Box::new(move |conn, workflow_id| {
                        diesel::update(
                            workflows.filter(schema::workflows::dsl::id.eq(workflow_id)),
                        )
                        .set((data.eq(serialized_workflow_clone),))
                        .execute(conn)?;
                        Ok(())
                    }),
                )?
            }
        }
        Ok(())
    })
}

fn upsert_notebooks(
    conn: &mut SqliteConnection,
    cloud_notebooks: Vec<CloudNotebook>,
) -> Result<(), Error> {
    use schema::notebooks::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for cloud_notebook in cloud_notebooks {
            // todo: wrap in an arc to avoid unnecessary cloning.
            let notebook_clone = cloud_notebook.clone();
            let title_clone = cloud_notebook.model().title.clone();
            let data_clone = cloud_notebook.model().data.clone();
            let ai_document_id_clone = cloud_notebook
                .model()
                .ai_document_id
                .as_ref()
                .map(|doc_id| doc_id.to_string());
            upsert_cloud_object(
                conn,
                ObjectType::Notebook,
                cloud_notebook.id,
                cloud_notebook.metadata,
                cloud_notebook.permissions,
                Box::new(move |conn| {
                    let new_notebook = NewNotebook {
                        title: Some(title_clone),
                        data: Some(data_clone),
                        ai_document_id: ai_document_id_clone,
                    };
                    diesel::insert_into(schema::notebooks::dsl::notebooks)
                        .values(new_notebook)
                        .execute(conn)?;
                    let notebook_id: i32 = schema::notebooks::dsl::notebooks
                        .select(schema::notebooks::columns::id)
                        .order(schema::notebooks::columns::id.desc())
                        .first(conn)?;
                    Ok(notebook_id)
                }),
                Box::new(move |conn, notebook_id| {
                    diesel::update(notebooks.filter(schema::notebooks::dsl::id.eq(notebook_id)))
                        .set((
                            title.eq(notebook_clone.model().title.clone()),
                            data.eq(notebook_clone.model().data.clone()),
                            ai_document_id.eq(notebook_clone
                                .model()
                                .ai_document_id
                                .as_ref()
                                .map(|doc_id| doc_id.to_string())),
                        ))
                        .execute(conn)?;
                    Ok(())
                }),
            )?
        }
        Ok(())
    })
}

fn upsert_folders(
    conn: &mut SqliteConnection,
    cloud_folders: Vec<CloudFolder>,
) -> Result<(), Error> {
    use schema::folders::dsl::*;
    conn.transaction::<(), Error, _>(|conn| {
        for cloud_folder in cloud_folders {
            let folder_clone = cloud_folder.clone();
            let folder_name = cloud_folder.model().name.clone();
            let folder_is_open = cloud_folder.model().is_open;
            let folder_is_warp_pack = cloud_folder.model().is_warp_pack;
            upsert_cloud_object(
                conn,
                ObjectType::Folder,
                cloud_folder.id,
                cloud_folder.metadata,
                cloud_folder.permissions,
                Box::new(move |conn| {
                    let new_folder = NewFolder {
                        name: folder_name,
                        is_open: folder_is_open,
                        is_warp_pack: folder_is_warp_pack,
                    };
                    diesel::insert_into(schema::folders::dsl::folders)
                        .values(new_folder)
                        .execute(conn)?;
                    let folder_id: i32 = schema::folders::dsl::folders
                        .select(schema::folders::columns::id)
                        .order(schema::folders::columns::id.desc())
                        .first(conn)?;
                    Ok(folder_id)
                }),
                Box::new(move |conn, folder_id| {
                    diesel::update(folders.filter(schema::folders::dsl::id.eq(folder_id)))
                        .set((
                            name.eq(folder_clone.model().name.clone()),
                            is_open.eq(folder_clone.model().is_open),
                            is_warp_pack.eq(folder_clone.model().is_warp_pack),
                        ))
                        .execute(conn)?;
                    Ok(())
                }),
            )?
        }
        Ok(())
    })
}

/// Parse conversation IDs from JSON string.
fn parse_conversation_ids(ids_json: &Option<String>) -> Vec<AIConversationId> {
    let Some(ids_str) = ids_json.as_ref() else {
        return vec![];
    };

    let Ok(id_strings) = serde_json::from_str::<Vec<String>>(ids_str) else {
        log::warn!("Failed to deserialize conversation IDs from column");
        return vec![];
    };

    id_strings
        .into_iter()
        .map(AIConversationId::try_from)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|_| {
            log::warn!("Failed to parse conversation IDs");
            vec![]
        })
}

fn read_root_node(conn: &mut SqliteConnection, tab_id_val: i32) -> Result<PaneNodeSnapshot> {
    use schema::pane_nodes::dsl::*;

    let pane_node: model::PaneNode = schema::pane_nodes::dsl::pane_nodes
        .filter(tab_id.eq(tab_id_val))
        .filter(parent_pane_node_id.is_null())
        .first(conn)?;
    read_node(conn, pane_node)
}

/// Reads a saved node back into a snapshot.
fn read_node(conn: &mut SqliteConnection, node: model::PaneNode) -> Result<PaneNodeSnapshot> {
    match node.is_leaf {
        true => {
            let pane = schema::pane_leaves::dsl::pane_leaves
                .filter(schema::pane_leaves::columns::pane_node_id.eq(node.id))
                .first::<model::PaneLeaf>(conn)?;

            let contents = match pane.kind.as_ref() {
                TERMINAL_PANE_KIND => {
                    let terminal_pane = schema::terminal_panes::dsl::terminal_panes
                        .find(node.id)
                        .select(model::TerminalPane::as_select())
                        .first(conn)?;

                    let shell_launch_data: Option<ShellLaunchData> = terminal_pane
                        .shell_launch_data
                        .and_then(|shell_str| serde_json::from_str(&shell_str).ok());
                    let input_config = terminal_pane
                        .input_config
                        .and_then(|config_str| serde_json::from_str(&config_str).ok());
                    let active_profile_id = terminal_pane
                        .active_profile_id
                        .and_then(|profile_str| serde_json::from_str(&profile_str).ok());
                    // Don't provide a fallback here - let the higher-level code with AppContext handle it

                    let conversation_ids_to_restore =
                        parse_conversation_ids(&terminal_pane.conversation_ids);

                    let active_conversation_id = terminal_pane
                        .active_conversation_id
                        .and_then(|id_str| AIConversationId::try_from(id_str).ok());

                    LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: terminal_pane.uuid,
                        cwd: terminal_pane.cwd,
                        is_active: terminal_pane.is_active,
                        is_read_only: false,
                        shell_launch_data,
                        input_config,
                        llm_model_override: terminal_pane.llm_model_override,
                        active_profile_id,
                        conversation_ids_to_restore,
                        active_conversation_id,
                    })
                }
                NOTEBOOK_PANE_KIND => {
                    let notebook_pane = schema::notebook_panes::dsl::notebook_panes
                        .find(node.id)
                        .select(model::NotebookPane::as_select())
                        .first(conn)?;

                    let notebook_id = notebook_pane.notebook_id.and_then(|id| {
                        ClientId::from_hash(&id).map(SyncId::ClientId).or_else(|| {
                            NotebookId::from_hash(&id).map(|id| SyncId::ServerId(id.into()))
                        })
                    });

                    let local_path = notebook_pane.local_path.map(decode_path);

                    // In the database schema, both the `notebook_id` and `local_path` are
                    // nullable. It's possible for either a file pane or a notebook pane to be open
                    // to an uneditable notebook. In that case, bias towards cloud notebooks. If
                    // both are null, it's more likely that the pane was a new, empty cloud
                    // notebook than an unreadable local file.
                    LeafContents::Notebook(match local_path {
                        Some(path) => NotebookPaneSnapshot::LocalFileNotebook { path: Some(path) },
                        None => NotebookPaneSnapshot::CloudNotebook {
                            notebook_id,
                            settings: OpenWarpDriveObjectSettings::default(),
                        },
                    })
                }
                WORKFLOW_PANE_KIND => {
                    let workflow_pane = schema::workflow_panes::dsl::workflow_panes
                        .find(node.id)
                        .select(model::WorkflowPane::as_select())
                        .first(conn)?;

                    let workflow_id = workflow_pane.workflow_id.and_then(|id| {
                        ClientId::from_hash(&id).map(SyncId::ClientId).or_else(|| {
                            WorkflowId::from_hash(&id).map(|id| SyncId::ServerId(id.into()))
                        })
                    });

                    LeafContents::Workflow(WorkflowPaneSnapshot::CloudWorkflow {
                        workflow_id,
                        settings: OpenWarpDriveObjectSettings::default(),
                    })
                }
                CODE_PANE_KIND => {
                    let code_pane = schema::code_panes::dsl::code_panes
                        .find(node.id)
                        .select(model::CodePane::as_select())
                        .first(conn)?;

                    // Read child code_pane_tabs rows ordered by tab_index.
                    let tab_rows: Vec<model::CodePaneTab> =
                        schema::code_pane_tabs::dsl::code_pane_tabs
                            .filter(schema::code_pane_tabs::columns::code_pane_id.eq(code_pane.id))
                            .order(schema::code_pane_tabs::columns::tab_index.asc())
                            .select(model::CodePaneTab::as_select())
                            .load(conn)?;

                    let tabs: Vec<CodePaneTabSnapshot> = tab_rows
                        .into_iter()
                        .map(|row| CodePaneTabSnapshot {
                            path: row.local_path.map(decode_path),
                        })
                        .collect();
                    let active_tab_index = code_pane.active_tab_index as usize;

                    let source = code_pane
                        .source_data
                        .as_deref()
                        .and_then(|data| serde_json::from_str::<CodeSource>(data).ok());

                    LeafContents::Code(CodePaneSnapShot::Local {
                        tabs,
                        active_tab_index,
                        source,
                    })
                }
                ENV_VAR_COLLECTION_PANE_KIND => {
                    let env_var_collection_pane =
                        schema::env_var_collection_panes::dsl::env_var_collection_panes
                            .find(node.id)
                            .select(model::EnvVarCollectionPane::as_select())
                            .first(conn)?;

                    let env_var_collection_id = env_var_collection_pane
                        .env_var_collection_id
                        .and_then(|id| {
                            ClientId::from_hash(&id).map(SyncId::ClientId).or_else(|| {
                                GenericStringObjectId::from_hash(&id)
                                    .map(|id| SyncId::ServerId(id.into()))
                            })
                        });

                    LeafContents::EnvVarCollection(
                        EnvVarCollectionPaneSnapshot::CloudEnvVarCollection {
                            env_var_collection_id,
                        },
                    )
                }
                SETTINGS_PANE_KIND => {
                    let settings_pane = schema::settings_panes::dsl::settings_panes
                        .find(node.id)
                        .select(model::SettingsPane::as_select())
                        .first(conn)?;

                    let current_page = SettingsSection::from_str(&settings_pane.current_page)
                        .ok()
                        .unwrap_or_default();
                    LeafContents::Settings(SettingsPaneSnapshot::Local {
                        current_page,
                        search_query: None,
                    })
                }
                AI_FACT_PANE_KIND => LeafContents::AIFact(AIFactPaneSnapshot::Personal),
                MCP_SERVER_PANE_KIND => {
                    // Legacy MCP server panes are no longer supported.
                    bail!("Legacy MCP server panes are no longer supported")
                }
                CODE_REVIEW_PANE_KIND => {
                    let code_review_pane = schema::code_review_panes::dsl::code_review_panes
                        .find(node.id)
                        .select(model::CodeReviewPane::as_select())
                        .first(conn)
                        .ok();

                    match code_review_pane {
                        Some(pane) => LeafContents::CodeReview(CodeReviewPaneSnapshot::Local {
                            terminal_uuid: pane.terminal_uuid,
                            repo_path: PathBuf::from(pane.repo_path),
                        }),
                        None => {
                            // Return empty fields; will be skipped during restoration
                            LeafContents::CodeReview(CodeReviewPaneSnapshot::Local {
                                terminal_uuid: Vec::new(),
                                repo_path: PathBuf::from(""),
                            })
                        }
                    }
                }
                GET_STARTED_PANE_KIND => LeafContents::GetStarted,
                WELCOME_PANE_KIND => {
                    let welcome_pane = schema::welcome_panes::dsl::welcome_panes
                        .find(node.id)
                        .select(model::WelcomePane::as_select())
                        .first(conn)?;
                    LeafContents::Welcome {
                        startup_directory: welcome_pane.startup_directory.map(PathBuf::from),
                    }
                }
                AI_DOCUMENT_PANE_KIND => {
                    let ai_document_pane = schema::ai_document_panes::dsl::ai_document_panes
                        .find(node.id)
                        .select(model::AIDocumentPane::as_select())
                        .first(conn)?;

                    LeafContents::AIDocument(crate::app_state::AIDocumentPaneSnapshot::Local {
                        document_id: ai_document_pane.document_id,
                        version: ai_document_pane.version,
                        content: ai_document_pane.content,
                        title: ai_document_pane.title,
                    })
                }
                AMBIENT_AGENT_PANE_KIND => {
                    let pane = schema::ambient_agent_panes::dsl::ambient_agent_panes
                        .find(node.id)
                        .select(model::AmbientAgentPane::as_select())
                        .first(conn)?;

                    let task_id = pane
                        .task_id
                        .and_then(|id_str| id_str.parse::<AmbientAgentTaskId>().ok());

                    LeafContents::AmbientAgent(AmbientAgentPaneSnapshot {
                        uuid: pane.uuid,
                        task_id,
                    })
                }
                other => bail!("Unrecognized pane kind: {other}"),
            };

            Ok(PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: pane.is_focused,
                custom_vertical_tabs_title: pane.custom_vertical_tabs_title,
                contents,
            }))
        }
        false => {
            let pane_branch = schema::pane_branches::dsl::pane_branches
                .filter(schema::pane_branches::columns::pane_node_id.eq(node.id))
                .first::<model::PaneBranch>(conn)?;

            let child_nodes = schema::pane_nodes::dsl::pane_nodes
                .filter(schema::pane_nodes::columns::parent_pane_node_id.eq(node.id))
                .order(schema::pane_nodes::columns::id.asc())
                .load::<model::PaneNode>(conn)?;

            let mut children = Vec::new();
            for child_node in child_nodes {
                children.push((
                    PaneFlex(child_node.flex.unwrap_or(1.)),
                    read_node(conn, child_node)?,
                ));
            }

            let direction = match pane_branch.horizontal {
                true => SplitDirection::Horizontal,
                false => SplitDirection::Vertical,
            };
            Ok(PaneNodeSnapshot::Branch(BranchSnapshot {
                direction,
                children,
            }))
        }
    }
}

/// This is not in a transaction. The interface for a transaction is a bit awkward,
/// and makes it invalid to write the logic recursively. It's ok it's not in a
/// transaction because we should be the only connection using the database.
///
/// One notable exception is the case where there may be two warp apps running
/// in the same bundle. In this case, we may read some garbage, but all that will
/// happen is the user won't have session restoration.
///
/// In the future, the awkwardness of the transaction interface is resolved in diesel 2.0.0.
fn read_sqlite_data(
    conn: &mut SqliteConnection,
    current_user_id: Option<UserUid>,
) -> Result<PersistedData, Error> {
    use schema::windows::dsl::*;

    let active_window_id = schema::app::dsl::app
        .select(schema::app::dsl::active_window_id)
        .first::<Option<i32>>(conn)
        .optional()?
        .flatten();
    let db_windows = windows.load::<Window>(conn)?;

    let mut active_window_index: Option<usize> = None;

    let db_tabs = Tab::belonging_to(&db_windows)
        .order_by(schema::tabs::columns::id.asc())
        .load::<Tab>(conn)?
        .grouped_by(&db_windows);

    let db_panels = schema::panels::dsl::panels
        .load::<model::Panel>(conn)?
        .into_iter()
        .map(|p| (p.tab_id, p))
        .collect::<HashMap<_, _>>();

    let saved_windows: Vec<_> = db_windows
        .into_iter()
        .enumerate()
        .zip(db_tabs)
        .map(|((idx, window), tabs_for_window)| {
            let saved_tabs: Vec<_> = tabs_for_window
                .into_iter()
                .filter_map(|tab| {
                    let root = read_root_node(conn, tab.id).ok()?;
                    let panel = db_panels.get(&tab.id);

                    let left_panel = panel
                        .and_then(|p| p.left_panel.as_ref())
                        .and_then(|s| serde_json::from_str::<LeftPanelSnapshot>(s).ok());

                    let right_panel = panel
                        .and_then(|p| p.right_panel.as_ref())
                        .and_then(|s| serde_json::from_str::<RightPanelSnapshot>(s).ok());

                    Some(TabSnapshot {
                        root,
                        custom_title: tab.custom_title,
                        default_directory_color: None,
                        selected_color: tab
                            .color
                            .as_deref()
                            .and_then(|s| {
                                serde_yaml::from_str::<SelectedTabColor>(s)
                                    .ok()
                                    .or_else(|| {
                                        // Fall back to the old format which stored a bare AnsiColorIdentifier
                                        serde_yaml::from_str::<AnsiColorIdentifier>(s)
                                            .ok()
                                            .map(SelectedTabColor::Color)
                                    })
                            })
                            .unwrap_or_default(),
                        left_panel,
                        right_panel,
                    })
                })
                .collect();

            if active_window_id
                .map(|window_id| window.id == window_id)
                .unwrap_or(false)
            {
                active_window_index = Some(idx);
            }

            // Default active tab index to 0 if we overflow when converting.
            let tab_index: usize = window.active_tab_index.try_into().unwrap_or(0);

            let fullscreen_state_val =
                FullscreenState::from_i32(window.fullscreen_state).unwrap_or_default();

            // The origin and size of the bound should be all null or all non-null.
            let bounds = match (
                window.window_width,
                window.window_height,
                window.origin_x,
                window.origin_y,
            ) {
                (Some(mut width), Some(mut height), Some(x), Some(y)) => {
                    // When fullscreen or maximized, the `inner_size` we snapshotted will be the
                    // size of the full screen. This will cause problems with winit. When you set
                    // maximized/fullscreen, setting the inner_size will by the size the window
                    // takes _after_ the user toggles _out_ of fullscreen/maximized. Therefore, we
                    // don't want to set the size to take the full screen because the window will
                    // appear to remain in maximized/fullscreen. We multiply each dimension by 0.8
                    // to prevent taking the full screen while choosing a reasonable size.
                    if !cfg!(target_os = "macos") && fullscreen_state_val != FullscreenState::Normal
                    {
                        width *= 0.8;
                        height *= 0.8;
                    }
                    Some(RectF::new(
                        Vector2F::new(x, y),
                        Vector2F::new(width, height),
                    ))
                }
                _ => None,
            };

            let left_panel_width: Option<f32> = saved_tabs.get(tab_index).and_then(|tab| match tab
                .left_panel
                .as_ref()
            {
                Some(LeftPanelSnapshot { width, .. }) => Some(*width as f32),
                _ => None,
            });

            let right_panel_width: Option<f32> =
                saved_tabs
                    .get(tab_index)
                    .and_then(|tab| match tab.right_panel.as_ref() {
                        Some(RightPanelSnapshot { width, .. }) => Some(*width as f32),
                        _ => None,
                    });

            let window_left_panel_open = window.left_panel_open.unwrap_or_else(|| {
                saved_tabs
                    .get(tab_index)
                    .and_then(|tab| tab.left_panel.as_ref())
                    .is_some()
            });

            WindowSnapshot {
                tabs: saved_tabs,
                active_tab_index: tab_index,
                quake_mode: window.quake_mode,
                bounds,
                universal_search_width: window.universal_search_width,
                warp_ai_width: window.warp_ai_width,
                voltron_width: window.voltron_width,
                warp_drive_index_width: window.warp_drive_index_width,
                left_panel_open: window_left_panel_open,
                vertical_tabs_panel_open: window.vertical_tabs_panel_open.unwrap_or(false),
                fullscreen_state: fullscreen_state_val,
                left_panel_width,
                right_panel_width,
                agent_management_filters: window
                    .agent_management_filters
                    .and_then(|s| serde_json::from_str(&s).ok()),
            }
        })
        .collect();

    let object_metadata =
        schema::object_metadata::dsl::object_metadata.load::<model::ObjectMetadata>(conn)?;
    let object_permissions = schema::object_permissions::dsl::object_permissions
        .load::<model::ObjectPermissions>(conn)?;

    // Cache metadata and permissions by id so that we aren't doing an n^2 lookups for each object type.
    let metadata_by_id = object_metadata
        .into_iter()
        .map(|metadata| {
            let object_type = if metadata
                .object_type
                .starts_with(GENERIC_STRING_OBJECT_PREFIX)
            {
                GENERIC_STRING_OBJECT_PREFIX.to_owned()
            } else {
                metadata.object_type.to_owned()
            };
            // Shareable object ids aren't unique across object types, so the object type needs to be
            // part of the hashmap key.  For generic objects, they are all in the same table,
            // so it's safe to use the generic prefix as part of the key.
            ((metadata.shareable_object_id, object_type), metadata)
        })
        .collect::<HashMap<_, _>>();
    let permissions_by_id = object_permissions
        .into_iter()
        .map(|permissions| (permissions.object_metadata_id, permissions))
        .collect::<HashMap<_, _>>();

    let mut cloud_objects: Vec<Box<dyn CloudObject>> = Vec::new();
    cloud_objects.extend(
        schema::workflows::dsl::workflows
            .load::<model::Workflow>(conn)?
            .iter()
            .filter_map(|workflow| {
                metadata_by_id
                    .get(&(
                        workflow.id,
                        ObjectType::Workflow.sqlite_object_type_as_str().to_string(),
                    ))
                    .and_then(|metadata| {
                        let workflow_content = serde_json::from_str(workflow.data.as_str()).ok();
                        let workflow_id = id_from_metadata::<WorkflowId>(metadata);
                        let permissions = permissions_by_id.get(&metadata.id)?;
                        let cloud_object_permissions =
                            to_cloud_object_permissions(permissions, current_user_id)?;
                        workflow_content
                            .zip(workflow_id)
                            .map(|(content, workflow_id)| {
                                let boxed: Box<dyn CloudObject> = Box::new(CloudWorkflow::new(
                                    workflow_id,
                                    CloudWorkflowModel::new(content),
                                    to_cloud_object_metadata(metadata),
                                    cloud_object_permissions,
                                ));
                                boxed
                            })
                    })
            })
            .collect::<Vec<_>>(),
    );

    cloud_objects.extend(
        schema::notebooks::dsl::notebooks
            .load::<model::Notebook>(conn)?
            .iter()
            .filter_map(|notebook| {
                metadata_by_id
                    .get(&(
                        notebook.id,
                        ObjectType::Notebook.sqlite_object_type_as_str().to_string(),
                    ))
                    .and_then(|metadata| {
                        let notebook_id = id_from_metadata::<NotebookId>(metadata);
                        let permissions = permissions_by_id.get(&metadata.id)?;
                        let cloud_object_permissions =
                            to_cloud_object_permissions(permissions, current_user_id)?;
                        notebook_id.map(|server_id| {
                            let ai_document_id =
                                notebook.ai_document_id.as_ref().and_then(|doc_id_str| {
                                    AIDocumentId::try_from(doc_id_str.as_str()).ok()
                                });
                            let boxed: Box<dyn CloudObject> = Box::new(CloudNotebook::new(
                                server_id,
                                CloudNotebookModel {
                                    title: notebook.title.clone().unwrap_or_default(),
                                    data: notebook.data.clone().unwrap_or_default(),
                                    ai_document_id,
                                    conversation_id: None,
                                },
                                to_cloud_object_metadata(metadata),
                                cloud_object_permissions,
                            ));
                            boxed
                        })
                    })
            })
            .collect::<Vec<_>>(),
    );

    cloud_objects.extend(
        schema::folders::dsl::folders
            .load::<model::Folder>(conn)?
            .iter()
            .filter_map(|folder| {
                metadata_by_id
                    .get(&(
                        folder.id,
                        ObjectType::Folder.sqlite_object_type_as_str().to_string(),
                    ))
                    .and_then(|metadata| {
                        let folder_id = id_from_metadata::<FolderId>(metadata);
                        let permissions = permissions_by_id.get(&metadata.id)?;
                        let cloud_object_permissions =
                            to_cloud_object_permissions(permissions, current_user_id)?;
                        folder_id.map(|server_id| {
                            let boxed: Box<dyn CloudObject> = Box::new(CloudFolder::new(
                                server_id,
                                CloudFolderModel {
                                    name: folder.name.clone(),
                                    is_open: folder.is_open,
                                    is_warp_pack: folder.is_warp_pack,
                                },
                                to_cloud_object_metadata(metadata),
                                cloud_object_permissions,
                            ));
                            boxed
                        })
                    })
            })
            .collect::<Vec<_>>(),
    );

    cloud_objects.extend(
        schema::generic_string_objects::dsl::generic_string_objects
            .load::<model::GenericStringObject>(conn)?
            .iter()
            .filter_map(|object| {
                metadata_by_id
                    .get(&(object.id, GENERIC_STRING_OBJECT_PREFIX.to_owned()))
                    .and_then(|metadata| {
                        let object_id = id_from_metadata::<GenericStringObjectId>(metadata);
                        let permissions = permissions_by_id.get(&metadata.id)?;
                        let cloud_object_permissions =
                            to_cloud_object_permissions(permissions, current_user_id)?;
                        let json_object_type: JsonObjectType = metadata
                            .object_type
                            .strip_prefix(&format!(
                                "{GENERIC_STRING_OBJECT_PREFIX}{JSON_OBJECT_PREFIX}"
                            ))?
                            .try_into()
                            .ok()?;
                        object_id.and_then(|server_id| match json_object_type {
                            JsonObjectType::Preference => {
                                let model = CloudPreferenceModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudPreference::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::EnvVarCollection => {
                                let model =
                                    CloudEnvVarCollectionModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudEnvVarCollection::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::WorkflowEnum => {
                                let model = CloudWorkflowEnumModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudWorkflowEnum::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::AIFact => {
                                let model = CloudAIFactModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> = Box::new(CloudAIFact::new(
                                        server_id,
                                        model,
                                        to_cloud_object_metadata(metadata),
                                        cloud_object_permissions,
                                    ));
                                    boxed
                                })
                            }
                            JsonObjectType::MCPServer => {
                                let model = CloudMCPServerModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudMCPServer::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::TemplatableMCPServer => {
                                let model =
                                    CloudTemplatableMCPServerModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudTemplatableMCPServer::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::AIExecutionProfile => {
                                let model =
                                    CloudAIExecutionProfileModel::deserialize_owned(&object.data);
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudAIExecutionProfile::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::CloudEnvironment => {
                                let model = CloudAmbientAgentEnvironmentModel::deserialize_owned(
                                    &object.data,
                                );
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudAmbientAgentEnvironment::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            JsonObjectType::ScheduledAmbientAgent => {
                                let model = CloudScheduledAmbientAgentModel::deserialize_owned(
                                    &object.data,
                                );
                                model.ok().map(|model| {
                                    let boxed: Box<dyn CloudObject> =
                                        Box::new(CloudScheduledAmbientAgent::new(
                                            server_id,
                                            model,
                                            to_cloud_object_metadata(metadata),
                                            cloud_object_permissions,
                                        ));
                                    boxed
                                })
                            }
                            // TODO: Implement CloudAgentConfig model when full sync support is added
                            JsonObjectType::CloudAgentConfig => None,
                        })
                    })
            })
            .collect::<Vec<_>>(),
    );

    let db_teams: Vec<model::Team> = schema::teams::dsl::teams.load(conn)?;

    let team_member_rows: Vec<model::TeamMemberRow> =
        schema::team_members::dsl::team_members.load(conn)?;
    let members_by_team_id: HashMap<i32, Vec<crate::workspaces::team::TeamMember>> =
        team_member_rows
            .into_iter()
            .fold(HashMap::new(), |mut acc, row| {
                let member = crate::workspaces::team::TeamMember {
                    uid: UserUid::new(&row.user_uid),
                    email: row.email,
                    role: serde_json::from_str(&row.role)
                        .unwrap_or(crate::workspaces::team::MembershipRole::User),
                };
                acc.entry(row.team_id).or_default().push(member);
                acc
            });

    let team_settings_rows: Vec<model::TeamSetting> =
        schema::team_settings::dsl::team_settings.load(conn)?;
    let settings_by_team_id: HashMap<i32, String> = team_settings_rows
        .into_iter()
        .map(|ts| (ts.team_id, ts.settings_json))
        .collect();

    let teams: Vec<TeamMetadata> = db_teams
        .into_iter()
        .map(|team| {
            let team_settings = settings_by_team_id
                .get(&team.id)
                .and_then(|json| serde_json::from_str(json).ok());

            let billing_metadata = team
                .billing_metadata_json
                .as_ref()
                .and_then(|json| serde_json::from_str(json).ok());

            let members = members_by_team_id.get(&team.id).cloned();

            TeamMetadata::from_local_cache(
                ServerId::from_string_lossy(team.server_uid),
                team.name,
                team_settings,
                billing_metadata,
                members,
            )
        })
        .collect();

    let workspace_teams: Vec<model::WorkspaceTeam> = schema::workspace_teams::dsl::workspace_teams
        .load_iter::<model::WorkspaceTeam, DefaultLoadingMode>(conn)?
        .filter_map(|workspace_team| workspace_team.ok())
        .collect();

    let workspaces: Vec<WorkspaceMetadata> = schema::workspaces::dsl::workspaces
        .load_iter::<model::Workspace, DefaultLoadingMode>(conn)?
        .filter_map(|workspace| {
            workspace.ok().map(|workspace| {
                let teams_for_workspace = workspace_teams
                    .iter()
                    .filter_map(|workspace_team| {
                        if workspace_team.workspace_server_uid == workspace.server_uid {
                            teams.iter().find(|team| {
                                team.uid
                                    == ServerId::from_string_lossy(&workspace_team.team_server_uid)
                            })
                        } else {
                            None
                        }
                    })
                    .cloned()
                    .collect();
                WorkspaceMetadata::from_local_cache(
                    workspace.server_uid.into(),
                    workspace.name,
                    Some(teams_for_workspace),
                )
            })
        })
        .collect();

    let current_workspace_uid: Option<WorkspaceUid> = schema::workspaces::dsl::workspaces
        .filter(schema::workspaces::dsl::is_selected.eq(true))
        .select(schema::workspaces::dsl::server_uid)
        .first::<String>(conn)
        .optional()?
        .map(|uid| uid.into());

    let commands = schema::commands::dsl::commands
        // Ensure the commands come into memory sorted chronologically.
        .order(schema::commands::columns::id.desc())
        .load_iter::<model::Command, DefaultLoadingMode>(conn)?
        .filter_map(|command| command.ok())
        .map(PersistedCommand::from)
        .collect();

    let user_profiles = schema::user_profiles::dsl::user_profiles
        .load_iter::<model::UserProfile, DefaultLoadingMode>(conn)?
        .filter_map(|user_profile| user_profile.ok())
        .map(UserProfileWithUID::from)
        .collect();

    let object_actions: Vec<ObjectAction> = schema::object_actions::dsl::object_actions
        .load_iter::<model::PersistedObjectAction, DefaultLoadingMode>(conn)?
        .filter_map(|object_action| object_action.ok()) // parse into PersistedObjectAction
        .filter_map(|action| action.try_into().ok())
        .collect();

    let server_experiments = schema::server_experiments::dsl::server_experiments
        .load_iter::<model::ServerExperiment, DefaultLoadingMode>(conn)?
        .filter_map(|server_experiment| server_experiment.ok())
        .filter_map(|server_experiment| {
            ServerExperiment::from_string(server_experiment.experiment).ok()
        })
        .collect();

    let restored_blocks = get_all_restored_blocks(conn)?;

    // Load active MCP servers from database
    let running_mcp_servers = load_active_mcp_servers(conn)?;

    let app_state = AppState {
        windows: saved_windows,
        active_window_index,
        block_lists: Arc::new(restored_blocks),
        running_mcp_servers,
    };

    // Find the smallest refresh timestamp to pass into CloudModel.
    let time_of_next_force_object_refresh: Option<DateTime<Utc>> =
        schema::cloud_objects_refreshes::dsl::cloud_objects_refreshes
            .load_iter::<model::CloudObjectsRefresh, DefaultLoadingMode>(conn)?
            .filter_map(|refresh| refresh.ok())
            .map(|refresh| refresh.time_of_next_refresh.and_utc())
            .min();

    let ai_queries = read_ai_queries(conn)?;

    let codebase_indices = get_all_codebase_index_metadata(conn)?;
    let workspace_language_servers = get_all_workspace_language_servers_by_workspace(conn)?;
    let multi_agent_conversations = read_agent_conversations(conn)?;
    let projects = get_all_projects(conn)?;
    let project_rules = get_all_project_rules(conn)?;
    let ignored_suggestions = get_all_ignored_suggestions(conn)?;
    let mcp_server_installations = get_all_mcp_server_installations(conn)?;
    let mcp_servers_to_restore = get_mcp_servers_to_restore(conn)?;

    Ok(PersistedData {
        app_state,
        cloud_objects,
        workspaces,
        current_workspace_uid,
        command_history: commands,
        user_profiles,
        time_of_next_force_object_refresh,
        object_actions,
        experiments: server_experiments,
        ai_queries,
        codebase_indices,
        workspace_language_servers,
        multi_agent_conversations,
        projects,
        project_rules,
        ignored_suggestions,
        mcp_server_installations,
        mcp_servers_to_restore,
    })
}

fn id_from_metadata<K: HashableId + ToServerId>(metadata: &ObjectMetadata) -> Option<SyncId> {
    match (&metadata.server_id, &metadata.client_id) {
        (Some(server_id), _) => {
            K::from_hash(server_id).map(|id| SyncId::ServerId(id.to_server_id()))
        }
        (None, Some(client_id)) => ClientId::from_hash(client_id).map(SyncId::ClientId),
        _ => None,
    }
}

fn to_cloud_object_metadata(metadata: &ObjectMetadata) -> CloudObjectMetadata {
    CloudObjectMetadata {
        current_editor_uid: metadata.current_editor.clone(),
        metadata_last_updated_ts: metadata
            .metadata_last_updated_ts
            .and_then(|epoch| ServerTimestamp::from_unix_timestamp_micros(epoch).ok()),
        revision: metadata
            .revision_ts
            .and_then(|epoch| Revision::from_unix_timestamp_micros(epoch).ok()),
        pending_changes_statuses: CloudObjectStatuses {
            pending_delete: false,
            content_sync_status: if metadata.is_pending {
                CloudObjectSyncStatus::InFlight(NumInFlightRequests(1))
            } else {
                CloudObjectSyncStatus::NoLocalChanges
            },
            has_pending_metadata_change: false,
            has_pending_permissions_change: false,
            pending_untrash: false,
        },
        trashed_ts: metadata
            .trashed_ts
            .and_then(|epoch| ServerTimestamp::from_unix_timestamp_micros(epoch).ok()),
        folder_id: metadata.folder_id.as_ref().and_then(|folder_id_str| {
            // First, attempt to convert the string into a server id.
            let as_server_id =
                FolderId::from_hash(folder_id_str).map(|id| SyncId::ServerId(id.into()));

            // If the string cannot be converted to server id, it may be a client id.
            if as_server_id.is_none() {
                ClientId::from_hash(folder_id_str).map(SyncId::ClientId)
            } else {
                as_server_id
            }
        }),
        is_welcome_object: metadata.is_welcome_object,
        creator_uid: metadata.creator_uid.clone(),
        last_editor_uid: metadata.last_editor_uid.clone(),
        last_task_run_ts: None,
    }
}

fn to_cloud_object_permissions(
    permissions: &ObjectPermissions,
    default_user_id: Option<UserUid>,
) -> Option<CloudObjectPermissions> {
    let owner = owner_for_permissions(permissions, default_user_id)?;
    let permissions_last_updated_ts = permissions
        .permissions_last_updated_at
        .and_then(|ts| ServerTimestamp::from_unix_timestamp_micros(ts).ok());

    let guests = if FeatureFlag::SharedWithMe.is_enabled() {
        permissions
            .object_guests
            .as_deref()
            // If deserializing guests fails, default to None and wait for an eventual refresh.
            .and_then(|guests| super::cloud_objects::decode_guests(guests).ok())
            .unwrap_or_default()
    } else {
        Default::default()
    };

    let anyone_with_link = if FeatureFlag::SharedWithMe.is_enabled() {
        permissions
            .anyone_with_link_access_level
            .as_deref()
            .and_then(|access_level| {
                super::cloud_objects::decode_link_sharing(
                    access_level,
                    permissions.anyone_with_link_source.as_deref(),
                )
                // If deserializing link sharing fails, default to None and wait for an
                // eventual refresh.
                .ok()
            })
    } else {
        None
    };

    Some(CloudObjectPermissions {
        owner,
        permissions_last_updated_ts,
        guests,
        anyone_with_link,
    })
}

fn owner_for_permissions(
    permissions: &ObjectPermissions,
    default_user_id: Option<UserUid>,
) -> Option<Owner> {
    match permissions.subject_type.as_str() {
        "USER" => {
            let user_uid = permissions
                .subject_id
                .as_deref()
                .map(UserUid::new)
                .or(default_user_id)?;
            Some(Owner::User { user_uid })
        }
        "TEAM" => Some(Owner::Team {
            team_uid: ServerId::from_string_lossy(&permissions.subject_uid),
        }),
        _ => None,
    }
}

impl From<StartedCommandMetadata> for model::NewCommand {
    fn from(metadata: StartedCommandMetadata) -> Self {
        Self {
            command: metadata.command,
            exit_code: None,
            start_ts: metadata.start_ts.map(|ts| ts.naive_utc()),
            completed_ts: None,
            pwd: metadata.pwd,
            shell: metadata.shell,
            username: metadata.username,
            hostname: metadata.hostname,
            session_id: metadata.session_id.and_then(|id| {
                // The `SessionID` is a wrapper around a `u64`. However diesel only allows
                // writing signed values for sqlite, which means we must convert it into an `i64`.
                // This is a shortcoming of how we represent the `SessionID`: we aren't guaranteed
                // (from a type safety perspective) that we can write it into SQLite. This is
                // another reason why the `SessionID` should be created within Rust and then passed
                // to our bootstrap scripts instead of the other way around: it would allow us to
                // create a random ID that could either be a `u16` or a `u32`.
                let id: u64 = id.into();
                id.try_into().ok()
            }),
            git_branch: metadata.git_branch,
            cloud_workflow_id: metadata
                .cloud_workflow_id
                .map(|id| id.sqlite_uid_hash(ObjectIdType::Workflow)),
            workflow_command: metadata.workflow_command,
            is_agent_executed: Some(metadata.is_agent_executed),
        }
    }
}

fn insert_command(
    conn: &mut SqliteConnection,
    command_metadata: StartedCommandMetadata,
) -> Result<(), Error> {
    use schema::commands::dsl::*;

    conn.transaction::<(), Error, _>(|conn| {
        let command_count: i64 = commands.count().first(conn)?;
        if command_count == COMMANDS_COUNT_LIMIT {
            let oldest_command_id: i32 =
                commands.select(id).order(id.asc()).limit(1).first(conn)?;
            diesel::delete(commands.filter(id.eq(oldest_command_id))).execute(conn)?;
        }

        let new_command: NewCommand = command_metadata.into();
        diesel::insert_into(schema::commands::dsl::commands)
            .values(new_command)
            .execute(conn)?;
        Ok(())
    })
}

fn update_finished_command(
    conn: &mut SqliteConnection,
    completed_command: FinishedCommandMetadata,
) -> Result<(), Error> {
    use schema::commands::dsl::*;

    let completed_command_session_id: Option<i64> =
        completed_command.session_id.as_u64().try_into().ok();

    conn.transaction::<(), Error, _>(|conn| {
        diesel::update(commands)
            .filter(start_ts.eq(Some(completed_command.start_ts.naive_utc())))
            .filter(session_id.eq(completed_command_session_id))
            .set((
                exit_code.eq(completed_command.exit_code.value()),
                completed_ts.eq(completed_command.completed_ts.naive_utc()),
            ))
            .execute(conn)?;
        Ok(())
    })
}

fn upsert_user_profiles(
    conn: &mut SqliteConnection,
    profiles: Vec<UserProfileWithUID>,
) -> Result<(), Error> {
    use schema::user_profiles::dsl::*;

    conn.transaction::<(), Error, _>(|conn| {
        for profile in profiles {
            // Delete any stale profile with that uid
            diesel::delete(
                schema::user_profiles::dsl::user_profiles
                    .filter(firebase_uid.eq(profile.firebase_uid.to_string())),
            )
            .execute(conn)?;

            // Insert a new user profile row
            let new_user_profile = UserProfile {
                firebase_uid: profile.firebase_uid.to_string(),
                photo_url: profile.photo_url,
                display_name: profile.display_name,
                email: profile.email,
            };
            diesel::insert_into(schema::user_profiles::dsl::user_profiles)
                .values(new_user_profile)
                .execute(conn)?;
        }
        Ok(())
    })
}

fn save_experiments(
    conn: &mut SqliteConnection,
    experiments: Vec<ServerExperiment>,
) -> Result<(), Error> {
    conn.transaction::<(), Error, _>(|conn| {
        diesel::delete(schema::server_experiments::dsl::server_experiments).execute(conn)?;

        let new_experiments = experiments
            .into_iter()
            .map(|experiment| NewServerExperiment {
                experiment: experiment.to_string(),
            })
            .collect_vec();

        diesel::insert_into(schema::server_experiments::dsl::server_experiments)
            .values(new_experiments)
            .execute(conn)?;
        Ok(())
    })
}

fn clear_user_profiles(conn: &mut SqliteConnection) -> Result<(), Error> {
    conn.transaction::<(), Error, _>(|conn| {
        diesel::delete(schema::user_profiles::dsl::user_profiles).execute(conn)?;

        Ok(())
    })
}

fn record_time_of_next_refresh(
    conn: &mut SqliteConnection,
    timestamp: DateTime<Utc>,
) -> Result<(), Error> {
    use schema::cloud_objects_refreshes::dsl::*;
    let refresh = NewCloudObjectsRefresh {
        time_of_next_refresh: timestamp.naive_utc(),
    };
    conn.transaction::<(), Error, _>(|conn| {
        diesel::delete(cloud_objects_refreshes).execute(conn)?;
        diesel::insert_into(cloud_objects_refreshes)
            .values(refresh)
            .execute(conn)?;
        Ok(())
    })
}

fn upsert_current_user_information(
    conn: &mut SqliteConnection,
    user_information: PersistedCurrentUserInformation,
) -> Result<(), Error> {
    conn.transaction::<(), Error, _>(|conn| {
        diesel::delete(schema::current_user_information::dsl::current_user_information)
            .execute(conn)?;

        diesel::insert_into(schema::current_user_information::dsl::current_user_information)
            .values(CurrentUserInformation {
                email: user_information.email,
            })
            .execute(conn)?;
        Ok(())
    })
}

fn upsert_mcp_server_environment_variables(
    conn: &mut SqliteConnection,
    mcp_server_uuid: Vec<u8>,
    environment_variables: String,
) -> Result<(), Error> {
    conn.transaction::<(), Error, _>(|conn| {
        let env_vars = MCPEnvironmentVariables {
            mcp_server_uuid,
            environment_variables,
        };
        diesel::insert_into(schema::mcp_environment_variables::dsl::mcp_environment_variables)
            .values(&env_vars)
            .on_conflict(schema::mcp_environment_variables::dsl::mcp_server_uuid)
            .do_update()
            .set(&env_vars)
            .execute(conn)?;
        Ok(())
    })
}

fn load_active_mcp_servers(conn: &mut SqliteConnection) -> Result<Vec<uuid::Uuid>, Error> {
    use schema::active_mcp_servers::dsl::*;

    Ok(active_mcp_servers
        .load::<ActiveMCPServer>(conn)?
        .into_iter()
        .filter_map(|active_server| uuid::Uuid::parse_str(&active_server.mcp_server_uuid).ok())
        .collect())
}

/// Converts the ObjectAction type into a uniform type that can be inserted into
/// the sqlite table.
impl From<ObjectAction> for model::NewPersistedObjectAction {
    fn from(action: ObjectAction) -> Self {
        match action.action_subtype {
            ObjectActionSubtype::SingleAction {
                timestamp,
                data,
                pending,
                processed_at_timestamp,
            } => Self {
                hashed_object_id: action.hashed_sqlite_id,
                timestamp: Some(timestamp.naive_utc()),
                action: action.action_type.to_string(),
                data,
                count: None,
                oldest_timestamp: None,
                latest_timestamp: None,
                pending: Some(pending),
                processed_at_timestamp: processed_at_timestamp.map(|t| t.naive_utc()),
            },
            ObjectActionSubtype::BundledActions {
                count,
                oldest_timestamp,
                latest_timestamp,
                latest_processed_at_timestamp,
            } => Self {
                hashed_object_id: action.hashed_sqlite_id,
                timestamp: None,
                action: action.action_type.to_string(),
                data: None,
                count: Some(count),
                oldest_timestamp: Some(oldest_timestamp.naive_utc()),
                latest_timestamp: Some(latest_timestamp.naive_utc()),
                pending: None,
                processed_at_timestamp: Some(latest_processed_at_timestamp.naive_utc()),
            },
        }
    }
}

fn insert_object_action(
    conn: &mut SqliteConnection,
    object_action: ObjectAction,
) -> Result<(), Error> {
    let action: NewPersistedObjectAction = object_action.into();
    conn.transaction::<(), Error, _>(|conn| {
        diesel::insert_into(schema::object_actions::dsl::object_actions)
            .values(action)
            .execute(conn)?;
        Ok(())
    })
}

fn sync_object_actions(
    conn: &mut SqliteConnection,
    actions_to_sync: Vec<ObjectAction>,
) -> Result<(), Error> {
    use schema::object_actions::dsl::*;

    let ids_to_delete: HashSet<String> =
        HashSet::from_iter(actions_to_sync.iter().map(|a| a.hashed_sqlite_id.clone()));
    // Insert the new ones
    let new_actions: Vec<NewPersistedObjectAction> =
        actions_to_sync.iter().map(|a| a.clone().into()).collect();
    conn.transaction::<(), Error, _>(|conn| {
        // Erase all the actions that currently have this object ID
        for hashed_sqlite_id in ids_to_delete {
            diesel::delete(object_actions.filter(hashed_object_id.eq(hashed_sqlite_id)))
                .execute(conn)?;
        }

        // Insert the new ones
        diesel::insert_into(schema::object_actions::dsl::object_actions)
            .values(new_actions)
            .execute(conn)?;
        Ok(())
    })
}

fn delete_objects(
    conn: &mut SqliteConnection,
    ids: Vec<(SyncId, ObjectIdType)>,
) -> Result<(), Error> {
    conn.transaction::<(), Error, _>(|conn| {
        for (sync_id, object_id_type) in ids {
            match object_id_type {
                ObjectIdType::Notebook => delete_cloud_object(
                    conn,
                    sync_id,
                    object_id_type,
                    Box::new(|conn, notebook_id| {
                        use schema::notebooks::dsl::*;
                        diesel::delete(notebooks.filter(id.eq(notebook_id))).execute(conn)?;
                        Ok(())
                    }),
                )?,
                ObjectIdType::Workflow => delete_cloud_object(
                    conn,
                    sync_id,
                    object_id_type,
                    Box::new(|conn, workflow_id| {
                        use schema::workflows::dsl::*;
                        diesel::delete(workflows.filter(id.eq(workflow_id))).execute(conn)?;
                        Ok(())
                    }),
                )?,
                ObjectIdType::Folder => delete_cloud_object(
                    conn,
                    sync_id,
                    object_id_type,
                    Box::new(|conn, folder_id| {
                        use schema::folders::dsl::*;
                        diesel::delete(folders.filter(id.eq(folder_id))).execute(conn)?;
                        Ok(())
                    }),
                )?,
                ObjectIdType::GenericStringObject => delete_cloud_object(
                    conn,
                    sync_id,
                    object_id_type,
                    Box::new(|conn, gso_id| {
                        use schema::generic_string_objects::dsl::*;
                        diesel::delete(generic_string_objects.filter(id.eq(gso_id)))
                            .execute(conn)?;
                        Ok(())
                    }),
                )?,
            }
        }
        Ok(())
    })
}

#[cfg(test)]
#[path = "sqlite_tests.rs"]
mod tests;
