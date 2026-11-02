use std::ffi::OsStr;
use std::path::Path;

use chrono::TimeDelta;
#[cfg(not(feature = "skip_login"))]
use mockito::Matcher;
use session_sharing_protocol::common::ParticipantId;
use warp_graphql::queries::execution_config::{CodeForge, ExecutionRepository};
use warp_multi_agent_api as api;
use warpui::App;

use super::environment_preparation::{FILE_ENV, MODE_ENV, PreparationPolicy};
use super::*;
use crate::ai::agent::conversation::AIConversation;
use crate::ai::ambient_agents::task::TaskAttachment;
use crate::ai::cloud_environments::{AwsProviderConfig, GcpProviderConfig, ProvidersConfig};
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::prewarmed_sandbox::PrewarmedSandboxControlEvent;
use crate::terminal::model::blocks::BlockHeightItem;
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
fn assignment_transport_defaults_to_pty_and_rejects_invalid_values() {
    assert_eq!(
        assignment_harness_transport(&HashMap::new()).unwrap(),
        HarnessTransport::Pty
    );
    let runtime_env = HashMap::from([("WARP_HARNESS_TRANSPORT".into(), "invalid".into())]);
    assert!(assignment_harness_transport(&runtime_env).is_err());
}

#[test]
fn claude_acp_assignment_applies_run_environment_and_model() {
    assert_third_party_assignment(
        warp_graphql::ai::AgentHarness::ClaudeCode,
        Harness::Claude,
        HarnessTransport::Acp,
        "acp",
    );
}

#[test]
fn codex_pty_assignment_applies_run_environment_and_model() {
    assert_third_party_assignment(
        warp_graphql::ai::AgentHarness::Codex,
        Harness::Codex,
        HarnessTransport::Pty,
        "pty",
    );
}

fn assert_third_party_assignment(
    server_harness: warp_graphql::ai::AgentHarness,
    harness: Harness,
    transport: HarnessTransport,
    transport_name: &str,
) {
    let transport_name = transport_name.to_owned();
    App::test((), move |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let root = tempfile::tempdir().unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_driver =
            app.update(|ctx| terminal::TerminalDriver::create_from_existing_view(terminal, ctx));
        let driver = app
            .add_model(|ctx| AgentDriver::new_for_test(root.path().into(), terminal_driver, ctx));
        let mut configuration = config();
        configuration.harness = server_harness;
        configuration.model_id = Some("assigned-model".into());
        configuration.factory_skill_dirs = vec![".agents/skills".into()];
        let runtime_env = HashMap::from([
            ("WARP_HARNESS_TRANSPORT".into(), transport_name),
            ("WARP_API_KEY".into(), "run-key".into()),
            ("RUN_SETTING".into(), "assigned".into()),
        ]);
        validate_assignment(&configuration).unwrap();
        let (mut options, task, _, _) = app.update(|ctx| {
            build_execution_task_and_options(
                None,
                assignment_harness_transport(&runtime_env).unwrap(),
                configuration,
                RepositoryContract {
                    supports_deferred_repositories: true,
                    use_factory_repositories: true,
                },
                root.path().into(),
                None,
                Vec::new(),
                ctx,
            )
            .unwrap()
        });
        let HarnessKind::ThirdParty(third_party) = &task.harness else {
            panic!("expected third-party harness");
        };
        assert_eq!(third_party.harness(), harness);
        assert_eq!(
            third_party.drives_cli_agent_session(),
            transport == HarnessTransport::Pty
        );
        assert_eq!(options.harness_transport, transport);
        assert!(options.computer_use_config.is_none());
        options.environment_variables = runtime_env;
        options.secrets.insert(
            "HARNESS_SECRET".into(),
            warp_managed_secrets::ManagedSecretValue::raw_value("assigned-secret"),
        );
        let prepared_configuration = options.workspace.clone();
        let (terminal_inputs, run_config) = split_run_options(options).unwrap();
        driver.update(&mut app, |driver, ctx| {
            driver.workspace = Some(WorkspaceState::Prepared(StandbyWorkspace {
                harness,
                configuration: prepared_configuration,
                repositories: Vec::new(),
            }));
            driver
                .apply_prewarmed_run_configuration(terminal_inputs, run_config, ctx)
                .unwrap();
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("WARP_API_KEY")],
                "run-key"
            );
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("RUN_SETTING")],
                "assigned"
            );
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("HARNESS_SECRET")],
                "assigned-secret"
            );
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("WARP_HARNESS")],
                harness.config_name()
            );
            assert_eq!(
                driver
                    .third_party_harness_model_config
                    .as_ref()
                    .unwrap()
                    .model_id,
                "assigned-model"
            );
            assert_eq!(driver.skill_dirs, [PathBuf::from(".agents/skills")]);
            assert!(driver.harness.is_none());
            assert!(matches!(
                driver.workspace,
                Some(WorkspaceState::Prepared(_))
            ));
        });
    });
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
        unsupported.harness = warp_graphql::ai::AgentHarness::Gemini;
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
    validate_assignment(&configuration).unwrap();
    configuration.skip_initial_turn = false;
    configuration.conversation_id = Some("conversation".into());
    validate_assignment(&configuration).unwrap();
    configuration.conversation_id = None;
    configuration.harness = warp_graphql::ai::AgentHarness::ClaudeCode;
    validate_assignment(&configuration).unwrap();
    configuration.harness = warp_graphql::ai::AgentHarness::Codex;
    validate_assignment(&configuration).unwrap();
    configuration.harness = warp_graphql::ai::AgentHarness::Gemini;
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
                HarnessTransport::Pty,
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

