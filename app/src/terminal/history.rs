use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Local, TimeZone as _};
use futures::Future;
use serde::{Deserialize, Serialize};
use warp_core::command::ExitCode;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::model::block::{AgentInteractionMetadata, Block, SerializedAIMetadata, SerializedBlock};
use super::shell::ShellType;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::Space;
use crate::report_error;
use crate::server::ids::{ClientId, HashableId as _, SyncId};
use crate::terminal::model::session::{Session, SessionId};
use crate::util::dedupe_from_last;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowId, WorkflowSource, WorkflowType};

mod up_arrow;
pub(crate) use up_arrow::UpArrowHistoryConfig;

/// Data model for a history command persisted to sqlite, used as an intermediate representation
/// between the sqlite schema (sqlite::model::Command) and the [`History`] model.
#[derive(Debug)]
pub struct PersistedCommand {
    pub id: i32,
    pub command: String,
    pub exit_code: Option<ExitCode>,
    pub start_ts: Option<DateTime<Local>>,
    pub completed_ts: Option<DateTime<Local>>,
    pub pwd: Option<String>,
    pub shell_host: Option<ShellHost>,
    pub session_id: Option<SessionId>,
    pub git_branch: Option<String>,
    pub workflow_id: Option<SyncId>,
    pub workflow_command: Option<String>,
    pub is_agent_executed: bool,
}

