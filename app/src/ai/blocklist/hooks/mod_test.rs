use ai::hooks::{CommandExecutionPosture, HookRegistry};
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{App, Element, SingletonEntity, TypedActionView, View};

use super::testing::hook_test_app;
use super::*;
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::features::FeatureFlag;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// An empty root view, so a test can open a window the posture read resolves a team through.
#[derive(Default)]
struct TestRootView;

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "HookTestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

fn set_execute_commands(app: &mut App, terminal_view_id: EntityId, setting: ActionPermission) {
    AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
        let profile_id = profiles
            .active_profile(Some(terminal_view_id), ctx)
            .id()
            .clone();
        profiles.set_execute_commands(&profile_id, &setting, ctx);
    });
}

#[test]
fn the_posture_is_the_strictest_setting_seen_through_any_scope() {
    use ActionPermission::{AgentDecides, AlwaysAllow, AlwaysAsk, Unknown};
    use CommandExecutionPosture::{AutoApproves, RequiresApproval};

    assert_eq!(posture_from_settings([AlwaysAllow]), AutoApproves);
    assert_eq!(posture_from_settings([AlwaysAsk]), RequiresApproval);
    assert_eq!(posture_from_settings([AgentDecides]), RequiresApproval);
    assert_eq!(
        posture_from_settings([Unknown]),
        CommandExecutionPosture::Unknown
    );
    assert_eq!(
        posture_from_settings([AlwaysAllow, AlwaysAllow]),
        AutoApproves
    );
    assert_eq!(
        posture_from_settings([AlwaysAllow, AgentDecides]),
        RequiresApproval
    );
    assert_eq!(
        posture_from_settings([AlwaysAllow, Unknown]),
        CommandExecutionPosture::Unknown
    );
    assert_eq!(
        posture_from_settings([Unknown, AlwaysAsk]),
        RequiresApproval
    );
    assert_eq!(
        posture_from_settings(Vec::new()),
        CommandExecutionPosture::Unknown,
        "no scope to read through means the posture cannot be known"
    );
}

#[test]
fn the_posture_follows_the_active_profile_and_ignores_the_conversation_toggle() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let scope = UserWorkspaces::teamless_context_for_operation_for_test();
        let scopes = std::slice::from_ref(&scope);
        let set_execute_commands = |app: &mut App, setting: ActionPermission| {
            set_execute_commands(app, harness.terminal_view_id, setting);
        };

        set_execute_commands(&mut app, ActionPermission::AlwaysAllow);
        app.read(|ctx| {
            assert_eq!(
                posture_for_scopes(Some(harness.terminal_view_id), scopes, ctx),
                CommandExecutionPosture::AutoApproves
            );
        });

        set_execute_commands(&mut app, ActionPermission::AgentDecides);
        app.read(|ctx| {
            assert_eq!(
                posture_for_scopes(Some(harness.terminal_view_id), scopes, ctx),
                CommandExecutionPosture::RequiresApproval
            );
        });

        // Auto-approving one conversation is not the run's configuration.
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(
                    &harness.conversation_id,
                    harness.terminal_view_id,
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            assert_eq!(
                posture_for_scopes(Some(harness.terminal_view_id), scopes, ctx),
                CommandExecutionPosture::RequiresApproval
            );
            assert_eq!(
                command_execution_posture(Some(harness.terminal_view_id), ctx),
                CommandExecutionPosture::Unknown,
                "a process with no window has no team policy to read through"
            );
        });
    });
}

