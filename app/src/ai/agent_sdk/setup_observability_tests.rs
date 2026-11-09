use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use chrono::Utc;
use futures::executor::block_on;
use serde_json::{Value, json};
use warp_cli::agent::HarnessTransport;
use warpui::r#async::executor::Background;

use super::{AcpActionEvent, OzRunTimelineEvent, SetupClientEventReporter, SetupScope, SetupStep};
use crate::server::server_api::ai::MockAIClient;

/// Expected scope classification for every setup step.
const EXPECTED: &[(SetupStep, SetupScope)] = &[
    (SetupStep::TeamMetadataRefresh, SetupScope::Agent),
    (SetupStep::WarpDriveSync, SetupScope::Agent),
    (SetupStep::AgentCredentialsFetch, SetupScope::Agent),
    (SetupStep::EnvironmentResolution, SetupScope::Agent),
    (SetupStep::SkillRepoClone, SetupScope::Agent),
    (SetupStep::TerminalBootstrap, SetupScope::Agent),
    (SetupStep::CloudProviderSetup, SetupScope::Agent),
    (SetupStep::McpServerStartup, SetupScope::Agent),
    (SetupStep::AgentProfileConfiguration, SetupScope::Agent),
    (SetupStep::GlobalSkillResolution, SetupScope::Agent),
    (SetupStep::GlobalSkillRepoClone, SetupScope::Agent),
    (SetupStep::EnvironmentRepoClone, SetupScope::Agent),
    (SetupStep::CacheSetup, SetupScope::Agent),
    (SetupStep::EnvironmentSetupCommands, SetupScope::Agent),
    (SetupStep::EnvironmentCodebaseIndexing, SetupScope::Agent),
    (SetupStep::FileBasedMcpDiscovery, SetupScope::Agent),
    (SetupStep::FileBasedMcpReadiness, SetupScope::Agent),
    (SetupStep::InitialGlobalMcpScan, SetupScope::Agent),
    (SetupStep::InitialGlobalMcpReadiness, SetupScope::Agent),
    (SetupStep::EnvironmentSkillLoading, SetupScope::Agent),
    (SetupStep::GlobalSkillLoading, SetupScope::Agent),
    (SetupStep::SkillsDirsLoading, SetupScope::Agent),
    (SetupStep::TaskDataFetch, SetupScope::Run),
    (SetupStep::SharedSessionEstablishment, SetupScope::Run),
    (SetupStep::ConversationResumeLoading, SetupScope::Run),
    (SetupStep::SkillReferenceResolution, SetupScope::Run),
    (SetupStep::ThirdPartyHarnessPreparation, SetupScope::Run),
    (SetupStep::ThirdPartyHarnessStart, SetupScope::Run),
    (
        SetupStep::ThirdPartyHarnessExternalConversation,
        SetupScope::Run,
    ),
    (
        SetupStep::ThirdPartyHarnessPreparationNotificationPluginInstall,
        SetupScope::Run,
    ),
    (
        SetupStep::ThirdPartyHarnessPreparationNotificationPluginUpdate,
        SetupScope::Run,
    ),
    (
        SetupStep::ThirdPartyHarnessPreparationPlatformPluginInstall,
        SetupScope::Run,
    ),
    (
        SetupStep::ThirdPartyHarnessPreparationPlatformPluginUpdate,
        SetupScope::Run,
    ),
    (SetupStep::AcpAuthPreflight, SetupScope::Run),
    (SetupStep::AcpBridgeAccept, SetupScope::Run),
    (SetupStep::AcpInitialize, SetupScope::Run),
    (SetupStep::AcpSessionOpen, SetupScope::Run),
    (SetupStep::AcpModelConfig, SetupScope::Run),
    (SetupStep::AcpBeginTurn, SetupScope::Run),
];

#[test]
fn every_setup_step_has_its_expected_scope() {
    for (step, expected) in EXPECTED {
        assert_eq!(step.scope(), *expected, "unexpected scope for {step:?}");
    }
}

/// Guards against a new variant being added to [`SetupStep`] without being classified here.
/// `scope()` is matched exhaustively, so a new variant cannot reach production unclassified,
/// but it could still be missing from `EXPECTED` and escape the assertions above.
#[test]
fn expected_scopes_cover_every_setup_step() {
    for step in enum_iterator::all::<SetupStep>() {
        assert!(
            EXPECTED.iter().any(|(listed, _)| *listed == step),
            "{step:?} is missing from EXPECTED"
        );
    }
}

