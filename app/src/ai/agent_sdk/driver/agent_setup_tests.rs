use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use ai::hooks::test_util::RecordingMessenger;
use ai::hooks::{DiagnosticSeverity, HookFormat, HookRegistry, HookSourceKind, HookSourcePolicy};
use ai::skills::{ParsedSkill, SkillProvider};
use cloud_object_models::CodeForge;
#[cfg(all(unix, feature = "local_tty"))]
use futures::FutureExt as _;
use tempfile::TempDir;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{App, ModelHandle};

use super::super::AgentDriverOptions;
use super::super::cloud_provider::{
    CloudProvider, CloudProviderSetupError, Result as ProviderResult,
};
use super::super::environment::WorkspaceConfiguration;
use super::super::hooks::{ConfiguredHookLocation, ConfiguredHookSource};
use super::super::terminal::TerminalDriver;
use super::*;
use crate::ai::blocklist::hooks::test_utils::install_test_hooks;
use crate::ai::blocklist::hooks::{HookRuntime, RunSurface};
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
            false,
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
            false,
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
            false,
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
fn environment_skill_setup_uses_harness_working_dir_as_listing_fallback() {
    assert_environment_skill_listing_fallback(true);
}

#[test]
fn environment_skill_setup_without_cwd_scoping_sets_no_listing_fallback() {
    assert_environment_skill_listing_fallback(false);
}

fn project_skill(name: &str, dir: &Path) -> ParsedSkill {
    ParsedSkill {
        name: name.into(),
        description: name.into(),
        content: name.into(),
        path: LocalOrRemotePath::Local(dir.join(format!(".agents/skills/{name}/SKILL.md"))),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    }
}

/// The names listed with no live cwd, where the harness directory stands in once it is set.
fn listed_skill_names(app: &mut App) -> Vec<String> {
    app.update(|ctx| {
        SkillManager::as_ref(ctx)
            .get_skills_for_working_directory(None, ctx)
            .into_iter()
            .map(|skill| skill.name)
            .collect()
    })
}

fn assert_environment_skill_listing_fallback(cwd_scoped: bool) {
    App::test((), move |mut app| async move {
        let _flag = FeatureFlag::CwdScopedCloudSkills.override_enabled(cwd_scoped);
        initialize_app_for_terminal_view(&mut app);
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let harness_dir = working_dir.join("repo");
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver = AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx);
            driver.harness_working_dir = harness_dir.clone();
            driver
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        load_environment_skills(&foreground, Vec::new()).await;

        app.update(|ctx| {
            SkillManager::handle(ctx).update(ctx, |manager, _| {
                manager.handle_skills_added(vec![
                    project_skill("harness", &harness_dir),
                    project_skill("sibling", &working_dir.join("sibling")),
                ]);
            });
        });
        let names = listed_skill_names(&mut app);

        assert_eq!(
            names.contains(&"harness".to_string()),
            cwd_scoped,
            "{names:?}"
        );
        assert!(!names.contains(&"sibling".to_string()), "{names:?}");
    });
}

/// A standby reads its skill files before it has an assignment, so the read must leave the
/// `SkillManager` alone: nothing registered, no listing mode, no fallback directory.
#[test]
fn read_skills_leaves_the_skill_manager_untouched() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let harness_dir = working_dir.join("repo");
        let skill_dir = working_dir.join("skill-repo/.agents/skills/standby-global");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: standby-global\ndescription: Selected globally.\n---\nGlobal.\n",
        )
        .unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver = AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx);
            driver.harness_working_dir = harness_dir.clone();
            driver
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        let setup_events = SetupClientEventReporter::standby(
            Arc::new(MockAIClient::new()),
            app.background_executor(),
        );

        let prepared = read_skills(
            &foreground,
            Vec::new(),
            GlobalSkillResolution {
                specs: vec!["org/skill-repo:standby-global".parse().unwrap()],
                repos: vec![GithubRepo::new("org".into(), "skill-repo".into())],
            },
            &setup_events,
        )
        .await;

        assert!(prepared.environment.is_empty());
        assert_eq!(
            prepared
                .global
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            ["standby-global"]
        );
        app.update(|ctx| {
            SkillManager::handle(ctx).update(ctx, |manager, _| {
                manager.handle_skills_added(vec![project_skill("harness", &harness_dir)]);
            });
        });
        let names = listed_skill_names(&mut app);
        assert!(!names.contains(&"standby-global".to_string()), "{names:?}");
        assert!(!names.contains(&"harness".to_string()), "{names:?}");
    });
}

