//! Driving third-party harnesses over the Agent Client Protocol.
//!
//! The harness runs as a command in the driver's terminal session, after environment setup, so
//! it inherits the shell state those commands established and shows up as a block in the shared
//! session; its stdio is bridged to the in-process ACP client over a socket (see [`bridge`]). The
//! agent's `session/update` stream is translated into MAA client actions and applied to the
//! driver's native conversation, so viewers and the task status model see an ordinary Warp agent
//! turn.
//!
//! The agent is launched in the driver's own terminal session and bridged over a local socket,
//! so this transport is local-only: the `fs/*` methods act on the driver's file system, which
//! is the agent's. If the bridge ever spans a remote session they must go through the same
//! session-aware file I/O as the native `ReadFiles` / `RequestFileEdits` executors.
//!
//! Local testing: `oz agent run --harness codex --harness-transport acp --share team:view
//! --idle-on-complete 15m --prompt "..."`. For a scripted agent, put a `codex-acp` shim on
//! `PATH` that runs `node app/src/ai/agent_sdk/driver/harness/acp/testdata/fake_agent.mjs`.
mod attachments;
mod bridge;
mod cli_transcript;
mod connection;
mod launch;
mod manifest;
mod mapping;
mod model_config;
mod policy;
mod protocol;
mod turn_driver;

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
pub(crate) use bridge::run_bridge;
use futures::{FutureExt, future, pin_mut, select};
use parking_lot::Mutex;
use prost::Message as _;
use serde_json::Value;
use session_sharing_protocol::common::ParticipantId;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::channel::ChannelState;
use warp_core::safe_warn;
use warp_errors::report_error;
use warp_managed_secrets::ManagedSecretValue;
use warp_multi_agent_api::response_event::StreamFinished;
use warp_multi_agent_api::{ClientAction, McpServerIdentity, ResponseEvent, response_event};
use warp_util::path::{EscapeChar, ShellFamily};
use warpui::r#async::Timer;
use warpui::r#async::executor::Background;
use warpui::{ModelContext, ModelHandle, ModelSpawner, SingletonEntity};

use self::attachments::AttachmentResolver;
use self::bridge::{BridgeListener, bridge_command};
use self::cli_transcript::CliTranscriptKind;
use self::connection::{AcpConnection, RpcError};
use self::launch::AcpLaunchSpec;
use self::manifest::HarnessManifest;
use self::mapping::AcpTurnMapper;
use self::model_config::configure_model_and_thinking_through_acp;
use self::policy::{PolicyDecision, PolicyRequest};
use self::protocol::{
    CancelParams, ClientCapabilities, ClientInfo, FsCapabilities, InitializeParams,
    InitializeResponse, McpServer, NewSessionParams, NewSessionResponse, PROTOCOL_VERSION,
    PermissionOptionKind, PermissionOutcome, ReadTextFileParams, ReadTextFileResponse,
    RequestPermissionParams, RequestPermissionResponse, WriteTextFileParams,
};
use self::turn_driver::{
    EXIT_CANCEL_DEADLINE, STEER_CANCEL_DEADLINE, TurnCancellation, TurnDriver, TurnHost, TurnState,
    cancel_deadline,
};
use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::claude_code::prepare_claude_environment_config;
use super::codex::{prepare_codex_environment_config, publish_skills_for_codex};
use super::gemini::{prepare_gemini_environment_config, publish_skills_for_gemini};
use super::harness_persistence::{
    HarnessPersistence, PersistenceOutcome, save_transcript_and_conversation_data,
};
use super::pi::{self, prepare_pi_environment_config, publish_skills_for_pi};
use super::skill_dirs_publish::HarnessSupportSkills;
use super::transcript_persistence::{
    UploadedTranscriptUsage, capture_transcript_with_retry, upload_captured_transcript,
};
use super::{
    HarnessCleanupDisposition, HarnessRunner, JSONMCPServer, ResolvedMCPServers, ResumePayload,
    SavePoint, ThirdPartyHarness, install_docs_url, validate_cli_installed,
};
use crate::ai::agent::CancellationReason;
use crate::ai::agent::api::{ServerConversationToken, mcp_server_identity};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::attachment_utils::{DownloadedAttachment, attachments_download_dir};
use crate::ai::blocklist::{
    BlocklistAIController, BlocklistAIHistoryModel, ExternalHarnessCancel, ExternalHarnessPrompt,
    ExternalHarnessTurn, QueuedQueryId, ResponseStreamId,
};
use crate::ai::mcp::JSONTransportType;
use crate::server::server_api::ServerApi;
use crate::server::server_api::harness_support::HarnessSupportClient;
use crate::terminal::CLIAgent;

/// Format slug sent to the server when creating the conversation: its history is stored as
/// native MAA `ConversationData`, which the driver uploads itself.
const MAA_FORMAT: &str = "maa";

/// Bound on the agent connecting back through the bridge plus its `initialize` and
/// `session/new` handshakes.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// A third-party harness driven over ACP with manifest-defined launch and setup metadata.
pub(super) struct AcpHarness {
    harness: Harness,
    manifest: HarnessManifest,
    initial_attachments: Vec<DownloadedAttachment>,
}

impl AcpHarness {
    pub(super) fn new(harness: Harness) -> Result<Self, AgentDriverError> {
        Ok(Self {
            harness,
            manifest: manifest::resolve(harness)?,
            initial_attachments: Vec::new(),
        })
    }

