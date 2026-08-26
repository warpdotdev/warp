use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use warp::tui_export::{UserWorkspaces, register_tui_session_view_test_singletons};
use warp_core::features::FeatureFlag;
use warpui::SingletonEntity as _;
use warpui_core::App;

use super::*;

fn row(
    id: &str,
    is_selectable: bool,
    is_key_connected: bool,
    is_profile_default: bool,
) -> TuiModelMenuRow {
    TuiModelMenuRow {
        id: id.into(),
        title: id.to_owned(),
        is_selectable,
        is_key_connected,
        is_profile_default,
        discount_percentage: None,
    }
}

#[test]
fn empty_query_prefers_active_model_and_filtered_query_prefers_best_match() {
    let rows = vec![
        row("auto", true, false, false),
        row("gpt-4", true, false, false),
        row("gpt-5", true, false, false),
    ];

    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("gpt-4"), true),
        Some(1)
    );
    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("gpt-4"), false),
        Some(2)
    );
}

#[test]
fn model_selection_skips_disabled_rows() {
    let rows = vec![
        row("auto", true, false, false),
        row("gpt-5", true, false, false),
        row("disabled", false, false, false),
    ];

    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("disabled"), true),
        Some(1)
    );
    assert_eq!(
        preferred_selection_index(&rows, &LLMId::from("auto"), false),
        Some(1)
    );
}

#[test]
fn snapshot_marks_only_key_connected_models() {
    let connected = snapshot_row(&row("gpt-5", true, true, false));
    assert_eq!(connected.state_suffix.as_deref(), Some("(key connected)"));
    let hosted = snapshot_row(&row("auto", true, false, false));
    assert_eq!(hosted.state_suffix, None);
}
#[test]
fn snapshot_marks_the_profile_default_model() {
    let default = snapshot_row(&row("auto", true, false, true));
    assert_eq!(default.state_suffix.as_deref(), Some("(default)"));

    let connected_default = snapshot_row(&row("gpt-5", true, true, true));
    assert_eq!(
        connected_default.state_suffix.as_deref(),
        Some("(default) (key connected)")
    );
}

#[test]
fn provider_key_controls_key_connected_callout() {
    App::test((), |mut app| async move {
        let _byok = FeatureFlag::SoloUserByok.override_enabled(true);
        register_tui_session_view_test_singletons(&mut app);
        let scope = UserWorkspaces::teamless_context_resolver_for_test();
        let mut llm = app.read(|ctx| {
            LLMPreferences::as_ref(ctx)
                .get_active_base_model(ctx, None)
                .clone()
        });
        llm.provider = LLMProvider::OpenAI;

        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("test-key".to_owned()), ctx)
            })
            .unwrap();
        let connected_row = app.read(|ctx| {
            let scope = (scope)(ctx);
            let choice =
                query_model_picker_choices(LLMPreferences::as_ref(ctx), [&llm], "", &scope, ctx)
                    .remove(0);
            model_menu_row(choice, &LLMId::from("profile-default"), &scope, ctx)
        });
        assert_eq!(
            snapshot_row(&connected_row).state_suffix.as_deref(),
            Some("(key connected)")
        );

        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, None, ctx)
            })
            .unwrap();
        let disconnected_row = app.read(|ctx| {
            let scope = (scope)(ctx);
            let choice =
                query_model_picker_choices(LLMPreferences::as_ref(ctx), [&llm], "", &scope, ctx)
                    .remove(0);
            model_menu_row(choice, &LLMId::from("profile-default"), &scope, ctx)
        });
        assert_eq!(snapshot_row(&disconnected_row).state_suffix, None);
    });
}