/// The real posture read goes through the open windows' team policy. A headless run has exactly
/// one window, so this is the path the posture rule rests on.
#[test]
fn the_posture_is_read_through_an_open_window_and_is_unknown_without_one() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        set_execute_commands(
            &mut app,
            harness.terminal_view_id,
            ActionPermission::AlwaysAllow,
        );
        app.read(|ctx| {
            assert_eq!(
                command_execution_posture(Some(harness.terminal_view_id), ctx),
                CommandExecutionPosture::Unknown,
                "no window means no team policy to read through"
            );
        });

        app.add_window(WindowStyle::NotStealFocus, |_| TestRootView);
        app.read(|ctx| {
            assert_eq!(
                command_execution_posture(Some(harness.terminal_view_id), ctx),
                CommandExecutionPosture::AutoApproves
            );
        });

        set_execute_commands(
            &mut app,
            harness.terminal_view_id,
            ActionPermission::AgentDecides,
        );
        app.read(|ctx| {
            assert_eq!(
                command_execution_posture(Some(harness.terminal_view_id), ctx),
                CommandExecutionPosture::RequiresApproval
            );
        });
    });
}

#[test]
fn the_pin_records_the_posture_read_through_the_window() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.add_window(WindowStyle::NotStealFocus, |_| TestRootView);
        set_execute_commands(
            &mut app,
            harness.terminal_view_id,
            ActionPermission::AlwaysAllow,
        );
        append_exchange(&mut app, harness.terminal_view_id, harness.conversation_id);
        assert_eq!(
            pinned(&app).map(|pinned| pinned.posture),
            Some(CommandExecutionPosture::AutoApproves)
        );
    });
}

fn append_exchange(app: &mut App, terminal_view_id: EntityId, conversation_id: AIConversationId) {
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::AppendedExchange {
                exchange_id: AIAgentExchangeId::new(),
                task_id: TaskId::new("task".to_owned()),
                terminal_surface_id: terminal_view_id,
                conversation_id,
                is_hidden: false,
                response_stream_id: None,
            });
        });
    });
}

fn pinned(app: &App) -> Option<ai::hooks::PinnedRun> {
    app.read(|ctx| HookRegistry::as_ref(ctx).pinned())
}

#[test]
fn the_first_appended_exchange_pins_the_registry_once() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        assert!(pinned(&app).is_none());

        append_exchange(&mut app, harness.terminal_view_id, harness.conversation_id);
        let first = pinned(&app).expect("the first exchange pins the run");
        assert_eq!(first.posture, CommandExecutionPosture::Unknown);

        append_exchange(&mut app, harness.terminal_view_id, harness.conversation_id);
        assert_eq!(
            pinned(&app),
            Some(first),
            "a later exchange changes nothing"
        );
        assert_eq!(
            app.read(|ctx| HookRegistry::as_ref(ctx).revision()),
            first.revision
        );
    });
}

#[test]
fn an_appended_exchange_does_not_pin_when_the_flag_is_off() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        append_exchange(&mut app, harness.terminal_view_id, harness.conversation_id);
        assert!(pinned(&app).is_none());
    });
}

#[cfg(feature = "local_fs")]
mod discovery {
    use std::fs;

    use ai::hooks::PROJECT_CURSOR_HOOKS_PATH;
    use repo_metadata::DirectoryWatcher;
    use repo_metadata::repositories::{
        DetectedRepositories, DetectedRepositoriesEvent, RepoDetectionSource,
    };
    use tempfile::TempDir;
    use warp_util::standardized_path::StandardizedPath;

    use super::*;

    const VALID: &str =
        r#"{ "version": 1, "hooks": { "beforeShellExecution": [ { "command": "./hook.sh" } ] } }"#;

