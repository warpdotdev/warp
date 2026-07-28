//! Wasm-specific minimal `AgentDriver` stub for wasm32-unknown-unknown
//! (REMOTE-2264).
//!
//! The full `AgentDriver` (in `driver.rs`) depends on many native-only backends
//! (MCP, skills/fs, blocklist recording, bedrock credentials, harness support,
//! etc.) that are `cfg(not(target_family="wasm"))`-gated. This stub provides
//! the minimum `AgentDriver` interface needed for the wasm CLI/Node prototype:
//! `new()` constructs with the no-op `TerminalDriver` (synthetic bootstrap,
//! no PTY), and `run()` drives the MAA request via
//! `warp_multi_agent_client::generate_multi_agent_output` and performs the
//! conversation-consumer / session-sharing registration so the session-sharing
//! boundary is exercised.
//!
//! See `agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md`.

// These stub types are used by the wasm dispatch_command in mod.rs but clippy
// sees them as dead code within this module. Allow it for the prototype.
#![allow(dead_code, unused_variables)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, error, info};
use warp_cli::agent::{Harness, OutputFormat};
use warp_managed_secrets::ManagedSecretValue;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::orchestration_event_streamer::{
    register_agent_event_consumer, unregister_agent_event_consumer,
};
use crate::server::server_api::ServerApiProvider;

/// Options for initializing the agent driver.
pub struct AgentDriverOptions {
    pub working_dir: PathBuf,
    pub secrets: HashMap<String, ManagedSecretValue>,
    pub task_id: Option<AmbientAgentTaskId>,
    pub parent_run_id: Option<String>,
    pub should_share: bool,
    pub idle_on_complete: Option<Duration>,
    pub resume: Option<ResumeOptions>,
    pub cloud_providers: Vec<Box<dyn CloudProvider>>,
    pub environment: Option<AmbientAgentEnvironment>,
    pub selected_harness: Harness,
    pub third_party_harness_model_config: Option<HarnessModelConfig>,
    pub snapshot_disabled: Option<bool>,
    pub snapshot_upload_timeout: Option<Duration>,
    pub snapshot_script_timeout: Option<Duration>,
    pub skip_initial_turn: bool,
    pub strict_mcp_startup: bool,
    pub mcp_startup_timeout: Option<Duration>,
}

/// Resume options (stub — not supported on wasm).
pub enum ResumeOptions {
    Oz(Box<crate::terminal::view::ConversationRestorationInNewPaneType>),
    ThirdParty(Box<ResumePayload>),
}

/// Resume payload (stub).
pub struct ResumePayload;

/// Harness model config (stub).
pub struct HarnessModelConfig;

/// Cloud provider trait (stub).
pub trait CloudProvider: Send + 'static {
    fn name(&self) -> &str;
}

/// Ambient agent environment (stub).
pub struct AmbientAgentEnvironment;

/// Task configuration for running an agent.
#[derive(Debug)]
pub struct Task {
    pub prompt: AgentRunPrompt,
    pub model: Option<crate::ai::llms::LLMId>,
    pub profile: Option<String>,
    pub mcp_specs: Vec<warp_cli::mcp::MCPSpec>,
    pub harness: HarnessKind,
}

/// Prompt that we initialize an agent driver with.
#[derive(Debug, Clone)]
pub enum AgentRunPrompt {
    Local(String),
    ServerSide {
        skill: Option<ai::skills::ParsedSkill>,
        attachments_dir: Option<String>,
    },
}

/// Harness kind (stub — only Oz is supported on wasm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessKind {
    Oz,
    ThirdParty(Harness),
}

/// Agent driver error.
#[derive(Debug, thiserror::Error)]
pub enum AgentDriverError {
    #[error("Terminal session is not available.")]
    TerminalUnavailable,
    #[error("Invalid runtime state - please file a bug report.")]
    InvalidRuntimeState,
    #[error("Not logged in")]
    NotLoggedIn,
    #[error("Session sharing failed: {0}")]
    ShareSessionFailed(String),
    #[error("Environment setup failed: {0}")]
    EnvironmentSetupFailed(String),
    #[error("MCP startup failed: {}", .details.join("; "))]
    MCPStartupFailed { details: Vec<String> },
    #[error("Agent profile \"{0}\" not found")]
    ProfileError(String),
    #[error("Failed to authenticate with server")]
    NotAuthenticated,
    #[error("Warp Drive sync timed out")]
    WarpDriveSyncFailed,
    #[error("Conversation error: {error}")]
    ConversationError {
        error: crate::ai::agent::RenderableAIError,
    },
    #[error("Conversation cancelled: {reason}")]
    ConversationCancelled {
        reason: crate::ai::agent::CancellationReason,
    },
    #[error("Conversation blocked: {blocked_action}")]
    ConversationBlocked { blocked_action: String },
    #[error("Task harness mismatch: expected {expected}, got {got}")]
    TaskHarnessMismatch {
        task_id: String,
        expected: String,
        got: String,
    },
    #[error("Skill resolution failed: {0}")]
    SkillResolutionFailed(String),
    #[error("Config build failed: {0}")]
    ConfigBuildFailed(String),
    #[error("AI workflow not found: {0}")]
    AIWorkflowNotFound(String),
    #[error("AWS Bedrock credentials failed: {0}")]
    AwsBedrockCredentialsFailed(String),
    #[error("Harness setup failed: {0}")]
    HarnessSetupFailed(String),
    #[error("Team metadata refresh timed out")]
    TeamMetadataRefreshTimeout,
}

