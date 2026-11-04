use std::ffi::OsStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures::channel::oneshot;
use tempfile::TempDir;
use warp_cli::agent::{Harness, HarnessTransport};
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelSpawner, SingletonEntity as _};

use super::{
    HARNESS_FAILURE_OUTPUT_TRUNCATION_MARKER, HarnessKind, HarnessPersistence, HarnessRunner,
    PersistenceOutcome, SavePoint, auth_check_command_for, harness_env_vars, harness_kind,
    prepare_harness_failure_output, validate_cli_installed,
};
use crate::ai::agent_sdk::driver::terminal::{CommandHandle, TerminalDriver};
use crate::ai::agent_sdk::driver::{AgentDriver, AgentDriverError, IdleTimeoutSender};
use crate::ai::agent_sdk::setup_observability::SetupClientEventReporter;
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

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
fn claude_model_env_vars_depend_on_transport() {
    let config = HarnessModelConfig {
        model_id: "claude-fable-5-1".to_string(),
        reasoning_level: None,
    };

    let (pty, _) = harness_env_vars(Harness::Claude, HarnessTransport::Pty, Some(&config)).unwrap();
    assert_eq!(
        pty.get(OsStr::new("ANTHROPIC_MODEL")).map(|v| v.to_str()),
        Some(Some("claude-fable-5-1"))
    );
    assert!(!pty.contains_key(OsStr::new("ANTHROPIC_CUSTOM_MODEL_OPTION")));

    let (acp, _) = harness_env_vars(Harness::Claude, HarnessTransport::Acp, Some(&config)).unwrap();
    assert_eq!(
        acp.get(OsStr::new("ANTHROPIC_MODEL")).map(|v| v.to_str()),
        Some(Some("claude-fable-5-1"))
    );
    assert_eq!(
        acp.get(OsStr::new("ANTHROPIC_CUSTOM_MODEL_OPTION"))
            .map(|v| v.to_str()),
        Some(Some("claude-fable-5-1"))
    );

    let default_config = HarnessModelConfig {
        model_id: "default".to_string(),
        reasoning_level: None,
    };
    let (acp_default, _) = harness_env_vars(
        Harness::Claude,
        HarnessTransport::Acp,
        Some(&default_config),
    )
    .unwrap();
    assert!(!acp_default.contains_key(OsStr::new("ANTHROPIC_CUSTOM_MODEL_OPTION")));

    assert!(
        harness_env_vars(Harness::Claude, HarnessTransport::Acp, None)
            .unwrap()
            .0
            .is_empty()
    );
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

#[test]
fn transport_fallback_only_when_an_acp_run_has_no_adapter() {
    use super::{AcpAdapter, TransportFallbackReason, transport_fallback_reason};

    let no_sidecar = AcpAdapter::Missing(TransportFallbackReason::NoSidecar);
    let empty_sidecar = AcpAdapter::Missing(TransportFallbackReason::SidecarWithoutAdapter);

    // A sidecar manifest, or an adapter on PATH, keeps the run on ACP.
    for harness in [Harness::Claude, Harness::Codex] {
        assert_eq!(
            transport_fallback_reason(harness, HarnessTransport::Acp, AcpAdapter::Available),
            None
        );
    }
    // No adapter at all: the CLI harnesses fall back to their PTY transport, saying whether a
    // sidecar was mounted.
    for harness in [Harness::Claude, Harness::Codex] {
        assert_eq!(
            transport_fallback_reason(harness, HarnessTransport::Acp, no_sidecar),
            Some(TransportFallbackReason::NoSidecar)
        );
        assert_eq!(
            transport_fallback_reason(harness, HarnessTransport::Acp, empty_sidecar),
            Some(TransportFallbackReason::SidecarWithoutAdapter)
        );
    }
    // A PTY run never falls back, and harnesses without a PTY are left alone.
    assert_eq!(
        transport_fallback_reason(Harness::Claude, HarnessTransport::Pty, no_sidecar),
        None
    );
    assert_eq!(
        transport_fallback_reason(Harness::Pi, HarnessTransport::Acp, no_sidecar),
        None
    );
    assert_eq!(
        transport_fallback_reason(Harness::Gemini, HarnessTransport::Acp, empty_sidecar),
        None
    );
}

#[test]
fn transport_fallback_reasons_have_stable_wire_values() {
    use super::TransportFallbackReason;

    // The reportHarnessTransport report maps exactly these values to its fallback reasons.
    assert_eq!(TransportFallbackReason::NoSidecar.as_str(), "no_sidecar");
    assert_eq!(
        TransportFallbackReason::SidecarWithoutAdapter.as_str(),
        "sidecar_without_adapter"
    );
    assert_eq!(TransportFallbackReason::NoSidecar.to_string(), "no_sidecar");
}

#[test]
fn transport_fallback_notice_names_the_harness_and_the_cause() {
    use super::{TransportFallbackReason, transport_fallback_notice};

    let notice = transport_fallback_notice(Harness::Claude, TransportFallbackReason::NoSidecar);
    assert_eq!(
        notice,
        "oz: no ACP sidecar is mounted and no ACP adapter is on PATH, so Claude Code runs in the \
         terminal instead of over ACP. Follow-ups to this run cannot carry file attachments."
    );
    let notice = transport_fallback_notice(
        Harness::Codex,
        TransportFallbackReason::SidecarWithoutAdapter,
    );
    assert!(
        notice.contains("the mounted sidecar has no ACP adapter"),
        "{notice}"
    );
    assert!(notice.contains(Harness::Codex.display_name()), "{notice}");
}

#[test]
fn transport_fallback_notice_command_echoes_the_notice_single_quoted() {
    use super::{
        TransportFallbackReason, transport_fallback_notice, transport_fallback_notice_command,
    };

    for harness in [Harness::Claude, Harness::Codex] {
        for reason in [
            TransportFallbackReason::NoSidecar,
            TransportFallbackReason::SidecarWithoutAdapter,
        ] {
            let command = transport_fallback_notice_command(harness, reason);
            let notice = transport_fallback_notice(harness, reason);
            assert!(!notice.contains('\''), "{notice}");
            assert_eq!(command, format!("echo '{notice}'"));
        }
    }
}

#[test]
fn apply_transport_fallback_swaps_the_harness_and_the_transport() {
    use super::{HarnessKind, apply_transport_fallback, harness_kind};

    // Oz has no ACP adapter to look for, so nothing changes.
    let mut kind = HarnessKind::Oz;
    let mut transport = HarnessTransport::Acp;
    assert_eq!(
        apply_transport_fallback(&mut kind, &mut transport).unwrap(),
        None
    );
    assert_eq!(transport, HarnessTransport::Acp);

    // A PTY harness reports no adapter availability and is left as is.
    let mut kind = harness_kind(Harness::Claude, HarnessTransport::Pty).unwrap();
    let mut transport = HarnessTransport::Pty;
    assert_eq!(
        apply_transport_fallback(&mut kind, &mut transport).unwrap(),
        None
    );
    assert!(matches!(kind, HarnessKind::ThirdParty(_)));
    assert_eq!(transport, HarnessTransport::Pty);
}
