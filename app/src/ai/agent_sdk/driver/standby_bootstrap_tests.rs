//! Exercises CLI dispatch and HTTP preparation with seeded auth, mock Drive data, and shell events.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use clap::Parser;
use cloud_object_client::{InitialLoadResponse, MockObjectClient};
use command::r#async::Command as ProcessCommand;
use futures::executor::block_on;
use mockito::Matcher;
use serde_json::json;
use tempfile::TempDir;
use warp_cli::{Args, Command};
use warp_core::channel::ChannelState;
use warp_core::execution_mode::AppExecutionMode;
use warp_core::features::FeatureFlag;
use warp_server_client::base_client::AMBIENT_WORKLOAD_TOKEN_HEADER;
use warp_server_client::network_logging::NetworkLogModel;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{App, SingletonEntity};

use super::AgentDriver;
use crate::LaunchMode;
use crate::ai::agent_sdk::dispatch_command;
use crate::auth::AuthStateProvider;
use crate::auth::credentials::Credentials;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::prewarmed_sandbox::PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER;
use crate::terminal::view::Event;
use crate::workspace::view::tests::initialize_app;

const CHILD_ENV: &str = "WARP_TEST_STANDBY_BOOTSTRAP";
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn standby_command_bootstrap_subprocess() {
    if std::env::var_os(CHILD_ENV).is_some() {
        verify_standby_bootstrap();
        return;
    }

    let root = TempDir::new().unwrap();
    let home = root.path().join("home");
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    let mut command = ProcessCommand::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "ai::agent_sdk::driver::standby_bootstrap_tests::standby_command_bootstrap_subprocess",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD_ENV, "1")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("OZ_WORKSPACE_ROOT", &workspace)
        .env("WARP_ISOLATION_PLATFORM", "docker")
        .env("WARP_WORKLOAD_TOKEN", "test-workload-token")
        .current_dir(&workspace)
        .kill_on_drop(true);
    for name in [
        "PATH",
        "SYSTEMROOT",
        "TMPDIR",
        "TMP",
        "TEMP",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "LD_LIBRARY_PATH",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = block_on(command.output().with_timeout(Duration::from_secs(45)))
        .expect("standby bootstrap subprocess timed out")
        .expect("could not start standby bootstrap subprocess");
    assert!(
        output.status.success(),
        "standby bootstrap subprocess failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "the child test did not run:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}

fn verify_standby_bootstrap() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let mut server = ChannelState::mock_server();
    let configuration = server
        .mock("POST", "/graphql/v2?op=PrewarmedSandboxConfig")
        .match_header("authorization", "Bearer wk-standby-test-key")
        .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, "test-workload-token")
        .match_body(Matcher::PartialJson(json!({
            "variables": {
                "input": {
                    "sandboxId": "bootstrap-test"
                }
            }
        })))
        .with_status(200)
        .with_body(
            json!({
                "data": {
                    "prewarmedSandboxConfig": {
                        "__typename": "PrewarmedSandboxConfiguration",
                        "sandboxId": "bootstrap-test",
                        "fingerprint": "bootstrap-generation",
                        "spec": {
                            "harness": "oz",
                            "repositories": [],
                            "deferredRepositories": [],
                            "setupCommands": [],
                            "mcpServersJson": "{}",
                            "skills": [],
                            "factorySkillDirs": [],
                            "computerUseEnabled": false,
                            "providers": null
                        },
                        "environmentVariables": [
                            {"name": "STANDBY_TEST_ENV", "value": "prepared"}
                        ],
                        "managedSecrets": [
                            {
                                "name": "STANDBY_TEST_SECRET",
                                "value": {
                                    "__typename": "ManagedSecretRawValue",
                                    "value": "test-secret"
                                }
                            }
                        ],
                        "gitCredentials": [],
                        "expiresAt": Utc::now() + TimeDelta::minutes(5)
                    }
                }
            })
            .to_string(),
        )
        .expect(1)
        .create();
    let bootstrapped = Arc::new(AtomicBool::new(false));
    let bootstrapped_for_request = bootstrapped.clone();
    let (status_tx, status_rx) = async_channel::unbounded();
    let ready = server
        .mock(
            "POST",
            "/internal/prewarmed-sandboxes/bootstrap-test/status",
        )
        .match_header("authorization", "Bearer wk-standby-test-key")
        .match_header(
            PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER,
            "test-workload-token",
        )
        .match_body(Matcher::Json(json!({
            "fingerprint": "bootstrap-generation",
            "state": "ready",
            "manifest_hash": "bootstrap-generation"
        })))
        .match_request(move |_| {
            status_tx
                .try_send(bootstrapped_for_request.load(Ordering::SeqCst))
                .unwrap();
            true
        })
        .with_status(204)
        .expect(1)
        .create();
    drop(server);

    App::test((), |mut app| async move {
        let args = Args::try_parse_from([
            "oz",
            "agent",
            "run-cloud-standby",
            "--standby-id",
            "bootstrap-test",
        ])
        .unwrap();
        let Some(Command::CommandLine(command)) = args.command() else {
            panic!("standby arguments must parse as a CLI command");
        };
        let launch_mode = LaunchMode::command_line(
            command.as_ref().clone(),
            args.global_options().clone(),
            args.debug(),
        );
        assert!(launch_mode.is_sandboxed());
        let _factory_mcp = FeatureFlag::FactoryMcp.override_enabled(false);
        let _preview_mcp = FeatureFlag::PreviewUrlsMcp.override_enabled(false);
        let _global_skills = FeatureFlag::OzPlatformSkills.override_enabled(false);
        let sdk_mode = app.add_model(|ctx| {
            AppExecutionMode::new(
                launch_mode.execution_mode(),
                launch_mode.is_sandboxed(),
                ctx,
            )
        });
        initialize_app(&mut app);
        AppExecutionMode::handle(&app).update(&mut app, |mode, ctx| {
            *mode = sdk_mode.as_ref(ctx).clone();
        });
        app.add_singleton_model(|_| NetworkLogModel::default());
        let auth_state = app.read(|ctx| AuthStateProvider::as_ref(ctx).get().clone());
        auth_state.set_credentials(Some(Credentials::ApiKey {
            key: "wk-standby-test-key".to_string(),
            owner_type: None,
        }));
        ServerApiProvider::handle(&app).update(&mut app, |provider, ctx| {
            *provider = ServerApiProvider::new(auth_state, None, None, ctx);
        });
        let mut object_client = MockObjectClient::new();
        object_client
            .expect_fetch_environment_last_task_run_timestamps()
            .returning(|| Ok(HashMap::new()));
        UpdateManager::handle(&app).update(&mut app, |manager, ctx| {
            *manager = UpdateManager::new(None, Arc::new(object_client), ctx);
            manager.mock_initial_load(InitialLoadResponse::default(), ctx);
        });
        app.update(|ctx| {
            dispatch_command(ctx, command.as_ref().clone(), args.global_options().clone()).unwrap();
        });
        let driver = async {
            loop {
                if let Some(driver) = app.models_of_type::<AgentDriver>().into_iter().next()
                    && driver.read(&app, |driver, _| {
                        !matches!(driver.workspace, Some(super::WorkspaceState::Pending(_)))
                    })
                {
                    break driver;
                }
                Timer::after(Duration::from_millis(10)).await;
            }
        }
        .with_timeout(STEP_TIMEOUT)
        .await
        .expect("standby did not start driver preparation");

        assert_eq!(AgentDriver::handle(&app).id(), driver.id());
        assert!(
            status_rx
                .recv()
                .with_timeout(Duration::from_millis(100))
                .await
                .is_err(),
            "standby reported readiness before shell bootstrap",
        );
        let terminal = driver.read(&app, |driver, ctx| {
            driver.terminal_driver.as_ref(ctx).terminal_view().clone()
        });
        // The test platform does not start a shell, so it must supply the bootstrap event.
        bootstrapped.store(true, Ordering::SeqCst);
        terminal.update(&mut app, |_, ctx| ctx.emit(Event::SessionBootstrapped));
        assert!(
            status_rx
                .recv()
                .with_timeout(STEP_TIMEOUT)
                .await
                .expect("standby did not report readiness")
                .unwrap(),
            "the ready request preceded the bootstrap event",
        );

        let workspace = PathBuf::from(std::env::var_os("OZ_WORKSPACE_ROOT").unwrap());
        driver.read(&app, |driver, _| {
            assert_eq!(driver.working_dir, workspace);
            assert_eq!(driver.harness_working_dir, workspace);
            let Some(super::WorkspaceState::Prepared(prepared)) = &driver.workspace else {
                panic!("standby preparation must leave a prepared workspace");
            };
            assert!(prepared.repositories.is_empty());
            assert!(driver.task_id.is_none());
            assert!(driver.harness.is_none());
            assert!(driver.run_conversation_id.is_none());
            assert!(driver.snapshot_file_writer.is_none());
            assert!(driver.checkpoint_coordinator.is_none());
            assert!(driver.computer_use_configured);
            assert!(driver.skill_dirs.is_empty());
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("STANDBY_TEST_ENV")],
                "prepared"
            );
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("STANDBY_TEST_SECRET")],
                "test-secret"
            );
        });
        let sharing = driver.update(&mut app, |driver, ctx| {
            driver
                .terminal_driver
                .update(ctx, |driver, _| driver.wait_for_session_shared())
        });
        sharing
            .with_timeout(STEP_TIMEOUT)
            .await
            .expect("standby must not wait for session sharing")
            .unwrap();
        assert!(app.termination_result().is_none());
    });
    configuration.assert();
    ready.assert();
}
