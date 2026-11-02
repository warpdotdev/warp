use std::sync::{Arc, Mutex};

use chrono::TimeDelta;
use futures::channel::oneshot;
use futures::future;
use warp_core::features::FeatureFlag;
use warp_graphql::queries::execution_config::CodeForge as ConfigCodeForge;
use warpui::App;

use self::support::*;
use super::*;
use crate::auth::AuthStateProvider;
use crate::auth::user::{PrincipalType, User};
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::prewarmed_sandbox::tests::parse_configuration;

#[path = "standby_test_support.rs"]
mod support;

#[test]
fn supported_harnesses_validate_the_workspace_before_metadata_access() {
    App::test((), |mut app| async move {
        let runner = app.add_singleton_model(|_| AgentDriverRunner);
        let foreground = runner.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        let mut claude = parse_configuration();
        claude.preparation.harness = "claude".into();
        claude.preparation.factory_skill_dirs = vec!["../outside".into()];
        let error = prepare(&foreground, claude, &setup_events)
            .await
            .err()
            .unwrap();
        let AgentDriverError::ConfigBuildFailed(source) = error else {
            panic!("expected invalid workspace");
        };
        assert_eq!(
            source.to_string(),
            "Execution skill directory must stay within the working directory"
        );

        let mut codex = parse_configuration();
        codex.preparation.harness = "codex".into();
        codex.preparation.factory_skill_dirs = vec!["../outside".into()];
        let error = prepare(&foreground, codex, &setup_events)
            .await
            .err()
            .unwrap();
        let AgentDriverError::ConfigBuildFailed(source) = error else {
            panic!("expected invalid workspace");
        };
        assert_eq!(
            source.to_string(),
            "Execution skill directory must stay within the working directory"
        );
    });
}

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
        wrong_harness.preparation.harness = "gemini".to_string();
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
                .err()
                .expect("preparation should fail");
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
                .err()
                .expect("preparation should fail");
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
                .err()
                .expect("preparation should fail");
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
fn reports_ready_after_a_successful_preparation() {
    let client = FakeClient::default();

    let reported = futures::executor::block_on(report_preparation_outcome(
        &client,
        "sandbox",
        "abc123",
        Ok("prepared"),
    ))
    .expect("a successful preparation is reported ready");

    assert_eq!(reported, "prepared");
    assert_eq!(client.calls(), [Call::Status(PrewarmedSandboxState::Ready)]);
}

#[test]
fn reports_failed_when_preparation_errors() {
    let client = FakeClient::default();

    let error = futures::executor::block_on(report_preparation_outcome::<()>(
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
fn expiry_has_priority_over_ready_preparation_and_connection() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (_, connections) = async_channel::unbounded();
    exercise(
        drive(
            async { panic!("must not prepare after expiry") },
            connections,
            &clock,
            &hooks,
            0,
        ),
        async {},
    )
    .unwrap();
    assert!(hooks.calls.lock().unwrap().is_empty());
}

#[test]
fn heartbeats_cover_preparation_and_status_reporting() {
    for state in [PrewarmedSandboxState::Ready, PrewarmedSandboxState::Failed] {
        let clock = TestClock::new();
        let hooks = Hooks::default();
        let (connect_tx, connections) = async_channel::unbounded();
        let (connection, server) = connection();
        connect_tx.try_send(connection).unwrap();
        let (prepared_tx, prepared_rx) = oneshot::channel();
        let (status_tx, status_rx) = oneshot::channel();
        let (status_events, reports) = async_channel::unbounded();
        let client = FakeClient {
            state: Mutex::new(FakeClientState {
                status_release: Some(status_rx),
                ..Default::default()
            }),
            events: Some(status_events),
        };
        let preparation = async {
            report_preparation_outcome(&client, "sandbox", "abc123", prepared_rx.await.unwrap())
                .await
        };
        exercise(
            drive(preparation, connections, &clock, &hooks, 600),
            async {
                hooks.expect(Call::Connected).await;
                clock.advance(30).await;
                server.expect_heartbeat("abc123").await;
                prepared_tx
                    .send(match state {
                        PrewarmedSandboxState::Ready => Ok(7),
                        PrewarmedSandboxState::Failed => Err(AgentDriverError::WarpDriveSyncFailed),
                    })
                    .unwrap();
                assert_eq!(reports.recv().await.unwrap(), Call::Status(state));
                clock.advance(30).await;
                server.expect_heartbeat("abc123").await;
                clock.advance(600).await;
            },
        )
        .unwrap();
        assert!(status_tx.is_canceled());
        assert_eq!(hooks.count(Call::Install), 0);
    }
}

#[test]
fn assignment_installs_then_runs_only_after_delivery_is_confirmed() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (connect_tx, connections) = async_channel::unbounded();
    let (standby, standby_server) = connection();
    let (assigned, assigned_server) = connection();
    connect_tx.try_send(standby).unwrap();
    connect_tx.try_send(assigned).unwrap();
    exercise(
        drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
        async {
            hooks.expect(Call::Connected).await;
            standby_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Install).await;
            hooks.expect(Call::Connected).await;
            assert_eq!(hooks.count(Call::Run), 0);
            assigned_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Run).await;
        },
    )
    .unwrap();
    assert_eq!(hooks.count(Call::Install), 1);
    assert_eq!(hooks.count(Call::Run), 1);
}

