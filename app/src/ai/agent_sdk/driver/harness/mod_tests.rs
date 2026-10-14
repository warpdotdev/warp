use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai::skills::parse_skill;
use anyhow::Result;
use async_trait::async_trait;
use command::blocking::Command as ProcessCommand;
use futures::channel::oneshot;
use mockito::Matcher;
use serde_json::json;
use tempfile::TempDir;
use warp_cli::agent::{Harness, HarnessTransport};
use warp_core::channel::ChannelState;
use warp_managed_secrets::ManagedSecretValue;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle, ModelSpawner, SingletonEntity as _};

use super::{
    HARNESS_FAILURE_OUTPUT_TRUNCATION_MARKER, HarnessKind, HarnessPersistence, HarnessRunner,
    PersistenceOutcome, ResumePayload, SavePoint, ThirdPartyHarness, auth_check_command_for,
    harness_kind, prepare_harness_failure_output, validate_cli_installed,
};
use crate::ai::agent_sdk::driver::terminal::{CommandHandle, TerminalDriver};
use crate::ai::agent_sdk::driver::{
    AgentDriver, AgentDriverError, AgentRunPrompt, IdleTimeoutSender,
};
use crate::ai::agent_sdk::setup_observability::SetupClientEventReporter;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::blocklist::pending_cli_harness_prompt_queue::PendingCliHarnessPromptQueue;
use crate::ai::mcp::JSONMCPServer;
use crate::server::server_api::ServerApi;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspace::view::tests::initialize_app as initialize_workspace_test_app;

fn assert_harness_setup_failed(err: &AgentDriverError) -> (&str, &str) {
    match err {
        AgentDriverError::HarnessSetupFailed { harness, reason } => (harness, reason),
        other => panic!("expected HarnessSetupFailed, got: {other}"),
    }
}

#[test]
fn harness_failure_output_uses_harness_truncation_marker() {
    let output = format!("START{}END", "x".repeat(4_096));

    let prepared = prepare_harness_failure_output(&output);
    assert!(prepared.contains(HARNESS_FAILURE_OUTPUT_TRUNCATION_MARKER));
}

#[cfg(not(windows))]
#[test]
fn validate_cli_installed_succeeds_for_known_binary() {
    assert!(validate_cli_installed("ls", None).is_ok());
}

#[test]
fn validate_cli_installed_fails_for_missing_binary() {
    let err = validate_cli_installed("__nonexistent_cli_abc123__", None).unwrap_err();
    let (harness, reason) = assert_harness_setup_failed(&err);
    assert_eq!(harness, "__nonexistent_cli_abc123__");
    assert!(reason.contains("not found"));
    assert!(!reason.contains("Install it first"));
}

#[test]
fn validate_cli_installed_includes_docs_url_in_error() {
    let url = "https://example.com/install";
    let err = validate_cli_installed("__nonexistent_cli_abc123__", Some(url)).unwrap_err();
    let (_, reason) = assert_harness_setup_failed(&err);
    assert!(reason.contains(url));
    assert!(reason.contains("Install it first"));
}

// --- Runtime error pattern tests ---

#[test]
fn claude_runtime_error_patterns_returns_slice() {
    use super::ThirdPartyHarness;
    use super::claude_code::ClaudeHarness;
    // Patterns are initially empty until validated needles are filled in.
    // The trait method must still be callable.
    let _: &[&str] = ClaudeHarness.runtime_error_patterns();
}

#[test]
fn codex_runtime_error_patterns_returns_slice() {
    use super::ThirdPartyHarness;
    use super::codex::CodexHarness;
    let _: &[&str] = CodexHarness.runtime_error_patterns();
}

#[test]
fn gemini_runtime_error_patterns_is_empty_by_default() {
    use super::ThirdPartyHarness;
    use super::gemini::GeminiHarness;
    assert!(GeminiHarness.runtime_error_patterns().is_empty());
}

#[test]
fn auth_check_command_for_gemini_is_none() {
    assert!(auth_check_command_for(Harness::Gemini).is_none());
}

#[test]
fn acp_transport_does_not_affect_the_oz_harness() {
    assert!(matches!(
        harness_kind(Harness::Oz, HarnessTransport::Acp),
        Ok(HarnessKind::Oz)
    ));
}

#[test]
fn auth_check_command_for_oz_is_none() {
    assert!(auth_check_command_for(Harness::Oz).is_none());
}