    /// The text of the initial prompt. ACP has no system-prompt channel of its own, so every
    /// server-composed block that the harness bootstrap did not already place is folded into
    /// the user turn ahead of the prompt itself.
    fn initial_prompt_text(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        resumption_prompt: Option<&str>,
        context: Option<&str>,
    ) -> String {
        let system_prompt_in_turn = match self.harness {
            Harness::Claude | Harness::Oz | Harness::OpenCode | Harness::Unknown => system_prompt,
            Harness::Codex | Harness::Gemini | Harness::Pi => None,
        };
        [
            resumption_prompt,
            context,
            system_prompt_in_turn,
            Some(prompt),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
    }

    /// Writes the harness's own config files (trust, auth, system prompt) the way the terminal
    /// transport does, so the ACP adapter finds the same environment its CLI would.
    #[allow(clippy::too_many_arguments)]
    fn prepare_harness_environment(
        &self,
        system_prompt: Option<&str>,
        workspace_root: &Path,
        harness_working_dir: &Path,
        resolved_env_vars: &HashMap<OsString, OsString>,
        skill_dirs: &[PathBuf],
        factory_repository_contract: bool,
        resolved_secrets: &HashMap<String, ManagedSecretValue>,
        resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
        third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<()> {
        match self.harness {
            Harness::Claude => prepare_claude_environment_config(
                workspace_root,
                harness_working_dir,
                resolved_env_vars,
                skill_dirs,
                factory_repository_contract,
                HarnessSupportSkills::Bundled,
            ),
            // MCP servers are handed to the agent in `session/new` and the model is selected
            // through the session's config options, so neither is written into `config.toml`.
            Harness::Codex => {
                prepare_codex_environment_config(
                    harness_working_dir,
                    system_prompt,
                    resolved_env_vars,
                    resolved_secrets,
                    &HashMap::new(),
                    None,
                )?;
                publish_skills_for_codex(
                    workspace_root,
                    harness_working_dir,
                    skill_dirs,
                    factory_repository_contract,
                    HarnessSupportSkills::Bundled,
                )
            }
            Harness::Gemini => {
                prepare_gemini_environment_config(harness_working_dir, system_prompt)?;
                publish_skills_for_gemini(
                    workspace_root,
                    harness_working_dir,
                    skill_dirs,
                    factory_repository_contract,
                    HarnessSupportSkills::Bundled,
                )
            }
            Harness::Pi => {
                let agent_dir = prepare_pi_environment_config(
                    harness_working_dir,
                    system_prompt,
                    resolved_env_vars,
                    resolved_mcp_servers,
                    resolved_secrets,
                    third_party_harness_model_config,
                )?;
                publish_skills_for_pi(
                    workspace_root,
                    &agent_dir,
                    skill_dirs,
                    factory_repository_contract,
                )
            }
            Harness::Oz | Harness::OpenCode | Harness::Unknown => Ok(()),
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for AcpHarness {
    fn harness(&self) -> Harness {
        self.harness
    }

    fn cli_agent(&self) -> CLIAgent {
        match self.harness {
            Harness::Claude => CLIAgent::Claude,
            Harness::Codex => CLIAgent::Codex,
            Harness::Gemini => CLIAgent::Gemini,
            Harness::Pi | Harness::Oz | Harness::OpenCode | Harness::Unknown => CLIAgent::Unknown,
        }
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        install_docs_url(self.harness)
    }

    fn validate(&self) -> Result<(), AgentDriverError> {
        if self.manifest.is_sidecar {
            Ok(())
        } else {
            validate_cli_installed(&self.manifest.launch.program, self.install_docs_url())
        }
    }

    fn auth_check_command(&self) -> Option<String> {
        self.manifest.auth_check_command.clone()
    }

    fn runtime_error_patterns_for_run(&self) -> Vec<String> {
        self.manifest.runtime_error_patterns.clone()
    }

    fn drives_cli_agent_session(&self) -> bool {
        false
    }

    fn set_initial_attachments(&mut self, downloaded: Vec<DownloadedAttachment>) {
        self.initial_attachments = downloaded;
    }

    fn build_runner(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        resumption_prompt: Option<&str>,
        context: Option<&str>,
        workspace_root: &Path,
        harness_working_dir: &Path,
        task_id: Option<AmbientAgentTaskId>,
        server_api: Arc<ServerApi>,
        terminal_driver: ModelHandle<TerminalDriver>,
        resume: Option<ResumePayload>,
        resolved_env_vars: &HashMap<OsString, OsString>,
        skill_dirs: &[PathBuf],
        factory_repository_contract: bool,
        resolved_secrets: &HashMap<String, ManagedSecretValue>,
        resolved_mcp_servers: &ResolvedMCPServers,
        third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        self.prepare_harness_environment(
            system_prompt,
            workspace_root,
            harness_working_dir,
            resolved_env_vars,
            skill_dirs,
            factory_repository_contract,
            resolved_secrets,
            &resolved_mcp_servers.configs,
            third_party_harness_model_config,
        )
        .map_err(|error| AgentDriverError::HarnessConfigSetupFailed {
            harness: self.harness.to_string(),
            error,
        })?;

        let prompt_text =
            self.initial_prompt_text(prompt, system_prompt, resumption_prompt, context);
        let attachments = AttachmentResolver::new(
            server_api.clone(),
            server_api.owned_http_client(),
            task_id,
            attachments_download_dir(harness_working_dir),
        );
        // ACP runs resume by restoring the native conversation, never through a harness payload.
        if resume.is_some() {
            log::error!("ACP harness given a resume payload for a terminal-driven harness");
            return Err(AgentDriverError::InvalidRuntimeState);
        }
        let (save_request_tx, save_request_rx) = async_channel::unbounded();
        let cli_session = CliTranscriptKind::for_harness(self.harness).and_then(|kind| match kind
            .root(resolved_env_vars)
        {
            Ok(root) => Some(CliSession {
                kind,
                session_id: OnceLock::new(),
                root,
                transcript_path: OnceLock::new(),
            }),
            Err(error) => {
                log::warn!("Cannot locate {kind:?} transcripts: {error:#}");
                None
            }
        });
        Ok(Box::new(AcpHarnessRunner {
            harness: self.harness,
            launch: self.manifest.launch.clone(),
            user_prompt: prompt.to_owned(),
            prompt_text,
            initial_attachments: self.initial_attachments.clone(),
            harness_working_dir: harness_working_dir.to_path_buf(),
            model_config: third_party_harness_model_config.cloned(),
            task_id,
            client: server_api,
            terminal_driver,
            attachments: Arc::new(attachments),
            mcp_servers: resolved_mcp_servers
                .configs
                .iter()
                .filter(|_| self.harness != Harness::Pi)
                .map(|(name, server)| mcp_server_for_acp(name, server))
                .collect(),
            mcp_server_identities: resolved_mcp_servers
                .configs
                .keys()
                .map(|name| {
                    let warp_id = resolved_mcp_servers.warp_ids.get(name).map(String::as_str);
                    (name.clone(), tool_call_server_identity(name, warp_id))
                })
                .collect(),
            conversation: OnceLock::new(),
            boundary_snapshot: Arc::default(),
            cli_session,
            // Save points are raised at request-stream boundaries, which is the cadence the
            // server persists its own conversations at, so they are uploaded as they come.
            persistence: HarnessPersistence::default().with_save_debounce(Duration::ZERO),
            save_request_tx,
            save_request_rx,
            agent: Mutex::new(None),
            turn_state: Arc::default(),
        }))
    }
}

/// The protocol session with the agent running in the terminal.
struct RunningAgent {
    connection: Arc<AcpConnection>,
    session_id: Option<String>,
}

/// The conversation for this run.
#[derive(Clone)]
struct RunConversation {
    local_id: AIConversationId,
    server_id: ServerConversationToken,
    /// Set when the conversation was restored from the server; the run then continues this
    /// root task instead of creating one.
    restored_root_task_id: Option<String>,
}

/// The CLI session behind the ACP session, whose own transcript is uploaded for the run.
struct CliSession {
    kind: CliTranscriptKind,
    /// The CLI's own session id, which names its transcript on disk.
    session_id: OnceLock<Uuid>,
    /// The directory the CLI keeps its transcripts under.
    root: PathBuf,
    /// Cached once the CLI has written the transcript.
    transcript_path: OnceLock<PathBuf>,
}

pub(crate) struct AcpHarnessRunner {
    harness: Harness,
    launch: AcpLaunchSpec,
    /// The prompt as the user wrote it, echoed into the conversation.
    user_prompt: String,
    /// The full text sent to the agent, including server-composed context.
    prompt_text: String,
    /// Task attachments downloaded before the run started. They are shaped for the agent only
    /// once the handshake has revealed what it accepts inline.
    initial_attachments: Vec<DownloadedAttachment>,
    harness_working_dir: PathBuf,
    model_config: Option<HarnessModelConfig>,
    task_id: Option<AmbientAgentTaskId>,
    client: Arc<dyn HarnessSupportClient>,
    terminal_driver: ModelHandle<TerminalDriver>,
    attachments: Arc<AttachmentResolver>,
    mcp_servers: Vec<McpServer>,
    /// Stamped on the agent's MCP tool calls, keyed by server name.
    mcp_server_identities: HashMap<String, McpServerIdentity>,
    /// Set before the first turn.
    conversation: OnceLock<RunConversation>,
    /// The encoded conversation as captured at the most recent request-stream boundary and not
    /// yet uploaded. Captured by the turn driver when it finishes a stream, so the upload never
    /// sees a stream that a later turn has since opened.
    boundary_snapshot: Arc<Mutex<Option<Vec<u8>>>>,
    cli_session: Option<CliSession>,
    persistence: HarnessPersistence,
    /// Save points raised by the turn driver at request-stream and turn boundaries.
    save_request_tx: async_channel::Sender<SavePoint>,
    save_request_rx: async_channel::Receiver<SavePoint>,
    agent: Mutex<Option<RunningAgent>>,
    /// Turn activity reported by the turn driver.
    turn_state: Arc<TurnState>,
}

impl AcpHarnessRunner {
    fn setup_failed(&self, reason: String) -> AgentDriverError {
        AgentDriverError::HarnessSetupFailed {
            harness: self.harness.to_string(),
            reason,
        }
    }

    fn running_agent(&self) -> Option<(Arc<AcpConnection>, Option<String>)> {
        self.agent
            .lock()
            .as_ref()
            .map(|agent| (agent.connection.clone(), agent.session_id.clone()))
    }

    /// Interrupts whatever the terminal session is running, which is the agent's block while
    /// the harness is up.
    async fn interrupt_terminal(&self, foreground: &ModelSpawner<AgentDriver>) {
        let terminal_driver = self.terminal_driver.clone();
        let _ = foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.send_interrupt_to_pty(ctx));
            })
            .await;
    }

    fn record_cli_session(&self, acp_session_id: &str) {
        let Some(cli) = &self.cli_session else {
            return;
        };
        let kind = cli.kind;
        let Some(session_id) = kind.cli_session_id(acp_session_id) else {
            log::warn!(
                "ACP session id {acp_session_id} does not identify a {kind:?} session, so its \
                 transcript will not be uploaded"
            );
            return;
        };
        let _ = cli.session_id.set(session_id);
    }

    async fn cli_transcript_path(
        &self,
        cli: &CliSession,
        session_id: Uuid,
    ) -> Result<Option<PathBuf>> {
        if let Some(path) = cli.transcript_path.get() {
            return Ok(Some(path.clone()));
        }
        let (kind, root, cwd) = (cli.kind, cli.root.clone(), self.harness_working_dir.clone());
        let path = tokio::task::spawn_blocking(move || kind.locate(&root, &cwd, session_id))
            .await
            .context("CLI transcript discovery task failed")?;
        if let Some(path) = &path {
            let _ = cli.transcript_path.set(path.clone());
        }
        Ok(path)
    }

    /// Captures and uploads the CLI's own session transcript, as its terminal runner does,
    /// including the usage derived from it.
    async fn upload_cli_transcript(
        &self,
        server_id: &ServerConversationToken,
        is_final: bool,
    ) -> Result<UploadedTranscriptUsage> {
        let Some(cli) = &self.cli_session else {
            return Ok(UploadedTranscriptUsage::empty());
        };
        let Some(&session_id) = cli.session_id.get() else {
            return Ok(UploadedTranscriptUsage::empty());
        };
        let capture =
            capture_transcript_with_retry(self.persistence.is_reporting_enabled(), || async {
                let Some(path) = self.cli_transcript_path(cli, session_id).await? else {
                    return Ok(None);
                };
                let identity = self.persistence.begin_capture();
                let (kind, root, cwd) =
                    (cli.kind, cli.root.clone(), self.harness_working_dir.clone());
                tokio::task::spawn_blocking(move || {
                    kind.capture(session_id, &path, &root, &cwd, is_final, identity)
                })
                .await
                .context("CLI transcript capture task failed")?
                .map(Some)
            })
            .await?;
        match capture {
            Some(capture) => {
                log::info!(
                    "Uploading {:?} transcript to conversation {server_id}",
                    cli.kind
                );
                upload_captured_transcript(self.client.as_ref(), server_id, capture).await
            }
            None => Ok(cli.kind.missing_transcript(is_final)),
        }
    }

    /// Binds the native conversation the run drives and gives it a server identity before the
    /// first turn, so every request stream carries the server-known id. A conversation restored
    /// from the server already has one; otherwise a new server conversation is created.
    async fn bind_server_conversation(
        &self,
        restored_conversation_id: Option<AIConversationId>,
        foreground: &ModelSpawner<AgentDriver>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<RunConversation, AgentDriverError> {
        let (local_id, restored) =
            with_ai_controller(foreground, &self.terminal_driver, move |controller, ctx| {
                let local_id = controller
                    .native_prompt_conversation_id()
                    .unwrap_or_else(|| {
                        controller.bind_native_prompt_conversation(restored_conversation_id, ctx)
                    });
                let restored = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&local_id)
                    .and_then(|conversation| {
                        let server_id = conversation.server_conversation_token()?.clone();
                        Some((server_id, conversation.get_root_task_id().to_string()))
                    });
                (local_id, restored)
            })
            .await?;
        if let Some((server_id, root_task_id)) = restored {
            log::info!("Continuing restored ACP conversation {server_id}");
            return Ok(RunConversation {
                local_id,
                server_id,
                restored_root_task_id: Some(root_task_id),
            });
        }

        let server_id = setup_events
            .record_result(SetupStep::ThirdPartyHarnessExternalConversation, async {
                self.client
                    .create_external_conversation(MAA_FORMAT, self.harness)
                    .await
                    .map_err(|error| {
                        report_error!(&error);
                        AgentDriverError::ConfigBuildFailed(error)
                    })
            })
            .await?;
        log::info!("Created ACP conversation {server_id}");
        let token = server_id.as_str().to_owned();
        foreground
            .spawn(move |_, ctx| {
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.set_server_conversation_token_for_conversation_and_persist(
                        local_id, token, ctx,
                    );
                });
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;
        Ok(RunConversation {
            local_id,
            server_id,
            restored_root_task_id: None,
        })
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for AcpHarnessRunner {
    fn harness_name(&self) -> &str {
        &self.launch.program
    }
    #[cfg(test)]
    fn prompt_text_for_test(&self) -> Option<&str> {
        Some(&self.prompt_text)
    }

    fn persistence(&self) -> &HarnessPersistence {
        &self.persistence
    }

    fn save_requests(&self) -> Option<async_channel::Receiver<SavePoint>> {
        Some(self.save_request_rx.clone())
    }

    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<CommandHandle, AgentDriverError> {
        let (background, shell_family, restored_conversation_id): (
            Arc<Background>,
            ShellFamily,
            Option<AIConversationId>,
        ) = foreground
            .spawn(|me, ctx| {
                let shell_family = me
                    .terminal_driver
                    .as_ref(ctx)
                    .active_session_shell_type(ctx)
                    .map(ShellFamily::from)
                    .unwrap_or(ShellFamily::Posix);
                (
                    ctx.background_executor(),
                    shell_family,
                    me.restored_conversation_id,
                )
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;

        let listener = BridgeListener::bind(&self.launch.program, &self.launch.args)
            .map_err(|error| self.setup_failed(format!("{error:#}")))?;
        let command = bridge_command(listener.launch_file(), shell_family)
            .map_err(|error| self.setup_failed(format!("{error:#}")))?;
        let terminal_driver = self.terminal_driver.clone();
        let command_handle = foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.execute_command(&command, ctx))
            })
            .await??
            .await?;

        let handshake_started = instant::Instant::now();
        let (reader, writer) = match listener.accept(HANDSHAKE_TIMEOUT).await {
            Ok(halves) => halves,
            Err(error) => {
                self.interrupt_terminal(foreground).await;
                return Err(self.setup_failed(format!("{error:#}")));
            }
        };
        drop(listener);
        let request_context = Arc::new(AgentRequestContext {
            foreground: foreground.clone(),
            terminal_driver: self.terminal_driver.clone(),
            escape_char: shell_family.escape_char(),
        });
        let (connection, notifications) = AcpConnection::new(
            reader,
            writer,
            agent_request_handler(request_context),
            &background,
        );
        let connection = Arc::new(connection);
        *self.agent.lock() = Some(RunningAgent {
            connection: connection.clone(),
            session_id: None,
        });

        let handshake_connection = connection.clone();
        let handshake = async move {
            let connection = handshake_connection;
            let initialize: InitializeResponse = connection
                .request(
                    protocol::METHOD_INITIALIZE,
                    InitializeParams {
                        protocol_version: PROTOCOL_VERSION,
                        client_capabilities: ClientCapabilities {
                            fs: FsCapabilities {
                                read_text_file: true,
                                write_text_file: true,
                            },
                            terminal: false,
                        },
                        client_info: ClientInfo {
                            name: "warp".to_owned(),
                            title: "Warp".to_owned(),
                            version: ChannelState::app_version()
                                .unwrap_or(env!("CARGO_PKG_VERSION"))
                                .to_owned(),
                        },
                    },
                )
                .await
                .context("initialize failed")?;
            let agent_description = initialize
                .agent_info
                .as_ref()
                .map(|info| format!("{} {}", info.name, info.version))
                .unwrap_or_else(|| "unknown".to_owned());
            // The agent answers with the highest version it supports that is no newer than
            // ours; anything else means the two sides would not agree on the wire format.
            if initialize.protocol_version != PROTOCOL_VERSION {
                anyhow::bail!(
                    "agent {agent_description} speaks ACP protocol v{} but this client requires v{PROTOCOL_VERSION}",
                    initialize.protocol_version
                );
            }
            let capabilities = &initialize.agent_capabilities;
            let extra_capabilities = capabilities.extra.keys().cloned().collect::<Vec<_>>();
            log::info!(
                "ACP agent initialized: protocol v{} agent={agent_description} auth_methods={} \
                 load_session={} prompt_capabilities={:?} other_capabilities={extra_capabilities:?}",
                initialize.protocol_version,
                initialize.auth_methods.len(),
                capabilities.load_session,
                capabilities.prompt_capabilities,
            );
            self.attachments
                .set_prompt_capabilities(capabilities.prompt_capabilities);
            let mut session: NewSessionResponse = connection
                .request(
                    protocol::METHOD_SESSION_NEW,
                    NewSessionParams {
                        cwd: self.harness_working_dir.display().to_string(),
                        mcp_servers: self.mcp_servers.clone(),
                    },
                )
                .await
                .context("session/new failed")?;
            configure_model_and_thinking_through_acp(
                &connection,
                &mut session,
                self.model_config.as_ref(),
            )
            .await?;
            let suppressed_startup_message = match self.harness {
                Harness::Pi => pi::startup_message(&session.meta),
                Harness::Claude
                | Harness::Codex
                | Harness::Gemini
                | Harness::Oz
                | Harness::OpenCode
                | Harness::Unknown => None,
            };
            Ok::<_, anyhow::Error>((session.session_id, suppressed_startup_message))
        }
        .fuse();
        pin_mut!(handshake);
        let handshake_budget = HANDSHAKE_TIMEOUT.saturating_sub(handshake_started.elapsed());
        let session_id = select! {
            result = handshake => result,
            _ = Timer::after(handshake_budget).fuse() => {
                Err(anyhow!("the agent did not complete its handshake within {HANDSHAKE_TIMEOUT:?}"))
            }
        };
        let (session_id, suppressed_startup_message) = match session_id {
            Ok(session) => session,
            Err(error) => {
                connection.close().await;
                self.interrupt_terminal(foreground).await;
                // Code -32000 is the agent refusing the session for lack of credentials, which
                // is the run's configuration rather than a defect worth paging on.
                if let Some(rpc_error) = error
                    .downcast_ref::<RpcError>()
                    .filter(|error| error.code == -32000)
                {
                    return Err(AgentDriverError::HarnessAuthCheckFailed {
                        harness: self.harness.to_string(),
                        detail: rpc_error.message.clone(),
                    });
                }
                let reason = format!("{error:#}");
                report_error!(
                    error.context("ACP handshake failed"),
                    extra: { "harness" => %self.harness }
                );
                return Err(self.setup_failed(reason));
            }
        };
        log::info!("ACP session {session_id} created");
        if let Some(agent) = self.agent.lock().as_mut() {
            agent.session_id = Some(session_id.clone());
        }
        self.record_cli_session(&session_id);

        let conversation = match self
            .bind_server_conversation(restored_conversation_id, foreground, setup_events)
            .await
        {
            Ok(conversation) => conversation,
            Err(error) => {
                connection.close().await;
                self.interrupt_terminal(foreground).await;
                return Err(error);
            }
        };
        let restored_root_task_id = conversation.restored_root_task_id.clone();
        let local_conversation_id = conversation.local_id;
        let _ = self.conversation.set(conversation);

        let run_id = self.task_id.map(|id| id.to_string());
        let host = Arc::new(DriverTurnHost {
            foreground: foreground.clone(),
            terminal_driver: self.terminal_driver.clone(),
            run_id,
            conversation_id: local_conversation_id,
        });
        let turn = match host.begin_turn(None).await {
            Ok(turn) => turn,
            Err(error) => {
                connection.close().await;
                self.interrupt_terminal(foreground).await;
                return Err(AgentDriverError::HarnessSetupFailed {
                    harness: "acp".to_owned(),
                    reason: format!("{error:#}"),
                });
            }
        };
        let (follow_up_tx, follow_up_rx) = async_channel::unbounded::<ExternalHarnessPrompt>();
        let (cancel_tx, cancel_rx) = async_channel::unbounded::<ExternalHarnessCancel>();
        with_ai_controller(foreground, &self.terminal_driver, move |controller, ctx| {
            controller.set_external_harness_prompt_sink(follow_up_tx, ctx);
            controller.set_external_harness_cancel_sink(cancel_tx);
        })
        .await?;

        let (mut mapper, opening_actions) = match restored_root_task_id {
            Some(root_task_id) => {
                let mapper = AcpTurnMapper::new(
                    root_task_id,
                    turn.request_id.clone(),
                    self.mcp_server_identities.clone(),
                );
                let actions = vec![mapper.user_query_action(&self.user_prompt, None)];
                (mapper, actions)
            }
            None => {
                let mapper = AcpTurnMapper::new(
                    Uuid::new_v4().to_string(),
                    turn.request_id.clone(),
                    self.mcp_server_identities.clone(),
                );
                let actions = mapper.initial_actions(&self.user_prompt);
                (mapper, actions)
            }
        };
        apply_actions(
            foreground,
            &self.terminal_driver,
            &turn.stream_id,
            opening_actions,
        )
        .await?;

        let turn_driver = TurnDriver {
            connection,
            notifications,
            follow_ups: Some(follow_up_rx),
            cancels: Some(cancel_rx),
            host,
            state: self.turn_state.clone(),
            turn,
            session_id,
            suppressed_startup_message,
            prompt_text: self.prompt_text.clone(),
            initial_attachments: self.initial_attachments.clone(),
            attachments: self.attachments.clone(),
            boundary_snapshot: self.boundary_snapshot.clone(),
            save_requests: self.save_request_tx.clone(),
            queued: VecDeque::new(),
            cancel_deadline: STEER_CANCEL_DEADLINE,
            timer_factory: Arc::new(cancel_deadline),
        };
        background
            .spawn(async move { turn_driver.run(&mut mapper).await })
            .detach();

        // The agent's block is the command the driver waits on. The driver decides when the run
        // is over (from the native conversation's status and its idle windows) and ends it
        // through `exit`; the turn driver just keeps the session serviceable until then.
        Ok(command_handle)
    }

    async fn save_conversation(
        &self,
        save_point: SavePoint,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> PersistenceOutcome {
        let Some(conversation) = self.conversation.get().cloned() else {
            return match save_point {
                SavePoint::Final => PersistenceOutcome::failed(anyhow!(
                    "Cannot finalize ACP persistence before the conversation is bound"
                )),
                SavePoint::PostTurn | SavePoint::Periodic => PersistenceOutcome::skipped(),
            };
        };
        let conversation_data = async {
            let snapshot = match save_point {
                // Mid-stream the conversation holds a request that has not finished; the
                // boundary that closes it raises the save that captures it.
                SavePoint::Periodic => return Ok(()),
                // Coalesced save points share one snapshot: the newest boundary. A request whose
                // snapshot a previous save already took has nothing left to upload.
                SavePoint::PostTurn => match self.boundary_snapshot.lock().take() {
                    Some(snapshot) => snapshot,
                    None => return Ok(()),
                },
                // Nothing streams after the harness exits, so the live conversation is boundary
                // state, including a turn that the exit cut short.
                SavePoint::Final => {
                    match capture_conversation_snapshot(conversation.local_id, foreground).await? {
                        Some(snapshot) => snapshot,
                        None => return Ok(()),
                    }
                }
            };
            upload_conversation_snapshot(self.client.as_ref(), &conversation.server_id, &snapshot)
                .await
        };
        save_transcript_and_conversation_data(
            self.upload_cli_transcript(
                &conversation.server_id,
                matches!(save_point, SavePoint::Final),
            ),
            conversation_data,
        )
        .await
    }

    /// Cancels any turn in flight and waits up to [`EXIT_CANCEL_DEADLINE`] for the agent to
    /// answer it before closing the connection, so an agent that persists its session at the end
    /// of a turn gets to. The driver's exit ladder only starts once this returns.
    async fn exit(&self, _foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        let Some((connection, session_id)) = self.running_agent() else {
            return Ok(());
        };
        if let Some(session_id) = session_id {
            connection
                .notify(
                    protocol::METHOD_SESSION_CANCEL,
                    CancelParams {
                        session_id: session_id.clone(),
                    },
                )
                .await?;
            connection
                .cancel_pending_permission_requests(&session_id)
                .await;
            let idle = self.turn_state.wait_until_idle().fuse();
            pin_mut!(idle);
            select! {
                _ = idle => {}
                _ = Timer::after(EXIT_CANCEL_DEADLINE).fuse() => log::warn!(
                    "ACP agent did not answer session/cancel within {EXIT_CANCEL_DEADLINE:?} \
                     on exit; closing anyway"
                ),
            }
        }
        connection.close().await;
        Ok(())
    }

    fn is_settling_turn(&self) -> bool {
        self.turn_state.is_settling()
    }

    /// Interrupts the agent's block; the driver's exit ladder force-kills the process group if
    /// that is not enough.
    async fn exit_followup(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        if self.running_agent().is_some() {
            self.interrupt_terminal(foreground).await;
        }
        Ok(())
    }

    async fn cleanup(
        &self,
        _cleanup_disposition: HarnessCleanupDisposition,
        _foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        self.agent.lock().take();
        Ok(())
    }
}

/// Runs `f` against the driver terminal's AI controller on the foreground thread, which owns
/// every entity involved. The terminal view handle is cloned only for the synchronous callback
/// (the borrow of `ctx` through the driver must end before the view can be updated) and is
/// released when it returns.
async fn with_ai_controller<T: Send + 'static>(
    foreground: &ModelSpawner<AgentDriver>,
    terminal_driver: &ModelHandle<TerminalDriver>,
    f: impl FnOnce(&mut BlocklistAIController, &mut ModelContext<BlocklistAIController>) -> T
    + Send
    + 'static,
) -> Result<T, AgentDriverError> {
    let terminal_driver = terminal_driver.clone();
    foreground
        .spawn(move |_, ctx| {
            let terminal = terminal_driver.as_ref(ctx).terminal_view().clone();
            terminal.update(ctx, |terminal, ctx| terminal.ai_controller().update(ctx, f))
        })
        .await
        .map_err(|_| AgentDriverError::InvalidRuntimeState)
}

/// Captures the native conversation on the foreground thread as encoded `ConversationData`.
/// Returns `None` when the conversation has no tasks (its only task was torn down by a failed
/// turn): there is nothing to upload, and the server rejects an empty snapshot.
async fn capture_conversation_snapshot(
    local_id: AIConversationId,
    foreground: &ModelSpawner<AgentDriver>,
) -> Result<Option<Vec<u8>>> {
    foreground
        .spawn(move |_, ctx| {
            let data = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&local_id)
                .map(AIConversation::to_conversation_data)?;
            Some((!data.tasks.is_empty()).then(|| data.encode_to_vec()))
        })
        .await
        .map_err(|_| anyhow!("Agent driver dropped before the ACP conversation was captured"))?
        .ok_or_else(|| anyhow!("ACP conversation {local_id:?} is no longer loaded"))
}

/// Replaces the server's copy of the conversation with the encoded `snapshot`.
async fn upload_conversation_snapshot(
    client: &dyn HarnessSupportClient,
    server_conversation_id: &ServerConversationToken,
    snapshot: &[u8],
) -> Result<()> {
    client
        .upload_conversation_data(server_conversation_id, snapshot)
        .await
        .with_context(|| {
            format!("Failed to upload ACP conversation data to {server_conversation_id}")
        })
}

/// The turn driver's view of the driver: the bound native conversation, reached on the
/// foreground thread, and the terminal the agent runs in.
struct DriverTurnHost {
    foreground: ModelSpawner<AgentDriver>,
    terminal_driver: ModelHandle<TerminalDriver>,
    run_id: Option<String>,
    conversation_id: AIConversationId,
}

#[async_trait]
impl TurnHost for DriverTurnHost {
    /// `initiator` names the shared-session participant whose prompt opens the stream; `None`
    /// keeps the previous one, as the streams a tool round trip chains together all answer the
    /// same prompt.
    async fn begin_turn(&self, initiator: Option<ParticipantId>) -> Result<ExternalHarnessTurn> {
        let run_id = self.run_id.clone();
        with_ai_controller(
            &self.foreground,
            &self.terminal_driver,
            move |controller, ctx| {
                let conversation_id = controller
                    .native_prompt_conversation_id()
                    .unwrap_or_else(|| controller.bind_native_prompt_conversation(None, ctx));
                if let Some(initiator) = initiator {
                    controller.set_current_response_initiator(initiator);
                }
                controller.begin_external_harness_turn(conversation_id, run_id, ctx)
            },
        )
        .await
        .map_err(|error| anyhow!("{error}"))?
        .map_err(|error| anyhow!("could not open conversation turn: {error}"))
    }

    async fn apply_actions(
        &self,
        stream_id: &ResponseStreamId,
        actions: Vec<ClientAction>,
    ) -> Result<()> {
        apply_actions(&self.foreground, &self.terminal_driver, stream_id, actions)
            .await
            .map_err(|error| anyhow!("{error}"))
    }

    async fn finish_turn(
        &self,
        stream_id: ResponseStreamId,
        finished: StreamFinished,
    ) -> Result<()> {
        apply_event(
            &self.foreground,
            &self.terminal_driver,
            stream_id,
            response_event::Type::Finished(finished),
        )
        .await
        .map_err(|error| anyhow!("{error}"))
    }

    async fn cancel_turn(&self, stream_id: ResponseStreamId, by: TurnCancellation) {
        let reason = match by {
            TurnCancellation::FollowUp => CancellationReason::FollowUpSubmitted {
                is_for_same_conversation: true,
            },
            TurnCancellation::Interrupt => CancellationReason::ManuallyCancelled,
        };
        let cancelled = with_ai_controller(
            &self.foreground,
            &self.terminal_driver,
            move |controller, ctx| {
                controller.cancel_external_harness_turn(&stream_id, reason, ctx);
            },
        )
        .await;
        if cancelled.is_err() {
            log::warn!("Agent driver dropped before the cancelled ACP turn could be finished");
        }
    }

    async fn mark_conversation_cancelled(&self) {
        let conversation_id = self.conversation_id;
        let marked = with_ai_controller(
            &self.foreground,
            &self.terminal_driver,
            move |controller, ctx| {
                controller.mark_external_harness_conversation_cancelled(conversation_id, ctx);
            },
        )
        .await;
        if marked.is_err() {
            log::warn!(
                "Agent driver dropped before the ACP conversation could be marked cancelled"
            );
        }
    }

    async fn capture_snapshot(&self) -> Result<Option<Vec<u8>>> {
        capture_conversation_snapshot(self.conversation_id, &self.foreground).await
    }

    async fn interrupt_terminal(&self) {
        let terminal_driver = self.terminal_driver.clone();
        let _ = self
            .foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.send_interrupt_to_pty(ctx));
            })
            .await;
    }

    async fn show_queued_prompt(&self, prompt: &ExternalHarnessPrompt) -> Option<QueuedQueryId> {
        let conversation_id = self.conversation_id;
        let prompt = prompt.clone();
        with_ai_controller(
            &self.foreground,
            &self.terminal_driver,
            move |controller, ctx| {
                controller.show_external_harness_prompt_queued(conversation_id, &prompt, ctx)
            },
        )
        .await
        .ok()
    }

    async fn hide_queued_prompt(&self, query_id: QueuedQueryId) {
        let conversation_id = self.conversation_id;
        let _ = with_ai_controller(
            &self.foreground,
            &self.terminal_driver,
            move |controller, ctx| {
                controller.hide_external_harness_prompt_queued(conversation_id, query_id, ctx);
            },
        )
        .await;
    }

    async fn turn_driver_stopped(&self) {
        let _ = with_ai_controller(&self.foreground, &self.terminal_driver, |controller, _| {
            controller.release_external_harness_sinks();
        })
        .await;
    }
}

