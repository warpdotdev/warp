//! Bridges durable lead-agent messages into Claude Code's hook-driven
//! next-turn context.
//!
//! The bridge uses an on-disk three-stage state machine inside the per-session
//! state directory:
//! - `staged/` holds newly observed message IDs from the event stream.
//! - `surfaced/` holds the fully hydrated records currently exposed to Claude.
//! - `pending-hook-output.json` plus `pending-hook-output.ack` coordinates the
//!   handoff between Warp's driver and the Claude hook process.
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;
use warpui::r#async::SpawnedFutureHandle;
use warpui::ModelSpawner;

use crate::ai::agent_events::{
    run_agent_event_driver, AgentEventConsumer, AgentEventConsumerControlFlow,
    AgentEventDriverConfig, AgentMessageEventMetadata, MessageHydrator, ServerApiAgentEventSource,
};
use crate::ai::agent_sdk::driver::{AgentDriver, OZ_MESSAGE_LISTENER_STATE_ROOT_ENV};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, AgentRunEvent};
use crate::server::server_api::ServerApi;

const LEGACY_MESSAGE_LISTENER_STATE_ROOT_ENV: &str = "OZ_PARENT_STATE_ROOT";
const PARENT_BRIDGE_DEFAULT_STATE_ROOT: &str = ".claude-code/oz-parent-bridge";
const PARENT_BRIDGE_SURFACED_DIR_NAME: &str = "surfaced";
const PARENT_BRIDGE_EVENT_CURSOR_FILE_NAME: &str = "event-cursor.json";
const PARENT_BRIDGE_HOOK_OUTPUT_FILE_NAME: &str = "pending-hook-output.json";
const PARENT_BRIDGE_HOOK_OUTPUT_ACK_FILE_NAME: &str = "pending-hook-output.ack";
const PARENT_BRIDGE_MAX_CONTEXT_CHARS_ENV: &str = "OZ_PARENT_MAX_CONTEXT_CHARS";
const PARENT_BRIDGE_DEFAULT_MAX_CONTEXT_CHARS: usize = 6000;
pub(super) const MESSAGE_BRIDGE_CONTEXT_PREAMBLE: &str = "Lead-agent updates arrived from Oz. Treat the latest lead-agent instructions below as authoritative.\n";
const PARENT_BRIDGE_REMAINING_MESSAGES_NOTE: &str =
    "\n\nMore lead-agent messages are still staged and will be surfaced on a later turn.";

pub(super) struct MessageBridge {
    run_id: String,
    state_dir: PathBuf,
    runtime: Mutex<Option<MessageBridgeRuntime>>,
    state_lock: AsyncMutex<()>,
}