/// Events emitted by the wasm AgentDriver.
pub enum TerminalDriverEvent {
    SlowBootstrap,
    EstablishedSharedSession {
        session_id: session_sharing_protocol::common::SessionId,
        join_url: String,
    },
}

/// `AgentDriver` is a model for driving an ambient Warp agent to completion.
///
/// On wasm, this is a minimal stub that constructs with a no-op TerminalDriver
/// and drives the MAA request via `generate_multi_agent_output`. Session-sharing
/// registration is performed via the conversation-consumer registration.
pub struct AgentDriver {
    // On wasm, there is no TerminalDriver — the agent driver runs without a terminal.
    // The full AgentDriver (driver.rs) uses TerminalDriver for PTY/shell, which is
    // not available on wasm32-unknown-unknown.
    working_dir: PathBuf,
    secrets: Arc<HashMap<String, ManagedSecretValue>>,
    resolved_env_vars: Arc<HashMap<OsString, OsString>>,
    output_format: OutputFormat,
    task_id: Option<AmbientAgentTaskId>,
    idle_on_complete: Option<Duration>,
    should_share: bool,
    run_conversation_id: Option<crate::ai::agent::conversation::AIConversationId>,
    parent_run_id: Option<String>,
    skip_initial_turn: bool,
}

impl Entity for AgentDriver {
    type Event = ();
}

impl AgentDriver {
    /// Construct a new AgentDriver on wasm with a no-op TerminalDriver.
    pub fn new(
        options: AgentDriverOptions,
        ctx: &mut ModelContext<Self>,
    ) -> Result<Self, AgentDriverError> {
        let AgentDriverOptions {
            working_dir,
            task_id,
            parent_run_id,
            should_share,
            idle_on_complete,
            secrets,
            ..
        } = options;

        info!("Initializing wasm agent driver: share={}", should_share);

        // Check auth — AgentDriver requires the user to be logged in.
        if !crate::auth::AuthStateProvider::as_ref(ctx)
            .get()
            .is_logged_in()
        {
            return Err(AgentDriverError::NotLoggedIn);
        }

        let env_vars = HashMap::new();

        Ok(Self {
            working_dir,
            secrets: Arc::new(secrets),
            resolved_env_vars: Arc::new(env_vars),
            output_format: OutputFormat::default(),
            task_id,
            idle_on_complete,
            should_share,
            run_conversation_id: None,
            parent_run_id,
            skip_initial_turn: false,
        })
    }

