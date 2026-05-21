use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use itertools::Itertools;
use parking_lot::Mutex;
use settings::Setting as _;
use warp_core::command::ExitCode;
use warpui::{App, SingletonEntity};
use warpui_extras::user_preferences;

#[cfg(feature = "local_fs")]
use crate::code_review::diff_state::DiffStats;
#[cfg(feature = "local_fs")]
use crate::code_review::git_status_update::{GitRepoStatusModel, GitStatusMetadata};
#[cfg(windows)]
use crate::system::SystemInfo;
use crate::{
    auth::{auth_manager::AuthManager, AuthStateProvider},
    context_chips::{
        context_chip::{ChipFingerprintInput, Environment},
        prompt::Prompt,
        ChipAvailability, ChipDisabledReason, ChipRuntimeCapabilities, ContextChipKind,
    },
    features::FeatureFlag,
    menu::MenuItem,
    server::{
        server_api::ServerApiProvider, telemetry::context_provider::AppTelemetryContextProvider,
    },
    settings::WarpPromptSeparator,
    terminal::{
        model::{
            block::BlockMetadata,
            session::{CommandExecutor, ExecuteCommandOptions, SessionId, SessionInfo, Sessions},
        },
        session_settings::{GithubPrPromptChipDefaultValidation, SessionSettings},
        shell::Shell,
        view::PromptPosition,
        History,
    },
};
#[cfg(feature = "local_fs")]
use repo_metadata::DirectoryWatcher;
use warp_completer::completer::{CommandExitStatus, CommandOutput};

use super::{ChipUpdateStatus, CurrentPrompt, PromptContext};

#[test]
fn test_context_menu_items() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [
                    ContextChipKind::WorkingDirectory,
                    ContextChipKind::VirtualEnvironment,
                ],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });

        let sessions = app.add_model(|_| Sessions::new_for_test());
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        // Set a value for the working directory, but not the virtual environment.
        current_prompt.update(&mut app, |current_prompt, ctx| {
            // Ensure there are state entries for the expected chips.
            current_prompt.update_states_with_new_context(ctx);
            current_prompt.update_chip_value(
                &ContextChipKind::WorkingDirectory,
                Some(crate::context_chips::ChipValue::Text(
                    "/path/to/dir".to_string(),
                )),
            );
        });

        app.read(|ctx| {
            let menu_items = current_prompt
                .as_ref(ctx)
                .copy_menu_items(PromptPosition::Input, ctx)
                .into_iter()
                .filter_map(|item| match item {
                    MenuItem::Item(fields) => Some(fields.label().to_string()),
                    _ => None,
                })
                .collect_vec();

            assert_eq!(menu_items, vec!["Copy Working Directory"]);
        })
    });
}

#[test]
fn test_prompt_to_string() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [
                    ContextChipKind::Username,
                    ContextChipKind::VirtualEnvironment,
                    ContextChipKind::WorkingDirectory,
                    ContextChipKind::ShellGitBranch,
                ],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });

        let sessions = app.add_model(|_| Sessions::new_for_test());
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        // Set a value for the working directory, but not the virtual environment.
        current_prompt.update(&mut app, |current_prompt, ctx| {
            // Ensure there are state entries for the expected chips.
            current_prompt.update_states_with_new_context(ctx);
            current_prompt.update_chip_value(
                &ContextChipKind::Username,
                Some(crate::context_chips::ChipValue::Text("user".to_string())),
            );
            current_prompt.update_chip_value(
                &ContextChipKind::WorkingDirectory,
                Some(crate::context_chips::ChipValue::Text(
                    "/path/to/dir".to_string(),
                )),
            );
            current_prompt.update_chip_value(
                &ContextChipKind::ShellGitBranch,
                Some(crate::context_chips::ChipValue::Text(
                    "my-branch".to_string(),
                )),
            );
        });

        app.read(|ctx| {
            let prompt_string = current_prompt.as_ref(ctx).prompt_as_string(ctx);
            // Components should be in order, and missing components should be skipped.
            assert_eq!(prompt_string, "user /path/to/dir git:(my-branch)");
        })
    });
}

