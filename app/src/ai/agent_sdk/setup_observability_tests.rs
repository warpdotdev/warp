use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use futures::executor::block_on;
use warpui::r#async::executor::Background;

use super::{OzRunTimelineEvent, SetupClientEventReporter, SetupScope, SetupStep};
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
