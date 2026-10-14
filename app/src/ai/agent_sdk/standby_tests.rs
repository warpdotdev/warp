use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::TimeDelta;
use futures::executor::block_on;
use warp_core::features::FeatureFlag;
use warp_graphql::queries::execution_config::CodeForge as ConfigCodeForge;
use warpui::App;
use warpui::r#async::FutureExt as _;

use super::*;
use crate::auth::AuthStateProvider;
use crate::auth::user::{PrincipalType, User};
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::prewarmed_sandbox::tests::parse_configuration;

#[test]
fn invalid_preparation_inputs_fail_before_driver_creation_or_metadata_access() {
    App::test((), |mut app| async move {
        let runner = app.add_singleton_model(|_| AgentDriverRunner);
        let foreground = runner.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        let mut wrong_harness = parse_configuration();
        wrong_harness.preparation.harness = "claude".to_string();
        let mut escaping_skills = parse_configuration();
        escaping_skills.preparation.factory_skill_dirs = vec!["../outside".to_string()];
        let mut duplicate_repository = parse_configuration();
        duplicate_repository
            .preparation
            .repositories
            .push(duplicate_repository.preparation.repositories[0].clone());
        let mut invalid_mcp = parse_configuration();
        invalid_mcp.preparation.mcp_servers =
            serde_json::from_value(serde_json::json!({"invalid": {"url": 4}})).unwrap();
        let mut unknown_repository = parse_configuration();
        unknown_repository.preparation.repositories[0].forge = ConfigCodeForge::Unknown;
        let mut unknown_deferred_repository = parse_configuration();
        unknown_deferred_repository
            .preparation
            .deferred_repositories[0]
            .code_forge = ConfigCodeForge::Unknown;
        for config in [
            wrong_harness,
            escaping_skills,
            duplicate_repository,
            invalid_mcp,
            unknown_repository,
            unknown_deferred_repository,
        ] {
            let error = prepare(&foreground, config, &setup_events)
                .await
                .unwrap_err();
            assert!(
                matches!(error, AgentDriverError::ConfigBuildFailed(_)),
                "{error}"
            );
            assert!(!app.update(|ctx| ctx.has_singleton_model::<AgentDriver>()));
        }
    });
}

#[test]
fn skill_checkout_collisions_fail_before_cloning_or_metadata_access() {
    App::test((), |mut app| async move {
        let runner = app.add_singleton_model(|_| AgentDriverRunner);
        let foreground = runner.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        for specs in [
            vec!["another-org/warp:review"],
            vec!["first-org/tools:review", "second-org/tools:review"],
        ] {
            let mut config = parse_configuration();
            config.preparation.skills = specs.into_iter().map(str::to_owned).collect();
            let error = prepare(&foreground, config, &setup_events)
                .await
                .unwrap_err();
            let AgentDriverError::ConfigBuildFailed(source) = error else {
                panic!("expected a checkout collision, got {error}");
            };
            assert!(
                source.to_string().contains("share a clone directory name"),
                "{source:#}"
            );
            assert!(!app.update(|ctx| ctx.has_singleton_model::<AgentDriver>()));
        }
    });
}

#[test]
fn global_skill_checkout_collisions_fail_before_cloning_or_metadata_access() {
    App::test((), |mut app| async move {
        let _skills_enabled = FeatureFlag::OzPlatformSkills.override_enabled(true);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        let runner = app.add_singleton_model(|_| AgentDriverRunner);
        let foreground = runner.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        for (configured, global) in [
            (vec![], vec!["another-org/warp:review"]),
            (
                vec!["first-org/tools:review"],
                vec!["second-org/tools:review"],
            ),
            (
                vec![],
                vec!["first-org/tools:review", "second-org/tools:review"],
            ),
        ] {
            app.read(|ctx| {
                AuthStateProvider::as_ref(ctx).get().set_user(Some(User {
                    principal_type: PrincipalType::ServiceAccount,
                    global_skills: global.into_iter().map(str::to_owned).collect(),
                    ..User::test()
                }));
            });
            let mut config = parse_configuration();
            config.preparation.skills = configured.into_iter().map(str::to_owned).collect();
            let error = prepare(&foreground, config, &setup_events)
                .await
                .unwrap_err();
            let AgentDriverError::ConfigBuildFailed(source) = error else {
                panic!("expected a checkout collision, got {error}");
            };
            assert!(
                source.to_string().contains("share a clone directory name"),
                "{source:#}"
            );
            assert!(!app.update(|ctx| ctx.has_singleton_model::<AgentDriver>()));
        }
    });
}