fn background() -> Arc<Background> {
    Arc::new(Background::new(1, |_| "setup-scope-test".to_string()))
}

fn standby_reporter() -> SetupClientEventReporter {
    let mut client = MockAIClient::new();
    client.expect_post_agent_run_client_event().times(0);
    SetupClientEventReporter::standby(Arc::new(client), background())
}

fn assert_steps_execute(
    reporter: &SetupClientEventReporter,
    steps: impl IntoIterator<Item = SetupStep>,
) {
    for step in steps {
        assert_eq!(
            block_on(reporter.record_result(step, async { Ok::<_, std::io::Error>(42) })).unwrap(),
            42,
        );
        assert_eq!(block_on(reporter.record_value(step, async { 42 })), 42);

        let (tx, rx) = mpsc::channel();
        reporter.record_value_detached(step, async move {
            tx.send(42).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 42);
    }
}

#[test]
fn standby_allows_agent_steps_without_posting_run_events() {
    let reporter = standby_reporter();
    assert_steps_execute(
        &reporter,
        enum_iterator::all::<SetupStep>().filter(|step| step.scope() == SetupScope::Agent),
    );
    block_on(reporter.post_timeline_event(OzRunTimelineEvent::AgentStarted));
}

fn assert_rejected_before_polling(step: SetupStep, record: impl FnOnce(Arc<AtomicBool>)) {
    let polled = Arc::new(AtomicBool::new(false));
    let panic = catch_unwind(AssertUnwindSafe(|| record(polled.clone())))
        .expect_err("standby must reject run-scoped setup");
    assert_eq!(
        panic.downcast_ref::<String>().unwrap(),
        &format!("Run-scoped setup step {step:?} cannot run during standby preparation"),
    );
    assert!(!polled.load(Ordering::SeqCst), "{step:?} was polled");
}

#[test]
fn standby_record_result_rejects_run_steps_before_polling() {
    let reporter = standby_reporter().clone();
    for step in enum_iterator::all::<SetupStep>().filter(|step| step.scope() == SetupScope::Run) {
        assert_rejected_before_polling(step, |polled| {
            block_on(reporter.record_result(step, async move {
                polled.store(true, Ordering::SeqCst);
                Ok::<_, std::io::Error>(())
            }))
            .unwrap();
        });
    }
}

#[test]
fn standby_record_value_rejects_run_steps_before_polling() {
    let reporter = standby_reporter();
    for step in enum_iterator::all::<SetupStep>().filter(|step| step.scope() == SetupScope::Run) {
        assert_rejected_before_polling(step, |polled| {
            block_on(reporter.record_value(step, async move {
                polled.store(true, Ordering::SeqCst);
            }));
        });
    }
}

#[test]
fn standby_record_value_detached_rejects_run_steps_before_spawning() {
    let reporter = standby_reporter();
    for step in enum_iterator::all::<SetupStep>().filter(|step| step.scope() == SetupScope::Run) {
        assert_rejected_before_polling(step, |polled| {
            reporter.record_value_detached(step, async move {
                polled.store(true, Ordering::SeqCst);
            });
        });
    }
}

#[test]
fn noop_reporter_allows_all_steps_without_a_run_id() {
    let mut client = MockAIClient::new();
    client.expect_post_agent_run_client_event().times(0);
    let reporter = SetupClientEventReporter::noop(Arc::new(client), background());
    assert_steps_execute(&reporter, enum_iterator::all::<SetupStep>());
}

#[test]
fn run_reporter_allows_and_reports_all_steps() {
    let (tx, rx) = mpsc::channel();
    let mut client = MockAIClient::new();
    client
        .expect_post_agent_run_client_event()
        .times(EXPECTED.len())
        .returning(move |_, _| {
            tx.send(()).unwrap();
            Ok(())
        });
    let reporter = SetupClientEventReporter::new(
        "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        Arc::new(client),
        background(),
    );
    for step in enum_iterator::all::<SetupStep>() {
        assert_eq!(block_on(reporter.record_value(step, async { 42 })), 42);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

#[test]
fn payload_timeline_events_post_in_the_background_and_tolerate_rejection() {
    let (tx, rx) = mpsc::channel();
    let mut client = MockAIClient::new();
    client
        .expect_post_agent_run_client_event()
        .times(1)
        .returning(move |_, request| {
            tx.send(serde_json::to_value(request).unwrap()).unwrap();
            Err(anyhow::anyhow!("400 Bad Request: unknown event"))
        });
    let reporter = SetupClientEventReporter::new(
        "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        Arc::new(client),
        background(),
    );
    let payload = json!({"references": []});
    reporter.post_timeline_event_with_payload(
        OzRunTimelineEvent::SkillSourcesResolved,
        payload.clone(),
    );
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "skill_sources_resolved");
    assert_eq!(request["payload"], payload);
}

/// A run reporter whose client forwards every posted request, serialized, to the returned channel.
fn recording_run_reporter(
    expected_posts: usize,
) -> (SetupClientEventReporter, mpsc::Receiver<Value>) {
    let (tx, rx) = mpsc::channel();
    let mut client = MockAIClient::new();
    client
        .expect_post_agent_run_client_event()
        .times(expected_posts)
        .returning(move |_, request| {
            tx.send(serde_json::to_value(request).unwrap()).unwrap();
            Ok(())
        });
    let reporter = SetupClientEventReporter::new(
        "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        Arc::new(client),
        background(),
    );
    (reporter, rx)
}

#[test]
fn metric_events_omit_outcome_and_transport_unless_given() {
    let (reporter, rx) = recording_run_reporter(1);
    assert_eq!(
        block_on(reporter.record_result(SetupStep::TerminalBootstrap, async {
            Ok::<_, std::io::Error>(1)
        }))
        .unwrap(),
        1
    );
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "setup_terminal_bootstrap");
    assert_eq!(request["payload"]["is_error"], false);
    assert!(request["payload"].get("outcome").is_none(), "{request}");
    assert!(request["payload"].get("transport").is_none(), "{request}");
}

#[test]
fn acp_setup_steps_carry_their_outcome_and_transport() {
    let (reporter, rx) = recording_run_reporter(2);
    let reporter = reporter.with_transport(HarnessTransport::Acp);
    let outcome = block_on(reporter.record_result_with_outcome(
        SetupStep::AcpSessionOpen,
        async { Ok::<_, std::io::Error>("resumed") },
        |origin| Some(*origin),
    ))
    .unwrap();
    assert_eq!(outcome, "resumed");
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "setup_acp_session_open");
    assert_eq!(request["payload"]["is_error"], false);
    assert_eq!(request["payload"]["outcome"], "resumed");
    assert_eq!(request["payload"]["transport"], "acp");

    // A failed step reports no outcome, only its error flag.
    block_on(reporter.record_result_with_outcome(
        SetupStep::AcpInitialize,
        async { Err::<&str, _>(std::io::Error::other("nope")) },
        |origin| Some(*origin),
    ))
    .unwrap_err();
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "setup_acp_initialize");
    assert_eq!(request["payload"]["is_error"], true);
    assert!(request["payload"].get("outcome").is_none(), "{request}");
    assert_eq!(request["payload"]["transport"], "acp");
}

#[test]
fn acp_action_events_are_posted_in_the_background() {
    let (reporter, rx) = recording_run_reporter(1);
    let reporter = reporter.with_transport(HarnessTransport::Acp);
    reporter.post_action_event(AcpActionEvent::Steer, "injected", Utc::now(), false);
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "acp_steer");
    assert_eq!(request["payload"]["outcome"], "injected");
    assert_eq!(request["payload"]["is_error"], false);
    assert_eq!(request["payload"]["transport"], "acp");
    assert!(request["payload"]["latency_ms"].as_i64().unwrap() >= 0);
}

#[test]
fn first_output_is_posted_only_after_agent_started_even_from_another_clone() {
    let (reporter, rx) = recording_run_reporter(2);
    reporter
        .with_transport(HarnessTransport::Acp)
        .post_timeline_event_detached(OzRunTimelineEvent::AgentFirstOutput);
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "the first output must wait for agent_started"
    );

    block_on(reporter.post_timeline_event(OzRunTimelineEvent::AgentStarted));
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "agent_started");
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request["event_name"], "agent_first_output");
    assert!(request.get("payload").is_none(), "{request}");
}
