use serde_json::Value;
use warpui::{App, SingletonEntity};

use super::Prompt;
use crate::auth::AuthStateProvider;
use crate::context_chips::prompt::{PromptConfiguration, PromptSelection};
use crate::context_chips::ContextChipKind;
use crate::settings::WarpPromptSeparator;
use crate::terminal::session_settings::SessionSettings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
}

#[test]
// Legacy prompt configs do not have git diff stats, so it should be added after normalization.
// `did_separate_git_diff_stats` is set to `false`.
fn test_prompt_config_adds_git_diff_stats_for_legacy_config() {
    let config = PromptConfiguration::from_chips(
        [
            ContextChipKind::WorkingDirectory,
            ContextChipKind::ShellGitBranch,
        ],
        false,
        WarpPromptSeparator::None,
    );
    let mut serialized = serde_json::to_value(config).expect("serialize prompt config");

    let Value::Object(ref mut map) = serialized else {
        panic!("expected object");
    };
    map.remove("did_separate_git_diff_stats");

    let legacy_config: PromptConfiguration =
        serde_json::from_value(serialized).expect("deserialize legacy config");
    let normalized = legacy_config.normalize_custom_prompt_config();

    assert_eq!(
        normalized.chip_kinds(),
        vec![
            ContextChipKind::WorkingDirectory,
            ContextChipKind::ShellGitBranch,
            ContextChipKind::GitDiffStats,
        ]
    );
}

#[test]
// Ensure that prompt configs don't re-insert git diff stats if they were explicitly removed.
// `did_separate_git_diff_stats` is set to `true`.
fn test_prompt_config_after_nomalization() {
    let config = PromptConfiguration::from_chips(
        [ContextChipKind::ShellGitBranch],
        false,
        WarpPromptSeparator::None,
    );
    let normalized = config.normalize_custom_prompt_config();

    assert_eq!(
        normalized.chip_kinds(),
        vec![ContextChipKind::ShellGitBranch]
    );
}

#[test]
fn test_prompt_settings() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let session_settings = SessionSettings::handle(&app);
        let default_prompt = PromptConfiguration::default_prompt();

        let prompt = app.add_singleton_model(Prompt::new);

        // First, the default prompt should be set.
        let current_prompt_chips = prompt.read(&app, |prompt, _| prompt.chip_kinds());
        assert_eq!(current_prompt_chips, default_prompt.chip_kinds());
        session_settings.read(&app, |settings, _| {
            assert_eq!(settings.saved_prompt.to_owned(), PromptSelection::Default)
        });

        // Now, set a new prompt.
        let new_chips = [ContextChipKind::Ssh, ContextChipKind::WorkingDirectory];
        prompt.update(&mut app, |prompt, ctx| {
            prompt
                .update(new_chips.clone(), false, WarpPromptSeparator::None, ctx)
                .expect("Saving prompt failed")
        });

        // The configuration should be updated both in-memory and in settings.
        let new_prompt_chips = prompt.read(&app, |prompt, _| prompt.chip_kinds());
        assert_eq!(
            new_prompt_chips,
            vec![ContextChipKind::Ssh, ContextChipKind::WorkingDirectory]
        );
        session_settings.read(&app, |settings, _| {
            assert_eq!(
                settings.saved_prompt.to_owned(),
                PromptConfiguration::from_chips(new_chips, false, WarpPromptSeparator::None).into()
            );
        });

        // If we reset the prompt, settings are cleared.
        prompt.update(&mut app, |prompt, ctx| {
            prompt.reset(ctx).expect("Saving prompt failed");
        });
        let reset_prompt_chips = prompt.read(&app, |prompt, _| prompt.chip_kinds());
        assert_eq!(reset_prompt_chips, default_prompt.chip_kinds());
        session_settings.read(&app, |settings, _| {
            assert_eq!(settings.saved_prompt.to_owned(), PromptSelection::Default);
        });
    });
}