/// A run assigned to a standby registers the skills the standby read under the run's own listing
/// mode: scoped to the harness directory with global skills at home tier when `CwdScopedCloudSkills`
/// is enabled, and unscoped otherwise.
#[test]
fn prepared_skills_register_under_the_runs_listing_mode() {
    for cwd_scoped in [true, false] {
        App::test((), move |mut app| async move {
            let _flag = FeatureFlag::CwdScopedCloudSkills.override_enabled(cwd_scoped);
            initialize_app_for_terminal_view(&mut app);
            let root = TempDir::new().unwrap();
            let working_dir = dunce::canonicalize(root.path()).unwrap();
            let harness_dir = working_dir.join("repo");
            let terminal = add_window_with_terminal(&mut app, None);
            let driver = app.add_model(|ctx| {
                let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
                let mut driver =
                    AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx);
                driver.harness_working_dir = harness_dir.clone();
                driver.skill_dirs.clear();
                driver
            });
            let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
            let setup_events = SetupClientEventReporter::noop(
                Arc::new(MockAIClient::new()),
                app.background_executor(),
            );

            load_skills(
                &foreground,
                RunSkills::Prepared(PreparedSkills {
                    environment: vec![
                        project_skill("standby-harness", &harness_dir),
                        project_skill("standby-sibling", &working_dir.join("sibling")),
                    ],
                    global: vec![project_skill(
                        "standby-global",
                        &working_dir.join("skill-repo"),
                    )],
                }),
                &setup_events,
            )
            .await;

            let skills = app.update(|ctx| {
                SkillManager::as_ref(ctx).get_skills_for_working_directory(None, ctx)
            });
            let scope_of = |name: &str| {
                skills
                    .iter()
                    .find(|skill| skill.name == name)
                    .map(|skill| skill.scope)
            };
            assert_eq!(scope_of("standby-harness"), Some(SkillScope::Project));
            if cwd_scoped {
                assert_eq!(scope_of("standby-sibling"), None);
                assert_eq!(scope_of("standby-global"), Some(SkillScope::Home));
            } else {
                assert_eq!(scope_of("standby-sibling"), Some(SkillScope::Project));
                assert_eq!(scope_of("standby-global"), Some(SkillScope::Project));
            }
        });
    }
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
            RunSkills::FromRepositories {
                repositories: Vec::new(),
                global_skills: GlobalSkillResolution {
                    specs: Vec::new(),
                    repos: Vec::new(),
                },
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

fn repository_source(repo: &str, path: &str, strict: bool) -> ConfiguredHookSource {
    ConfiguredHookSource {
        format: HookFormat::Cursor,
        location: ConfiguredHookLocation::Repository {
            repo: SourceRepo::new(CodeForge::GitHub, "acme".to_owned(), repo.to_owned()),
            path: path.to_owned(),
        },
        kind: HookSourceKind::FactoryAgent,
        policy: HookSourcePolicy {
            strict,
            ..Default::default()
        },
    }
}

fn worker_source(path: &str, strict: bool) -> ConfiguredHookSource {
    ConfiguredHookSource {
        format: HookFormat::Cursor,
        location: ConfiguredHookLocation::Worker {
            path: path.to_owned(),
        },
        kind: HookSourceKind::FactoryWide,
        policy: HookSourcePolicy {
            strict,
            ..Default::default()
        },
    }
}

fn cloned(repo: &str) -> SourceRepo {
    SourceRepo::new(CodeForge::GitHub, "acme".to_owned(), repo.to_owned())
}

/// A repository source is found in its clone at `working_dir/<name>` and runs from the clone's
/// root; a worker source runs from its own directory. Agent entries come before factory entries.
#[test]
fn repository_and_worker_sources_load_where_the_run_put_them() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        write_hooks_file(&working_dir.join("api/.cursor"));
        let outside = TempDir::new().unwrap();
        let worker_dir = dunce::canonicalize(outside.path()).unwrap().join("hooks");
        write_hooks_file(&worker_dir);
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![
                worker_source(&worker_dir.display().to_string(), true),
                repository_source("API", ".cursor", true),
            ];
            driver.load_hook_sources_for_run(&[cloned("api")], ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        assert!(
            messenger.diagnostics().is_empty(),
            "{:?}",
            messenger.diagnostics()
        );
        let bindings = app.read(|ctx| HookRegistry::as_ref(ctx).snapshot().bindings.clone());
        assert_eq!(bindings.len(), 2);
        let repository = bindings
            .iter()
            .find(|active| active.binding.id.source.kind == HookSourceKind::FactoryAgent)
            .expect("the repository source loaded");
        assert_eq!(repository.working_directory, working_dir.join("api"));
        assert_eq!(
            repository.project_dir.as_deref(),
            Some(working_dir.join("api").as_path())
        );
        let worker = bindings
            .iter()
            .find(|active| active.binding.id.source.kind == HookSourceKind::FactoryWide)
            .expect("the worker source loaded");
        assert_eq!(worker.working_directory, worker_dir);
        assert_eq!(worker.project_dir, None);
    });
}