#[test]
fn auth_check_command_for_unsupported_is_none() {
    // OpenCode is mapped to HarnessKind::Unsupported and therefore has no
    // auth check command of its own.
    assert!(auth_check_command_for(Harness::OpenCode).is_none());
}

#[test]
fn auth_check_command_for_unknown_is_none() {
    // Harness::Unknown causes harness_kind to return Err; the helper still
    // returns None instead of panicking.
    assert!(auth_check_command_for(Harness::Unknown).is_none());
}

#[derive(Debug, PartialEq)]
enum Request {
    Refresh,
    Save(SavePoint),
}

struct RecordingRunner {
    requests: async_channel::Sender<Request>,
    persistence: HarnessPersistence,
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for RecordingRunner {
    fn harness_name(&self) -> &str {
        "recording"
    }

    async fn start(
        &self,
        _: &ModelSpawner<AgentDriver>,
        _: &SetupClientEventReporter,
    ) -> Result<CommandHandle, AgentDriverError> {
        unreachable!()
    }

    async fn exit(&self, _: &ModelSpawner<AgentDriver>) -> Result<()> {
        unreachable!()
    }

    fn persistence(&self) -> &HarnessPersistence {
        &self.persistence
    }

    async fn save_conversation(
        &self,
        _: SavePoint,
        _: &ModelSpawner<AgentDriver>,
    ) -> PersistenceOutcome {
        unreachable!()
    }

    async fn enqueue_save(
        self: Arc<Self>,
        point: SavePoint,
        _: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        self.requests.send(Request::Save(point)).await?;
        Ok(())
    }

    async fn handle_session_update(&self, _: &ModelSpawner<AgentDriver>) -> Result<()> {
        self.requests.send(Request::Refresh).await?;
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
struct PreparedPrompt {
    prompt: String,
    system_prompt: Option<String>,
    resumption_prompt: Option<String>,
    context: Option<String>,
}

struct RecordingHarness {
    transport: HarnessTransport,
    prepared: Arc<Mutex<Option<PreparedPrompt>>>,
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for RecordingHarness {
    fn harness(&self) -> Harness {
        Harness::Claude
    }

    fn cli_agent(&self) -> CLIAgent {
        CLIAgent::Claude
    }

    fn drives_cli_agent_session(&self) -> bool {
        self.transport == HarnessTransport::Pty
    }

    #[allow(clippy::too_many_arguments)]
    fn build_runner(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        resumption_prompt: Option<&str>,
        context: Option<&str>,
        _: &Path,
        _: &Path,
        _: Option<AmbientAgentTaskId>,
        _: Arc<ServerApi>,
        _: ModelHandle<TerminalDriver>,
        resume: Option<ResumePayload>,
        _: &HashMap<OsString, OsString>,
        _: &[PathBuf],
        _: bool,
        _: &HashMap<String, ManagedSecretValue>,
        _: &HashMap<String, JSONMCPServer>,
        _: Option<&HarnessModelConfig>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        assert!(resume.is_none());
        *self.prepared.lock().unwrap() = Some(PreparedPrompt {
            prompt: prompt.to_owned(),
            system_prompt: system_prompt.map(str::to_owned),
            resumption_prompt: resumption_prompt.map(str::to_owned),
            context: context.map(str::to_owned),
        });
        let (requests, _) = async_channel::unbounded();
        Ok(Box::new(RecordingRunner {
            requests,
            persistence: HarnessPersistence::default(),
        }))
    }
}

const DEFERRED_INSTRUCTION: &str = "clone acme/billing first";
const TASK_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

#[test]
fn external_harness_preparation_keeps_server_skill_and_non_query_sections_unchanged() {
    if std::env::var_os("WARP_TEST_HARNESS_RESOLUTION").is_none() {
        let output = ProcessCommand::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ai::agent_sdk::driver::harness::tests::external_harness_preparation_keeps_server_skill_and_non_query_sections_unchanged",
                "--nocapture",
            ])
            .env("WARP_TEST_HARNESS_RESOLUTION", "1")
            .env("WARP_ISOLATION_PLATFORM", "docker")
            .env("WARP_WORKLOAD_TOKEN", "synthetic-workload-token")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    for transport in [HarnessTransport::Pty, HarnessTransport::Acp] {
        let root = TempDir::new().unwrap();
        let skill_path = root.path().join("authored/SKILL.md");
        std::fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        let skill_content = "---\nname: authored\n---\n\nDo the authored task.\n";
        std::fs::write(&skill_path, skill_content).unwrap();
        let skill = parse_skill(&skill_path).unwrap();
        let expected_body = format!(
            r#"{{"skill":{{"name":"authored","content":{},"path":{}}},"attachments_dir":"/workspace/attachments"}}"#,
            serde_json::to_string(skill_content).unwrap(),
            serde_json::to_string(&skill_path.to_string_lossy()).unwrap(),
        );
        let mut server = ChannelState::mock_server();
        let resolution = server
            .mock("POST", "/api/v1/harness-support/resolve-prompt")
            .match_header(
                warp_server_client::base_client::CLOUD_AGENT_ID_HEADER,
                TASK_ID,
            )
            .match_header(
                warp_server_client::base_client::AMBIENT_WORKLOAD_TOKEN_HEADER,
                "synthetic-workload-token",
            )
            .match_body(Matcher::Exact(expected_body))
            .with_status(200)
            .with_body(
                json!({
                    "prompt": "do the task",
                    "system_prompt": "system identity",
                    "resumption_prompt": "resume state",
                    "context": "retrieved memory",
                })
                .to_string(),
            )
            .expect(1)
            .create();
        App::test((), |mut app| async move {
            initialize_workspace_test_app(&mut app);
            let mut options = crate::ai::agent_sdk::tests::agent_driver_options();
            options.working_dir = root.path().to_path_buf();
            options.task_id = Some(TASK_ID.parse().unwrap());
            let driver = app.add_model(|ctx| AgentDriver::new(options, ctx).unwrap());
            let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
            let prepared = Arc::new(Mutex::new(None));
            let harness = RecordingHarness {
                transport,
                prepared: prepared.clone(),
            };
            AgentDriver::prepare_harness(
                &AgentRunPrompt::ServerSide {
                    skill: Some(skill),
                    attachments_dir: Some("/workspace/attachments".into()),
                },
                Some(DEFERRED_INSTRUCTION),
                &[],
                &harness,
                &foreground,
            )
            .await
            .unwrap();
            assert_eq!(
                prepared.lock().unwrap().take().unwrap(),
                PreparedPrompt {
                    prompt: format!("{DEFERRED_INSTRUCTION}\n\ndo the task"),
                    system_prompt: Some("system identity".into()),
                    resumption_prompt: Some("resume state".into()),
                    context: Some("retrieved memory".into()),
                },
                "{transport:?}"
            );
        });
        resolution.assert();
    }
}

#[test]
fn external_harness_preparation_routes_local_and_promptless_first_turns() {
    for transport in [HarnessTransport::Pty, HarnessTransport::Acp] {
        App::test((), |mut app| async move {
            initialize_workspace_test_app(&mut app);
            let root = TempDir::new().unwrap();
            let mut options = crate::ai::agent_sdk::tests::agent_driver_options();
            options.working_dir = root.path().to_path_buf();
            options.task_id = Some(TASK_ID.parse().unwrap());
            let driver = app.add_model(|ctx| AgentDriver::new(options, ctx).unwrap());
            let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
            let prepared = Arc::new(Mutex::new(None));
            let harness = RecordingHarness {
                transport,
                prepared: prepared.clone(),
            };
            AgentDriver::prepare_harness(
                &AgentRunPrompt::Local("do the task".into()),
                Some(DEFERRED_INSTRUCTION),
                &[],
                &harness,
                &foreground,
            )
            .await
            .unwrap();
            assert_eq!(
                prepared.lock().unwrap().take().unwrap(),
                PreparedPrompt {
                    prompt: format!("{DEFERRED_INSTRUCTION}\n\ndo the task"),
                    system_prompt: None,
                    resumption_prompt: None,
                    context: None,
                },
                "{transport:?}"
            );
        });

        App::test((), |mut app| async move {
            initialize_workspace_test_app(&mut app);
            app.add_singleton_model(PendingCliHarnessPromptQueue::new);
            let root = TempDir::new().unwrap();
            let mut options = crate::ai::agent_sdk::tests::agent_driver_options();
            options.working_dir = root.path().to_path_buf();
            options.task_id = Some(TASK_ID.parse().unwrap());
            let driver = app.add_model(|ctx| AgentDriver::new(options, ctx).unwrap());
            let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
            let prepared = Arc::new(Mutex::new(None));
            let harness = RecordingHarness {
                transport,
                prepared: prepared.clone(),
            };
            AgentDriver::prepare_harness(
                &AgentRunPrompt::Local(String::new()),
                Some(DEFERRED_INSTRUCTION),
                &[],
                &harness,
                &foreground,
            )
            .await
            .unwrap();
            assert_eq!(prepared.lock().unwrap().take().unwrap().prompt, "");
            match transport {
                HarnessTransport::Pty => {
                    let queue = PendingCliHarnessPromptQueue::handle(&app);
                    queue.update(&mut app, |queue, _| {
                        let task_id = TASK_ID.parse().unwrap();
                        assert_eq!(queue.prompt_for_delivery(task_id, String::new()), "");
                        assert_eq!(
                            queue.prompt_for_delivery(task_id, "inspect".into()),
                            format!("{DEFERRED_INSTRUCTION}\n\ninspect")
                        );
                        assert_eq!(
                            queue.prompt_for_delivery(task_id, "follow-up".into()),
                            "follow-up"
                        );
                    });
                }
                HarnessTransport::Acp => {
                    let terminal_driver =
                        driver.read(&app, |driver, _| driver.terminal_driver.clone());
                    let terminal =
                        terminal_driver.read(&app, |driver, _| driver.terminal_view().clone());
                    let (sink, prompts) = async_channel::unbounded();
                    terminal.update(&mut app, |terminal, ctx| {
                        terminal.ai_controller().update(ctx, |controller, ctx| {
                            controller.bind_native_prompt_conversation(None, ctx);
                            controller.set_external_harness_prompt_sink(sink);
                            for prompt in ["", "inspect", "follow-up"] {
                                controller.execute_warp_agent_prompt_from_shared_session_injection(
                                    prompt.into(),
                                    None,
                                    vec![],
                                    session_sharing_protocol::common::ParticipantId::new(),
                                    None,
                                    ctx,
                                );
                            }
                        });
                    });
                    assert_eq!(prompts.try_recv().unwrap().text, "");
                    assert_eq!(
                        prompts.try_recv().unwrap().text,
                        format!("{DEFERRED_INSTRUCTION}\n\ninspect")
                    );
                    assert_eq!(prompts.try_recv().unwrap().text, "follow-up");
                }
            }
        });
    }
}
#[test]
fn cli_activity_refreshes_metadata_without_saving_until_turn_completion() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.id();
        let temp = TempDir::new().unwrap();
        let (record, requests) = async_channel::unbounded();
        let (exit, _exited) = oneshot::channel();
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver =
                AgentDriver::new_for_test(temp.path().to_path_buf(), terminal_driver, ctx);
            driver.harness = Some(Arc::new(RecordingRunner {
                requests: record,
                persistence: HarnessPersistence::default(),
            }));
            driver.subscribe_to_cli_agent_session_events(IdleTimeoutSender::new(exit), ctx);
            driver
        });
        let sessions = CLIAgentSessionsModel::handle(&app);
        sessions.update(&mut app, |model, ctx| {
            model.set_session(
                terminal_view_id,
                CLIAgentSession {
                    agent: CLIAgent::Claude,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    plugin_version: None,
                    remote_host: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                },
                ctx,
            );
        });

