use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use ai::custom_endpoints::CustomEndpointDefinitionsConfig;
use warp::editor::CodeEditorModel;
use warp::settings::AISettings;
use warp::tui_export::register_tui_session_view_test_singletons;
use warp_core::settings::Setting as _;
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::elements::tui::{TuiBufferExt as _, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, ModelHandle};

use super::{TuiApiKeysFooter, TuiApiKeysMenuModel, input_text};
use crate::inline_menu::{TuiInlineMenuInputOwnership, render_inline_menu};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::tui_builder::TuiUiBuilder;

/// Renders the menu's current snapshot to real character-cell text, the way
/// it would actually appear on screen. Used to capture rendered-TUI evidence
/// for the PR description in place of an interactive `tmux`/login session
/// (see the `tui-verify-change` skill) when no authenticated session is
/// reachable in this environment.
fn render_menu_lines(
    app: &App,
    menu: &ModelHandle<TuiApiKeysMenuModel>,
    width: u16,
    height: u16,
) -> Vec<String> {
    app.read(|ctx| {
        let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu should be open");
        let builder = TuiUiBuilder::from_app(ctx);
        let mut presenter = TuiPresenter::new();
        let frame = presenter.present_element(
            render_inline_menu(&snapshot, &builder),
            TuiRect::new(0, 0, width, height),
            ctx,
        );
        frame.buffer.to_lines()
    })
}

/// Installs a settings-backed custom endpoint definitions collection built
/// from a `cloud_platform.custom_endpoints`-shaped JSON object.
fn set_custom_endpoint_definitions(app: &mut App, object: serde_json::Value) {
    let object = object
        .as_object()
        .expect("test definitions must be a JSON object")
        .clone();
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    app.update(|ctx| {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .custom_endpoints
                .set_value(config, ctx)
                .expect("custom endpoint definitions should persist");
        });
    });
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
        let menu = ctx.add_model(|ctx| TuiApiKeysMenuModel::new(input.clone(), mode.clone(), ctx));
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
fn browsing_rows_are_alphabetical_with_fallback_last() {
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
                    "Custom endpoints",
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
            let menu = ctx.add_model(|ctx| TuiApiKeysMenuModel::new(input, mode, ctx));
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
            let menu = ctx.add_model(|ctx| TuiApiKeysMenuModel::new(input, mode, ctx));
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
            // Index 4 is the non-selectable "Custom endpoints" status row;
            // the Warp credit fallback row now sits at index 5.
            assert!(menu.select_at_snapshot_index(5, ctx));
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

#[test]
fn renders_expected_screen_text_across_the_custom_endpoint_lifecycle() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);
        set_custom_endpoint_definitions(
            &mut app,
            serde_json::json!({
                "Acme Gateway": {
                    "url": "https://llm.acme.example/v1",
                    "models": [
                        {"name": "gpt-4o", "alias": "Acme GPT-4o"},
                        {"name": "o3-mini"},
                    ],
                },
                "Broken Gateway": {
                    "url": "not-a-url",
                    "models": [{"name": "m"}],
                },
            }),
        );

        let before = render_menu_lines(&app, &menu, 90, 12);
        println!("--- /api-keys before a key is set ---");
        for line in &before {
            println!("{line}");
        }
        let before_text = before.join("\n");
        assert!(
            before_text.contains("Acme Gateway") && before_text.contains("custom endpoint"),
            "expected the valid endpoint row with its custom-endpoint annotation"
        );
        assert!(
            !before_text.contains("Connected"),
            "an unkeyed endpoint must not show (Connected)"
        );
        assert!(
            before_text.contains("Invalid custom endpoint: Broken Gateway")
                && before_text.contains("Skipped"),
            "expected the invalid endpoint's (Skipped) row"
        );

        ApiKeyManager::handle(&app)
            .update(&mut app, |manager, ctx| {
                manager.persist_custom_endpoint_key("Acme Gateway", Some("sk-acme".to_owned()), ctx)
            })
            .expect("key should persist");

        let after = render_menu_lines(&app, &menu, 90, 12);
        println!("--- /api-keys after setting the key ---");
        for line in &after {
            println!("{line}");
        }
        let after_text = after.join("\n");
        assert!(
            after_text.contains("Acme Gateway") && after_text.contains("Connected"),
            "expected the endpoint to show (Connected) once a key is set"
        );
    });
}

