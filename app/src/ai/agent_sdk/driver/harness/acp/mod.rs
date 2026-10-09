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
mod connection;
mod launch;
mod mapping;
mod policy;
mod protocol;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
pub(crate) use bridge::run_bridge;
use futures::{FutureExt, future, pin_mut, select};
pub(crate) use launch::AcpLaunchSpec;
use parking_lot::Mutex;
use serde_json::Value;
use uuid::Uuid;
use warp_cli::agent::{Harness, HarnessTransport};
use warp_core::channel::ChannelState;
use warp_core::safe_warn;
use warp_errors::report_error;
use warp_managed_secrets::ManagedSecretValue;
use warp_multi_agent_api::response_event::{StreamFinished, stream_finished};
use warp_multi_agent_api::{ClientAction, ResponseEvent, response_event};
use warp_util::path::{EscapeChar, ShellFamily};
use warpui::r#async::Timer;
use warpui::r#async::executor::Background;
use warpui::{ModelContext, ModelHandle, ModelSpawner, SingletonEntity};

use self::attachments::AttachmentResolver;
use self::bridge::{BridgeListener, bridge_command};
use self::connection::{AcpConnection, InboundNotification, RpcError};
use self::mapping::{AcpTurnMapper, TurnEvent};
use self::policy::{PolicyDecision, PolicyRequest};
use self::protocol::{
    CancelParams, ClientCapabilities, ClientInfo, ContentBlock, FsCapabilities, InitializeParams,
    InitializeResponse, McpServer, NewSessionParams, NewSessionResponse, PROTOCOL_VERSION,
    PermissionOptionKind, PermissionOutcome, PromptParams, PromptResponse, ReadTextFileParams,
    ReadTextFileResponse, RequestPermissionParams, RequestPermissionResponse, SessionUpdateParams,
    StopReason, WriteTextFileParams,
};
use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::claude_code::prepare_claude_environment_config;
use super::codex::{prepare_codex_environment_config, publish_skills_for_codex};
use super::gemini::prepare_gemini_environment_config;
use super::harness_persistence::{HarnessPersistence, PersistenceOutcome};
use super::{
    HarnessCleanupDisposition, HarnessKind, HarnessRunner, JSONMCPServer, ResumePayload, SavePoint,
    ThirdPartyHarness, harness_kind, validate_cli_installed,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::attachment_utils::attachments_download_dir;
use crate::ai::blocklist::{
    BlocklistAIController, BlocklistAIHistoryModel, ExternalHarnessPrompt, ExternalHarnessTurn,
    ResponseStreamId,
};
use crate::ai::mcp::JSONTransportType;
use crate::server::server_api::ServerApi;
use crate::server::server_api::harness_support::HarnessSupportClient;
use crate::terminal::CLIAgent;

/// Format slug for the server conversation backing an ACP-driven run. The server stores its
/// history as client-reduced native conversation data rather than reducing a harness transcript.
const ACP_CONVERSATION_FORMAT: &str = "acp_v0";

/// Bound on the agent connecting back through the bridge plus its `initialize` and
/// `session/new` handshakes.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// A third-party harness driven over ACP. `harness` is the harness identity (which decides
/// bootstrap, auth checks, and server-side bookkeeping); `launch` is the ACP adapter process.
pub(crate) struct AcpHarness {
    harness: Harness,
    launch: AcpLaunchSpec,
}

impl AcpHarness {
    pub(crate) fn new(harness: Harness, launch: AcpLaunchSpec) -> Self {
        Self { harness, launch }
    }

    /// The terminal-based harness with the same identity, whose per-harness knowledge (install
    /// docs, auth checks) applies regardless of transport.
    fn pty_harness(&self) -> Option<Box<dyn ThirdPartyHarness>> {
        match harness_kind(self.harness, HarnessTransport::Pty) {
            Ok(HarnessKind::ThirdParty(harness)) => Some(harness),
            Ok(HarnessKind::Oz | HarnessKind::Unsupported(_)) | Err(_) => None,
        }
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
        resolved_secrets: &HashMap<String, ManagedSecretValue>,
        third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<()> {
        match self.harness {
            Harness::Claude => prepare_claude_environment_config(
                workspace_root,
                harness_working_dir,
                resolved_env_vars,
                skill_dirs,
            ),
            // MCP servers are handed to the agent in `session/new` rather than written into
            // `config.toml`, so they are not registered twice.
            Harness::Codex => {
                prepare_codex_environment_config(
                    harness_working_dir,
                    system_prompt,
                    resolved_env_vars,
                    resolved_secrets,
                    &HashMap::new(),
                    third_party_harness_model_config,
                )?;
                publish_skills_for_codex(workspace_root, harness_working_dir, skill_dirs);
                Ok(())
            }
            Harness::Gemini => {
                prepare_gemini_environment_config(harness_working_dir, system_prompt)
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
            Harness::Oz | Harness::OpenCode | Harness::Unknown => CLIAgent::Unknown,
        }
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        self.pty_harness()
            .and_then(|harness| harness.install_docs_url())
    }

    fn validate(&self) -> Result<(), AgentDriverError> {
        validate_cli_installed(&self.launch.program, self.install_docs_url())
    }

    fn auth_check_command(&self) -> Option<String> {
        self.pty_harness()
            .and_then(|harness| harness.auth_check_command())
    }

    /// The agent's stderr lands in its terminal block, so the CLI's failure signatures apply.
    fn runtime_error_patterns(&self) -> &'static [&'static str] {
        self.pty_harness()
            .map(|harness| harness.runtime_error_patterns())
            .unwrap_or(&[])
    }

    fn drives_cli_agent_session(&self) -> bool {
        false
    }

    /// Only reached when the conversation has no stored native history to restore; the run then
    /// continues the same server conversation from an empty native one. The agent's own session
    /// is never resumed.
    async fn fetch_resume_payload(
        &self,
        conversation_id: &ServerConversationToken,
        _harness_support_client: Arc<dyn HarnessSupportClient>,
    ) -> Result<Option<ResumePayload>, AgentDriverError> {
        Ok(Some(ResumePayload::Acp(conversation_id.clone())))
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
        resolved_secrets: &HashMap<String, ManagedSecretValue>,
        resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
        third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        self.prepare_harness_environment(
            system_prompt,
            workspace_root,
            harness_working_dir,
            resolved_env_vars,
            skill_dirs,
            resolved_secrets,
            third_party_harness_model_config,
        )
        .map_err(|error| AgentDriverError::HarnessConfigSetupFailed {
            harness: self.harness.to_string(),
            error,
        })?;

        // ACP has no system-prompt channel of its own, so every server-composed block that the
        // harness bootstrap did not already place is folded into the user turn ahead of the
        // prompt itself.
        let system_prompt_in_turn = match self.harness {
            Harness::Claude | Harness::Oz | Harness::OpenCode | Harness::Unknown => system_prompt,
            Harness::Codex | Harness::Gemini => None,
        };
        let prompt_text = [
            resumption_prompt,
            context,
            system_prompt_in_turn,
            Some(prompt),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        let attachments = AttachmentResolver::new(
            server_api.clone(),
            server_api.owned_http_client(),
            task_id,
            attachments_download_dir(harness_working_dir),
        );
        let preexisting_conversation_id = match resume {
            Some(ResumePayload::Acp(conversation_id)) => Some(conversation_id),
            Some(ResumePayload::Claude(_) | ResumePayload::Codex(_)) => {
                log::error!("ACP harness given a resume payload for a terminal-driven harness");
                return Err(AgentDriverError::InvalidRuntimeState);
            }
            None => None,
        };
        let (save_request_tx, save_request_rx) = async_channel::unbounded();
        Ok(Box::new(AcpHarnessRunner {
            harness: self.harness,
            launch: self.launch.clone(),
            user_prompt: prompt.to_owned(),
            prompt_text,
            harness_working_dir: harness_working_dir.to_path_buf(),
            task_id,
            client: server_api,
            terminal_driver,
            attachments: Arc::new(attachments),
            mcp_servers: resolved_mcp_servers
                .iter()
                .map(|(name, server)| mcp_server_for_acp(name, server))
                .collect(),
            preexisting_conversation_id,
            conversation: OnceLock::new(),
            persistence: HarnessPersistence::default(),
            save_request_tx,
            save_request_rx,
            agent: Mutex::new(None),
        }))
    }
}

/// The protocol session with the agent running in the terminal.
struct RunningAgent {
    connection: Arc<AcpConnection>,
    session_id: Option<String>,
}

/// The native conversation the run drives and the server conversation that persists it.
#[derive(Clone)]
struct RunConversation {
    server_id: ServerConversationToken,
    local_id: AIConversationId,
    /// The root task of a conversation restored from the server, which the run continues
    /// instead of creating a new root.
    restored_root_task_id: Option<String>,
}

/// What the bound native conversation already carries before the run's first turn.
struct BoundConversation {
    local_id: AIConversationId,
    server_id: Option<ServerConversationToken>,
    restored_root_task_id: Option<String>,
}

pub(crate) struct AcpHarnessRunner {
    harness: Harness,
    launch: AcpLaunchSpec,
    /// The prompt as the user wrote it, echoed into the conversation.
    user_prompt: String,
    /// The full text sent to the agent, including server-composed context.
    prompt_text: String,
    harness_working_dir: PathBuf,
    task_id: Option<AmbientAgentTaskId>,
    client: Arc<dyn HarnessSupportClient>,
    terminal_driver: ModelHandle<TerminalDriver>,
    attachments: Arc<AttachmentResolver>,
    mcp_servers: Vec<McpServer>,
    /// The server conversation a resumed run continues when it had no stored history to
    /// restore.
    preexisting_conversation_id: Option<ServerConversationToken>,
    /// Set once the server conversation is bound to the native conversation, before any turn.
    conversation: OnceLock<RunConversation>,
    persistence: HarnessPersistence,
    /// Save points raised by the turn driver at request-stream and turn boundaries.
    save_request_tx: async_channel::Sender<SavePoint>,
    save_request_rx: async_channel::Receiver<SavePoint>,
    agent: Mutex<Option<RunningAgent>>,
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

    /// Binds the native conversation the run drives and gives it a server identity before the
    /// first turn, so every request stream carries the server-known id. A conversation restored
    /// from the server keeps its own; otherwise the resumed run's id is reused or a new server
    /// conversation is created.
    async fn bind_server_conversation(
        &self,
        restored_conversation_id: Option<AIConversationId>,
        foreground: &ModelSpawner<AgentDriver>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<RunConversation, AgentDriverError> {
        let bound =
            with_ai_controller(foreground, &self.terminal_driver, move |controller, ctx| {
                let local_id = controller
                    .native_prompt_conversation_id()
                    .unwrap_or_else(|| {
                        controller.bind_native_prompt_conversation(restored_conversation_id, ctx)
                    });
                let conversation = BlocklistAIHistoryModel::as_ref(ctx).conversation(&local_id);
                BoundConversation {
                    local_id,
                    server_id: conversation
                        .and_then(|conversation| conversation.server_conversation_token().cloned()),
                    restored_root_task_id: conversation
                        .filter(|conversation| {
                            conversation
                                .get_root_task()
                                .is_some_and(|task| task.source().is_some())
                        })
                        .map(|conversation| conversation.get_root_task_id().to_string()),
                }
            })
            .await?;
        if let Some(server_id) = bound.server_id {
            log::info!("Continuing restored ACP conversation {server_id}");
            return Ok(RunConversation {
                server_id,
                local_id: bound.local_id,
                restored_root_task_id: bound.restored_root_task_id,
            });
        }

        let server_id = match &self.preexisting_conversation_id {
            Some(id) => {
                log::info!("Resuming ACP conversation {id} without stored history");
                id.clone()
            }
            None => {
                let id = setup_events
                    .record_result(SetupStep::ThirdPartyHarnessExternalConversation, async {
                        self.client
                            .create_external_conversation(
                                ACP_CONVERSATION_FORMAT,
                                Some(self.harness),
                            )
                            .await
                            .map_err(|error| {
                                report_error!(&error);
                                AgentDriverError::ConfigBuildFailed(error)
                            })
                    })
                    .await?;
                log::info!("Created ACP conversation {id}");
                id
            }
        };
        let token = server_id.as_str().to_owned();
        let local_id = bound.local_id;
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
            server_id,
            local_id,
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
            log::info!(
                "ACP agent initialized: protocol v{} agent={agent_description} auth_methods={}",
                initialize.protocol_version,
                initialize.auth_methods.len(),
            );
            let session: NewSessionResponse = connection
                .request(
                    protocol::METHOD_SESSION_NEW,
                    NewSessionParams {
                        cwd: self.harness_working_dir.display().to_string(),
                        mcp_servers: self.mcp_servers.clone(),
                    },
                )
                .await
                .context("session/new failed")?;
            Ok::<_, anyhow::Error>(session.session_id)
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
        let session_id = match session_id {
            Ok(session_id) => session_id,
            Err(error) => {
                connection.close().await;
                self.interrupt_terminal(foreground).await;
                return Err(self.setup_failed(format!("{error:#}")));
            }
        };
        log::info!("ACP session {session_id} created");
        if let Some(agent) = self.agent.lock().as_mut() {
            agent.session_id = Some(session_id.clone());
        }

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
        let _ = self.conversation.set(conversation);

        let run_id = self.task_id.map(|id| id.to_string());
        let turn = begin_turn(foreground, &self.terminal_driver, run_id.clone()).await?;
        let (follow_up_tx, follow_up_rx) = async_channel::unbounded::<ExternalHarnessPrompt>();
        with_ai_controller(foreground, &self.terminal_driver, move |controller, _| {
            controller.set_external_harness_prompt_sink(follow_up_tx);
        })
        .await?;

        let (mut mapper, opening_actions) = match restored_root_task_id {
            Some(root_task_id) => {
                let mapper = AcpTurnMapper::new(root_task_id, turn.request_id.clone());
                let actions = vec![mapper.user_query_action(&self.user_prompt)];
                (mapper, actions)
            }
            None => {
                let mapper =
                    AcpTurnMapper::new(Uuid::new_v4().to_string(), turn.request_id.clone());
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
            follow_ups: follow_up_rx,
            foreground: foreground.clone(),
            terminal_driver: self.terminal_driver.clone(),
            run_id,
            turn: Mutex::new(turn),
            session_id,
            prompt_text: self.prompt_text.clone(),
            attachments: self.attachments.clone(),
            save_requests: self.save_request_tx.clone(),
        };
        background
            .spawn(async move { turn_driver.run(&mut mapper).await })
            .detach();

        // The agent's block is the command the driver waits on. The driver decides when the run
        // is over (from the native conversation's status and its idle windows) and ends it
        // through `exit`; the turn driver just keeps the session serviceable until then.
        Ok(command_handle)
    }

    /// Uploads the native conversation at the boundaries the turn driver raises and at the end
    /// of the run. Periodic saves are skipped: between boundaries the conversation only gains
    /// streamed text, which the next boundary or the final save captures.
    async fn save_conversation(
        &self,
        save_point: SavePoint,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> PersistenceOutcome {
        match save_point {
            SavePoint::Periodic => return PersistenceOutcome::skipped(),
            SavePoint::PostTurn | SavePoint::Final => {}
        }
        let Some(conversation) = self.conversation.get().cloned() else {
            return match save_point {
                SavePoint::Final => PersistenceOutcome::failed(anyhow!(
                    "Cannot finalize ACP persistence before the conversation is bound"
                )),
                SavePoint::PostTurn | SavePoint::Periodic => PersistenceOutcome::skipped(),
            };
        };
        PersistenceOutcome::without_transcript(
            upload_conversation_snapshot(self.client.as_ref(), &conversation, foreground).await,
        )
    }

    async fn exit(&self, _foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        let Some((connection, session_id)) = self.running_agent() else {
            return Ok(());
        };
        if let Some(session_id) = session_id {
            connection
                .notify(protocol::METHOD_SESSION_CANCEL, CancelParams { session_id })
                .await?;
        }
        connection.close().await;
        Ok(())
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

/// Runs the initial prompt turn, then any follow-up prompts that arrive while the session is
/// open. The session ends when the driver closes the connection (its idle window elapsed or it
/// is shutting down), when the controller unbinds the conversation, or when the agent leaves.
///
/// A prompt turn spans one or more MAA request streams: every time the agent reports results
/// for tool calls announced in the current stream, that stream is finished and a new one is
/// opened, as the MAA server does for its own tool round trips.
struct TurnDriver {
    connection: Arc<AcpConnection>,
    notifications: async_channel::Receiver<InboundNotification>,
    /// Follow-up prompts injected into the conversation (e.g. by shared-session viewers).
    follow_ups: async_channel::Receiver<ExternalHarnessPrompt>,
    foreground: ModelSpawner<AgentDriver>,
    terminal_driver: ModelHandle<TerminalDriver>,
    run_id: Option<String>,
    /// The currently open request stream.
    turn: Mutex<ExternalHarnessTurn>,
    session_id: String,
    prompt_text: String,
    attachments: Arc<AttachmentResolver>,
    save_requests: async_channel::Sender<SavePoint>,
}

impl TurnDriver {
    /// A failed turn is already reflected in the conversation status (the stream finishes with
    /// an internal error), which is what the task status follows; the block's exit code remains
    /// the agent's own.
    async fn run(self, mapper: &mut AcpTurnMapper) {
        let initial = vec![ContentBlock::Text {
            text: self.prompt_text.clone(),
        }];
        let mut turn_result = self.run_turn(mapper, initial).await;

        log::info!("ACP turn finished; accepting follow-ups until the driver ends the run");
        loop {
            select! {
                prompt = self.follow_ups.recv().fuse() => match prompt {
                    Ok(prompt) => {
                        log::info!("Running ACP follow-up turn");
                        turn_result = self.run_follow_up(mapper, prompt).await;
                    }
                    // The controller unbound the conversation; nothing more can be routed here.
                    Err(_) => break,
                },
                notification = self.notifications.recv().fuse() => match notification {
                    Ok(notification) => log::debug!(
                        "Ignoring ACP notification {} received between turns",
                        notification.method
                    ),
                    // The agent closed its side, typically because the driver closed ours.
                    Err(_) => break,
                },
            }
        }
        self.connection.close().await;
        match turn_result {
            Ok(stop_reason) => log::info!("ACP turns finished (last stop reason {stop_reason:?})"),
            Err(error) => log::warn!("ACP turns finished with an error: {error:#}"),
        }
    }

    /// Opens a new request stream for a follow-up prompt, echoes it, and runs the turn. The
    /// echo shows the text only; attachments reach the agent but are not yet rendered on the
    /// user-query entry.
    async fn run_follow_up(
        &self,
        mapper: &mut AcpTurnMapper,
        prompt: ExternalHarnessPrompt,
    ) -> Result<StopReason> {
        let next = begin_turn(&self.foreground, &self.terminal_driver, self.run_id.clone())
            .await
            .map_err(|error| anyhow!("{error}"))?;
        mapper.start_segment(next.request_id.clone());
        *self.turn.lock() = next;
        let echo = vec![mapper.user_query_action(&prompt.text)];
        self.apply_events(mapper, vec![TurnEvent::Actions(echo)])
            .await?;
        let content = self
            .attachments
            .prompt_content(prompt.text, prompt.attachments)
            .await;
        self.run_turn(mapper, content).await
    }

    /// Runs one prompt turn to completion and finishes its last request stream.
    async fn run_turn(
        &self,
        mapper: &mut AcpTurnMapper,
        prompt: Vec<ContentBlock>,
    ) -> Result<StopReason> {
        let turn_result = self.prompt(mapper, prompt).await;
        let finished = match &turn_result {
            Ok(stop_reason) => stream_finished_for(*stop_reason),
            Err(error) => {
                log::error!("ACP turn failed: {error:#}");
                StreamFinished {
                    reason: Some(stream_finished::Reason::InternalError(
                        stream_finished::InternalError {
                            message: format!("{error:#}"),
                        },
                    )),
                    ..Default::default()
                }
            }
        };
        let finish_events = mapper.finish_events();
        if let Err(error) = self.apply_events(mapper, finish_events).await {
            log::warn!("Failed to settle ACP tool calls: {error}");
        }
        let stream_id = self.turn.lock().stream_id.clone();
        if finish_turn(&self.foreground, &self.terminal_driver, stream_id, finished)
            .await
            .is_err()
        {
            log::warn!("Agent driver dropped before the ACP turn could be finished");
        }
        self.request_save();
        turn_result
    }

    /// Asks the driver to persist the conversation as it stands once the save runs.
    fn request_save(&self) {
        if self.save_requests.try_send(SavePoint::PostTurn).is_err() {
            log::debug!("ACP save request dropped; the runner is gone");
        }
    }

    /// Sends the prompt and applies every update the agent streams before responding.
    async fn prompt(
        &self,
        mapper: &mut AcpTurnMapper,
        prompt: Vec<ContentBlock>,
    ) -> Result<StopReason> {
        let prompt_fut = self
            .connection
            .request::<_, PromptResponse>(
                protocol::METHOD_SESSION_PROMPT,
                PromptParams {
                    session_id: self.session_id.clone(),
                    prompt,
                },
            )
            .fuse();
        pin_mut!(prompt_fut);
        loop {
            select! {
                notification = self.notifications.recv().fuse() => match notification {
                    Ok(notification) => self.handle_notification(mapper, notification).await?,
                    // The agent closed its stdout; the pending prompt resolves with the error.
                    Err(_) => break,
                },
                response = prompt_fut => {
                    let response = response?;
                    while let Ok(notification) = self.notifications.try_recv() {
                        self.handle_notification(mapper, notification).await?;
                    }
                    return Ok(response.stop_reason);
                }
            }
        }
        prompt_fut.await.map(|response| response.stop_reason)
    }

    async fn handle_notification(
        &self,
        mapper: &mut AcpTurnMapper,
        notification: InboundNotification,
    ) -> Result<()> {
        if notification.method != protocol::METHOD_SESSION_UPDATE {
            log::debug!("Ignoring ACP notification {}", notification.method);
            return Ok(());
        }
        let params: SessionUpdateParams = match serde_json::from_value(notification.params) {
            Ok(params) => params,
            Err(error) => {
                log::warn!("Ignoring unparseable ACP session/update: {error}");
                return Ok(());
            }
        };
        let events = mapper.map_update(params.update);
        self.apply_events(mapper, events).await
    }

    async fn apply_events(&self, mapper: &mut AcpTurnMapper, events: Vec<TurnEvent>) -> Result<()> {
        for event in events {
            match event {
                TurnEvent::Actions(actions) => {
                    let stream_id = self.turn.lock().stream_id.clone();
                    apply_actions(&self.foreground, &self.terminal_driver, &stream_id, actions)
                        .await
                        .map_err(|error| anyhow!("{error}"))?;
                }
                TurnEvent::SegmentBoundary => {
                    let stream_id = self.turn.lock().stream_id.clone();
                    finish_turn(
                        &self.foreground,
                        &self.terminal_driver,
                        stream_id,
                        stream_finished_for(StopReason::EndTurn),
                    )
                    .await
                    .map_err(|error| anyhow!("{error}"))?;
                    self.request_save();
                    let next =
                        begin_turn(&self.foreground, &self.terminal_driver, self.run_id.clone())
                            .await
                            .map_err(|error| anyhow!("{error}"))?;
                    mapper.start_segment(next.request_id.clone());
                    *self.turn.lock() = next;
                }
            }
        }
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

/// The native conversation the driver's terminal is bound to, binding a fresh one if needed.
fn bound_conversation_id(
    controller: &mut BlocklistAIController,
    ctx: &mut ModelContext<BlocklistAIController>,
) -> AIConversationId {
    controller
        .native_prompt_conversation_id()
        .unwrap_or_else(|| controller.bind_native_prompt_conversation(None, ctx))
}

/// Captures the native conversation on the foreground thread and replaces the server's copy
/// with it.
async fn upload_conversation_snapshot(
    client: &dyn HarnessSupportClient,
    conversation: &RunConversation,
    foreground: &ModelSpawner<AgentDriver>,
) -> Result<()> {
    let local_id = conversation.local_id;
    let snapshot = foreground
        .spawn(move |_, ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&local_id)
                .map(AIConversation::to_conversation_data)
        })
        .await
        .map_err(|_| anyhow!("Agent driver dropped before the ACP conversation was captured"))?
        .ok_or_else(|| anyhow!("ACP conversation {local_id:?} is no longer loaded"))?;
    client
        .upload_conversation_data(&conversation.server_id, &snapshot)
        .await
        .with_context(|| {
            format!(
                "Failed to upload ACP conversation data to {}",
                conversation.server_id
            )
        })
}

/// Opens a request stream on the driver's bound native conversation.
async fn begin_turn(
    foreground: &ModelSpawner<AgentDriver>,
    terminal_driver: &ModelHandle<TerminalDriver>,
    run_id: Option<String>,
) -> Result<ExternalHarnessTurn, AgentDriverError> {
    with_ai_controller(foreground, terminal_driver, move |controller, ctx| {
        let conversation_id = bound_conversation_id(controller, ctx);
        controller.begin_external_harness_turn(conversation_id, run_id, ctx)
    })
    .await?
    .map_err(|error| AgentDriverError::HarnessSetupFailed {
        harness: "acp".to_owned(),
        reason: format!("could not open conversation turn: {error}"),
    })
}

async fn finish_turn(
    foreground: &ModelSpawner<AgentDriver>,
    terminal_driver: &ModelHandle<TerminalDriver>,
    stream_id: ResponseStreamId,
    finished: StreamFinished,
) -> Result<(), AgentDriverError> {
    apply_event(
        foreground,
        terminal_driver,
        stream_id,
        response_event::Type::Finished(finished),
    )
    .await
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

fn stream_finished_for(stop_reason: StopReason) -> StreamFinished {
    let reason = match stop_reason {
        StopReason::EndTurn | StopReason::Cancelled => {
            stream_finished::Reason::Done(stream_finished::Done {})
        }
        StopReason::MaxTokens => {
            stream_finished::Reason::MaxTokenLimit(stream_finished::ReachedMaxTokenLimit {})
        }
        StopReason::MaxTurnRequests | StopReason::Refusal | StopReason::Unknown => {
            stream_finished::Reason::InternalError(stream_finished::InternalError {
                message: format!("ACP agent stopped: {stop_reason:?}"),
            })
        }
    };
    StreamFinished {
        reason: Some(reason),
        ..Default::default()
    }
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