#[test]
fn test_fingerprint_skips_contextual_chip_recompute_when_context_is_unchanged() {
    App::test((), |mut app| async move {
        let session_id = SessionId::from(777);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::WorkingDirectory],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });

        let sessions = app.add_model(|_| Sessions::new_for_test());
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        current_prompt.update(&mut app, |current_prompt, ctx| {
            current_prompt.latest_context = Some(PromptContext {
                active_block_metadata: BlockMetadata::new(
                    Some(session_id),
                    Some("/tmp/project".to_string()),
                ),
                environment: Environment::default(),
            });
            current_prompt.update_states_with_new_context(ctx);

            let state = current_prompt
                .states
                .get(&ContextChipKind::WorkingDirectory)
                .expect("expected working directory state");
            assert_eq!(state.update_status, ChipUpdateStatus::Ready);
            assert!(state.last_fingerprint.is_some());
        });

        current_prompt.update(&mut app, |current_prompt, ctx| {
            current_prompt.update_states_with_new_context(ctx);

            let state = current_prompt
                .states
                .get(&ContextChipKind::WorkingDirectory)
                .expect("expected working directory state");
            assert_eq!(state.update_status, ChipUpdateStatus::Cached);
            assert!(matches!(
                state.last_computed_value.as_ref().and_then(|v| v.as_text()),
                Some("/tmp/project")
            ));
        });
    });
}

#[test]
fn test_shell_chip_is_disabled_when_required_executable_is_missing() {
    App::test((), |mut app| async move {
        let session_id = SessionId::from(456);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::ShellGitBranch],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::default());
        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let sessions_for_prompt = sessions.clone();
        let current_prompt =
            app.add_model(move |ctx| CurrentPrompt::new(sessions_for_prompt.clone(), ctx));

        let session = app
            .read(|ctx| sessions.as_ref(ctx).get(session_id))
            .expect("session should exist");
        session.load_external_commands().await;
        executor.clear();

        current_prompt.update(&mut app, |current_prompt, ctx| {
            current_prompt.latest_context = Some(PromptContext {
                active_block_metadata: BlockMetadata::new(
                    Some(session_id),
                    Some("/tmp/project".to_string()),
                ),
                environment: Environment::default(),
            });
            current_prompt.update_states_with_new_context(ctx);

            let state = current_prompt
                .states
                .get(&ContextChipKind::ShellGitBranch)
                .expect("expected git branch state");
            assert_eq!(
                state.availability,
                ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
                    command: "git".to_string(),
                })
            );
            assert_eq!(state.update_status, ChipUpdateStatus::Disabled);
            assert!(state.generator_handle.is_none());
            assert!(state.on_click_generator_handle.is_none());
        });

        assert!(executor.commands.lock().is_empty());
    });
}

#[test]
fn test_github_pr_chip_runtime_policy_configuration() {
    let _flag_guard = FeatureFlag::GithubPrPromptChip.override_enabled(true);
    let chip = ContextChipKind::GithubPullRequest
        .to_chip()
        .expect("github pr chip should exist");
    let policy = chip.runtime_policy();

    assert_eq!(
        policy.required_executables(),
        &["gh".to_string(), "git".to_string()]
    );
    assert_eq!(policy.shell_command_timeout(), Some(Duration::from_secs(5)));
    assert!(policy.suppress_on_failure());
    assert!(policy
        .fingerprint_inputs()
        .contains(&ChipFingerprintInput::SessionId));
    assert!(policy
        .fingerprint_inputs()
        .contains(&ChipFingerprintInput::WorkingDirectory));
    assert!(policy
        .fingerprint_inputs()
        .contains(&ChipFingerprintInput::GitBranch));
    assert!(policy
        .fingerprint_inputs()
        .contains(&ChipFingerprintInput::RequiredExecutablesPresence));
    assert_eq!(
        chip.availability(&ChipRuntimeCapabilities {
            session_is_local: Some(false),
            ..Default::default()
        }),
        ChipAvailability::Disabled(ChipDisabledReason::RequiresLocalSession)
    );
    assert_eq!(
        policy.invalidate_on_commands(),
        &["git".to_string(), "gh".to_string(), "gt".to_string()]
    );
    assert!(policy
        .fingerprint_inputs()
        .contains(&ChipFingerprintInput::InvalidatingCommandCount));
}