/// A source that cannot be found or loaded is reported. Only a strict one fails the run, and the
/// failure names only the strict sources.
#[test]
fn only_a_strict_source_that_cannot_load_fails_the_run() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let (driver, messenger) = driver_with_test_hooks(&mut app, root.path());

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![
                repository_source("not-cloned", ".cursor", false),
                worker_source("/no/such/hook/dir", true),
            ];
            driver.load_hook_sources_for_run(&[], ctx)
        });

        let Err(AgentDriverError::HookSourcesUnavailable { details }) = result else {
            panic!("expected the strict source to fail the run, got {result:?}");
        };
        assert_eq!(details.len(), 1, "{details:?}");
        assert!(details[0].contains("/no/such/hook/dir"), "{details:?}");
        let diagnostics = messenger.diagnostics();
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        );
        assert!(
            diagnostics[0].message.contains("not checked out"),
            "{diagnostics:?}"
        );
    });
}

#[test]
fn a_source_that_is_not_strict_is_reported_and_the_run_goes_on() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let (driver, messenger) = driver_with_test_hooks(&mut app, root.path());

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![worker_source("/no/such/hook/dir", false)];
            driver.load_hook_sources_for_run(&[], ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(messenger.diagnostics().len(), 1);
        assert!(app.read(|ctx| HookRegistry::as_ref(ctx).is_finalized()));
    });
}

/// A path the server should never send is a failure of that entry alone, under its own `strict`.
#[test]
fn a_malformed_path_fails_only_its_own_entry() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        write_hooks_file(&working_dir.join("api/.cursor"));
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![
                repository_source("api", "../outside", false),
                repository_source("api", "/etc", false),
                worker_source("relative/hooks", false),
                repository_source("api", ".cursor", false),
            ];
            driver.load_hook_sources_for_run(&[cloned("api")], ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            messenger.diagnostics().len(),
            3,
            "{:?}",
            messenger.diagnostics()
        );
        assert_eq!(
            app.read(|ctx| HookRegistry::as_ref(ctx).snapshot().bindings.len()),
            1
        );
    });
}

/// A process with no hook runtime cannot run hooks, so it fails a run that has any, strict or not,
/// rather than running without them. A run with none goes on.
#[test]
fn a_process_without_the_hook_runtime_fails_a_run_that_has_hooks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        assert!(!app.read(|ctx| ctx.has_singleton_model::<HookRuntime>()));
        let root = TempDir::new().unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            AgentDriver::new_for_test(root.path().to_path_buf(), terminal_driver, ctx)
        });
        for strict in [true, false] {
            let result = driver.update(&mut app, |driver, ctx| {
                driver.hooks.sources = vec![worker_source("/no/such/hook/dir", strict)];
                driver.load_hook_sources_for_run(&[], ctx)
            });
            assert!(
                matches!(result, Err(AgentDriverError::HooksUnsupportedByThisBuild)),
                "strict {strict}: {result:?}"
            );
        }
        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources.clear();
            driver.load_hook_sources_for_run(&[], ctx)
        });
        assert!(result.is_ok(), "{result:?}");
    });
}