fn completed_conversation(working_dir: &Path) -> AIConversation {
    let mut conversation = AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".into(),
            messages: vec![
                api::Message {
                    id: "user".into(),
                    task_id: "root-task".into(),
                    request_id: "previous-request".into(),
                    message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                        query: "previous prompt".into(),
                        context: Some(api::InputContext {
                            directory: Some(api::input_context::Directory {
                                pwd: working_dir.to_string_lossy().into_owned(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                api::Message {
                    id: "answer".into(),
                    task_id: "root-task".into(),
                    request_id: "previous-request".into(),
                    message: Some(api::message::Message::AgentOutput(
                        api::message::AgentOutput {
                            text: "previous answer".into(),
                        },
                    )),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        None,
    )
    .unwrap();
    conversation.set_server_conversation_token("conversation".into());
    conversation
}

fn install_resumed_configuration(
    app: &mut App,
    root: &Path,
    conversation: AIConversation,
    skip_initial_turn: bool,
) -> (
    ModelHandle<AgentDriver>,
    warpui::ViewHandle<crate::terminal::TerminalView>,
    Task,
) {
    let terminal = add_window_with_terminal(app, None);
    terminal.update(app, |terminal, _| {
        terminal
            .model
            .lock()
            .simulate_block("standby setup", "prepared");
    });
    let terminal_driver = app
        .update(|ctx| terminal::TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
    let driver = app.add_model(|ctx| AgentDriver::new_for_test(root.into(), terminal_driver, ctx));
    let mut configuration = config();
    configuration.conversation_id = Some("conversation".into());
    configuration.parent_run_id = Some("parent-run".into());
    configuration.skip_initial_turn = skip_initial_turn;
    configuration.idle_on_complete_seconds = Some(300);
    let (mut options, task, conversation_id, _) = app.update(|ctx| {
        build_execution_task_and_options(
            None,
            HarnessTransport::Pty,
            configuration,
            RepositoryContract {
                supports_deferred_repositories: true,
                use_factory_repositories: true,
            },
            root.into(),
            None,
            Vec::new(),
            ctx,
        )
        .unwrap()
    });
    assert_eq!(conversation_id.as_deref(), Some("conversation"));
    options.resume = Some(ResumeOptions::Native(Box::new(
        ConversationRestorationInNewPaneType::Historical {
            conversation,
            should_use_live_appearance: false,
            ambient_agent_task_id: None,
        },
    )));
    let prepared_configuration = options.workspace.clone();
    let (terminal_inputs, run_config) = split_run_options(options).unwrap();
    driver.update(app, |driver, ctx| {
        driver.workspace = Some(WorkspaceState::Prepared(StandbyWorkspace {
            harness: Harness::Oz,
            configuration: prepared_configuration,
            repositories: Vec::new(),
        }));
        driver
            .apply_prewarmed_run_configuration(terminal_inputs, run_config, ctx)
            .unwrap();
        driver.terminal_driver.update(ctx, |terminal_driver, ctx| {
            terminal_driver
                .bind_prewarmed_task(
                    terminal::PrewarmedTaskBinding {
                        task_id: TASK_ID.parse().unwrap(),
                        working_dir: root.into(),
                        team_scope: &HeadlessTeamScope::Personal,
                        share_requests: Vec::new(),
                        should_share: false,
                        restored_conversation_id: driver.restored_conversation_id,
                    },
                    ctx,
                )
                .unwrap();
        });
    });
    (driver, terminal, task)
}

#[test]
fn prewarmed_resume_restores_conversation_without_replacing_workspace_root() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let root = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let conversation = completed_conversation(other_dir.path());
        let conversation_id = conversation.id();

        let (driver, terminal, _) =
            install_resumed_configuration(&mut app, root.path(), conversation, false);

        driver.read(&app, |driver, _| {
            assert_eq!(driver.restored_conversation_id, Some(conversation_id));
            assert_eq!(driver.run_conversation_id, Some(conversation_id));
            assert_eq!(driver.working_dir, root.path());
            assert_eq!(driver.harness_working_dir, root.path());
            assert!(matches!(
                driver.workspace,
                Some(WorkspaceState::Prepared(_))
            ));
        });
        terminal.read(&app, |terminal, ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let conversation = history.conversation(&conversation_id).unwrap();
            assert_eq!(conversation.exchange_count(), 1);
            assert_eq!(conversation.parent_agent_id(), Some("parent-run"));
            assert_eq!(
                conversation.initial_working_directory().as_deref(),
                other_dir.path().to_str()
            );
            assert_eq!(
                history.terminal_surface_id_for_conversation(&conversation_id),
                Some(terminal.id())
            );
            let ai_block = terminal.last_ai_block().unwrap();
            let model = terminal.model.lock();
            let block_heights = model.block_list().block_heights().items();
            let setup_position = block_heights
                .iter()
                .position(|item| matches!(item, BlockHeightItem::Block(_)))
                .unwrap();
            let restored_position = block_heights
                .iter()
                .position(|item| matches!(item, BlockHeightItem::RichContent(item) if item.view_id == ai_block.id()))
                .unwrap();
            assert!(setup_position < restored_position);
            drop(model);
            assert_eq!(
                terminal.ai_controller().as_ref(ctx).native_prompt_conversation_id(),
                Some(conversation_id)
            );
            assert!(QueuedQueryModel::as_ref(ctx).has_pending_native_injections(conversation_id));
        });
    });
}

#[test]
fn prewarmed_promptless_resume_idles_until_a_queued_followup_arrives() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let root = tempfile::tempdir().unwrap();
        let conversation = completed_conversation(root.path());
        let conversation_id = conversation.id();
        let (driver, terminal, task) =
            install_resumed_configuration(&mut app, root.path(), conversation, true);

        let mut completion =
            driver.update(&mut app, |driver, ctx| driver.execute_run(task.prompt, ctx));
        assert!(completion.try_recv().unwrap().is_none());
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history
                    .conversation(&conversation_id)
                    .unwrap()
                    .exchange_count(),
                1
            );
        });
        QueuedQueryModel::handle(&app).read(&app, |queue, _| {
            assert!(!queue.has_pending_native_injections(conversation_id));
        });
        let controller = terminal.read(&app, |terminal, _| terminal.ai_controller().clone());
        controller.update(&mut app, |controller, ctx| {
            controller.execute_warp_agent_prompt_from_shared_session_injection(
                "queued followup".into(),
                None,
                Vec::new(),
                ParticipantId::new(),
                None,
                ctx,
            );
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let conversation = history.conversation(&conversation_id).unwrap();
            assert_eq!(conversation.exchange_count(), 2);
            assert!(matches!(
                &conversation.all_exchanges().last().unwrap().input[0],
                AIAgentInput::UserQuery { query, .. } if query == "queued followup"
            ));
        });
        assert!(completion.try_recv().unwrap().is_none());
        driver.update(&mut app, |driver, ctx| {
            driver.unregister_streamer_consumer(ctx)
        });
    });
}