#[test]
fn valid_custom_endpoint_row_shows_annotation_without_key_connected_suffix() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);
        set_custom_endpoint_definitions(
            &mut app,
            serde_json::json!({
                "Acme Gateway": {
                    "url": "https://llm.acme.example/v1",
                    "models": [{"name": "gpt-4o"}],
                },
            }),
        );
        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            let row = snapshot
                .rows
                .iter()
                .find(|row| row.title == "Acme Gateway")
                .expect("valid custom endpoint row should be present");
            assert_eq!(row.description.as_deref(), Some("custom endpoint"));
            // No key yet: no `(Connected)` suffix.
            assert_eq!(row.state_suffix, None);
            assert!(row.is_selectable);
        });
    });
}

#[test]
fn invalid_custom_endpoint_renders_as_skipped_and_non_selectable() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);
        set_custom_endpoint_definitions(
            &mut app,
            serde_json::json!({
                "Broken": {
                    "url": "not-a-url",
                    "models": [{"name": "m"}],
                },
            }),
        );
        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            let row = snapshot
                .rows
                .iter()
                .find(|row| row.title == "Invalid custom endpoint: Broken")
                .expect("invalid custom endpoint row should be present");
            assert_eq!(row.state_suffix.as_deref(), Some("(Skipped)"));
            assert!(!row.is_selectable);
            // The settings error is a standard invalid-values hint surfaced
            // elsewhere; the row itself must never be selectable.
        });
    });
}

#[test]
fn selecting_and_saving_a_custom_endpoint_key_connects_it() {
    App::test((), |mut app| async move {
        let (input, _, menu) = add_menu(&mut app);
        set_custom_endpoint_definitions(
            &mut app,
            serde_json::json!({
                "Acme Gateway": {
                    "url": "https://llm.acme.example/v1",
                    "models": [{"name": "gpt-4o"}],
                },
            }),
        );
        let row_index = app.read(|ctx| {
            menu.as_ref(ctx)
                .snapshot(ctx)
                .unwrap()
                .rows
                .iter()
                .position(|row| row.title == "Acme Gateway")
                .expect("row should be present")
        });
        menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(row_index, ctx));
            menu.accept_selected(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuMasked
            );
            // No existing key: the masked editor starts empty.
            assert_eq!(input_text(&input, ctx), "");
        });
        input.update(&mut app, |input, ctx| {
            input.user_insert("sk-acme", ctx);
        });
        menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        app.read(|ctx| {
            assert!(ApiKeyManager::as_ref(ctx).custom_endpoint_key_is_connected("Acme Gateway"));
            assert_eq!(
                menu.as_ref(ctx).input_ownership(ctx),
                TuiInlineMenuInputOwnership::InlineMenuPlainText
            );
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            let row = snapshot
                .rows
                .iter()
                .find(|row| row.title == "Acme Gateway")
                .unwrap();
            assert_eq!(row.state_suffix.as_deref(), Some("(Connected)"));
        });

        // Clearing removes the key without removing the definition.
        menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(row_index, ctx));
            menu.clear_selected(ctx);
        });
        app.read(|ctx| {
            assert!(!ApiKeyManager::as_ref(ctx).custom_endpoint_key_is_connected("Acme Gateway"));
            let snapshot = menu.as_ref(ctx).snapshot(ctx).unwrap();
            assert!(
                snapshot.rows.iter().any(|row| row.title == "Acme Gateway"),
                "clearing the key must not remove the endpoint definition's row"
            );
        });
    });
}