/// A runner with the command runner installed, a hook directory whose `beforeShellExecution` hook
/// writes the `PATH` it was given to `path.txt`, and a driver whose run selects that directory.
#[cfg(all(unix, feature = "local_tty"))]
fn driver_with_path_recording_hook(
    app: &mut App,
    working_dir: &Path,
) -> (ModelHandle<AgentDriver>, Arc<RecordingMessenger>, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    use crate::process_environment::InitialProcessEnvironment;

    let dir = working_dir.join("hooks");
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("record-path.sh");
    fs::write(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s' \"$PATH\" >> \"$PWD/path.txt\"\nprintf '\\n' >> \"$PWD/path.txt\"\nprintf '{\"permission\":\"allow\"}'\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        dir.join("hooks.json"),
        r#"{"version":1,"hooks":{"beforeShellExecution":[{"command":"./record-path.sh"}]}}"#,
    )
    .unwrap();
    let (driver, messenger) = driver_with_test_hooks(app, working_dir);
    app.update(|ctx| {
        ctx.add_singleton_model(|_| InitialProcessEnvironment::capture());
        HookRuntime::install_command_runner(ctx);
    });
    driver.update(app, |driver, _| {
        driver.hooks.sources = vec![worker_source(&dir.display().to_string(), true)];
    });
    (driver, messenger, dir.join("path.txt"))
}

/// Runs the loaded `beforeShellExecution` hook once, through the runtime's own dispatcher.
#[cfg(all(unix, feature = "local_tty"))]
async fn run_shell_hook(app: &mut App) {
    use ai::agent::action::AIAgentActionType;
    use ai::hooks::HookPayload;
    use ai::hooks::test_util::test_envelope;

    let dispatch = app.update(|ctx| {
        let runtime = HookRuntime::as_ref(ctx);
        let envelope = test_envelope(
            crate::ai::agent::conversation::AIConversationId::new(),
            HookPayload::ToolPre {
                action: AIAgentActionType::RequestCommandOutput {
                    command: "ls".to_owned(),
                    is_read_only: None,
                    is_risky: None,
                    wait_until_completion: true,
                    uses_pager: None,
                    rationale: None,
                    citations: Vec::new(),
                },
            },
        );
        runtime.dispatcher().dispatch(
            envelope,
            runtime.snapshot(ctx),
            true,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    });
    let outcome = dispatch.await;
    assert_eq!(outcome.invocations.len(), 1, "the hook ran");
}

/// Installing the command runner starts nothing. The capture starts when a run that has hooks
/// begins its setup, and not for a run that has none.
#[cfg(feature = "local_tty")]
#[test]
fn the_login_shell_path_capture_starts_only_for_a_run_with_hooks() {
    use crate::process_environment::InitialProcessEnvironment;

    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let (driver, _) = driver_with_test_hooks(&mut app, root.path());
        app.update(|ctx| {
            ctx.add_singleton_model(|_| InitialProcessEnvironment::capture());
            HookRuntime::install_command_runner(ctx);
        });
        let pending = |app: &App| {
            app.read(|ctx| HookRuntime::as_ref(ctx).login_shell_path_capture_is_pending())
        };
        assert!(pending(&app), "installing the runner starts no capture");

        let foreground = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources.clear();
            ctx.spawner()
        });
        start_login_shell_path_capture(&foreground).await.unwrap();
        assert!(pending(&app), "a run with no hooks starts no capture");

        driver.update(&mut app, |driver, _| {
            driver.hooks.sources = vec![worker_source("/opt/hooks", false)];
        });
        start_login_shell_path_capture(&foreground).await.unwrap();
        assert!(!pending(&app), "a run with hooks starts the capture");
    });
}

/// The first hook of a run already gets the login shell's PATH, even when reading it takes a
/// while: the setup step the run calls loads the hooks and then waits for it before any hook can
/// run.
#[cfg(all(unix, feature = "local_tty"))]
#[test]
fn the_first_hook_gets_the_login_shell_path() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let (driver, _, recorded) = driver_with_path_recording_hook(&mut app, &working_dir);
        HookRuntime::handle(&app).update(&mut app, |runtime, _| {
            runtime.set_login_shell_path_capture_for_test(
                async {
                    warpui::r#async::Timer::after(Duration::from_millis(200)).await;
                    Some("/login/bin:/usr/bin:/bin".to_owned())
                }
                .boxed(),
            );
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());

        setup_hooks_for_run(&foreground, Vec::new())
            .await
            .expect("the hooks load");
        run_shell_hook(&mut app).await;

        assert_eq!(
            fs::read_to_string(&recorded)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec!["/login/bin:/usr/bin:/bin"]
        );
    });
}