impl From<crate::persistence::model::Command> for PersistedCommand {
    fn from(command: crate::persistence::model::Command) -> Self {
        PersistedCommand {
            id: command.id,
            command: command.command,
            exit_code: command.exit_code.map(ExitCode::from),
            start_ts: command
                .start_ts
                .as_ref()
                .map(|time| Local.from_utc_datetime(time)),
            completed_ts: command
                .completed_ts
                .as_ref()
                .map(|time| Local.from_utc_datetime(time)),
            pwd: command.pwd,
            shell_host: match (command.shell, command.username, command.hostname) {
                (Some(shell), Some(username), Some(hostname)) => {
                    ShellType::from_name(shell.as_str()).map(|shell_type| ShellHost {
                        shell_type,
                        user: username,
                        hostname,
                    })
                }
                _ => None,
            },
            session_id: command.session_id.and_then(|session_id| {
                TryInto::<u64>::try_into(session_id)
                    .ok()
                    .map(SessionId::from)
            }),
            git_branch: command.git_branch,
            workflow_id: command.cloud_workflow_id.and_then(|workflow_id| {
                if let Some(client_id) = ClientId::from_hash(workflow_id.as_str()) {
                    Some(SyncId::ClientId(client_id))
                } else {
                    WorkflowId::from_hash(workflow_id.as_str())
                        .map(|id| SyncId::ServerId(id.into()))
                }
            }),
            workflow_command: command.workflow_command,
            is_agent_executed: command.is_agent_executed.unwrap_or(false),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub struct ShellHost {
    // This field was originally named `shell` so mark it as an alias for
    // backwards compatibility.
    #[serde(alias = "shell")]
    pub shell_type: ShellType,
    pub user: String,
    pub hostname: String,
}

impl ShellHost {
    pub fn from_session(session: &Session) -> Self {
        Self {
            shell_type: session.shell().shell_type(),
            user: session.user().to_owned(),
            hostname: session.hostname().to_owned(),
        }
    }

    #[cfg(test)]
    pub fn from_session_info(session_info: &super::model::session::SessionInfo) -> Self {
        Self {
            shell_type: session_info.shell.shell_type(),
            user: session_info.user.clone(),
            hostname: session_info.hostname.clone(),
        }
    }

    pub fn try_from_persisted_block(block: &crate::persistence::model::Block) -> Option<Self> {
        block
            .shell
            .as_ref()
            .and_then(|name| ShellType::from_name(&name[..]))
            .map(|shell| ShellHost {
                shell_type: shell,
                user: block
                    .user
                    .clone()
                    .unwrap_or_else(|| "local:user".to_owned()),
                hostname: block
                    .host
                    .clone()
                    .unwrap_or_else(|| "local:host".to_owned()),
            })
    }
}

/// Represents the state of the async task for reading the shell's history file for a given session.
#[derive(Debug)]

enum ReadHistoryFileState {
    InProgress {
        /// Commands that were executed by the user while the history file was being loaded. When
        /// the history file has finished loading, these commands are appended to the history
        /// file's commands in the in-memory representation of the session's history.
        ///
        /// These commands chronologically ordered by time of execution.
        session_commands_to_append: Vec<Arc<HistoryEntry>>,

        /// Session IDs for sessions that are bootstrapped and awaiting a loaded histfile for the
        /// corresponding shell host.
        session_ids: HashSet<SessionId>,
    },
    Done,
}

pub enum HistoryEvent {
    /// History has been initialized for the session with the contained ID.
    Initialized(SessionId),
}

/// This holds the aggregated data from the "commands" table in sqlite. We aggregate as a means of
/// de-duping, and store data mostly for the most recent execution for each command.
#[derive(Debug)]
struct CommandHistorySummary {
    /// The execution metadata from the latest time a particular command was run.
    most_recent_entry: HistoryEntry,
    /// Counts the number of executions in the "commands" table. Note that this may not match the
    /// count in the HISTFILE.
    count: u32,
}

impl CommandHistorySummary {
    fn new(most_recent_entry: HistoryEntry) -> Self {
        Self {
            most_recent_entry,
            count: 1,
        }
    }
}

#[derive(Default, Debug)]
pub struct History {
    /// For each ShellHost, the de-duped commands from the sqlite "commands" table is stored here.
    /// Each time a history file is read, it gets "joined" to the commands in here to add the
    /// execution metadata from the most recent run.
    persisted_commands_summary: HashMap<ShellHost, HashMap<String, CommandHistorySummary>>,

    /// Entries from the history file for the host.  Immutable once loaded and
    /// shared between sessions.
    history_file_commands: HashMap<ShellHost, Vec<Arc<HistoryEntry>>>,

    /// Global history entries across all sessions for each host.  Only grows.  Deduping
    /// is handled by marking session_skip_indices.
    /// Note that restored block commands are appended to session_commands on startup.
    /// Note: To present commands chronologically across hosts, we can add a timestamp to each history entry
    session_commands: HashMap<ShellHost, Vec<Arc<HistoryEntry>>>,

    /// Indices to skip when rendering, by session.  Indices are into the concatenation of
    /// history file commands + session commands, which is the history list for that host.
    session_skip_indices: HashMap<SessionId, HashSet<usize>>,
    session_start_indices: HashMap<SessionId, usize>,

    /// A map of session ID to the state of the background task to read the shell history file for
    /// the corresponding session.
    read_history_file_state: HashMap<ShellHost, ReadHistoryFileState>,

    session_id_to_shell_host: HashMap<SessionId, ShellHost>,
}

#[derive(Clone, Debug)]
pub enum LinkedWorkflowData {
    /// The history entry is linked to a `CloudWorkflow` by its ID.
    Id(SyncId),

    /// The history entry is linked to a local `Workflow` by its command.
    ///
    /// Local workflows are not keyed by any common ID.
    Command(String),
}

impl LinkedWorkflowData {
    /// Returns the WorkflowType and WorkflowSource corresponding to this `LinkedWorkflowData`, if
    /// any.
    pub fn linked_workflow(&self, ctx: &AppContext) -> Option<(WorkflowType, WorkflowSource)> {
        match self {
            LinkedWorkflowData::Id(id) => {
                let cloud_model = CloudModel::as_ref(ctx);
                let workflow = cloud_model.get_workflow(id);
                let workflow_source = match CloudViewModel::as_ref(ctx).object_space(&id.uid(), ctx)
                {
                    Some(Space::Team { team_uid }) => WorkflowSource::Team { team_uid },
                    _ => WorkflowSource::PersonalCloud,
                };
                workflow.map(|workflow| {
                    (
                        WorkflowType::Cloud(Box::new(workflow.clone())),
                        workflow_source,
                    )
                })
            }
            LinkedWorkflowData::Command(workflow_command) => {
                if let Some((workflow_source, workflow)) = LocalWorkflows::as_ref(ctx)
                    .workflow_with_command(ctx, workflow_command.as_str())
                {
                    Some((WorkflowType::Local(workflow.clone()), workflow_source))
                } else {
                    None
                }
            }
        }
    }
}

/// For history entries coming from the shell history file, only the command is populated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub session_id: Option<SessionId>,
    pub command: String,
    pub pwd: Option<String>,
    pub start_ts: Option<DateTime<Local>>,
    pub completed_ts: Option<DateTime<Local>>,
    pub exit_code: Option<ExitCode>,
    pub git_head: Option<String>,
    pub shell_host: Option<ShellHost>,

    /// The ID of the `CloudWorkflow` used to construct this command.
    workflow_id: Option<SyncId>,

    /// The templated command contained in the `Workflow` used to construct the executed
    /// command.
    workflow_command: Option<String>,

    pub is_for_restored_block: bool,

    /// Whether this command was executed by an AI agent.
    pub is_agent_executed: bool,
}

fn serialized_block_is_agent_executed(block: &SerializedBlock) -> bool {
    let Some(ai_metadata) = block.ai_metadata.as_ref() else {
        return false;
    };

    serde_json::from_str::<SerializedAIMetadata>(ai_metadata)
        .ok()
        .map(AgentInteractionMetadata::from)
        .is_some_and(|metadata| metadata.requested_command_action_id().is_some())
}

impl HistoryEntry {
    pub fn command_only<S: Into<String>>(command: S) -> Self {
        Self {
            command: command.into(),
            session_id: None,
            pwd: None,
            start_ts: None,
            completed_ts: None,
            workflow_id: None,
            workflow_command: None,
            exit_code: None,
            git_head: None,
            shell_host: None,
            is_for_restored_block: false,
            is_agent_executed: false,
        }
    }

    pub fn command_at_time(
        command: String,
        start_ts: DateTime<Local>,
        session_id: Option<SessionId>,
        is_for_restored_block: bool,
    ) -> Self {
        let mut entry = Self::command_only(command);
        entry.start_ts = Some(start_ts);
        entry.session_id = session_id;
        entry.is_for_restored_block = is_for_restored_block;
        entry
    }

    pub fn for_session_command(
        command: String,
        active_block: &Block,
        session: &Session,
        workflow_id: Option<SyncId>,
        workflow_command: Option<String>,
        is_agent_executed: bool,
    ) -> Self {
        HistoryEntry {
            session_id: Some(session.id()),
            command,
            pwd: active_block.pwd().map(|pwd| pwd.to_owned()),
            start_ts: active_block.start_ts().copied(),
            workflow_id,
            workflow_command,
            git_head: active_block
                .git_branch()
                .map(|git_branch| git_branch.to_owned()),
            completed_ts: None,
            exit_code: None,
            shell_host: active_block.shell_host().clone(),
            is_for_restored_block: false,
            is_agent_executed,
        }
    }

    pub fn for_restored_block(command: String, block: &Block) -> Self {
        HistoryEntry {
            session_id: block.session_id(),
            command,
            pwd: block.pwd().map(|pwd| pwd.to_owned()),
            start_ts: block.start_ts().copied(),
            workflow_id: None,
            workflow_command: None,
            git_head: block.git_branch().map(|git_branch| git_branch.to_owned()),
            shell_host: block.shell_host().clone(),
            completed_ts: block.completed_ts().copied(),
            exit_code: Some(block.exit_code()),
            is_for_restored_block: true,
            is_agent_executed: block.requested_command_action_id().is_some(),
        }
    }

    pub fn for_completed_block(command: String, block: &SerializedBlock) -> Self {
        HistoryEntry {
            session_id: block.session_id,
            command,
            pwd: block.pwd.clone(),
            start_ts: block.start_ts,
            completed_ts: block.completed_ts,
            workflow_id: None,
            workflow_command: None,
            exit_code: Some(block.exit_code),
            git_head: block.git_head.clone(),
            shell_host: block.shell_host.clone(),
            is_for_restored_block: false,
            is_agent_executed: serialized_block_is_agent_executed(block),
        }
    }

    /// Returns an `Option` containing the workflow linked to this command, if any.
    ///
    /// First looks up the workflow using `self.workflow_id`, then falls back to looking up the
    /// workflow using `self.workflow_command`, if any.
    pub fn linked_workflow(&self, app: &AppContext) -> Option<Workflow> {
        match (&self.workflow_id, &self.workflow_command) {
            (Some(workflow_id), _) => CloudModel::as_ref(app)
                .get_workflow(workflow_id)
                .map(|workflow| workflow.model().data.clone()),
            (_, Some(workflow_command)) => LocalWorkflows::as_ref(app)
                .workflow_with_command(app, workflow_command)
                .map(|(_, workflow)| workflow.clone()),
            _ => None,
        }
    }

    /// Indicates that at least one of the optional rich history fields is Some.
    pub fn has_metadata(&self) -> bool {
        // Destructure this so that we _must_ update this method when new metadata fields are added
        // to Self. `completed_ts` isn't useful without start_ts, so that is omitted in this check.
        let HistoryEntry {
            session_id: _,
            command: _,
            is_for_restored_block: _,
            is_agent_executed: _,
            pwd,
            start_ts,
            completed_ts: _,
            workflow_id,
            exit_code,
            git_head,
            workflow_command,
            shell_host: _,
        } = self;
        pwd.is_some()
            || start_ts.is_some()
            || workflow_id.is_some()
            || exit_code.is_some()
            || git_head.is_some()
            || workflow_command.is_some()
    }

    /// Returns `LinkedWorkflowData` referring to the workflow used to create this history command,
    /// if any.
    pub fn linked_workflow_data(&self) -> Option<LinkedWorkflowData> {
        match (&self.workflow_id, &self.workflow_command) {
            (Some(workflow_id), _) => Some(LinkedWorkflowData::Id(*workflow_id)),
            (_, Some(workflow_command)) => {
                Some(LinkedWorkflowData::Command(workflow_command.clone()))
            }
            _ => None,
        }
    }
}

impl From<PersistedCommand> for HistoryEntry {
    fn from(command: PersistedCommand) -> Self {
        HistoryEntry {
            session_id: command.session_id,
            command: command.command,
            exit_code: command.exit_code,
            start_ts: command.start_ts,
            completed_ts: command.completed_ts,
            pwd: command.pwd,
            git_head: command.git_branch,
            workflow_id: command.workflow_id,
            workflow_command: command.workflow_command,
            shell_host: command.shell_host,
            is_for_restored_block: false,
            is_agent_executed: command.is_agent_executed,
        }
    }
}

impl Entity for History {
    type Event = HistoryEvent;
}

impl SingletonEntity for History {}

impl History {
    pub fn new(persisted_commands: Vec<PersistedCommand>) -> Self {
        log::debug!("Creating new History model with persisted commands {persisted_commands:?}");
        let mut persisted_commands_summary =
            HashMap::<ShellHost, HashMap<String, CommandHistorySummary>>::new();

        for command in persisted_commands {
            if let Some(shell_host) = command.shell_host.as_ref() {
                let summaries = persisted_commands_summary
                    .entry(shell_host.clone())
                    .or_default();
                let hist_entry: HistoryEntry = command.into();
                summaries
                    .entry(hist_entry.command.clone())
                    .and_modify(|summary| summary.count += 1)
                    .or_insert(CommandHistorySummary::new(hist_entry));
            }
        }

        Self {
            persisted_commands_summary,
            ..Default::default()
        }
    }

    pub fn all_live_session_ids(&self) -> HashSet<SessionId> {
        self.session_id_to_shell_host.keys().cloned().collect()
    }

    /// Initializes the history model for the given session.
    ///
    /// Command history from the shell's history file is read asynchronously on a background
    /// thread. Depending on whether or not the session is local or remote, history may be read
    /// directly from disk or via an in-band command.
    pub fn init_session(&mut self, session: Arc<Session>, ctx: &mut ModelContext<Self>) {
        if self.session_start_indices.contains_key(&session.id()) {
            log::debug!(
                "In init_session but history was already initialized for session {:?}",
                session.id()
            );
        } else {
            let session_clone = session.clone();
            let is_kaspersky_running = Self::is_kaspersky_running(ctx);
            self.init_session_with(
                session,
                async move { session_clone.read_history(is_kaspersky_running).await },
                ctx,
            );
        }
    }

    /// Determines whether Kaspersky is running on the system. We only care if
    /// Kaspersky is running on Windows, so we return false for other platforms.
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn is_kaspersky_running(ctx: &mut ModelContext<Self>) -> bool {
        cfg_if::cfg_if! {
            if #[cfg(windows)] {
                crate::util::windows::is_kaspersky_running(ctx)
            } else {
                false
            }
        }
    }

    /// Initializes the history model history model for the given session, where
    /// `read_history_file_future` is a Future that completes with the contents of the shell's
    /// history file.
    ///
    /// `read_history_file_state` is executed in a background task.
    pub fn init_session_with<F>(
        &mut self,
        session: Arc<Session>,
        read_history_file_future: F,
        ctx: &mut ModelContext<Self>,
    ) where
        F: 'static + Future<Output = Vec<String>> + Send,
    {
        let host = ShellHost {
            shell_type: session.shell().shell_type(),
            user: session.user().to_owned(),
            hostname: session.hostname().to_owned(),
        };

        let session_id = session.id();
        log::debug!(
            "Setting shell history for shell {:?} and session id {:?}",
            host.shell_type.name(),
            session_id
        );

        self.session_id_to_shell_host
            .insert(session_id, host.clone());

        match self.read_history_file_state.get_mut(&host) {
            None => {
                let mut session_ids = HashSet::new();
                session_ids.insert(session_id);
                self.read_history_file_state.insert(
                    host.clone(),
                    ReadHistoryFileState::InProgress {
                        session_commands_to_append: vec![],
                        session_ids,
                    },
                );
                let host_clone = host.clone();
                ctx.spawn(
                    read_history_file_future,
                    move |me, history_file_commands, ctx| {
                        // `session_commands_to_append` are commands executed by the user while the
                        // history file was being loaded.
                        let (session_commands_to_append, session_ids) = match me
                            .read_history_file_state
                            .insert(host_clone, ReadHistoryFileState::Done)
                        {
                            Some(ReadHistoryFileState::InProgress {
                                session_commands_to_append,
                                session_ids,
                            }) => (Some(session_commands_to_append), session_ids),
                            _ => {
                                // This branch should be unreachable, but don't panic!
                                log::warn!("Marking ReadHistoryFileState::Done for session when \
                                           its previous state was not ReadHistoryFileState::InProgress.");
                                let mut session_ids = HashSet::new();
                                session_ids.insert(session_id);
                                (None, session_ids)
                            }
                        };
                        me.load_history_file_commands(
                            history_file_commands,
                            session_commands_to_append,
                            session_ids,
                            host,
                            ctx,
                        );
                    },
                );
            }
            Some(ReadHistoryFileState::Done) => {
                let Some(history_file_commands) = self.history_file_commands.get(&host) else {
                    report_error!(
                        "History file commands should exist if history file has been read."
                    );
                    return;
                };
                let session_commands_length = self
                    .session_commands
                    .get(&host)
                    .map(|entries| entries.len())
                    .unwrap_or(0);
                let session_start_index = history_file_commands.len() + session_commands_length;

                let mut session_ids = HashSet::new();
                session_ids.insert(session_id);
                self.initialize_session_start_and_skip_indices(
                    session_ids,
                    host,
                    session_start_index,
                    ctx,
                )
            }
            Some(ReadHistoryFileState::InProgress { session_ids, .. }) => {
                session_ids.insert(session_id);
            }
        }
    }

    /// Parses the given `history_file_commands` into `HistoryEntry`'s and sets the
    /// `history_file_commands` entry for the session with the given ID.
    ///
    /// `session_commands_to_append` are commands that have been executed by the user while loading
    /// history file commands; these commands are inserted directly into the session history.
    fn load_history_file_commands(
        &mut self,
        history_file_commands: Vec<String>,
        session_commands_to_append: Option<Vec<Arc<HistoryEntry>>>,
        session_ids: HashSet<SessionId>,
        host: ShellHost,
        ctx: &mut ModelContext<Self>,
    ) {
        let deduped_history_file_commands = dedupe_from_last(history_file_commands);

        let mut start_index = deduped_history_file_commands.len();
        self.history_file_commands.insert(
            host.clone(),
            deduped_history_file_commands
                .into_iter()
                .map(|command| {
                    self.persisted_commands_summary
                        .get(&host)
                        .and_then(|summaries| summaries.get(&command))
                        .map(|summary| summary.most_recent_entry.clone())
                        .unwrap_or_else(|| HistoryEntry::command_only(command))
                })
                .map(Arc::new)
                .collect(),
        );

        if let Some(session_commands_to_append) = session_commands_to_append {
            start_index += session_commands_to_append.len();
            self.session_commands
                .insert(host.clone(), session_commands_to_append);
        }

        self.initialize_session_start_and_skip_indices(session_ids, host, start_index, ctx);
    }

    /// Initializes the 'session start index' and 'skip indices' for the given session.
    fn initialize_session_start_and_skip_indices(
        &mut self,
        session_ids: HashSet<SessionId>,
        host: ShellHost,
        session_start_index: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        log::debug!("Loading command history from start index {session_start_index}.");
        for session_id in &session_ids {
            self.session_start_indices
                .insert(*session_id, session_start_index);
        }

        self.session_commands.entry(host).or_default();

        for session_id in session_ids {
            // Dedupe commands for the new session
            // There could be duplicate live commands from other sessions of the same host
            let mut seen_commands: HashSet<&str> = HashSet::new();
            let mut skip_index_set: HashSet<usize> = HashSet::new();
            self.session_skip_indices.insert(session_id, HashSet::new());
            for (idx, &entry) in self
                .commands(session_id)
                .unwrap_or_else(|| {
                    log::warn!("History commands are empty for session {session_id:?}");
                    Vec::new()
                })
                .iter()
                .enumerate()
                .rev()
            {
                if seen_commands.contains(entry.command.as_str()) {
                    skip_index_set.insert(idx);
                } else {
                    seen_commands.insert(entry.command.as_str());
                }
            }
            self.session_skip_indices.insert(session_id, skip_index_set);
            ctx.emit(HistoryEvent::Initialized(session_id));
        }
    }

    /// Returns true iff this session's history is ready to be queried.
    ///
    /// A session's history is only queryable after the histfile is read.
    pub fn is_queryable(&self, session_id: &SessionId) -> bool {
        self.session_start_indices.contains_key(session_id)
    }

    /// Returns true iff this session's history can be appended.
    ///
    /// A session's history is appendable as soon as the corresponding
    /// session is registered with the [`History`] model.
    pub fn is_appendable(&self, session_id: &SessionId) -> bool {
        self.session_id_to_shell_host.contains_key(session_id)
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn is_session_initialized(&self, session_id: &SessionId) -> bool {
        self.is_queryable(session_id) && self.is_appendable(session_id)
    }

    /// Appends restored block commands from session restoration.
    /// We don't persist session_id for restored blocks, so the restored HistoryEntries are missing session_id.
    /// We manually set the session_id of each HistoryEntry here to ensure it's populated and correct.
    pub fn append_restored_commands(&mut self, session_id: SessionId, commands: Vec<HistoryEntry>) {
        let commands: Vec<HistoryEntry> = commands
            .into_iter()
            .map(|mut c| {
                c.session_id = Some(session_id);
                c
            })
            .collect();
        // All entries already have metadata and is_for_restored_block set to true
        self.append_commands(session_id, commands);
    }

    /// Appends an array of commands to the session's history (session_commands, not history_file_commands).
    /// The commands should be chronologically ordered by time of execution.
    ///
    /// If the read for the history file is still in progress, queues a background task to append
    /// the given commands once the read is complete. Otherwise, synchronously appends the given
    /// commands.
    pub fn append_commands(&mut self, session_id: SessionId, commands: Vec<HistoryEntry>) {
        let Some(shell_host) = self.session_id_to_shell_host.get(&session_id) else {
            log::warn!("ShellHost should be populated in the map for all bootstrapped sessions.");
            return;
        };
        match self
            .read_history_file_state
            .get_mut(shell_host)
            .expect("ReadHistoryFileState should exist for session.")
        {
            ReadHistoryFileState::InProgress {
                session_commands_to_append,
                ..
            } => {
                session_commands_to_append.extend(commands.into_iter().map(Arc::new));
            }
            ReadHistoryFileState::Done => {
                let mut commands_set = HashSet::new();
                let mut commands_set_skip_indices = Vec::new();

                // First dedupe the commands to append to the history and filter out empty commands.
                for (idx, command) in commands.iter().enumerate().rev() {
                    if commands_set.contains(command.command.as_str()) || command.command.is_empty()
                    {
                        commands_set_skip_indices.push(idx);
                    } else {
                        commands_set.insert(command.command.as_str());
                    }
                }

                let Some(history_file_commands) = self.history_file_commands.get(shell_host) else {
                    log::warn!(
                        "history_file_commands should be set if ReadHistoryFileState is Done."
                    );
                    return;
                };

                let Some(session_commands) = self.session_commands.get_mut(shell_host) else {
                    log::warn!("session_commands should be set if ReadHistoryFileState is Done.");
                    return;
                };

                let skip_indices = &mut self.session_skip_indices;
                let mut last_index = 0;
                for (idx, h) in history_file_commands
                    .iter()
                    .chain(session_commands.iter())
                    .enumerate()
                {
                    // Mark to skip this command if it exists already
                    if commands_set.contains(h.command.as_str()) {
                        skip_indices.entry(session_id).or_default().insert(idx);
                    }
                    last_index = idx;
                }

                for command in commands {
                    session_commands.push(Arc::new(command));
                }

                // Offset the skip indices with the length of all commands in history.
                for idx in commands_set_skip_indices {
                    let entry = skip_indices.entry(session_id).or_default();
                    entry.insert(last_index + idx + 1);
                }
            }
        }
    }

    /// `commands` returns the history file commands for the session's host, concatenated
    /// with session commands.
    ///
    /// It is important that it is this concatenation specifically, because self.session_skip_indices is
    /// an index into this specific concatenation.
    pub fn commands(&self, session_id: SessionId) -> Option<Vec<&HistoryEntry>> {
        self.collect_visible_commands_for_session(session_id, |h| h.as_ref())
    }

    fn collect_visible_commands_for_session<'a, T>(
        &'a self,
        session_id: SessionId,
        mut to_output: impl FnMut(&'a Arc<HistoryEntry>) -> T,
    ) -> Option<Vec<T>> {
        let Some(shell_host) = self.session_id_to_shell_host.get(&session_id) else {
            log::warn!("ShellHost should be populated in the map for all bootstrapped sessions.");
            return None;
        };
        let Some(session_start_index) = self.session_start_indices.get(&session_id) else {
            log::warn!("Session start index for session {session_id:?} is None.");
            return None;
        };
        let Some(skip_indices) = self.session_skip_indices.get(&session_id) else {
            log::warn!("Skip indices for session {session_id:?} are empty.");
            return None;
        };

        let Some(histfile_commands) = self.history_file_commands.get(shell_host) else {
            log::warn!("Histfile commands for session {session_id:?} are empty.");
            return None;
        };
        let Some(session_commands) = self.session_commands.get(shell_host) else {
            log::warn!("Session commands for session {session_id:?} are empty.");
            return None;
        };
        let commands: Vec<T> = histfile_commands
            .iter()
            .chain(session_commands.iter())
            .enumerate()
            .filter_map(|(idx, h)| {
                if skip_indices.contains(&idx) {
                    return None;
                }
                // Restored blocks may appear before the session start index because they may be
                // loaded before the history file has been read. However, restored block commands
                // have the same semantics as post-start index session commands -- they should only
                // be shown in the session where they were restored.
                if idx < *session_start_index && !h.is_for_restored_block {
                    // This history entry was from before the session's history
                    // started, so we include it.
                    return Some(to_output(h));
                }
                // Otherwise, we need to check if the history entry is from this session.
                if let Some(history_session_id) = h.session_id {
                    if history_session_id == session_id {
                        Some(to_output(h))
                    } else {
                        None
                    }
                } else {
                    // No session id, so command is from the history file
                    Some(to_output(h))
                }
            })
            .collect();

        Some(commands)
    }

    /// `commands_shared` returns the same logical set as [`Self::commands`], but with shared
    /// ownership so callers can keep an owned snapshot without deep-cloning command data.
    pub fn commands_shared(&self, session_id: SessionId) -> Option<Vec<Arc<HistoryEntry>>> {
        self.collect_visible_commands_for_session(session_id, Arc::clone)
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self, session_id: SessionId) -> usize {
        self.commands(session_id)
            .map(|c| c.len())
            .unwrap_or_default()
    }

    pub fn is_empty(&self, session_id: SessionId) -> bool {
        self.len(session_id) == 0
    }

    #[cfg(feature = "integration_tests")]
    pub fn session_commands(&self) -> &HashMap<ShellHost, Vec<Arc<HistoryEntry>>> {
        &self.session_commands
    }

    pub fn mark_command_as_finished(
        &mut self,
        session_id: SessionId,
        command_start_ts: DateTime<Local>,
        command_completed_ts: DateTime<Local>,
        exit_code: ExitCode,
    ) {
        let Some(shell_host) = self.session_id_to_shell_host.get(&session_id) else {
            log::warn!("ShellHost should be populated in the map for all bootstrapped sessions.");
            return;
        };
        let session_commands = match self
            .read_history_file_state
            .get_mut(shell_host)
            .expect("ReadHistoryFileState should exist for session.")
        {
            ReadHistoryFileState::InProgress {
                session_commands_to_append,
                ..
            } => session_commands_to_append,
            ReadHistoryFileState::Done => {
                let Some(commands) = self.session_commands.get_mut(shell_host) else {
                    return;
                };
                commands
            }
        };

        for entry in session_commands.iter_mut().rev() {
            if let Some(entry_start_ts) = &entry.start_ts {
                if entry_start_ts.timestamp_millis() == command_start_ts.timestamp_millis() {
                    let entry = Arc::make_mut(entry);
                    entry.exit_code = Some(exit_code);
                    entry.completed_ts = Some(command_completed_ts);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
pub mod tests;
