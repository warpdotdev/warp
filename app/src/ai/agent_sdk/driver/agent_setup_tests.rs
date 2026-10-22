use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use ai::hooks::test_util::RecordingMessenger;
use ai::hooks::{
    DiagnosticSeverity, ExplicitHookSource, ExplicitSourceScope, HookFormat, HookRegistry,
    HookSourceKind,
};
use tempfile::TempDir;
use warpui::{App, ModelHandle};

use super::super::AgentDriverOptions;
use super::super::terminal::TerminalDriver;
use super::*;
use crate::ai::blocklist::hooks::HookRuntime;
use crate::ai::blocklist::hooks::testing::install_test_hooks;
use crate::server::server_api::ai::MockAIClient;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspace::view::tests::initialize_app as initialize_workspace_test_app;

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

const SHELL_HOOK: &str =
    r#"{"version":1,"hooks":{"beforeShellExecution":[{"command":"./check.sh"}]}}"#;

fn write_hooks_file(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("hooks.json"), SHELL_HOOK).unwrap();
}

/// A driver whose hook runtime records notices and diagnostics instead of writing them.
fn driver_with_test_hooks(
    app: &mut App,
    working_dir: &Path,
) -> (ModelHandle<AgentDriver>, Arc<RecordingMessenger>) {
    initialize_app_for_terminal_view(app);
    let hooks = app.update(install_test_hooks);
    let terminal = add_window_with_terminal(app, None);
    let working_dir = working_dir.to_path_buf();
    let driver = app.add_model(|ctx| {
        let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
        AgentDriver::new_for_test(working_dir, terminal_driver, ctx)
    });
    (driver, hooks.messenger)
}

fn loaded_hook_sources(app: &App) -> Vec<(HookSourceKind, String)> {
    app.read(|ctx| {
        HookRegistry::as_ref(ctx)
            .snapshot()
            .bindings
            .iter()
            .map(|active| {
                let source = &active.binding.id.source;
                (source.kind, source.to_string())
            })
            .collect()
    })
}

#[test]
fn hook_directories_load_as_run_delivered_sources() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let outside = TempDir::new().unwrap();
        let absolute = dunce::canonicalize(outside.path()).unwrap().join("abs");
        write_hooks_file(&absolute);
        write_hooks_file(&working_dir.join("rel"));
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.hook_dirs = vec![absolute.clone(), PathBuf::from("rel")];
            driver.load_hook_sources_for_run(ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        assert!(messenger.diagnostics().is_empty());
        assert_eq!(
            loaded_hook_sources(&app),
            vec![
                (
                    HookSourceKind::RunDelivered,
                    absolute.join("hooks.json").display().to_string()
                ),
                (
                    HookSourceKind::RunDelivered,
                    working_dir.join("rel/hooks.json").display().to_string()
                ),
            ]
        );
        let delivered = app.read(|ctx| HookRuntime::as_ref(ctx).delivered_sources().len());
        assert_eq!(delivered, 2);
    });
}

#[test]
fn a_missing_hook_directory_is_a_diagnostic_and_the_run_goes_on() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let root = TempDir::new().unwrap();
        let (driver, messenger) = driver_with_test_hooks(&mut app, root.path());

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.hook_dirs = vec![PathBuf::from("/no/such/hook/dir")];
            driver.load_hook_sources_for_run(ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        let diagnostics = messenger.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        assert!(
            diagnostics[0]
                .source
                .to_string()
                .contains("/no/such/hook/dir")
        );
    });
}

#[test]
fn a_missing_hook_directory_fails_a_strict_run_before_it_starts() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let root = TempDir::new().unwrap();
        let (driver, _messenger) = driver_with_test_hooks(&mut app, root.path());

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.hook_dirs = vec![PathBuf::from("/no/such/hook/dir")];
            driver.hook_options.strict = true;
            driver.load_hook_sources_for_run(ctx)
        });

        let Err(AgentDriverError::HookSourcesUnavailable { details }) = result else {
            panic!("expected the strict run to fail, got {result:?}");
        };
        assert_eq!(details.len(), 1);
        assert!(details[0].contains("/no/such/hook/dir"), "{details:?}");
    });
}

#[test]
fn a_configured_source_inside_a_repository_follows_the_rule_for_project_hooks() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let repo = working_dir.join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        write_hooks_file(&repo.join("hooks"));
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);
        let configured = ExplicitHookSource {
            path: repo.join("hooks/hooks.json"),
            format: HookFormat::Cursor,
            scope: ExplicitSourceScope::Run,
        };

        // The test surface is not a headless run that auto-approves commands, so a
        // repository's own hooks stay inert there, and so does this entry.
        let lenient = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.configured_sources = vec![configured.clone()];
            driver.load_hook_sources_for_run(ctx)
        });
        assert!(lenient.is_ok(), "{lenient:?}");
        assert!(loaded_hook_sources(&app).is_empty());
        let diagnostics = messenger.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("inside the repository"),
            "{diagnostics:?}"
        );

        let strict = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.strict = true;
            driver.load_hook_sources_for_run(ctx)
        });
        assert!(matches!(
            strict,
            Err(AgentDriverError::HookSourcesUnavailable { .. })
        ));
    });
}

#[test]
fn a_hook_directory_is_not_held_back_when_the_config_file_names_the_same_file() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let repo = working_dir.join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        write_hooks_file(&repo.join("hooks"));
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.hook_dirs = vec![repo.join("hooks")];
            driver.hook_options.configured_sources = vec![ExplicitHookSource {
                path: repo.join("hooks/hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::Run,
            }];
            driver.hook_options.strict = true;
            driver.load_hook_sources_for_run(ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        assert!(messenger.diagnostics().is_empty());
        assert_eq!(loaded_hook_sources(&app).len(), 1);
    });
}

#[test]
fn hook_sources_are_not_loaded_when_hooks_are_off() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(false);
        let root = TempDir::new().unwrap();
        let (driver, messenger) = driver_with_test_hooks(&mut app, root.path());
        let result = driver.update(&mut app, |driver, ctx| {
            driver.hook_options.hook_dirs = vec![PathBuf::from("/no/such/hook/dir")];
            driver.hook_options.strict = true;
            driver.load_hook_sources_for_run(ctx)
        });
        assert!(result.is_ok());
        assert!(messenger.diagnostics().is_empty());
    });
}