/// When the login shell's PATH cannot be read in time, every hook in the run gets this process's
/// PATH, even after the read finishes late; the run's output says so once, and the run goes on.
#[cfg(all(unix, feature = "local_tty"))]
#[test]
fn without_the_login_shell_path_every_hook_gets_the_process_path() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let (driver, messenger, recorded) = driver_with_path_recording_hook(&mut app, &working_dir);
        let (late_path, read) = futures::channel::oneshot::channel::<Option<String>>();
        HookRuntime::handle(&app).update(&mut app, |runtime, _| {
            runtime.set_login_shell_path_capture_for_test(
                async move { read.await.ok().flatten() }.boxed(),
            );
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());

        let loaded = driver.update(&mut app, |driver, ctx| {
            driver.load_hook_sources_for_run(&[], ctx)
        });
        assert!(loaded.is_ok(), "{loaded:?}");
        wait_for_login_shell_path(&foreground, Duration::from_secs(1))
            .await
            .unwrap();
        run_shell_hook(&mut app).await;
        // The run gave up on the capture when the wait timed out, so this late result is either
        // dropped on the floor or lands nowhere the hooks read from.
        let _ = late_path.send(Some("/login/bin:/usr/bin:/bin".to_owned()));
        crate::ai::blocklist::hooks::test_utils::let_background_work_settle(&mut app).await;
        run_shell_hook(&mut app).await;

        let process_path = std::env::var("PATH").unwrap();
        assert_eq!(
            fs::read_to_string(&recorded)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec![process_path.as_str(), process_path.as_str()]
        );
        assert_eq!(
            app.read(|ctx| HookRuntime::as_ref(ctx).login_shell_path_for_test()),
            None
        );
        let diagnostics = messenger.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert!(
            diagnostics[0]
                .message
                .contains("hooks run with the worker's PATH because the login shell's PATH"),
            "{diagnostics:?}"
        );
    });
}

/// A capture that finishes without a PATH is reported the same way, and only once however often
/// the run asks.
#[cfg(all(unix, feature = "local_tty"))]
#[test]
fn a_login_shell_that_gives_no_path_is_reported_once() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let (driver, messenger, _) = driver_with_path_recording_hook(&mut app, &working_dir);
        HookRuntime::handle(&app).update(&mut app, |runtime, _| {
            runtime.set_login_shell_path_capture_for_test(futures::future::ready(None).boxed());
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        let loaded = driver.update(&mut app, |driver, ctx| {
            driver.load_hook_sources_for_run(&[], ctx)
        });
        assert!(loaded.is_ok(), "{loaded:?}");
        for _ in 0..2 {
            wait_for_login_shell_path(&foreground, LOGIN_SHELL_PATH_WAIT)
                .await
                .unwrap();
        }
        let diagnostics = messenger.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].message.ends_with("could not be read"),
            "{diagnostics:?}"
        );
    });
}

/// Load warnings reach the run's output beside the failures. Two entries that reach one file but
/// would run it from different directories produce a warning naming both directories.
#[test]
fn a_file_selected_from_two_locations_is_reported_as_a_warning() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let cursor_dir = working_dir.join("api/.cursor");
        write_hooks_file(&cursor_dir);
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![
                repository_source("api", ".cursor", false),
                worker_source(&cursor_dir.display().to_string(), false),
            ];
            driver.load_hook_sources_for_run(&[cloned("api")], ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        let diagnostics = messenger.diagnostics();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert!(
            diagnostics[0].message.contains("selected more than once")
                && diagnostics[0]
                    .message
                    .contains(&working_dir.join("api").display().to_string())
                && diagnostics[0]
                    .message
                    .contains(&cursor_dir.display().to_string()),
            "{diagnostics:?}"
        );
    });
}