#[test]
fn prewarmed_handoff_assignment_downloads_snapshot_before_execution() {
    let mut server = mockito::Server::new();
    let download = server
        .mock("GET", "/snapshot")
        .with_body("snapshot patch")
        .expect(1)
        .create();
    let attachment = TaskAttachment {
        file_id: "snapshot".into(),
        filename: "repository.patch".into(),
        download_url: format!("{}/snapshot", server.url()),
        mime_type: "text/plain".into(),
    };
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _handoff = FeatureFlag::OzHandoff.override_enabled(true);
        let root = tempfile::tempdir().unwrap();
        let (mut options, mut task, _, _) = app.update(|ctx| {
            build_execution_task_and_options(
                None,
                HarnessTransport::Pty,
                config(),
                RepositoryContract {
                    supports_deferred_repositories: true,
                    use_factory_repositories: true,
                },
                root.path().into(),
                None,
                Vec::new(),
                ctx,
            )
            .unwrap()
        });
        let server_api = app.read(|ctx| ServerApiProvider::as_ref(ctx).get());
        let mut ai_client = MockAIClient::new();
        ai_client
            .expect_get_handoff_snapshot_attachments()
            .withf(|task_id| task_id.to_string() == TASK_ID)
            .times(1)
            .returning(move |_| Ok(vec![attachment.clone()]));
        let ai_client: Arc<dyn AIClient> = Arc::new(ai_client);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _enter = runtime.enter();

        AgentDriverRunner::prepare_execution_task_data(
            server_api,
            &ai_client,
            TASK_ID.parse().unwrap(),
            HashMap::new(),
            Ok(Vec::new()),
            &mut options,
            &mut task,
        )
        .await
        .unwrap();

        let AgentRunPrompt::ServerSide {
            attachments_dir: Some(directory),
            ..
        } = task.prompt
        else {
            panic!("expected handoff attachments");
        };
        assert_eq!(
            std::fs::read(Path::new(&directory).join("handoff/repository.patch")).unwrap(),
            b"snapshot patch"
        );
    });
    download.assert();
}