#[test]
fn test_invalidating_command_count_unaffected_for_chips_without_invalidate_on_commands() {
    App::test((), |mut app| async move {
        let session_id = SessionId::from(888);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::WorkingDirectory],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });

        let sessions = app.add_model(|_| Sessions::new_for_test());
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        current_prompt.update(&mut app, |current_prompt, ctx| {
            current_prompt.latest_context = Some(PromptContext {
                active_block_metadata: BlockMetadata::new(
                    Some(session_id),
                    Some("/tmp/project".to_string()),
                ),
                environment: Environment::default(),
            });
            current_prompt.update_states_with_new_context(ctx);

            // WorkingDirectory has no invalidate_on_commands, so the counter should be 0.
            let state = current_prompt
                .states
                .get(&ContextChipKind::WorkingDirectory)
                .expect("expected working directory state");
            assert_eq!(state.invalidating_command_count, 0);
        });
    });
}

#[test]
fn test_github_pr_chip_is_disabled_when_github_cli_is_missing() {
    let _flag_guard = FeatureFlag::GithubPrPromptChip.override_enabled(true);
    App::test((), |mut app| async move {
        let session_id = SessionId::from(654);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::GithubPullRequest],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::default());
        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let sessions_for_prompt = sessions.clone();
        let current_prompt =
            app.add_model(move |ctx| CurrentPrompt::new(sessions_for_prompt.clone(), ctx));

        let session = app
            .read(|ctx| sessions.as_ref(ctx).get(session_id))
            .expect("session should exist");
        session.load_external_commands().await;
        executor.clear();

        current_prompt.update(&mut app, |current_prompt, ctx| {
            current_prompt.latest_context = Some(PromptContext {
                active_block_metadata: BlockMetadata::new(
                    Some(session_id),
                    Some("/tmp/project".to_string()),
                ),
                environment: Environment::default(),
            });
            current_prompt.update_states_with_new_context(ctx);

            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(
                state.availability,
                ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
                    command: "gh".to_string(),
                })
            );
            assert_eq!(state.update_status, ChipUpdateStatus::Disabled);
            assert!(state.generator_handle.is_none());
            assert!(state.on_click_generator_handle.is_none());
        });

        assert!(executor.commands.lock().is_empty());
    });
}

#[test]
fn test_github_pr_chip_empty_success_does_not_set_failure_suppression() {
    let _flag_guard = FeatureFlag::GithubPrPromptChip.override_enabled(true);
    App::test((), |mut app| async move {
        let session_id = SessionId::from(789);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::GithubPullRequest],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::with_success_responses([
            "gh\ngit\n",
            "",
        ]));
        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let sessions_for_prompt = sessions.clone();
        let current_prompt =
            app.add_model(move |ctx| CurrentPrompt::new(sessions_for_prompt.clone(), ctx));

        let session = app
            .read(|ctx| sessions.as_ref(ctx).get(session_id))
            .expect("session should exist");
        session.load_external_commands().await;
        executor.clear();

        // First update: chip command runs and returns empty.
        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/project".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;

        // The chip command should have run once.
        assert_eq!(executor.commands.lock().len(), 1);

        // Verify the empty success cleared the chip value without setting failure suppression.
        current_prompt.update(&mut app, |current_prompt, _ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(state.last_failure_fingerprint, None);
            assert!(
                state.last_computed_value.is_none(),
                "chip value should be None after empty result"
            );
            assert_eq!(state.update_status, ChipUpdateStatus::Ready);
        });
    });
}

