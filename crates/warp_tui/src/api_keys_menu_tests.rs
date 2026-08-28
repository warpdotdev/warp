use ai::LLMProvider;
use ai::api_keys::{
    ApiKeyManager, CustomEndpoint, CustomEndpointDefinitions, CustomEndpointModel,
    CustomEndpointSchema,
};
use warp::editor::CodeEditorModel;
use warp::settings::AISettings;
use warp::tui_export::{UserWorkspaces, register_tui_session_view_test_singletons};
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::{App, ModelHandle};

use super::{TuiApiKeysFooter, TuiApiKeysMenuModel, input_text};
use crate::inline_menu::TuiInlineMenuInputOwnership;
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

fn endpoint_definitions(names: &[&str]) -> CustomEndpointDefinitions {
    let endpoints = names
        .iter()
        .enumerate()
        .map(|(index, name)| CustomEndpoint {
            name: (*name).to_owned(),
            url: format!("https://endpoint-{index}.example.com/v1"),
            api_key: String::new(),
            schema: CustomEndpointSchema::default(),
            models: vec![CustomEndpointModel {
                name: format!("model-{index}"),
                alias: None,
                config_key: format!("config-{index}"),
            }],
        })
        .collect::<Vec<_>>();
    CustomEndpointDefinitions::from_legacy(&endpoints)
        .expect("test endpoints should be valid")
        .0
}

fn add_menu(
    app: &mut App,
) -> (
    ModelHandle<CodeEditorModel>,
    ModelHandle<TuiInputSuggestionsModeModel>,
    ModelHandle<TuiApiKeysMenuModel>,
) {
    register_tui_session_view_test_singletons(app);
    app.update(|ctx| {
        let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
        let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
        let menu = ctx.add_model(|ctx| {
            TuiApiKeysMenuModel::new(
                input.clone(),
                mode.clone(),
                UserWorkspaces::teamless_context_resolver_for_test(),
                ctx,
            )
        });
        menu.update(ctx, |menu, ctx| menu.open(ctx));
        (input, mode, menu)
    })
}

#[test]
fn changing_the_shared_menu_mode_deactivates_api_keys_state() {
    App::test((), |mut app| async move {
        let (input, mode, menu) = add_menu(&mut app);
        input.update(&mut app, |input, ctx| input.user_insert("query", ctx));
        mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::ModelSelector, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::ModelSelector
            );
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::Composer
            );
            assert!(!menu.as_ref(ctx).uses_credential_border(ctx));
            assert_eq!(menu.as_ref(ctx).footer(ctx), None);
            assert_eq!(input_text(&input, ctx), "");
        });
    });
}

#[test]
fn custom_endpoint_rows_use_user_names_and_support_key_editing_and_clearing() {
    App::test((), |mut app| async move {
        let (input, _, menu) = add_menu(&mut app);
        let definitions = endpoint_definitions(&["Zulu", "OpenRouter"]);
        let openrouter_id = definitions.id_at(1).unwrap().clone();
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_custom_endpoint_definitions(definitions, ctx);
        });

        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .map(|row| (row.title.as_str(), row.state_suffix.as_deref()))
                    .collect::<Vec<_>>(),
                vec![
                    ("Anthropic API key", Some("(Not connected)")),
                    ("Google API key", Some("(Not connected)")),
                    ("OpenAI API key", Some("(Not connected)")),
                    (
                        "X premium or SuperGrok subscription",
                        Some("(Not connected)")
                    ),
                    ("OpenRouter custom endpoint", Some("(Not connected)")),
                    ("Zulu custom endpoint", Some("(Not connected)")),
                    ("Warp credit fallback", Some("(off)")),
                ]
            );
        });

        menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(4, ctx));
            menu.accept_selected(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::EditingCustomEndpoint(
                    "OpenRouter".to_owned()
                ))
            );
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuMasked
            );
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert("openrouter-secret", ctx);
        });
        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert_eq!(
                ApiKeyManager::as_ref(ctx).custom_endpoint_key(&openrouter_id),
                Some("openrouter-secret")
            );
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert_eq!(
                snapshot.rows[4].state_suffix.as_deref(),
                Some("(Connected)")
            );
        });

        menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(4, ctx));
            assert!(menu.can_clear_selected(ctx));
            menu.clear_selected(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                ApiKeyManager::as_ref(ctx).custom_endpoint_key(&openrouter_id),
                None
            );
            assert_eq!(
                menu.as_ref(ctx).snapshot(ctx).unwrap().rows[4]
                    .state_suffix
                    .as_deref(),
                Some("(Not connected)")
            );
        });
    });
}

#[test]
fn invalid_custom_endpoint_settings_render_a_non_selectable_error_row() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);
        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.invalidate_custom_endpoint_definitions(ctx);
        });

        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            let row = snapshot
                .rows
                .iter()
                .find(|row| row.title == "Custom endpoints")
                .expect("configuration row should be visible");
            assert_eq!(
                row.description.as_deref(),
                Some("Fix agents.custom_endpoints in settings")
            );
            assert_eq!(row.state_suffix.as_deref(), Some("(Configuration error)"));
            assert!(!row.is_selectable);
        });
    });
}