/// A recorded call against the fake client.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Status(PrewarmedSandboxState),
    Heartbeat,
}

#[derive(Default)]
struct FakeClientState {
    calls: Vec<Call>,
    fail_status: bool,
    fail_heartbeat: bool,
    status_release: Option<oneshot::Receiver<()>>,
}

#[derive(Default)]
struct FakeClient {
    state: Mutex<FakeClientState>,
    events: Option<async_channel::Sender<Call>>,
}

impl FakeClient {
    fn calls(&self) -> Vec<Call> {
        self.state.lock().expect("uncontended").calls.clone()
    }
}

#[async_trait]
impl PrewarmedSandboxClient for FakeClient {
    async fn fetch_configuration(
        &self,
        sandbox_id: &str,
    ) -> anyhow::Result<PrewarmedSandboxConfiguration> {
        panic!("Unexpected configuration fetch for {sandbox_id}")
    }

    async fn report_status(
        &self,
        sandbox_id: &str,
        generation: &str,
        state: PrewarmedSandboxState,
    ) -> anyhow::Result<()> {
        assert_eq!(sandbox_id, "sandbox");
        assert_eq!(generation, "abc123");
        let (fail_status, status_release) = {
            let mut client = self.state.lock().expect("uncontended");
            client.calls.push(Call::Status(state));
            if let Some(events) = &self.events {
                events.try_send(Call::Status(state)).unwrap();
            }
            (client.fail_status, client.status_release.take())
        };
        if let Some(release) = status_release {
            release.await.unwrap();
        }
        if fail_status {
            anyhow::bail!("workload token acquisition failed");
        }
        Ok(())
    }

    async fn heartbeat(&self, sandbox_id: &str, generation: &str) -> anyhow::Result<()> {
        assert_eq!(sandbox_id, "sandbox");
        assert_eq!(generation, "abc123");
        let mut client = self.state.lock().expect("uncontended");
        client.calls.push(Call::Heartbeat);
        if let Some(events) = &self.events {
            events.try_send(Call::Heartbeat).unwrap();
        }
        if client.fail_heartbeat {
            return Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into());
        }
        Ok(())
    }
}

#[test]
fn reports_ready_after_a_successful_preparation() {
    let client = FakeClient::default();
    let prepared = PreparedSandbox {
        generation: "abc123".to_string(),
        harness_working_dir: PathBuf::from("/workspace/warp"),
        deferred_repositories: Vec::new(),
    };

    let reported = futures::executor::block_on(report_preparation_outcome(
        &client,
        "sandbox",
        "abc123",
        Ok(prepared.clone()),
    ))
    .expect("a successful preparation is reported ready");

    assert_eq!(reported, prepared);
    assert_eq!(client.calls(), [Call::Status(PrewarmedSandboxState::Ready)]);
}

#[test]
fn reports_failed_when_preparation_errors() {
    let client = FakeClient::default();

    let error = futures::executor::block_on(report_preparation_outcome(
        &client,
        "sandbox",
        "abc123",
        Err(AgentDriverError::WarpDriveSyncFailed),
    ))
    .expect_err("a failed preparation propagates");

    assert_eq!(
        client.calls(),
        [Call::Status(PrewarmedSandboxState::Failed)]
    );
    assert!(
        error.to_string().contains("preparation failed"),
        "{error:#}"
    );
}

/// The reconciler marks a sandbox unhealthy after three minutes without a heartbeat, so the
/// cadence must leave room for a failed beat before that window closes.
#[test]
fn heartbeat_cadence_stays_inside_the_server_tolerance() {
    let timeout = crate::server::server_api::prewarmed_sandbox::REQUEST_TIMEOUT;
    assert!(
        (HEARTBEAT_INTERVAL + timeout) * 2 + HEARTBEAT_INTERVAL + timeout
            < SERVER_HEARTBEAT_TOLERANCE,
        "three complete heartbeat attempts must fit inside the server's tolerance"
    );
}