#[test]
fn test_github_pr_chip_revisiting_empty_result_directory_reruns_and_clears_previous_value() {
    let _flag_guard = FeatureFlag::GithubPrPromptChip.override_enabled(true);
    App::test((), |mut app| async move {
        let session_id = SessionId::from(790);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::GithubPullRequest],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::with_success_responses([
            "gh\ngit\n",
            "",
            "https://github.com/warp/warp/pull/456\n",
            "",
        ]));
        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let sessions_for_prompt = sessions.clone();
        let current_prompt =
            app.add_model(move |ctx| CurrentPrompt::new(sessions_for_prompt.clone(), ctx));

        let session = app
            .read(|ctx| sessions.as_ref(ctx).get(session_id))
            .expect("session should exist");
        session.load_external_commands().await;
        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/no-pr".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;
        current_prompt.update(&mut app, |current_prompt, _ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(state.last_computed_value, None);
            assert_eq!(state.last_failure_fingerprint, None);
        });

        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/has-pr".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;

        current_prompt.update(&mut app, |current_prompt, _ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(
                state.last_computed_value.as_ref().and_then(|v| v.as_text()),
                Some("https://github.com/warp/warp/pull/456")
            );
        });

        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/no-pr".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;

        assert_eq!(executor.commands.lock().len(), 1);

        current_prompt.update(&mut app, |current_prompt, _ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(state.last_computed_value, None);
            assert_eq!(state.last_failure_fingerprint, None);
            assert_eq!(state.update_status, ChipUpdateStatus::Ready);
        });
    });
}

#[test]
fn test_github_pr_chip_revisiting_failed_directory_uses_failure_suppression() {
    let _flag_guard = FeatureFlag::GithubPrPromptChip.override_enabled(true);
    App::test((), |mut app| async move {
        let session_id = SessionId::from(791);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::GithubPullRequest],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::with_outputs([
            RecordingCommandExecutor::success_output("gh\ngit\n"),
            RecordingCommandExecutor::failure_output("authentication required", ExitCode::from(4)),
            RecordingCommandExecutor::success_output("https://github.com/warp/warp/pull/456\n"),
        ]));
        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let sessions_for_prompt = sessions.clone();
        let current_prompt =
            app.add_model(move |ctx| CurrentPrompt::new(sessions_for_prompt.clone(), ctx));

        let session = app
            .read(|ctx| sessions.as_ref(ctx).get(session_id))
            .expect("session should exist");
        session.load_external_commands().await;
        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/gh-failure".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;

        current_prompt.update(&mut app, |current_prompt, ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(state.last_computed_value, None);
            assert!(state.last_failure_fingerprint.is_some());
            assert_eq!(state.update_status, ChipUpdateStatus::Error);
            assert_eq!(
                *SessionSettings::as_ref(ctx).github_pr_chip_default_validation,
                GithubPrPromptChipDefaultValidation::Suppressed
            );
        });

        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/has-pr".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;

        current_prompt.update(&mut app, |current_prompt, _ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(
                state.last_computed_value.as_ref().and_then(|v| v.as_text()),
                Some("https://github.com/warp/warp/pull/456")
            );
        });

        executor.clear();

        current_prompt.update(&mut app, |current_prompt, ctx| {
            current_prompt.latest_context = Some(PromptContext {
                active_block_metadata: BlockMetadata::new(
                    Some(session_id),
                    Some("/tmp/gh-failure".to_string()),
                ),
                environment: Environment::default(),
            });
            current_prompt.update_states_with_new_context(ctx);
        });

        assert!(executor.commands.lock().is_empty());

        current_prompt.update(&mut app, |current_prompt, _ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(state.last_computed_value, None);
            assert_eq!(state.update_status, ChipUpdateStatus::Cached);
            assert!(state.last_failure_fingerprint.is_some());
        });
    });
}

