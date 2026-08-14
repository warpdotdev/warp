use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use ai::custom_endpoints::CustomEndpointDefinitionsConfig;
use warp::settings::AISettings;
use warp::tui_export::register_tui_session_view_test_singletons;
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting as _;
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
        custom_endpoint_description: None,
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
            let choice =
                query_model_picker_choices(LLMPreferences::as_ref(ctx), [&llm], "", ctx).remove(0);
            model_menu_row(choice, &LLMId::from("profile-default"), ctx)
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
            let choice =
                query_model_picker_choices(LLMPreferences::as_ref(ctx), [&llm], "", ctx).remove(0);
            model_menu_row(choice, &LLMId::from("profile-default"), ctx)
        });
        assert_eq!(snapshot_row(&disconnected_row).state_suffix, None);
    });
}

#[test]
fn custom_endpoint_model_shows_description_without_key_connected_suffix() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);

        app.update(|ctx| {
            let mut object = serde_json::Map::new();
            object.insert(
                "Acme Gateway".to_owned(),
                serde_json::json!({
                    "url": "https://llm.acme.example/v1",
                    "models": [{"name": "gpt-4o"}],
                }),
            );
            let config = CustomEndpointDefinitionsConfig::from_object(&object);
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .custom_endpoints
                    .set_value(config, ctx)
                    .expect("custom endpoint definitions should persist");
            });
        });

        // Unkeyed: the endpoint is defined but its model is not in the
        // picker's registry at all yet.
        let has_custom_model_before_key = app.read(|ctx| {
            LLMPreferences::as_ref(ctx)
                .custom_llm_choices(ctx)
                .next()
                .is_some()
        });
        assert!(
            !has_custom_model_before_key,
            "an unkeyed endpoint's models must not appear in the model picker"
        );

        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_custom_endpoint_key("Acme Gateway", Some("sk-acme".to_owned()), ctx)
            })
            .unwrap();

        let custom_llm = app.read(|ctx| {
            LLMPreferences::as_ref(ctx)
                .custom_llm_choices(ctx)
                .next()
                .cloned()
                .expect("custom endpoint model should be available once keyed")
        });

        let row = app.read(|ctx| {
            let choice =
                query_model_picker_choices(LLMPreferences::as_ref(ctx), [&custom_llm], "", ctx)
                    .remove(0);
            model_menu_row(choice, &LLMId::from("profile-default"), ctx)
        });
        let snapshot = snapshot_row(&row);
        println!("--- Model picker row for a keyed custom endpoint model ---");
        println!(
            "title={:?} description={:?} state_suffix={:?}",
            snapshot.title, snapshot.description, snapshot.state_suffix
        );
        assert_eq!(
            snapshot.description.as_deref(),
            Some("Custom · Acme Gateway")
        );
        // A custom model cannot appear until its endpoint has a key, so the
        // generic key-connected suffix would be redundant here.
        assert_eq!(snapshot.state_suffix, None);
    });
}