async fn apply_actions(
    foreground: &ModelSpawner<AgentDriver>,
    terminal_driver: &ModelHandle<TerminalDriver>,
    stream_id: &ResponseStreamId,
    actions: Vec<ClientAction>,
) -> Result<(), AgentDriverError> {
    if actions.is_empty() {
        return Ok(());
    }
    apply_event(
        foreground,
        terminal_driver,
        stream_id.clone(),
        response_event::Type::ClientActions(response_event::ClientActions { actions }),
    )
    .await
}

async fn apply_event(
    foreground: &ModelSpawner<AgentDriver>,
    terminal_driver: &ModelHandle<TerminalDriver>,
    stream_id: ResponseStreamId,
    event: response_event::Type,
) -> Result<(), AgentDriverError> {
    with_ai_controller(foreground, terminal_driver, move |controller, ctx| {
        controller.apply_external_harness_event(
            &stream_id,
            ResponseEvent {
                r#type: Some(event),
            },
            ctx,
        );
    })
    .await
}

/// What answering the agent's requests needs from the driver: a way onto the foreground thread
/// to consult the permission model, and the shell whose quoting rules the policy parses
/// commands with.
struct AgentRequestContext {
    foreground: ModelSpawner<AgentDriver>,
    terminal_driver: ModelHandle<TerminalDriver>,
    escape_char: EscapeChar,
}