#[test]
fn test_github_pr_chip_transient_failure_retries_with_same_fingerprint() {
    let _flag_guard = FeatureFlag::GithubPrPromptChip.override_enabled(true);
    App::test((), |mut app| async move {
        let session_id = SessionId::from(792);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::GithubPullRequest],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::with_outputs([
            RecordingCommandExecutor::success_output("gh\ngit\n"),
            RecordingCommandExecutor::failure_output(
                "Post \"https://api.github.com/graphql\": dial tcp: lookup api.github.com: no such host",
                ExitCode::from(1),
            ),
            RecordingCommandExecutor::success_output("https://github.com/warp/warp/pull/456\n"),
        ]));
        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let sessions_for_prompt = sessions.clone();
        let current_prompt =
            app.add_model(move |ctx| CurrentPrompt::new(sessions_for_prompt.clone(), ctx));

        let session = app
            .read(|ctx| sessions.as_ref(ctx).get(session_id))
            .expect("session should exist");
        session.load_external_commands().await;
        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(
                        Some(session_id),
                        Some("/tmp/network-failure".to_string()),
                    ),
                    environment: Environment::default(),
                });
                current_prompt.update_states_with_new_context(ctx);
                current_prompt.await_generators(ctx)
            })
            .await;

        assert_eq!(executor.commands.lock().len(), 1);

        current_prompt.update(&mut app, |current_prompt, ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(state.last_computed_value, None);
            assert_eq!(state.last_failure_fingerprint, None);
            assert_eq!(state.update_status, ChipUpdateStatus::Error);
            assert_eq!(
                *SessionSettings::as_ref(ctx).github_pr_chip_default_validation,
                GithubPrPromptChipDefaultValidation::Unvalidated
            );
        });

        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                let chip = ContextChipKind::GithubPullRequest
                    .to_chip()
                    .expect("expected github pr chip");
                let generator = chip.generator().clone();
                // Pass `allow_fingerprint_skip = true` to exercise the same
                // path the periodic timer uses. The previous attempt left the
                // chip in `Error` state with the fingerprint already recorded;
                // without status-aware skip handling, this call would short-
                // circuit as `Cached` and the transient failure would become
                // sticky.
                current_prompt.fetch_chip_value_once(
                    &ContextChipKind::GithubPullRequest,
                    &generator,
                    None,
                    true,
                    ctx,
                );
                current_prompt.await_generators(ctx)
            })
            .await;

        assert_eq!(executor.commands.lock().len(), 1);

        current_prompt.update(&mut app, |current_prompt, ctx| {
            let state = current_prompt
                .states
                .get(&ContextChipKind::GithubPullRequest)
                .expect("expected github pr state");
            assert_eq!(
                state.last_computed_value.as_ref().and_then(|v| v.as_text()),
                Some("https://github.com/warp/warp/pull/456")
            );
            assert_eq!(state.last_failure_fingerprint, None);
            assert_eq!(state.update_status, ChipUpdateStatus::Ready);
            assert_eq!(
                *SessionSettings::as_ref(ctx).github_pr_chip_default_validation,
                GithubPrPromptChipDefaultValidation::Validated
            );
        });
    });
}