#[test]
fn waits_until_shortly_before_the_deadline() {
    let now = Utc::now();

    let window = wait_window(now + TimeDelta::minutes(30), now);
    assert_eq!(window, Duration::from_secs(30 * 60) - EXPIRY_MARGIN);

    // Already expired, or so close to expiry that the margin swallows the window.
    assert_eq!(
        wait_window(now - TimeDelta::minutes(1), now),
        Duration::ZERO
    );
    assert_eq!(
        wait_window(now + TimeDelta::seconds(5), now),
        Duration::ZERO
    );
}

#[test]
fn exits_cleanly_at_expiry() {
    let client = FakeClient::default();
    block_on(maintain_sandbox(
        &client,
        "sandbox",
        "abc123",
        async { panic!("expired sandboxes must not prepare") },
        future::ready(StandbyExit::Expired),
        stream::repeat(()),
    ))
    .unwrap();
    assert!(client.calls().is_empty(), "{:?}", client.calls());
}

#[test]
fn preparation_error_survives_a_status_credential_failure() {
    let client = FakeClient::default();
    client.state.lock().unwrap().fail_status = true;
    let error = block_on(maintain_sandbox(
        &client,
        "sandbox",
        "abc123",
        future::ready(Err(AgentDriverError::WarpDriveSyncFailed)),
        future::pending(),
        stream::pending(),
    ))
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AgentDriverError>(),
        Some(AgentDriverError::WarpDriveSyncFailed)
    ));
    assert_eq!(
        client.calls(),
        [Call::Status(PrewarmedSandboxState::Failed)]
    );
}

#[test]
fn expiry_cancels_in_flight_preparation_without_reporting_ready() {
    block_on(async {
        let (events_tx, events) = async_channel::unbounded();
        let client = FakeClient {
            events: Some(events_tx),
            ..Default::default()
        };
        let (started_tx, started_rx) = oneshot::channel();
        let (prepared_tx, prepared_rx) = oneshot::channel();
        let (expire_tx, expire_rx) = oneshot::channel();
        let (ticks_tx, ticks) = futures::channel::mpsc::unbounded();
        let lifecycle = maintain_sandbox(
            &client,
            "sandbox",
            "abc123",
            async {
                started_tx.send(()).unwrap();
                prepared_rx.await.unwrap()
            },
            expire_rx.map(|_| StandbyExit::Expired),
            ticks,
        );
        let exercise = async {
            started_rx.await.unwrap();
            ticks_tx.unbounded_send(()).unwrap();
            assert_eq!(events.recv().await.unwrap(), Call::Heartbeat);
            expire_tx.send(()).unwrap();
        };
        let (result, ()) = futures::future::join(lifecycle, exercise)
            .with_timeout(Duration::from_secs(5))
            .await
            .expect("expiry must cancel unfinished preparation");
        result.unwrap();
        assert!(prepared_tx.is_canceled());
        assert_eq!(client.calls(), [Call::Heartbeat]);
    });
}

#[test]
fn expiry_wins_when_preparation_completes_at_the_deadline() {
    block_on(async {
        let client = FakeClient::default();
        let (started_tx, started_rx) = oneshot::channel();
        let (prepared_tx, prepared_rx) = oneshot::channel();
        let (expire_tx, expire_rx) = oneshot::channel();
        let lifecycle = maintain_sandbox(
            &client,
            "sandbox",
            "abc123",
            async {
                started_tx.send(()).unwrap();
                prepared_rx.await.unwrap()
            },
            expire_rx.map(|_| StandbyExit::Expired),
            stream::pending(),
        );
        let exercise = async {
            started_rx.await.unwrap();
            prepared_tx
                .send(Ok(PreparedSandbox {
                    generation: "abc123".to_string(),
                    harness_working_dir: PathBuf::from("/workspace"),
                    deferred_repositories: Vec::new(),
                }))
                .unwrap();
            expire_tx.send(()).unwrap();
        };
        let (result, ()) = futures::future::join(lifecycle, exercise)
            .with_timeout(Duration::from_secs(5))
            .await
            .expect("expiry must take precedence over readiness");
        result.unwrap();
        assert!(client.calls().is_empty());
    });
}