#[test]
fn assignment_before_readiness_is_held_and_replays_install_once() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (connect_tx, connections) = async_channel::unbounded();
    let (standby, standby_server) = connection();
    let (assigned, assigned_server) = connection();
    connect_tx.try_send(standby).unwrap();
    connect_tx.try_send(assigned).unwrap();
    let (prepared_tx, prepared_rx) = oneshot::channel();
    exercise(
        drive(
            async { prepared_rx.await.unwrap() },
            connections,
            &clock,
            &hooks,
            600,
        ),
        async {
            hooks.expect(Call::Connected).await;
            standby_server.send(assignment("chosen", &clock));
            standby_server.send(assignment("chosen", &clock));
            yield_once().await;
            assert_eq!(hooks.count(Call::Install), 0);
            prepared_tx.send(Ok(7)).unwrap();
            hooks.expect(Call::Install).await;
            hooks.expect(Call::Connected).await;
            assigned_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Run).await;
        },
    )
    .unwrap();
    assert_eq!(hooks.count(Call::Install), 1);
}

#[test]
fn invalid_assignments_never_install() {
    for invalid in ["second", "generation", "expired"] {
        let clock = TestClock::new();
        let hooks = Hooks::default();
        let (connect_tx, connections) = async_channel::unbounded();
        let (connection, server) = connection();
        connect_tx.try_send(connection).unwrap();
        let result = exercise(
            drive(future::pending(), connections, &clock, &hooks, 600),
            async {
                hooks.expect(Call::Connected).await;
                let mut event = assignment("chosen", &clock);
                if let PrewarmedSandboxControlEvent::Assignment {
                    generation,
                    expires_at,
                    ..
                } = &mut event
                {
                    match invalid {
                        "second" => server.send(assignment("first", &clock)),
                        "generation" => *generation = "wrong".into(),
                        "expired" => *expires_at = clock.now(),
                        _ => unreachable!(),
                    }
                }
                server.send(event);
            },
        );
        assert!(result.is_err(), "{invalid} assignment was accepted");
        assert_eq!(hooks.count(Call::Install), 0);
    }
}

#[test]
fn retirement_and_unavailable_connection_never_start_execution() {
    for unavailable in [false, true] {
        let clock = TestClock::new();
        let hooks = Hooks::default();
        let (connect_tx, connections) = async_channel::unbounded();
        let (connection, server) = connection();
        connect_tx
            .try_send(if unavailable {
                ConnectionAttempt::Unavailable
            } else {
                connection
            })
            .unwrap();
        let result = exercise(
            drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
            async {
                if !unavailable {
                    hooks.expect(Call::Connected).await;
                    server.send(PrewarmedSandboxControlEvent::Retired { reason: None });
                }
            },
        );
        assert_eq!(result.is_err(), unavailable);
        assert_eq!(hooks.count(Call::Install), 0);
    }
}

#[test]
fn an_inactive_run_is_installed_but_never_started() {
    for outcome in ["gone", "retired", "different"] {
        let clock = TestClock::new();
        let hooks = Hooks::default();
        let (connect_tx, connections) = async_channel::unbounded();
        let (standby, standby_server) = connection();
        let (assigned, assigned_server) = connection();
        connect_tx.try_send(standby).unwrap();
        connect_tx
            .try_send(if outcome == "gone" {
                ConnectionAttempt::Unavailable
            } else {
                assigned
            })
            .unwrap();
        let result = exercise(
            drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
            async {
                hooks.expect(Call::Connected).await;
                standby_server.send(assignment("chosen", &clock));
                hooks.expect(Call::Install).await;
                match outcome {
                    "gone" => {}
                    "retired" => {
                        hooks.expect(Call::Connected).await;
                        assigned_server.send(PrewarmedSandboxControlEvent::Retired {
                            reason: Some("retired".into()),
                        });
                    }
                    "different" => {
                        hooks.expect(Call::Connected).await;
                        assigned_server.send(assignment("other", &clock));
                    }
                    _ => unreachable!(),
                }
            },
        );
        assert_eq!(result.is_err(), outcome == "different", "{outcome}");
        assert_eq!(hooks.count(Call::Install), 1);
        assert_eq!(hooks.count(Call::Run), 0);
    }
}