impl AgentRequestContext {
    async fn evaluate(&self, request: PolicyRequest) -> Result<PolicyDecision, RpcError> {
        let terminal_driver = self.terminal_driver.clone();
        let escape_char = self.escape_char;
        self.foreground
            .spawn(move |_, ctx| {
                let terminal = terminal_driver.as_ref(ctx).terminal_view().clone();
                let controller = terminal.as_ref(ctx).ai_controller().clone();
                request.evaluate(controller.as_ref(ctx), terminal.id(), escape_char, ctx)
            })
            .await
            .map_err(|_| RpcError::internal("the agent driver is no longer running"))
    }
}

/// Answers the requests an ACP agent may make of its client. Runs on the connection's read
/// loop (a background runtime thread), so file system work is moved to blocking threads to keep
/// protocol reads flowing.
fn agent_request_handler(context: Arc<AgentRequestContext>) -> connection::AgentRequestHandler {
    Arc::new(move |method, params| {
        let context = context.clone();
        match method {
            protocol::METHOD_REQUEST_PERMISSION => {
                async move { answer_permission_request(&context, params).await }.boxed()
            }
            protocol::METHOD_FS_READ_TEXT_FILE => blocking(move || read_text_file(params)).boxed(),
            protocol::METHOD_FS_WRITE_TEXT_FILE => {
                async move { write_text_file(&context, params).await }.boxed()
            }
            other => future::ready(Err(RpcError::method_not_found(other))).boxed(),
        }
    })
}