pub(super) enum MessageBridgeCleanupDisposition {
    RemoveState,
    PreserveState,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct MessageBridgeEventCursor {
    since_sequence: i64,
}
struct MessageBridgeRuntime {
    task: SpawnedFutureHandle,
}

struct MessageBridgeEventConsumer {
    run_id: String,
    state_dir: PathBuf,
    server_api: Arc<ServerApi>,
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventConsumer for MessageBridgeEventConsumer {
    async fn on_event(
        &mut self,
        event: AgentRunEvent,
    ) -> anyhow::Result<AgentEventConsumerControlFlow> {
        if event.event_type != "new_message" || event.run_id != self.run_id {
            return Ok(AgentEventConsumerControlFlow::Continue);
        }

        let Some(message_id) = event.ref_id else {
            return Ok(AgentEventConsumerControlFlow::Continue);
        };

        if let Err(err) = stage_parent_bridge_message(
            &self.state_dir,
            &MessageBridgeMessageRecord {
                sequence: event.sequence,
                message_id: message_id.clone(),
                sender_run_id: String::new(),
                subject: String::new(),
                body: String::new(),
                occurred_at: event.occurred_at,
            },
        ) {
            log::warn!(
                "Failed to stage Claude lead-agent message {message_id} at sequence {}: {err:#}",
                event.sequence
            );
        }

        Ok(AgentEventConsumerControlFlow::Continue)
    }

    async fn persist_cursor(&mut self, sequence: i64) -> anyhow::Result<()> {
        write_parent_bridge_event_cursor(&self.state_dir, sequence)?;
        self.server_api
            .update_event_sequence_on_server(&self.run_id, sequence)
            .await
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MessageBridgeMessageRecord {
    pub sequence: i64,
    pub message_id: String,
    #[serde(default)]
    pub sender_run_id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    pub occurred_at: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct MessageBridgeHookOutput {
    pub additional_context: String,
    pub remaining_staged_count: usize,
    pub surfaced_count: usize,
}

struct RenderedMessageBridgeMessage {
    block: String,
    block_chars: usize,
}
struct SelectedMessageBridgeMessage {
    path: PathBuf,
    record: MessageBridgeMessageRecord,
    rendered: RenderedMessageBridgeMessage,
}

struct SelectedMessageBridgeMessages {
    messages: Vec<SelectedMessageBridgeMessage>,
    context_chars: usize,
    total_available_count: usize,
}

impl MessageBridge {
    fn hydrator(&self, server_api: Arc<ServerApi>) -> MessageHydrator {
        match self.run_id.parse::<AmbientAgentTaskId>() {
            Ok(task_id) => MessageHydrator::for_task(server_api, task_id),
            Err(_) => MessageHydrator::new(server_api),
        }
    }
    pub(super) fn new(run_id: String, session_id: Uuid) -> Result<Self> {
        Ok(Self {
            run_id,
            state_dir: parent_bridge_root()?.join(session_id.to_string()),
            runtime: Mutex::new(None),
            state_lock: AsyncMutex::new(()),
        })
    }

    pub(super) async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
        server_api: Arc<ServerApi>,
    ) -> Result<()> {
        if self.runtime.lock().is_some() {
            return Ok(());
        }

        ensure_parent_bridge_state_dir(&self.state_dir)?;
        let run_id = self.run_id.clone();
        let state_dir = self.state_dir.clone();
        let task = foreground
            .spawn(move |_, ctx| {
                ctx.spawn(
                    async move {
                        if let Err(err) =
                            run_parent_bridge_forever(server_api, run_id, state_dir.clone()).await
                        {
                            log::warn!(
                                "Claude message bridge stopped for {}: {err:#}",
                                state_dir.display()
                            );
                        }
                    },
                    |_, _, _| {},
                )
            })
            .await
            .map_err(|_| anyhow!("Agent driver dropped while starting Claude message bridge"))?;
        *self.runtime.lock() = Some(MessageBridgeRuntime { task });
        Ok(())
    }

    pub(super) async fn handle_session_update(&self, server_api: Arc<ServerApi>) -> Result<()> {
        if !self.state_dir.exists() {
            return Ok(());
        }
        let hydrator = self.hydrator(server_api);
        let _guard = self.state_lock.lock().await;
        acknowledge_parent_bridge_hook_output(&hydrator, &self.state_dir).await?;
        prepare_parent_bridge_hook_output(
            &hydrator,
            &self.state_dir,
            parent_bridge_max_context_chars(),
        )
        .await
    }

    pub(super) async fn flush_acks(&self, server_api: Arc<ServerApi>) -> Result<()> {
        if !self.state_dir.exists() {
            return Ok(());
        }
        let hydrator = self.hydrator(server_api);
        let _guard = self.state_lock.lock().await;
        acknowledge_parent_bridge_hook_output(&hydrator, &self.state_dir).await
    }

    pub(super) fn cleanup(&self, disposition: MessageBridgeCleanupDisposition) -> Result<()> {
        if let Some(runtime) = self.runtime.lock().take() {
            runtime.task.abort();
        }
        if matches!(disposition, MessageBridgeCleanupDisposition::PreserveState) {
            return Ok(());
        }
        match fs::remove_dir_all(&self.state_dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(anyhow::Error::from(err).context(format!(
                    "Failed to remove Claude message bridge state dir {}",
                    self.state_dir.display()
                )));
            }
        }
        Ok(())
    }
}

pub(super) fn parent_bridge_root() -> Result<PathBuf> {
    for env_name in [
        OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
        LEGACY_MESSAGE_LISTENER_STATE_ROOT_ENV,
    ] {
        if let Ok(dir) = std::env::var(env_name) {
            if !dir.is_empty() {
                return Ok(PathBuf::from(dir));
            }
        }
    }
    dirs::home_dir()
        .map(|home| home.join(PARENT_BRIDGE_DEFAULT_STATE_ROOT))
        .ok_or_else(|| anyhow!("could not determine home directory"))
}

fn parent_bridge_staged_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("staged")
}

fn parent_bridge_surfaced_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(PARENT_BRIDGE_SURFACED_DIR_NAME)
}

pub(super) fn parent_bridge_hook_output_file(state_dir: &Path) -> PathBuf {
    state_dir.join(PARENT_BRIDGE_HOOK_OUTPUT_FILE_NAME)
}

pub(super) fn parent_bridge_hook_output_ack_file(state_dir: &Path) -> PathBuf {
    state_dir.join(PARENT_BRIDGE_HOOK_OUTPUT_ACK_FILE_NAME)
}

pub(super) fn parent_bridge_event_cursor_file(state_dir: &Path) -> PathBuf {
    state_dir.join(PARENT_BRIDGE_EVENT_CURSOR_FILE_NAME)
}

fn parent_bridge_message_path(dir: &Path, sequence: i64, message_id: &str) -> PathBuf {
    dir.join(format!("{sequence:020}-{message_id}.json"))
}

pub(super) fn parent_bridge_staged_message_path(
    state_dir: &Path,
    sequence: i64,
    message_id: &str,
) -> PathBuf {
    parent_bridge_message_path(&parent_bridge_staged_dir(state_dir), sequence, message_id)
}

pub(super) fn parent_bridge_surfaced_message_path(
    state_dir: &Path,
    sequence: i64,
    message_id: &str,
) -> PathBuf {
    parent_bridge_message_path(&parent_bridge_surfaced_dir(state_dir), sequence, message_id)
}

pub(super) fn ensure_parent_bridge_state_dir(state_dir: &Path) -> Result<()> {
    fs::create_dir_all(parent_bridge_staged_dir(state_dir))
        .with_context(|| format!("Failed to create {}", state_dir.display()))?;
    fs::create_dir_all(parent_bridge_surfaced_dir(state_dir))
        .with_context(|| format!("Failed to create {}", state_dir.display()))?;
    Ok(())
}

pub(super) fn read_parent_bridge_event_cursor(state_dir: &Path) -> Result<i64> {
    let path = parent_bridge_event_cursor_file(state_dir);
    if !path.exists() {
        return Ok(0);
    }

    let cursor = serde_json::from_slice::<MessageBridgeEventCursor>(
        &fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(cursor.since_sequence)
}

pub(super) fn write_parent_bridge_event_cursor(state_dir: &Path, sequence: i64) -> Result<()> {
    write_parent_bridge_json_atomically(
        &parent_bridge_event_cursor_file(state_dir),
        &MessageBridgeEventCursor {
            since_sequence: sequence,
        },
    )
}

pub(super) fn stage_parent_bridge_message(
    state_dir: &Path,
    record: &MessageBridgeMessageRecord,
) -> Result<()> {
    let target = parent_bridge_staged_message_path(state_dir, record.sequence, &record.message_id);
    if !target.exists() {
        write_parent_bridge_json_atomically(&target, record)?;
    }
    Ok(())
}

pub(super) async fn prime_parent_bridge_for_wake(
    hydrator: &MessageHydrator,
    state_dir: &Path,
    wake_message: Option<&AgentMessageEventMetadata>,
) -> Result<()> {
    acknowledge_parent_bridge_hook_output(hydrator, state_dir).await?;

    let Some(wake_message) = wake_message else {
        return Ok(());
    };

    stage_parent_bridge_message(
        state_dir,
        &MessageBridgeMessageRecord {
            sequence: wake_message.sequence,
            message_id: wake_message.message_id.clone(),
            sender_run_id: String::new(),
            subject: String::new(),
            body: String::new(),
            occurred_at: wake_message.occurred_at.clone(),
        },
    )?;
    write_parent_bridge_event_cursor(state_dir, wake_message.sequence)?;
    prepare_parent_bridge_hook_output(hydrator, state_dir, parent_bridge_max_context_chars()).await
}

pub(super) fn parent_bridge_max_context_chars() -> usize {
    std::env::var(PARENT_BRIDGE_MAX_CONTEXT_CHARS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(PARENT_BRIDGE_DEFAULT_MAX_CONTEXT_CHARS)
}

fn parent_bridge_sorted_message_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut paths = fs::read_dir(dir)
        .with_context(|| format!("Failed to read {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn parent_bridge_message_records(dir: &Path) -> Result<Vec<(PathBuf, MessageBridgeMessageRecord)>> {
    parent_bridge_sorted_message_paths(dir)?
        .into_iter()
        .map(|path| {
            let record = serde_json::from_slice::<MessageBridgeMessageRecord>(
                &fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?,
            )
            .with_context(|| format!("Failed to parse {}", path.display()))?;
            Ok((path, record))
        })
        .collect()
}

pub(super) fn parent_bridge_char_count(text: &str) -> usize {
    text.chars().count()
}

fn parent_bridge_truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

pub(super) fn render_parent_bridge_message_block(record: &MessageBridgeMessageRecord) -> String {
    let subject = if record.subject.is_empty() {
        "(no subject)"
    } else {
        record.subject.as_str()
    };

    let mut block = String::from("---\nLead-agent message");
    if record.sequence != 0 {
        let _ = write!(block, " #{}", record.sequence);
    }
    if !record.sender_run_id.is_empty() {
        let _ = write!(block, " from {}", record.sender_run_id);
    }
    let _ = write!(block, "\nSubject: {subject}\n\n{}", record.body);
    block
}

fn render_parent_bridge_message(
    record: &MessageBridgeMessageRecord,
) -> RenderedMessageBridgeMessage {
    let block = render_parent_bridge_message_block(record);
    let block_chars = parent_bridge_char_count(&block);
    RenderedMessageBridgeMessage { block, block_chars }
}

fn truncate_parent_bridge_message(rendered: &mut RenderedMessageBridgeMessage, max_chars: usize) {
    if rendered.block_chars <= max_chars || max_chars <= 3 {
        return;
    }

    rendered.block = parent_bridge_truncate_chars(&rendered.block, max_chars - 3);
    rendered.block.push_str("...");
    rendered.block_chars = parent_bridge_char_count(&rendered.block);
}

fn build_parent_bridge_hook_output(
    selected: &SelectedMessageBridgeMessages,
    max_context_chars: usize,
) -> Option<MessageBridgeHookOutput> {
    if selected.messages.is_empty() {
        return None;
    }

    let mut additional_context = String::from(MESSAGE_BRIDGE_CONTEXT_PREAMBLE);
    for (index, message) in selected.messages.iter().enumerate() {
        if index > 0 {
            additional_context.push_str("\n\n");
        }
        additional_context.push_str(&message.rendered.block);
    }

    let remaining_staged_count = selected
        .total_available_count
        .saturating_sub(selected.messages.len());
    let remaining_note_chars = parent_bridge_char_count(PARENT_BRIDGE_REMAINING_MESSAGES_NOTE);
    if remaining_staged_count > 0
        && selected.context_chars + remaining_note_chars <= max_context_chars
    {
        additional_context.push_str(PARENT_BRIDGE_REMAINING_MESSAGES_NOTE);
    }

    Some(MessageBridgeHookOutput {
        additional_context,
        remaining_staged_count,
        surfaced_count: selected.messages.len(),
    })
}

fn write_parent_bridge_hook_output(
    state_dir: &Path,
    output: &MessageBridgeHookOutput,
) -> Result<()> {
    let path = parent_bridge_hook_output_file(state_dir);
    write_parent_bridge_json_atomically(&path, output)
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(anyhow::Error::from(err).context(format!("Failed to remove {}", path.display())))
        }
    }
}

async fn hydrate_parent_bridge_message_record(
    hydrator: &MessageHydrator,
    record: &MessageBridgeMessageRecord,
) -> Result<MessageBridgeMessageRecord> {
    if !record.sender_run_id.is_empty() {
        return Ok(record.clone());
    }

    let message = hydrator
        .read_message_with_timeout(&record.message_id)
        .await
        .with_context(|| format!("Failed to read lead-agent message {}", record.message_id))?;
    Ok(MessageBridgeMessageRecord {
        sequence: record.sequence,
        message_id: message.message_id,
        sender_run_id: message.sender_run_id,
        subject: message.subject,
        body: message.body,
        occurred_at: record.occurred_at.clone(),
    })
}

async fn select_parent_bridge_messages_for_hook_output(
    hydrator: &MessageHydrator,
    records: Vec<(PathBuf, MessageBridgeMessageRecord)>,
    max_context_chars: usize,
) -> Result<SelectedMessageBridgeMessages> {
    let total_available_count = records.len();
    let mut messages = Vec::new();
    let mut context_chars = parent_bridge_char_count(MESSAGE_BRIDGE_CONTEXT_PREAMBLE);

    for (path, record) in records {
        let separator_chars = if messages.is_empty() { 0 } else { 2 };
        let remaining = max_context_chars.saturating_sub(context_chars + separator_chars);
        if remaining <= 3 && !messages.is_empty() {
            break;
        }

        let record = hydrate_parent_bridge_message_record(hydrator, &record).await?;
        let mut rendered = render_parent_bridge_message(&record);
        if context_chars + separator_chars + rendered.block_chars > max_context_chars {
            if remaining > 3 && rendered.block_chars > remaining {
                truncate_parent_bridge_message(&mut rendered, remaining);
            } else if !messages.is_empty() {
                break;
            }
        }

        context_chars += separator_chars + rendered.block_chars;
        messages.push(SelectedMessageBridgeMessage {
            path,
            record,
            rendered,
        });
    }

    Ok(SelectedMessageBridgeMessages {
        messages,
        context_chars,
        total_available_count,
    })
}

pub(super) async fn prepare_parent_bridge_hook_output(
    hydrator: &MessageHydrator,
    state_dir: &Path,
    max_context_chars: usize,
) -> Result<()> {
    let hook_output_path = parent_bridge_hook_output_file(state_dir);
    if hook_output_path.exists() {
        return Ok(());
    }

    let surfaced_records = parent_bridge_message_records(&parent_bridge_surfaced_dir(state_dir))?;
    if !surfaced_records.is_empty() {
        let selected = select_parent_bridge_messages_for_hook_output(
            hydrator,
            surfaced_records,
            max_context_chars,
        )
        .await?;
        for message in &selected.messages {
            write_parent_bridge_json_atomically(&message.path, &message.record)?;
        }
        if let Some(output) = build_parent_bridge_hook_output(&selected, max_context_chars) {
            write_parent_bridge_hook_output(state_dir, &output)?;
        }
        return Ok(());
    }

    let staged_records = parent_bridge_message_records(&parent_bridge_staged_dir(state_dir))?;
    if staged_records.is_empty() {
        return Ok(());
    }

    let selected =
        select_parent_bridge_messages_for_hook_output(hydrator, staged_records, max_context_chars)
            .await?;
    let Some(output) = build_parent_bridge_hook_output(&selected, max_context_chars) else {
        return Ok(());
    };

    for message in &selected.messages {
        let target = parent_bridge_surfaced_message_path(
            state_dir,
            message.record.sequence,
            &message.record.message_id,
        );
        fs::rename(&message.path, &target).with_context(|| {
            format!(
                "Failed to move message bridge record {} to {}",
                message.path.display(),
                target.display()
            )
        })?;
        write_parent_bridge_json_atomically(&target, &message.record)?;
    }

    remove_file_if_exists(&parent_bridge_hook_output_ack_file(state_dir))?;
    write_parent_bridge_hook_output(state_dir, &output)
}

pub(super) async fn acknowledge_parent_bridge_hook_output(
    hydrator: &MessageHydrator,
    state_dir: &Path,
) -> Result<()> {
    let ack_path = parent_bridge_hook_output_ack_file(state_dir);
    if !ack_path.exists() {
        return Ok(());
    }

    // Remove the hook output first so an acknowledged block cannot be re-emitted
    // if the harness restarts while delivery cleanup is still in progress.
    remove_file_if_exists(&parent_bridge_hook_output_file(state_dir))?;

    let surfaced_records = parent_bridge_message_records(&parent_bridge_surfaced_dir(state_dir))?;
    let message_ids = surfaced_records
        .iter()
        .map(|(_, record)| record.message_id.clone())
        .collect::<Vec<_>>();
    let delivery_failures = hydrator
        .mark_messages_delivered_best_effort(message_ids.iter().map(String::as_str))
        .await;
    for (message_id, err) in delivery_failures {
        log::warn!(
            "Failed to mark Claude message bridge message {message_id} as delivered: {err:#}"
        );
    }

    for (path, _) in surfaced_records {
        remove_file_if_exists(&path)?;
    }
    remove_file_if_exists(&ack_path)
}

async fn run_parent_bridge_forever(
    server_api: Arc<ServerApi>,
    run_id: String,
    state_dir: PathBuf,
) -> Result<()> {
    ensure_parent_bridge_state_dir(&state_dir)?;
    let since_sequence =
        read_parent_bridge_resume_cursor(server_api.as_ref(), &run_id, &state_dir).await?;
    // The shared driver keeps `since_sequence` in memory across its own retry
    // loop and we also persist it inside the session state dir so dormant runs
    // can resume without replaying already handled events.
    let config = AgentEventDriverConfig::retry_forever(vec![run_id.clone()], since_sequence);
    let source = ServerApiAgentEventSource::new(server_api.clone());
    let mut consumer = MessageBridgeEventConsumer {
        run_id,
        state_dir,
        server_api,
    };
    run_agent_event_driver(source, config, &mut consumer).await
}

async fn read_parent_bridge_resume_cursor(
    server_api: &ServerApi,
    run_id: &str,
    state_dir: &Path,
) -> Result<i64> {
    // The server cursor is the durable cross-client source of truth, but the
    // bridge also keeps a local cursor for same-machine recovery. If Warp or
    // Claude restarts after the bridge has staged events locally but before the
    // server cursor update is visible, the local cursor prevents replaying
    // messages already handed to this Claude session.
    let local_sequence = read_parent_bridge_event_cursor(state_dir)?;
    let Ok(task_id) = run_id.parse() else {
        return Ok(local_sequence);
    };

    let server_sequence = match server_api.get_ambient_agent_task(&task_id).await {
        Ok(task) => task.last_event_sequence.unwrap_or(0),
        Err(err) => {
            log::warn!(
                "Failed to read server-backed event cursor for Claude message bridge run {run_id}: {err:#}"
            );
            0
        }
    };

    Ok(local_sequence.max(server_sequence))
}

fn write_parent_bridge_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_parent_bridge_bytes_atomically(path, &serde_json::to_vec(value)?)
}

fn write_parent_bridge_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(anyhow!("{} has no parent directory", path.display()));
    };
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;

    let prefix = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("parent-bridge");
    let mut temp_file = NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temp file for {}", path.display()))?;
    temp_file
        .write_all(bytes)
        .with_context(|| format!("Failed to write temp file for {}", path.display()))?;
    temp_file
        .flush()
        .with_context(|| format!("Failed to flush temp file for {}", path.display()))?;
    temp_file
        .persist(path)
        .map(|_| ())
        .map_err(|err| {
            anyhow::Error::from(err.error).context(format!("Failed to write {}", path.display()))
        })
        .with_context(|| format!("Failed to persist temporary {prefix} file"))?;
    Ok(())
}