    fn detect_root(app: &mut App, root: &std::path::Path, source: RepoDetectionSource) {
        let canonical = StandardizedPath::from_local_canonicalized(root).expect("canonical");
        app.update(|ctx| {
            let repository = DirectoryWatcher::handle(ctx)
                .update(ctx, |watcher, ctx| watcher.add_directory(canonical, ctx))
                .expect("watched directory");
            DetectedRepositories::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(DetectedRepositoriesEvent::DetectedGitRepo { repository, source });
            });
        });
    }

    #[test]
    fn a_detected_root_feeds_its_hook_file_to_the_registry() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let _harness = hook_test_app(&mut app);
            let dir = TempDir::new().expect("tempdir");
            let root = dunce::canonicalize(dir.path()).expect("canonical");
            fs::create_dir_all(root.join(".cursor")).expect("mkdir");
            fs::write(root.join(PROJECT_CURSOR_HOOKS_PATH), VALID).expect("write");

            detect_root(&mut app, &root, RepoDetectionSource::TerminalNavigation);

            let snapshot = app.read(|ctx| HookRegistry::as_ref(ctx).snapshot());
            assert_eq!(
                snapshot.protected_paths,
                vec![root.join(PROJECT_CURSOR_HOOKS_PATH)],
                "the file is known and protected"
            );
            assert!(
                snapshot.is_empty(),
                "a project file stays inert on an interactive surface"
            );
            assert!(
                snapshot
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("loaded but not active"))
            );
        });
    }

    #[test]
    fn roots_the_surface_does_not_watch_are_ignored() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            let _harness = hook_test_app(&mut app);
            let dir = TempDir::new().expect("tempdir");
            let root = dunce::canonicalize(dir.path()).expect("canonical");
            fs::create_dir_all(root.join(".cursor")).expect("mkdir");
            fs::write(root.join(PROJECT_CURSOR_HOOKS_PATH), VALID).expect("write");

            // The test app runs as the desktop app, which leaves cloud-prepared roots alone.
            detect_root(&mut app, &root, RepoDetectionSource::CloudEnvironmentPrep);
            detect_root(&mut app, &root, RepoDetectionSource::ProjectRulesIndexing);
            detect_root(
                &mut app,
                &root,
                RepoDetectionSource::CodeReviewInitialization,
            );

            assert_eq!(
                app.read(|ctx| HookRegistry::as_ref(ctx).revision()),
                0,
                "nothing was loaded"
            );
        });
    }

    /// The remote server daemon serves SSH sessions on a remote host. It detects repositories
    /// as the client navigates, but remote hosts are not a hook surface.
    #[test]
    fn the_remote_server_daemon_neither_watches_roots_nor_pins() {
        use std::sync::Arc;

        use ai::hooks::{LogOnlyMessenger, UnavailableRunner};
        use warp_core::execution_mode::ExecutionMode;

        use crate::test_util::settings::initialize_settings_for_tests_with_mode;

        App::test((), |mut app| async move {
            let _flag = FeatureFlag::AgentHooks.override_enabled(true);
            initialize_settings_for_tests_with_mode(
                &mut app,
                ExecutionMode::RemoteServerDaemon,
                false,
            );
            app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(|_| HookRegistry::new());
            app.add_singleton_model(|ctx| {
                HookRuntime::new(Arc::new(UnavailableRunner), Arc::new(LogOnlyMessenger), ctx)
            });
            let dir = TempDir::new().expect("tempdir");
            let root = dunce::canonicalize(dir.path()).expect("canonical");
            fs::create_dir_all(root.join(".cursor")).expect("mkdir");
            fs::write(root.join(PROJECT_CURSOR_HOOKS_PATH), VALID).expect("write");

            detect_root(&mut app, &root, RepoDetectionSource::TerminalNavigation);
            assert_eq!(app.read(|ctx| HookRegistry::as_ref(ctx).revision()), 0);

            let runtime = HookRuntime::handle(&app);
            runtime.update(&mut app, |runtime, ctx| {
                runtime.pin_for_run(None, ctx);
            });
            assert!(pinned(&app).is_none());
        });
    }

    #[test]
    fn a_detected_root_is_ignored_when_the_flag_is_off() {
        App::test((), |mut app| async move {
            let _harness = hook_test_app(&mut app);
            let dir = TempDir::new().expect("tempdir");
            let root = dunce::canonicalize(dir.path()).expect("canonical");
            fs::create_dir_all(root.join(".cursor")).expect("mkdir");
            fs::write(root.join(PROJECT_CURSOR_HOOKS_PATH), VALID).expect("write");
            detect_root(&mut app, &root, RepoDetectionSource::TerminalNavigation);
            assert_eq!(app.read(|ctx| HookRegistry::as_ref(ctx).revision()), 0);
        });
    }
}