        let emit = |body, app: &mut App| {
            let event = parse_event(Some("warp://cli-agent"), body).unwrap();
            sessions.update(app, |model, ctx| {
                model.update_from_event(terminal_view_id, &event, ctx);
            });
        };
        emit(
            r#"{"agent":"claude","event":"session_start","session_id":"native"}"#,
            &mut app,
        );
        assert_eq!(requests.recv().await.unwrap(), Request::Refresh);
        emit(
            r#"{"agent":"claude","event":"prompt_submit","query":"work"}"#,
            &mut app,
        );
        assert_eq!(requests.recv().await.unwrap(), Request::Refresh);
        emit(
            r#"{"agent":"claude","event":"permission_request"}"#,
            &mut app,
        );
        emit(r#"{"agent":"claude","event":"tool_complete"}"#, &mut app);
        assert_eq!(requests.recv().await.unwrap(), Request::Refresh);
        assert!(requests.is_empty());
        assert_eq!(
            sessions.read(&app, |model, _| model
                .session(terminal_view_id)
                .unwrap()
                .session_context
                .session_id
                .clone()),
            Some("native".to_owned()),
        );

        emit(r#"{"agent":"claude","event":"stop"}"#, &mut app);
        assert_eq!(
            requests
                .recv()
                .with_timeout(Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap(),
            Request::Save(SavePoint::PostTurn),
        );
        drop(driver);
    });
}