#[test]
fn unknown_control_messages_are_ignored_on_the_same_connection() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (connect_tx, connections) = async_channel::unbounded();
    let (standby, standby_server) = connection();
    let (assigned, assigned_server) = connection();
    connect_tx.try_send(standby).unwrap();
    connect_tx.try_send(assigned).unwrap();
    exercise(
        drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
        async {
            hooks.expect(Call::Connected).await;
            standby_server.send(PrewarmedSandboxControlEvent::Unknown);
            standby_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Install).await;
            hooks.expect(Call::Connected).await;
            assigned_server.send(PrewarmedSandboxControlEvent::Unknown);
            assigned_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Run).await;
        },
    )
    .unwrap();
    assert_eq!(hooks.count(Call::Connected), 2);
}

#[test]
fn delivery_confirmation_retries_transient_failures() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (connect_tx, connections) = async_channel::unbounded();
    let (standby, standby_server) = connection();
    let (dropped, dropped_server) = connection();
    let (assigned, assigned_server) = connection();
    dropped_server.events.close();
    for attempt in [standby, ConnectionAttempt::Retry, dropped, assigned] {
        connect_tx.try_send(attempt).unwrap();
    }
    exercise(
        drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
        async {
            hooks.expect(Call::Connected).await;
            standby_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Install).await;
            clock.advance(1).await;
            hooks.expect(Call::Connected).await;
            clock.advance(1).await;
            hooks.expect(Call::Connected).await;
            assigned_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Run).await;
        },
    )
    .unwrap();
    assert_eq!(hooks.count(Call::Install), 1);
}

#[test]
fn unconfirmed_delivery_fails_after_its_window() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (connect_tx, connections) = async_channel::unbounded();
    let (standby, standby_server) = connection();
    connect_tx.try_send(standby).unwrap();
    let error = exercise(
        drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
        async {
            hooks.expect(Call::Connected).await;
            standby_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Install).await;
            clock
                .advance(DELIVERY_CONFIRMATION_WINDOW.as_secs() as i64)
                .await;
        },
    )
    .unwrap_err();
    drop(connect_tx);
    assert!(
        error.to_string().contains("did not confirm delivery"),
        "{error:#}"
    );
    assert_eq!(hooks.count(Call::Run), 0);
}

#[test]
fn committed_assignment_uses_its_new_deadline() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (release, gate) = oneshot::channel();
    *hooks.install_gate.lock().unwrap() = Some(gate);
    let (connect_tx, connections) = async_channel::unbounded();
    let (connection, server) = connection();
    connect_tx.try_send(connection).unwrap();
    let result = exercise(
        drive(future::ready(Ok(7)), connections, &clock, &hooks, 60),
        async {
            hooks.expect(Call::Connected).await;
            server.send(assignment("chosen", &clock));
            hooks.expect(Call::Install).await;
            clock.advance(60).await;
            assert!(!release.is_canceled());
            clock.advance(600).await;
        },
    );
    assert!(result.unwrap_err().to_string().contains("deadline"));
    assert!(release.is_canceled());
    assert_eq!(hooks.count(Call::Run), 0);
}

#[test]
fn preparation_failure_survives_a_failed_status_report() {
    let client = FakeClient::default();
    client.state.lock().unwrap().fail_status = true;
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (_, connections) = async_channel::unbounded();
    let preparation = report_preparation_outcome::<usize>(
        &client,
        "sandbox",
        "abc123",
        Err(AgentDriverError::WarpDriveSyncFailed),
    );
    let error = exercise(
        drive(preparation, connections, &clock, &hooks, 600),
        async {},
    )
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AgentDriverError>(),
        Some(AgentDriverError::WarpDriveSyncFailed)
    ));
}

#[test]
fn stalled_connection_times_out_without_losing_preparation() {
    let clock = TestClock::new();
    let hooks = Hooks::default();
    let (connect_tx, connections) = async_channel::unbounded();
    let (standby, standby_server) = connection();
    let (assigned, assigned_server) = connection();
    exercise(
        drive(future::ready(Ok(7)), connections, &clock, &hooks, 600),
        async {
            clock.advance(20).await;
            connect_tx.try_send(standby).unwrap();
            connect_tx.try_send(assigned).unwrap();
            clock.advance(1).await;
            hooks.expect(Call::Connected).await;
            standby_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Install).await;
            hooks.expect(Call::Connected).await;
            assigned_server.send(assignment("chosen", &clock));
            hooks.expect(Call::Run).await;
        },
    )
    .unwrap();
    assert_eq!(hooks.count(Call::Install), 1);
}