#[test]
fn heartbeats_and_expiry_cover_in_flight_status_reports() {
    for state in [PrewarmedSandboxState::Ready, PrewarmedSandboxState::Failed] {
        block_on(async {
            let (events_tx, events) = async_channel::unbounded();
            let (release_tx, release_rx) = oneshot::channel();
            let client = FakeClient {
                state: Mutex::new(FakeClientState {
                    status_release: Some(release_rx),
                    ..Default::default()
                }),
                events: Some(events_tx),
            };
            let preparation = match state {
                PrewarmedSandboxState::Ready => Ok(PreparedSandbox {
                    generation: "abc123".to_string(),
                    harness_working_dir: PathBuf::from("/workspace"),
                    deferred_repositories: Vec::new(),
                }),
                PrewarmedSandboxState::Failed => Err(AgentDriverError::WarpDriveSyncFailed),
            };
            let (ticks_tx, ticks) = futures::channel::mpsc::unbounded();
            let (expire_tx, expire_rx) = oneshot::channel();
            let lifecycle = maintain_sandbox(
                &client,
                "sandbox",
                "abc123",
                future::ready(preparation),
                expire_rx.map(|_| StandbyExit::Expired),
                ticks,
            );
            let exercise = async {
                assert_eq!(events.recv().await.unwrap(), Call::Status(state));
                ticks_tx.unbounded_send(()).unwrap();
                assert_eq!(events.recv().await.unwrap(), Call::Heartbeat);
                expire_tx.send(()).unwrap();
            };
            let (result, ()) = futures::future::join(lifecycle, exercise)
                .with_timeout(Duration::from_secs(5))
                .await
                .expect("a blocked status report must not block heartbeats or expiry");
            result.unwrap();
            assert!(release_tx.is_canceled());
            assert_eq!(client.calls(), [Call::Status(state), Call::Heartbeat]);
        });
    }
}

#[test]
fn ready_report_failure_ends_standby() {
    let client = FakeClient::default();
    client.state.lock().unwrap().fail_status = true;
    let error = block_on(
        maintain_sandbox(
            &client,
            "sandbox",
            "abc123",
            future::ready(Ok(PreparedSandbox {
                generation: "abc123".to_string(),
                harness_working_dir: PathBuf::from("/workspace"),
                deferred_repositories: Vec::new(),
            })),
            future::pending(),
            stream::pending(),
        )
        .with_timeout(Duration::from_secs(5)),
    )
    .expect("failed readiness reporting must not leave standby idle")
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("workload token acquisition failed")
    );
    assert_eq!(client.calls(), [Call::Status(PrewarmedSandboxState::Ready)]);
}

#[test]
fn heartbeats_cover_preparation_and_idle_time_and_recover_after_timeouts() {
    block_on(async {
        let (events_tx, events) = async_channel::unbounded();
        let client = FakeClient {
            events: Some(events_tx),
            ..Default::default()
        };
        let (ticks_tx, ticks) = futures::channel::mpsc::unbounded();
        let (prepared_tx, prepared_rx) = oneshot::channel();
        let (expire_tx, expire_rx) = oneshot::channel();
        let lifecycle = maintain_sandbox(
            &client,
            "sandbox",
            "abc123",
            async { prepared_rx.await.unwrap() },
            expire_rx.map(|_| StandbyExit::Expired),
            ticks,
        );
        let exercise = async {
            client.state.lock().unwrap().fail_heartbeat = true;
            for _ in 0..3 {
                ticks_tx.unbounded_send(()).unwrap();
                assert_eq!(events.recv().await.unwrap(), Call::Heartbeat);
            }
            client.state.lock().unwrap().fail_heartbeat = false;
            prepared_tx
                .send(Ok(PreparedSandbox {
                    generation: "abc123".to_string(),
                    harness_working_dir: PathBuf::from("/workspace"),
                    deferred_repositories: Vec::new(),
                }))
                .unwrap();
            assert_eq!(
                events.recv().await.unwrap(),
                Call::Status(PrewarmedSandboxState::Ready)
            );
            ticks_tx.unbounded_send(()).unwrap();
            assert_eq!(events.recv().await.unwrap(), Call::Heartbeat);
            expire_tx.send(()).unwrap();
        };
        let (result, ()) = futures::join!(lifecycle, exercise);
        result.unwrap();
        assert_eq!(
            client.calls(),
            [
                Call::Heartbeat,
                Call::Heartbeat,
                Call::Heartbeat,
                Call::Status(PrewarmedSandboxState::Ready),
                Call::Heartbeat,
            ]
        );
    });
}