#[test]
fn test_github_pr_chip_caches_only_deterministic_failures() {
    let auth_failure =
        RecordingCommandExecutor::failure_output("authentication required", ExitCode::from(4));
    let network_failure = RecordingCommandExecutor::failure_output(
        "Post \"https://api.github.com/graphql\": dial tcp: lookup api.github.com: no such host",
        ExitCode::from(1),
    );

    assert!(CurrentPrompt::should_cache_failure_fingerprint(
        &ContextChipKind::GithubPullRequest,
        Some(&auth_failure),
        false,
    ));
    assert!(!CurrentPrompt::should_cache_failure_fingerprint(
        &ContextChipKind::GithubPullRequest,
        Some(&network_failure),
        false,
    ));
    assert!(!CurrentPrompt::should_cache_failure_fingerprint(
        &ContextChipKind::GithubPullRequest,
        None,
        true,
    ));
    assert!(CurrentPrompt::should_cache_failure_fingerprint(
        &ContextChipKind::ShellGitBranch,
        None,
        true,
    ));
}
#[test]
fn test_disabling_chips() {
    App::test((), |mut app| async move {
        let session_id = SessionId::from(123);
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::ShellGitBranch],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_| History::new(vec![]));
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);

        // Register required singleton models to fix the singleton model error
        app.add_singleton_model(|_| crate::settings::manager::SettingsManager::default());
        crate::settings::InputSettings::register(&mut app);
        app.update(crate::settings::AISettings::register_and_subscribe_to_events);
        app.add_singleton_model(crate::workspaces::user_workspaces::UserWorkspaces::default_mock);
        #[cfg(windows)]
        app.add_singleton_model(SystemInfo::new);

        let executor = Arc::new(RecordingCommandExecutor::default());

        let sessions = app.add_model(|ctx| {
            let mut sessions = Sessions::new_for_test().with_command_executor(executor.clone());
            sessions.initialize_bootstrapped_session(
                SessionInfo::new_for_test().with_id(session_id),
                "test command".to_string(),
                vec![],
                None,
                ctx,
            );
            sessions
        });
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        // Context chips can only be disabled in Classic mode.
        app.update(|ctx| {
            crate::settings::InputSettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .input_box_type
                    .set_value(crate::settings::InputBoxType::Classic, ctx);
            });
        });

        executor.clear();

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                current_prompt.latest_context = Some(PromptContext {
                    active_block_metadata: BlockMetadata::new(Some(session_id), None),
                    environment: Environment::default(),
                });
                // This is needed because we set latest_context directly.
                current_prompt.update_states_with_new_context(ctx);
                assert!(current_prompt.are_any_generators_running());
                current_prompt.await_generators(ctx)
            })
            .await;

        // By default, context chips are enabled, so the git branch command should run. It may run
        // twice due to how periodically-refreshing chips are implemented.
        assert!(!executor.commands.lock().is_empty());

        // If PS1 is enabled, the command should not run.
        app.update(|ctx| {
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings.honor_ps1.set_value(true, ctx);
            });
        });
        // Clear the command history right after changing the PS1 setting, to ensure that the
        // CurrentPrompt model has processed the change.
        executor.clear();

        current_prompt.update(&mut app, |current_prompt, ctx| {
            // Ensure that, if the model were going to run generators, it had a chance to.
            current_prompt.update_states_with_new_context(ctx);
            // There may be some shell generators still pending in the background, which won't be
            // directly cancelled. Instead of asserting that no commands run, assert that the
            // CurrentPrompt model is not still trying to run generators.
            assert!(!current_prompt.are_any_generators_running());
        });

        // If context chips are re-enabled, generator commands should start running again.
        app.update(|ctx| {
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings.honor_ps1.set_value(false, ctx);
            });
        });

        current_prompt
            .update(&mut app, |current_prompt, ctx| {
                assert!(current_prompt.are_any_generators_running());
                current_prompt.await_generators(ctx)
            })
            .await;

        assert!(!executor.commands.lock().is_empty());
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_externally_driven_chip_skips_periodic_timer() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::ShellGitBranch],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });

        let temp_dir = tempfile::TempDir::new().unwrap();
        let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let repo_handle = watcher_handle.update(&mut app, |watcher, ctx| {
            watcher
                .add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(
                        temp_dir.path(),
                    )
                    .unwrap(),
                    ctx,
                )
                .unwrap()
        });
        let git_status =
            app.add_model(move |_| GitRepoStatusModel::new_for_test(repo_handle, None));

        let sessions = app.add_model(|_| Sessions::new_for_test());
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        current_prompt.update(&mut app, |cp, ctx| {
            cp.set_git_repo_status(Some(git_status.downgrade()), ctx);
            cp.update_states_with_new_context(ctx);
        });

        app.read(|ctx| {
            let cp = current_prompt.as_ref(ctx);
            let state = cp
                .states
                .get(&ContextChipKind::ShellGitBranch)
                .expect("ShellGitBranch state should exist after set_git_repo_status");
            assert!(
                state.refresh_handle.is_none(),
                "Externally-driven chip should not have a periodic refresh handle"
            );
        });
    });
}

