use std::ffi::OsStr;

use chrono::TimeDelta;
#[cfg(not(feature = "skip_login"))]
use mockito::Matcher;
use warp_graphql::queries::execution_config::{CodeForge, ExecutionRepository};
use warpui::App;

use super::environment_preparation::{FILE_ENV, MODE_ENV, PreparationPolicy};
use super::*;
use crate::ai::cloud_environments::{AwsProviderConfig, GcpProviderConfig, ProvidersConfig};
use crate::server::server_api::prewarmed_sandbox::PrewarmedSandboxControlEvent;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

const TASK_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn server_assignment() -> ExecutionConfiguration {
    let event: PrewarmedSandboxControlEvent = serde_json::from_str(include_str!(
        "../../../server/server_api/testdata/prewarmed_assignment_fresh_run.json"
    ))
    .unwrap();
    let PrewarmedSandboxControlEvent::Assignment {
        execution_config, ..
    } = event
    else {
        panic!("expected assignment");
    };
    *execution_config
}

fn config() -> ExecutionConfiguration {
    let mut execution_config = server_assignment();
    execution_config.conversation_id = None;
    execution_config.skip_initial_turn = false;
    execution_config.providers = None;
    execution_config.inference_providers = None;
    execution_config
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn installation_failures_are_reported_against_the_run_with_its_credential() {
    if std::env::var_os("WARP_TEST_PREWARMED_ASSIGNMENT_HOME").is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = command::blocking::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ai::agent_sdk::driver::prewarmed::tests::installation_failures_are_reported_against_the_run_with_its_credential",
                "--nocapture",
            ])
            .env("WARP_TEST_PREWARMED_ASSIGNMENT_HOME", home.path())
            .env("HOME", home.path())
            .env("GH_CONFIG_DIR", home.path().join("gh"))
            .env("WARP_ISOLATION_PLATFORM", "docker")
            .env("WARP_WORKLOAD_TOKEN", "test-workload-token")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let home = PathBuf::from(std::env::var_os("WARP_TEST_PREWARMED_ASSIGNMENT_HOME").unwrap());
    std::fs::write(
        home.join(".git-credentials"),
        "https://x-access-token:standby-token@github.com\n",
    )
    .unwrap();
    std::fs::create_dir_all(home.join("gh")).unwrap();
    std::fs::write(
        home.join("gh/hosts.yml"),
        "github.com:\n    oauth_token: standby-token\n",
    )
    .unwrap();
    let mut server = crate::ChannelState::mock_server();
    let report = server
        .mock("POST", "/graphql/v2?op=UpdateAgentTask")
        .match_header("authorization", "Bearer assigned-run-key")
        .match_body(Matcher::PartialJson(serde_json::json!({
            "variables": { "input": { "taskId": TASK_ID } }
        })))
        .with_body(
            serde_json::json!({"data": {"updateAgentTask": {
                "__typename": "UpdateAgentTaskOutput",
                "responseContext": {"serverVersion": null}
            }}})
            .to_string(),
        )
        .expect(1)
        .create();
    drop(server);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        // Production builds the server API over the shared auth state, so installing the run
        // credential changes what every later request sends.
        app.add_singleton_model(|_| {
            warp_server_client::network_logging::NetworkLogModel::default()
        });
        let auth_state = app.read(|ctx| AuthStateProvider::as_ref(ctx).get().clone());
        ServerApiProvider::handle(&app).update(&mut app, |provider, ctx| {
            *provider = ServerApiProvider::new(auth_state, None, None, ctx);
        });
        let root = tempfile::tempdir().unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_driver = app.update(|ctx| {
            terminal::TerminalDriver::create_from_existing_view(terminal.clone(), ctx)
        });
        let driver = app
            .add_model(|ctx| AgentDriver::new_for_test(root.path().into(), terminal_driver, ctx));
        let (spawner, credential_paths) = driver.update(&mut app, |driver, ctx| {
            driver.cloud_providers = cloud_provider::load_providers(
                &ProvidersConfig {
                    aws: Some(AwsProviderConfig {
                        role_arn: "arn:aws:iam::123:role/Standby".into(),
                    }),
                    gcp: Some(GcpProviderConfig {
                        project_number: "123".into(),
                        workload_identity_federation_pool_id: "pool".into(),
                        workload_identity_federation_provider_id: "provider".into(),
                        service_account_email: None,
                    }),
                },
                None,
            )
            .unwrap();
            let mut vars = HashMap::new();
            cloud_provider::collect_env_vars(&driver.cloud_providers, &mut vars).unwrap();
            let paths = [
                PathBuf::from(&vars[OsStr::new("AWS_WEB_IDENTITY_TOKEN_FILE")]),
                PathBuf::from(&vars[OsStr::new("GOOGLE_APPLICATION_CREDENTIALS")]),
            ];
            assert!(paths.iter().all(|path| path.exists()));
            (ctx.spawner(), paths)
        });
        let mut unsupported = config();
        unsupported.harness = warp_graphql::ai::AgentHarness::ClaudeCode;
        let error = AgentDriver::prepare_prewarmed_assignment(
            &spawner,
            unsupported,
            HashMap::new(),
            "assigned-run-key".into(),
            Utc::now() + TimeDelta::hours(1),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AgentDriverError::ConfigBuildFailed(_)));
        assert!(credential_paths.iter().all(|path| !path.exists()));
    });
    report.assert();
    assert!(!home.join(".git-credentials").exists());
    assert!(!home.join("gh/hosts.yml").exists());
}

