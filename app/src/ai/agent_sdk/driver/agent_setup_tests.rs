use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::sync::Arc;

use tempfile::TempDir;
use warpui::App;

use super::super::AgentDriverOptions;
use super::super::cloud_provider::{
    CloudProvider, CloudProviderSetupError, Result as ProviderResult,
};
use super::super::environment::WorkspaceConfiguration;
use super::super::terminal::TerminalDriver;
use super::*;
use crate::server::server_api::ai::MockAIClient;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspace::view::tests::initialize_app as initialize_workspace_test_app;

#[test]
fn claude_standby_prepares_without_starting_oz_mcp_servers() {
    assert_third_party_standby_preparation(Harness::Claude);
}

#[test]
fn codex_standby_prepares_without_starting_oz_mcp_servers() {
    assert_third_party_standby_preparation(Harness::Codex);
}

fn assert_third_party_standby_preparation(harness: Harness) {
    App::test((), move |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let root = TempDir::new().unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver = AgentDriver::new_for_test(root.path().into(), terminal_driver, ctx);
            driver.strict_mcp_startup = true;
            driver
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        let events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        let working_dir = AgentDriver::prepare_resources(
            &foreground,
            harness,
            vec![MCPSpec::Json("invalid MCP configuration".into())],
            GlobalSkillResolution {
                specs: Vec::new(),
                repos: Vec::new(),
            },
            &events,
        )
        .await
        .unwrap();

        assert_eq!(working_dir, root.path());
        driver.read(&app, |driver, _| {
            let Some(WorkspaceState::Prepared(prepared)) = &driver.workspace else {
                panic!("expected prepared workspace");
            };
            assert_eq!(prepared.harness, harness);
            assert!(driver.task_id.is_none());
            assert!(driver.harness.is_none());
        });
    });
}

#[test]
fn driver_without_a_task_keeps_preparation_inputs_without_run_state() {
    App::test((), |mut app| async move {
        initialize_workspace_test_app(&mut app);
        let root = TempDir::new().unwrap();
        let mut options = AgentDriverOptions {
            working_dir: root.path().to_path_buf(),
            environment_variables: HashMap::from([
                ("WARP_FACTORY_REPO_DIR".into(), "factory".into()),
                ("WARP_SKILL_DIRS".into(), "ignored".into()),
            ]),
            secrets: HashMap::from([(
                "WARP_PREPARATION_TEST_SECRET".into(),
                warp_managed_secrets::ManagedSecretValue::raw_value("test-value"),
            )]),
            computer_use_config: Some((true, None)),
            ..Default::default()
        };
        options.workspace.factory_skill_dirs = Some(vec![PathBuf::from("skills")]);
        let driver = app.update(|ctx| AgentDriver::create(options, ctx).unwrap());
        assert_eq!(AgentDriver::handle(&app).id(), driver.id());
        let sharing = driver.update(&mut app, |driver, ctx| {
            assert!(driver.task_id.is_none());
            assert!(driver.harness.is_none());
            assert!(driver.run_conversation_id.is_none());
            assert!(driver.snapshot_file_writer.is_none());
            assert!(driver.checkpoint_coordinator.is_none());
            assert!(driver.computer_use_configured);
            assert_eq!(driver.skill_dirs, [PathBuf::from("skills")]);
            assert_eq!(
                driver.resolved_env_vars[&OsString::from("WARP_FACTORY_REPO_DIR")],
                "factory"
            );
            assert_eq!(
                driver.resolved_env_vars[&OsString::from("WARP_SKILL_DIRS")],
                "skills"
            );
            assert_eq!(
                driver.resolved_env_vars[&OsString::from("WARP_PREPARATION_TEST_SECRET")],
                "test-value"
            );
            driver
                .terminal_driver
                .update(ctx, |driver, _| driver.wait_for_session_shared())
        });
        sharing.await.unwrap();
    });
}

#[test]
fn resource_preparation_rejects_a_consumed_workspace() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let root = TempDir::new().unwrap();
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            AgentDriver::new_for_test(root.path().to_path_buf(), terminal_driver, ctx)
        });
        let foreground = driver.update(&mut app, |driver, ctx| {
            driver.workspace = None;
            ctx.spawner()
        });
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        let error = AgentDriver::prepare_resources(
            &foreground,
            Harness::Oz,
            Vec::new(),
            GlobalSkillResolution {
                specs: Vec::new(),
                repos: Vec::new(),
            },
            &setup_events,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AgentDriverError::InvalidRuntimeState));
    });
}

#[test]
fn standby_provider_setup_precedes_workspace_preparation() {
    struct FailingProvider;
    impl CloudProvider for FailingProvider {
        fn env_vars(&self) -> ProviderResult<HashMap<OsString, OsString>> {
            Ok(HashMap::new())
        }

        fn setup(
            &mut self,
            spawner: ModelSpawner<TerminalDriver>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ProviderResult<()>> + Send + '_>>
        {
            Box::pin(async move {
                spawner.spawn(|_, _| ()).await.unwrap();
                Err(CloudProviderSetupError::new(
                    "test",
                    anyhow::anyhow!("provider setup failed"),
                ))
            })
        }
    }

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let root = TempDir::new().unwrap();
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver = AgentDriver::new_for_test(root.path().into(), terminal_driver, ctx);
            driver.cloud_providers = vec![Box::new(FailingProvider)];
            let mut workspace = WorkspaceConfiguration::default();
            workspace.setup_commands = vec!["must-not-run".into()];
            driver.workspace = Some(WorkspaceState::Pending(workspace));
            driver
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        let error = AgentDriver::prepare_resources(
            &foreground,
            Harness::Oz,
            Vec::new(),
            GlobalSkillResolution {
                specs: Vec::new(),
                repos: Vec::new(),
            },
            &setup_events,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AgentDriverError::CloudProviderSetupFailed(_)
        ));
    });
}

#[test]
fn configured_skills_load_without_environment_or_global_repositories() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _skills_enabled = FeatureFlag::OzPlatformSkills.override_enabled(true);
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let repo = working_dir.join("skill-repo");
        let skills_dir = repo.join(".agents/skills");
        let skill_dir = skills_dir.join("review");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: Review code.\n---\nReview the changes.\n",
        )
        .unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );
        let (skill, dirs) = configured_skills::resolve_configured_skills(
            &foreground,
            vec!["org/skill-repo:review".parse().unwrap()],
            true,
            &working_dir,
            &setup_events,
        )
        .await
        .unwrap();
        assert_eq!(skill.unwrap().name, "review");
        assert_eq!(dirs.as_slice(), std::slice::from_ref(&skills_dir));
        driver.update(&mut app, |driver, _| {
            driver.skill_dirs.clear();
            configured_skills::merge_skill_dirs(&mut driver.skill_dirs, dirs.clone());
            configured_skills::merge_skill_dirs(&mut driver.skill_dirs, dirs);
            assert_eq!(driver.skill_dirs, [skills_dir]);
        });
        load_skills(
            &foreground,
            Vec::new(),
            GlobalSkillResolution {
                specs: Vec::new(),
                repos: Vec::new(),
            },
            &setup_events,
        )
        .await;
        let names = app.update(|ctx| {
            SkillManager::as_ref(ctx)
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|skill| skill.name.clone())
                .collect::<Vec<_>>()
        });
        assert!(names.contains(&"review".to_string()), "{names:?}");
        let error = configured_skills::resolve_configured_skills(
            &foreground,
            vec!["org/skill-repo:missing".parse().unwrap()],
            true,
            &working_dir,
            &setup_events,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AgentDriverError::SkillResolutionFailed(_)));
    });
}