/// Only load warnings that describe something that already changed what a source does reach the
/// run's output. A prediction of what a hook might emit (`sessionStart` may return model context,
/// which is not delivered yet) is recorded as `Info` and stays in the log.
#[test]
fn only_definite_load_warnings_reach_the_run_output() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        let dir = working_dir.join("hooks");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("hooks.json"),
            r#"{"version":1,"hooks":{
                "sessionStart":[{"command":"./start.sh"}],
                "beforeShellExecution":[{"command":"./check.sh","matcher":"("}]
            }}"#,
        )
        .unwrap();
        let (driver, messenger) = driver_with_test_hooks(&mut app, &working_dir);

        let result = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![worker_source(&dir.display().to_string(), true)];
            driver.load_hook_sources_for_run(&[], ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        let snapshot = app.read(|ctx| HookRegistry::as_ref(ctx).snapshot());
        assert!(
            snapshot.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity == DiagnosticSeverity::Info
                    && diagnostic
                        .message
                        .contains("model context is not supported yet")
            }),
            "{:?}",
            snapshot.diagnostics
        );
        let forwarded = messenger.diagnostics();
        assert_eq!(forwarded.len(), 1, "{forwarded:?}");
        assert_eq!(forwarded[0].severity, DiagnosticSeverity::Warning);
        assert!(
            forwarded[0]
                .message
                .contains("matcher is not a valid regex"),
            "{forwarded:?}"
        );
    });
}

/// A run on a harness other than Oz fails when its execution configuration selected hooks, and
/// goes on when it selected none.
#[test]
fn hooks_on_another_harness_fail_the_run() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let (driver, _) = driver_with_test_hooks(&mut app, root.path());
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
        let harness = HarnessKind::Unsupported(Harness::OpenCode);

        refuse_hooks_for_other_harness(&foreground, &harness)
            .await
            .expect("a run with no hooks goes on");
        driver.update(&mut app, |driver, _| {
            driver.hooks.sources = vec![worker_source("/opt/hooks", false)];
        });
        let error = refuse_hooks_for_other_harness(&foreground, &harness)
            .await
            .unwrap_err();
        let AgentDriverError::HooksRequireOzHarness { harness: name } = error else {
            panic!("expected the hooks to be refused, got {error:?}");
        };
        assert_eq!(name, Harness::OpenCode.to_string());
    });
}

/// The setup step the run calls after environment preparation loads the run's sources and
/// finalizes the registry.
#[test]
fn setup_hooks_for_run_loads_the_runs_sources() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(root.path()).unwrap();
        write_hooks_file(&working_dir.join("api/.cursor"));
        let (driver, _) = driver_with_test_hooks(&mut app, &working_dir);
        let foreground = driver.update(&mut app, |driver, ctx| {
            driver.hooks.surface = RunSurface::Oz;
            driver.hooks.sources = vec![repository_source("api", ".cursor", true)];
            ctx.spawner()
        });

        setup_hooks_for_run(&foreground, vec![cloned("api")])
            .await
            .expect("the strict source loads");
        app.read(|ctx| {
            let registry = HookRegistry::as_ref(ctx);
            assert!(registry.is_finalized());
            assert_eq!(registry.snapshot().bindings.len(), 1);
        });
    });
}

#[test]
fn setup_hooks_for_run_fails_when_a_strict_source_cannot_load() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let (driver, _) = driver_with_test_hooks(&mut app, root.path());
        let foreground = driver.update(&mut app, |driver, ctx| {
            driver.hooks.sources = vec![repository_source("api", ".cursor", true)];
            ctx.spawner()
        });

        let error = setup_hooks_for_run(&foreground, Vec::new())
            .await
            .unwrap_err();
        assert!(
            matches!(error, AgentDriverError::HookSourcesUnavailable { .. }),
            "{error:?}"
        );
        assert!(app.read(|ctx| HookRegistry::as_ref(ctx).is_finalized()));
    });
}

#[test]
fn an_empty_hook_source_list_finalizes_the_registry() {
    App::test((), |mut app| async move {
        let root = TempDir::new().unwrap();
        let (driver, messenger) = driver_with_test_hooks(&mut app, root.path());

        let result = driver.update(&mut app, |driver, ctx| {
            driver.load_hook_sources_for_run(&[], ctx)
        });

        assert!(result.is_ok(), "{result:?}");
        assert!(messenger.diagnostics().is_empty());
        app.read(|ctx| {
            let registry = HookRegistry::as_ref(ctx);
            assert!(registry.is_finalized());
            assert!(registry.snapshot().is_empty());
        });
    });
}