async fn blocking(
    work: impl FnOnce() -> Result<Value, RpcError> + Send + 'static,
) -> Result<Value, RpcError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(RpcError::internal)?
}

/// Grants the request with the broadest "allow" option unless Warp's permission policy refuses
/// it outright (see [`policy`]), in which case the narrowest "reject" option is chosen.
async fn answer_permission_request(
    context: &AgentRequestContext,
    params: Value,
) -> Result<Value, RpcError> {
    let params: RequestPermissionParams =
        serde_json::from_value(params).map_err(RpcError::internal)?;
    let decision = match PolicyRequest::for_tool_call(&params.tool_call) {
        Some(request) => context.evaluate(request).await?,
        None => PolicyDecision::Allow,
    };
    let preferred_kinds: &[PermissionOptionKind] = match &decision {
        PolicyDecision::Allow => &[
            PermissionOptionKind::AllowAlways,
            PermissionOptionKind::AllowOnce,
        ],
        PolicyDecision::Deny { .. } => &[
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ],
    };
    let chosen = preferred_kinds
        .iter()
        .find_map(|kind| params.options.iter().find(|option| option.kind == *kind));
    let outcome = match (&decision, chosen) {
        (PolicyDecision::Allow, Some(option)) => {
            log::info!(
                "Auto-approving ACP permission request with `{}`",
                option.name
            );
            PermissionOutcome::Selected {
                option_id: option.option_id.clone(),
            }
        }
        // Agents that offer no allow option are answered with whatever they did offer, as
        // before; anything else would wedge an unattended run.
        (PolicyDecision::Allow, None) => match params.options.first() {
            Some(option) => PermissionOutcome::Selected {
                option_id: option.option_id.clone(),
            },
            None => PermissionOutcome::Cancelled,
        },
        (PolicyDecision::Deny { reason }, chosen) => {
            // The title is agent-authored and can quote commands or paths; keep it out of the
            // reported breadcrumb.
            safe_warn!(
                safe: ("Refusing ACP permission request: {reason}"),
                full: (
                    "Refusing ACP permission request for `{}`: {reason}",
                    params.tool_call.title.as_deref().unwrap_or("tool call")
                )
            );
            match chosen {
                Some(option) => PermissionOutcome::Selected {
                    option_id: option.option_id.clone(),
                },
                None => PermissionOutcome::Cancelled,
            }
        }
    };
    serde_json::to_value(RequestPermissionResponse { outcome }).map_err(RpcError::internal)
}

