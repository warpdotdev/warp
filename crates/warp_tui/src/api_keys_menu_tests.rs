use ai::LLMProvider;
use ai::api_keys::ApiKeyManager;
use warp::editor::CodeEditorModel;
use warp::settings::AISettings;
use warp::tui_export::{UserWorkspaces, register_tui_session_view_test_singletons};
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity as _;
use warpui_core::{App, ModelHandle};

use super::{TuiApiKeysFooter, TuiApiKeysMenuModel, TuiApiKeysMenuState, input_text};
use crate::inline_menu::{TuiInlineMenuHeader, TuiInlineMenuInputOwnership};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

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

/// The bind itself (see `OauthAttempt::start`) retries off the UI thread, but
/// the menu represents the attempt as `ConnectingGrokPending` synchronously
/// before that resolves. A Cancel (Esc/dismiss) arriving during that window
/// must invalidate the attempt so its eventual bind result is dropped instead
/// of reopening `ConnectingGrok` on a menu the user already backed out of.
///
/// Drives `handle_grok_oauth_bound` directly with the pending attempt's own
/// id, rather than waiting on the real background bind to actually complete:
/// there's no externally observable difference between "discarded" and
/// "hasn't arrived yet", so a fixed wait before asserting could pass without
/// the discard ever having been exercised.
#[test]
fn cancel_while_grok_bind_is_pending_discards_the_late_result() {
    App::test((), |mut app| async move {
        let (_, _, menu) = add_menu(&mut app);

        let attempt_id = menu.update(&mut app, |menu, ctx| {
            assert!(menu.select_at_snapshot_index(3, ctx));
            menu.accept_selected(ctx);
            let TuiApiKeysMenuState::ConnectingGrokPending { attempt_id } = &menu.state else {
                panic!("accepting the Grok row should enter ConnectingGrokPending");
            };
            let attempt_id = *attempt_id;
            menu.dismiss(ctx);
            attempt_id
        });

        app.read(|ctx| {
            assert!(matches!(
                menu.as_ref(ctx).footer(ctx),
                Some(TuiApiKeysFooter::ProviderList { .. })
            ));
        });

        // Simulate the bind that was already in flight finally resolving,
        // after the user backed out of it. A distinctive error message lets
        // the assertion below tell "discarded" apart from "handled as a
        // normal failure" -- without the id guard, this would still land in
        // `Browsing` (via the error branch's own `transition_to_browsing`),
        // just with this message surfaced instead of dropped.
        menu.update(&mut app, |menu, ctx| {
            menu.handle_grok_oauth_bound(attempt_id, Err(anyhow::anyhow!("late bind result")), ctx);
        });
        app.read(|ctx| {
            assert!(
                matches!(
                    menu.as_ref(ctx).footer(ctx),
                    Some(TuiApiKeysFooter::ProviderList { .. })
                ),
                "a late bind result must not reopen ConnectingGrok on a cancelled attempt"
            );
            assert_eq!(
                menu.as_ref(ctx)
                    .snapshot(ctx)
                    .and_then(|snapshot| snapshot.header),
                Some(TuiInlineMenuHeader {
                    title: Some("API keys".to_owned()),
                    tabs: Vec::new(),
                }),
                "the discarded result's error must not surface on a menu the user backed out of"
            );
            assert!(menu.as_ref(ctx).is_open(ctx));
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
            menu.select_at_snapshot_index(4, ctx);
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