#[cfg(feature = "local_fs")]
#[test]
fn test_git_status_change_updates_chip_value() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            Prompt::mock_with(
                [ContextChipKind::ShellGitBranch],
                false,
                WarpPromptSeparator::None,
            )
        });
        app.add_singleton_model(SessionSettings::new_with_defaults);
        app.add_singleton_model(|_ctx| {
            settings::PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        app.add_singleton_model(|_| {
            settings::PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });

        let temp_dir = tempfile::TempDir::new().unwrap();
        let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let repo_handle = watcher_handle.update(&mut app, |watcher, ctx| {
            watcher
                .add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(
                        temp_dir.path(),
                    )
                    .unwrap(),
                    ctx,
                )
                .unwrap()
        });

        let initial_metadata = GitStatusMetadata {
            current_branch_name: "main".to_string(),
            main_branch_name: "main".to_string(),
            stats_against_head: DiffStats::default(),
        };
        let git_status = app.add_model(move |_| {
            GitRepoStatusModel::new_for_test(repo_handle, Some(initial_metadata))
        });

        let sessions = app.add_model(|_| Sessions::new_for_test());
        let current_prompt = app.add_model(move |ctx| CurrentPrompt::new(sessions, ctx));

        // Subscribe to the git status model and run chips.
        current_prompt.update(&mut app, |cp, ctx| {
            cp.set_git_repo_status(Some(git_status.downgrade()), ctx);
            cp.update_states_with_new_context(ctx);
        });

        // Simulate a branch change by updating the model's metadata.
        git_status.update(&mut app, |model, ctx| {
            model.set_metadata_for_test(
                Some(GitStatusMetadata {
                    current_branch_name: "feature-branch".to_string(),
                    main_branch_name: "main".to_string(),
                    stats_against_head: DiffStats::default(),
                }),
                ctx,
            );
        });

        app.read(|ctx| {
            let value = current_prompt
                .as_ref(ctx)
                .latest_chip_value(&ContextChipKind::ShellGitBranch);
            assert_eq!(
                value,
                Some(&crate::context_chips::ChipValue::Text(
                    "feature-branch".to_string(),
                )),
                "Chip value should reflect the new branch name after metadata change"
            );
        });
    });
}

/// A [`CommandExecutor`] implementation that records which commands were run, but does not
/// execute them.
#[derive(Debug, Default)]
struct RecordingCommandExecutor {
    commands: Mutex<Vec<String>>,
    response_queue: Mutex<VecDeque<CommandOutput>>,
}

impl RecordingCommandExecutor {
    pub fn with_success_responses(responses: impl IntoIterator<Item = &'static str>) -> Self {
        Self::with_outputs(
            responses
                .into_iter()
                .map(Self::success_output)
                .collect::<Vec<_>>(),
        )
    }

    pub fn with_outputs(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
        Self {
            commands: Mutex::default(),
            response_queue: Mutex::new(outputs.into_iter().collect()),
        }
    }

    pub fn success_output(stdout: impl AsRef<[u8]>) -> CommandOutput {
        CommandOutput {
            stdout: stdout.as_ref().to_vec(),
            stderr: vec![],
            status: CommandExitStatus::Success,
            exit_code: Some(ExitCode::from(0)),
        }
    }

    pub fn failure_output(stderr: impl AsRef<[u8]>, exit_code: ExitCode) -> CommandOutput {
        CommandOutput {
            stdout: vec![],
            stderr: stderr.as_ref().to_vec(),
            status: CommandExitStatus::Failure,
            exit_code: Some(exit_code),
        }
    }

    pub fn clear(&self) {
        self.commands.lock().clear();
    }
}

#[async_trait]
impl CommandExecutor for RecordingCommandExecutor {
    async fn execute_command(
        &self,
        command: &str,
        _shell: &Shell,
        _current_directory_path: Option<&str>,
        _environment_variables: Option<HashMap<String, String>>,
        _execute_command_options: ExecuteCommandOptions,
    ) -> anyhow::Result<CommandOutput> {
        self.commands.lock().push(command.to_string());
        let output = self
            .response_queue
            .lock()
            .pop_front()
            .unwrap_or_else(|| Self::success_output("test"));
        Ok(output)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        false
    }
}