    /// Run the agent task. On wasm, this drives the MAA request via
    /// `generate_multi_agent_output` and performs real conversation-consumer
    /// registration/unregistration for session sharing using `ctx.spawn` with
    /// a `ModelContext` callback (not the bare `Foreground::spawn`).
    pub fn run(
        &mut self,
        task: Task,
        ctx: &mut ModelContext<Self>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AgentDriverError>>>> {
        let (tx, rx) = futures::channel::oneshot::channel();
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let base_client = server_api.base_client();

        // Extract the prompt.
        let prompt_str = match &task.prompt {
            AgentRunPrompt::Local(text) => text.clone(),
            AgentRunPrompt::ServerSide { .. } => {
                let _ = tx.send(Err(AgentDriverError::InvalidRuntimeState));
                return Box::pin(async move {
                    rx.await
                        .unwrap_or(Err(AgentDriverError::InvalidRuntimeState))
                });
            }
        };

        // Use ctx.spawn (ModelContext::spawn) instead of Foreground::spawn.
        // The callback receives (&mut AgentDriver, S::Output, &mut ModelContext<AgentDriver>),
        // giving us access to the ModelContext needed for
        // register_agent_event_consumer / unregister_agent_event_consumer.
        //
        // The future returns the conversation ID (if received from StreamInit)
        // so the callback can perform the real session-sharing register/unregister.
        let model_id = ctx.model_id();

        ctx.spawn(
            async move {
                // Build the MAA request using the same proto wire format as the CLI.
                let request = build_wasm_maa_request(&prompt_str, task.model.as_ref());

                info!("Sending MAA request for prompt: {}", prompt_str);

                let mut registered_conversation_id: Option<String> = None;

                match warp_multi_agent_client::generate_multi_agent_output(
                    base_client.as_ref(),
                    &request,
                )
                .await
                {
                    Ok(stream) => {
                        use futures::StreamExt as _;
                        let mut stream = stream;
                        while let Some(event) = stream.next().await {
                            match event {
                                Ok(response_event) => {
                                    if let Some(warp_multi_agent_api::response_event::Type::Init(init)) =
                                        &response_event.r#type
                                    {
                                        info!(
                                            "MAA StreamInit: conversation_id={}, request_id={}, run_id={}",
                                            init.conversation_id,
                                            init.request_id,
                                            init.run_id
                                        );
                                        registered_conversation_id =
                                            Some(init.conversation_id.clone());
                                    }
                                    debug!("MAA response event: {:?}", response_event.r#type);
                                }
                                Err(err) => {
                                    // 403 capture instrumentation (REMOTE-2264).
                                    if let warp_multi_agent_client::Error::EventSource(es_err) = &err
                                        && let reqwest_eventsource::Error::InvalidStatusCode(status, response) =
                                            es_err.as_ref()
                                    {
                                        error!("MAA 403 capture — status: {}", status);
                                        error!(
                                            "MAA 403 capture — response headers: {:?}",
                                            response.headers()
                                        );
                                        error!("MAA 403 capture — response url: {}", response.url());
                                    }
                                    error!("MAA stream error: {:?}", err);
                                    break;
                                }
                            }
                        }
                        info!("MAA stream completed");
                    }
                    Err(err) => {
                        error!("MAA request failed: {}", err);
                        let _ = tx.send(Err(AgentDriverError::InvalidRuntimeState));
                        return registered_conversation_id;
                    }
                }

                let _ = tx.send(Ok(()));
                registered_conversation_id
            },
            // Callback: receives (&mut AgentDriver, Option<String>, &mut ModelContext<AgentDriver>).
            // The Option<String> is the conversation ID returned by the spawned future.
            // Here we perform the real session-sharing register/unregister calls
            // that require a ModelContext.
            move |driver, conv_id_opt, ctx| {
                if let Some(conv_id_str) = conv_id_opt {
                    info!(
                        "Session-sharing: registering conversation consumer for {}",
                        conv_id_str
                    );
                    // AIConversationId implements TryFrom<String> (not FromStr).
                    if let Ok(conv_id) = AIConversationId::try_from(conv_id_str) {
                        register_agent_event_consumer(conv_id, model_id, ctx);
                        // Store on the driver so we can unregister later.
                        driver.run_conversation_id = Some(conv_id);
                    }
                }

                // Unregister the conversation consumer (session sharing teardown).
                // In the native driver this happens in a separate cleanup step;
                // here we do it immediately since the stream is already complete.
                if let Some(conv_id) = driver.run_conversation_id.take() {
                    info!(
                        "Session-sharing: unregistering conversation consumer"
                    );
                    unregister_agent_event_consumer(conv_id, model_id, ctx);
                }
            },
        );

        Box::pin(async move {
            rx.await
                .unwrap_or(Err(AgentDriverError::InvalidRuntimeState))
        })
    }
}
fn build_wasm_maa_request(
    prompt: &str,
    model: Option<&crate::ai::llms::LLMId>,
) -> warp_multi_agent_api::Request {
    use warp_multi_agent_api as api;
    use warp_multi_agent_api::request::input::user_inputs::UserInput;
    use warp_multi_agent_api::request::input::user_inputs::user_input::Input as UserInputKind;
    use warp_multi_agent_api::request::input::{Type as InputType, UserInputs, UserQuery};
    use warp_multi_agent_api::request::settings::ModelConfig;
    use warp_multi_agent_api::request::{Input, Metadata, Settings, TaskContext};

    let user_query = UserQuery {
        query: prompt.to_string(),
        referenced_attachments: Default::default(),
        mode: None,
        intended_agent: Default::default(),
    };

    api::Request {
        task_context: Some(TaskContext { tasks: vec![] }),
        input: Some(Input {
            context: None,
            r#type: Some(InputType::UserInputs(UserInputs {
                inputs: vec![UserInput {
                    input: Some(UserInputKind::UserQuery(user_query)),
                }],
            })),
        }),
        settings: Some(Settings {
            model_config: Some(ModelConfig {
                base: model.map(|m| m.to_string()).unwrap_or_default(),
                ..Default::default()
            }),
            web_context_retrieval_enabled: true,
            supports_parallel_tool_calls: true,
            planning_enabled: true,
            supports_create_files: true,
            supports_long_running_commands: true,
            should_preserve_file_content_in_history: true,
            supports_todos_ui: true,
            supports_started_child_task_message: true,
            supports_suggest_prompt: true,
            supports_reasoning_message: true,
            ..Default::default()
        }),
        metadata: Some(Metadata {
            conversation_id: String::new(),
            logging: Default::default(),
            ambient_agent_task_id: String::new(),
            forked_from_conversation_id: String::new(),
            parent_agent_id: String::new(),
            agent_name: String::new(),
        }),
        existing_suggestions: None,
        mcp_context: None,
    }
}