fn read_text_file(params: Value) -> Result<Value, RpcError> {
    let params: ReadTextFileParams = serde_json::from_value(params).map_err(RpcError::internal)?;
    let content = std::fs::read_to_string(&params.path)
        .with_context(|| format!("reading {}", params.path))
        .map_err(RpcError::internal)?;
    let content = match (params.line, params.limit) {
        (None, None) => content,
        (line, limit) => {
            let start = line.unwrap_or(1).saturating_sub(1) as usize;
            let lines = content.lines().skip(start);
            match limit {
                Some(limit) => lines.take(limit as usize).collect::<Vec<_>>().join("\n"),
                None => lines.collect::<Vec<_>>().join("\n"),
            }
        }
    };
    serde_json::to_value(ReadTextFileResponse { content }).map_err(RpcError::internal)
}

async fn write_text_file(context: &AgentRequestContext, params: Value) -> Result<Value, RpcError> {
    let params: WriteTextFileParams = serde_json::from_value(params).map_err(RpcError::internal)?;
    if let PolicyDecision::Deny { reason } =
        context.evaluate(PolicyRequest::write(&params.path)).await?
    {
        log::warn!("Refusing ACP fs/write_text_file: {reason}");
        return Err(RpcError::internal(format!(
            "writing {} is not permitted: {reason}",
            params.path
        )));
    }
    blocking(move || {
        if let Some(parent) = Path::new(&params.path).parent() {
            std::fs::create_dir_all(parent).map_err(RpcError::internal)?;
        }
        std::fs::write(&params.path, params.content)
            .with_context(|| format!("writing {}", params.path))
            .map_err(RpcError::internal)?;
        Ok(Value::Null)
    })
    .await
}

/// The identity of an MCP server as its tool calls carry it. No server request sees these calls,
/// so the display name warp-server would otherwise fill in is set here.
fn tool_call_server_identity(name: &str, warp_id: Option<&str>) -> McpServerIdentity {
    McpServerIdentity {
        display_name: name.to_owned(),
        ..warp_id.and_then(mcp_server_identity).unwrap_or_default()
    }
}

fn mcp_server_for_acp(name: &str, server: &JSONMCPServer) -> McpServer {
    match &server.transport_type {
        JSONTransportType::CLIServer {
            command, args, env, ..
        } => McpServer::Stdio {
            name: name.to_owned(),
            command: command.clone(),
            args: args.clone(),
            env: env
                .iter()
                .map(|(name, value)| protocol::EnvVariable {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        },
        JSONTransportType::SSEServer { url, headers } => McpServer::Remote {
            kind: "http",
            name: name.to_owned(),
            url: url.clone(),
            headers: headers
                .iter()
                .map(|(name, value)| protocol::HttpHeader {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        },
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