#[test]
fn unconfigured_custom_endpoints_are_hidden_and_fallback_stays_last() {
    App::test((), |mut app| async move {
        let (_, mode, menu) = add_menu(&mut app);
        app.read(|ctx| {
            assert_eq!(mode.as_ref(ctx).mode(), TuiInputSuggestionsMode::ApiKeys);
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            );
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .map(|row| row.title.as_str())
                    .collect::<Vec<_>>(),
                vec![
                    "Anthropic API key",
                    "Google API key",
                    "OpenAI API key",
                    "X premium or SuperGrok subscription",
                    "Warp credit fallback",
                ]
            );
            assert_eq!(snapshot.selected_index, Some(0));
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { can_clear: false })
            );
        });
    });
}

#[test]
fn filtering_keeps_warp_credit_fallback_pinned() {
    App::test((), |mut app| async move {
        let (input, _, menu) = add_menu(&mut app);
        input.update(&mut app, |input, ctx| input.user_insert("google", ctx));
        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .map(|row| row.title.as_str())
                    .collect::<Vec<_>>(),
                vec!["Google API key", "Warp credit fallback"]
            );
        });
    });
}

#[test]
fn connected_provider_prefills_secret_input_and_saves_replacement() {
    App::test((), |mut app| async move {
        let (input, _, menu) = add_menu(&mut app);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(
                    LLMProvider::Anthropic,
                    Some("existing-secret".to_owned()),
                    ctx,
                )
            })
            .unwrap();

        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuMasked
            );
            assert_eq!(input_text(&input, ctx), "existing-secret");
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::EditingProvider(LLMProvider::Anthropic))
            );
        });

        input.update(&mut app, |input, ctx| {
            input.clear_buffer(ctx);
            input.user_insert("replacement-secret", ctx);
        });
        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert_eq!(
                ApiKeyManager::as_ref(ctx).keys().anthropic.as_deref(),
                Some("replacement-secret")
            );
            assert_eq!(input_text(&input, ctx), "");
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            );
        });
    });
}

#[test]
fn open_and_connect_grok_matches_selecting_the_grok_row() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);

        // Reference path: open the menu, then select and accept the Grok row.
        let reference = app.update(|ctx| {
            let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let menu = ctx.add_model(|ctx| {
                TuiApiKeysMenuModel::new(
                    input,
                    mode,
                    UserWorkspaces::teamless_context_resolver_for_test(),
                    ctx,
                )
            });
            menu.update(ctx, |menu, ctx| {
                menu.open(ctx);
                assert!(menu.select_at_snapshot_index(3, ctx));
                menu.accept_selected(ctx);
            });
            menu
        });

        // Shortcut path: a single call jumps straight into the Grok connect flow.
        let shortcut = app.update(|ctx| {
            let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let menu = ctx.add_model(|ctx| {
                TuiApiKeysMenuModel::new(
                    input,
                    mode,
                    UserWorkspaces::teamless_context_resolver_for_test(),
                    ctx,
                )
            });
            menu.update(ctx, |menu, ctx| menu.open_and_connect_grok(ctx));
            menu
        });

        app.read(|ctx| {
            assert!(shortcut.as_ref(ctx).is_open(ctx));
            assert_eq!(
                shortcut.as_ref(ctx).footer(ctx),
                reference.as_ref(ctx).footer(ctx),
                "the shortcut should land in the same footer state as selecting the Grok row",
            );
            assert_eq!(
                shortcut
                    .as_ref(ctx)
                    .snapshot(ctx)
                    .map(|snapshot| snapshot.header),
                reference
                    .as_ref(ctx)
                    .snapshot(ctx)
                    .map(|snapshot| snapshot.header),
            );
        });
    });
}

#[test]
fn clear_selected_provider_and_toggle_fallback_keep_menu_open() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);
        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_provider_key(LLMProvider::OpenAI, Some("secret".to_owned()), ctx)
            })
            .unwrap();
        menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(2, ctx));
            assert_eq!(
                menu.footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { can_clear: true })
            );
            menu.clear_selected(ctx);
        });
        app.read(|ctx| {
            assert_eq!(ApiKeyManager::as_ref(ctx).keys().openai, None);
            assert!(menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                menu.as_ref(ctx).snapshot(ctx).unwrap().selected_index,
                Some(2)
            );
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { can_clear: false })
            );
        });

        menu.update(&mut app, |menu, ctx| {
            let fallback_index = menu
                .snapshot(ctx)
                .unwrap()
                .rows
                .iter()
                .position(|row| row.title == "Warp credit fallback")
                .unwrap();
            assert!(menu.select_at_snapshot_index(fallback_index, ctx));
            menu.accept_selected(ctx);
        });
        app.read(|ctx| {
            assert!(*AISettings::as_ref(ctx).can_use_warp_credits_for_fallback);
            assert_eq!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::WarpCreditFallback)
            );
            assert!(menu.as_ref(ctx).is_open(ctx));
        });
    });
}