/// The server commits runs with cloud providers to pre-warmed sandboxes, so validation must
/// accept them.
#[test]
fn prewarmed_accepts_server_assignments_and_rejects_unsupported_runs() {
    let mut configuration = server_assignment();
    assert!(configuration.providers.is_some());
    validate_assignment(&configuration).unwrap();
    configuration.skip_initial_turn = true;
    assert!(validate_assignment(&configuration).is_err());
    configuration.skip_initial_turn = false;
    configuration.conversation_id = Some("conversation".into());
    assert!(validate_assignment(&configuration).is_err());
    configuration.conversation_id = None;
    configuration.harness = warp_graphql::ai::AgentHarness::ClaudeCode;
    assert!(validate_assignment(&configuration).is_err());
}

#[test]
fn prewarmed_rejects_deferred_repositories_without_client_support() {
    let mut configuration = config();
    configuration.deferred_repositories = Some(vec![ExecutionRepository {
        forge: CodeForge::GitHub,
        owner: "benchmarks".into(),
        name: "frozen-api".into(),
        ref_: None,
        clone_from: None,
        preserve_origin: false,
        expected_sha: None,
    }]);
    {
        let _unsupported = FeatureFlag::FactoryDeferredRepositories.override_enabled(false);
        assert!(validate_assignment(&configuration).is_err());
    }
    let _supported = FeatureFlag::FactoryDeferredRepositories.override_enabled(true);
    validate_assignment(&configuration).unwrap();
}

#[test]
fn prewarmed_config_conversion_and_state_binding_preserve_run_settings() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _factory = FeatureFlag::FactoryDeferredRepositories.override_enabled(true);
        let root = tempfile::tempdir().unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_driver = app.update(|ctx| {
            terminal::TerminalDriver::create_from_existing_view(terminal.clone(), ctx)
        });
        let terminal_id = terminal_driver.id();
        let driver = app
            .add_model(|ctx| AgentDriver::new_for_test(root.path().into(), terminal_driver, ctx));
        let mut configuration = config();
        configuration.profile_id = Some("run-profile".into());
        configuration.parent_run_id = Some("parent-run".into());
        configuration.idle_on_complete_seconds = Some(17);
        configuration.idle_on_fail_seconds = Some(23);
        configuration.factory_skill_dirs = vec![".agents/skills".into()];
        configuration.snapshot_disabled = true;
        let runtime_env = HashMap::from([
            ("WARP_API_KEY".into(), "assigned-key".into()),
            ("WARP_FACTORY_ID".into(), "factory-id".into()),
            ("WARP_METADATA_TICKET_ID".into(), "ENG-42".into()),
        ]);
        let (mut options, task, _, _) = app.update(|ctx| {
            build_execution_task_and_options(
                None,
                configuration,
                RepositoryContract {
                    supports_deferred_repositories: true,
                    use_factory_repositories: factory_repository_contract(
                        TASK_ID.parse().ok(),
                        true,
                        runtime_env.get(FACTORY_ID_ENV),
                    ),
                },
                root.path().into(),
                None,
                vec![root.path().join("discovered")],
                ctx,
            )
            .unwrap()
        });
        assert_eq!(task.profile.as_deref(), Some("run-profile"));
        assert!(matches!(task.prompt, AgentRunPrompt::ServerSide { .. }));
        options.team_scope = Some(HeadlessTeamScope::Personal);
        options.environment_variables = runtime_env;
        let deadline = Utc::now() + TimeDelta::hours(1);
        let (_, run_config) = split_run_options(options).unwrap();
        driver.update(&mut app, |driver, ctx| {
            driver.apply_run_configuration(run_config, ctx);
            driver.sandbox_deadline = Some(deadline.into());
        });
        driver.read(&app, |driver, _| {
            let value = |name: &str| {
                driver
                    .resolved_env_vars
                    .get(OsStr::new(name))
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_owned()
            };
            assert_eq!(value("WARP_API_KEY"), "assigned-key");
            assert_eq!(value("WARP_RUN_ID"), TASK_ID);
            assert_eq!(value("WARP_METADATA_TICKET_ID"), "ENG-42");
            assert_eq!(value(MODE_ENV), "execution");
            assert_eq!(value("WARP_EXECUTION_ID"), "42");
            let policy: PreparationPolicy =
                serde_json::from_slice(&std::fs::read(value(FILE_ENV)).unwrap()).unwrap();
            assert_eq!(policy.task_id, TASK_ID);
            assert_eq!(policy.execution_id, "42");
            assert_eq!(policy.working_dir, root.path());
            assert_eq!(driver.terminal_driver.id(), terminal_id);
            assert_eq!(driver.task_id.unwrap().to_string(), TASK_ID);
            assert_eq!(driver.parent_run_id.as_deref(), Some("parent-run"));
            assert_eq!(driver.idle_on_complete, Some(Duration::from_secs(17)));
            assert_eq!(driver.idle_on_fail, Some(Duration::from_secs(23)));
            assert_eq!(driver.sandbox_deadline, Some(deadline.into()));
            assert!(driver.use_factory_repositories);
            assert_eq!(
                driver.skill_dirs,
                vec![
                    PathBuf::from(".agents/skills"),
                    root.path().join("discovered")
                ]
            );
            assert!(driver.snapshot_file_writer.is_none());
        });
    });
}

#[test]
fn prewarmed_workspace_reuse_checks_setup_and_checkouts() {
    let first = environment::WorkspaceConfiguration::from_resolved(
        Vec::new(),
        Vec::new(),
        vec!["prepare".into()],
    )
    .unwrap();
    let mut second = first.clone();
    second.factory_skill_dirs = Some(vec!["new-skills".into()]);
    assert!(first.same_preparation(&second));
    second.setup_commands.push("different-setup".into());
    assert!(!first.same_preparation(&second));
}
