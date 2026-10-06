//! Click-handler regression tests for [`ConversationUsageView`].
//!
//! The original bug was that clicks on the "View details" / "Show N more"
//! affordances did nothing because the view was created via `add_view`
//! instead of `add_typed_action_view`, so the framework had no handler
//! registered for `ConversationUsageViewAction::*` and silently logged
//! `Dispatched action has no handlers: ToggleDetailsExpanded`.
//!
//! The fix lives at the view-creation site in `terminal/view.rs`. These
//! tests are a defense-in-depth layer that exercises the view's
//! `handle_action` implementation directly, so:
//!
//! * If the `TypedActionView` impl is removed or broken, the test won't
//!   compile (compile-time guard).
//! * If the handler logic for toggling `details_expanded` / resetting
//!   `show_all_clicked` regresses, the assertions below will fail
//!   (runtime guard).
//!
//! The tests use the same `view.update(&mut app, |view, ctx|
//! view.handle_action(...))` pattern as the existing
//! `number_shortcut_buttons_tests.rs` so they stay decoupled from the
//! framework's render path (which needs `Appearance` / theme singletons
//! that aren't relevant to the handler's correctness).

use std::collections::HashMap;

use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::persistence::model::{ModelTokenUsage, PRIMARY_AGENT_CATEGORY};
use crate::server::server_api::ServerApiProvider;
use crate::test_util::request_usage::{add_request_usage_model, set_billed_in_dollars};
use crate::test_util::settings::initialize_settings_for_tests;

fn placeholder_usage_info() -> ConversationUsageInfo {
    ConversationUsageInfo {
        credits_spent: 0.0,
        platform_credits_spent: 0.0,
        credits_spent_for_last_block: None,
        tool_calls: 0,
        models: Vec::new(),
        context_window_usage: 0.0,
        context_window_segments: Vec::new(),
        files_changed: 0,
        lines_added: 0,
        lines_removed: 0,
        commands_executed: 0,
        total_tokens: None,
        total_cost_in_cents: None,
        tokens_for_last_block: None,
        cost_in_cents_for_last_block: None,
    }
}

/// Registers the singletons that the view touches when constructed and
/// when `ctx.notify()` runs (theme lookups, settings, etc.). Keep this
/// minimal: the goal is to satisfy the runtime, not to mirror the full
/// production app.
fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
}

fn build_view(_ctx: &mut warpui::ViewContext<ConversationUsageView>) -> ConversationUsageView {
    ConversationUsageView::new(
        placeholder_usage_info(),
        DisplayMode::Footer,
        None,
        MouseStateHandle::default(),
    )
}

#[test]
fn toggle_details_expanded_flips_state_and_resets_show_all_on_collapse() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        // `add_window` registers the root view via `add_typed_action_view`
        // internally, so simply standing up the window proves
        // `ConversationUsageView: TypedActionView` is wired correctly.
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        view.read(&app, |view, _| {
            assert!(
                !view.details_expanded,
                "view starts collapsed before any action is dispatched"
            );
            assert!(
                !view.show_all_clicked,
                "show_all_clicked starts false before any action is dispatched"
            );
        });

        // Expand the breakdown.
        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ToggleDetailsExpanded, ctx);
        });
        view.read(&app, |view, _| {
            assert!(
                view.details_expanded,
                "ToggleDetailsExpanded should expand the breakdown"
            );
        });

        // Reveal-more should set the flag while keeping the view expanded.
        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ShowAllAgentRows, ctx);
        });
        view.read(&app, |view, _| {
            assert!(view.details_expanded, "still expanded after Show N more");
            assert!(
                view.show_all_clicked,
                "Show N more should set show_all_clicked"
            );
        });

        // Toggling collapse should both flip the expanded flag and reset
        // the show-all state so the next expand lands on the truncated
        // list.
        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ToggleDetailsExpanded, ctx);
        });
        view.read(&app, |view, _| {
            assert!(
                !view.details_expanded,
                "collapsing should toggle details_expanded back off"
            );
            assert!(
                !view.show_all_clicked,
                "collapsing should reset show_all_clicked"
            );
        });
    });
}

#[test]
fn custom_endpoint_models_use_the_external_key_icon_bucket() {
    let view = ConversationUsageView::new(
        ConversationUsageInfo {
            models: vec![ModelTokenUsage {
                model_id: "Friendly alias".to_string(),
                custom_endpoint_tokens: 6,
                custom_endpoint_token_usage_by_category: HashMap::from([(
                    PRIMARY_AGENT_CATEGORY.to_string(),
                    6,
                )]),
                ..Default::default()
            }],
            ..placeholder_usage_info()
        },
        DisplayMode::Footer,
        None,
        MouseStateHandle::default(),
    );

    assert_eq!(
        view.collect_models_by_category()
            .get(PRIMARY_AGENT_CATEGORY),
        Some(&vec![("Friendly alias".to_string(), true)])
    );
}

/// A subject billed in dollars sees each usage figure in dollars where a cents figure exists
/// and in credits where none does, independent of the dogfood flag.
#[test]
fn usage_summary_follows_the_server_billing_unit_per_figure() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::PricingTransparency.override_enabled(false);
        initialize_test_app(&mut app);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        add_request_usage_model(&mut app);
        set_billed_in_dollars(&mut app, true);

        let view = ConversationUsageView::new(
            ConversationUsageInfo {
                credits_spent: 20.0,
                credits_spent_for_last_block: Some(5.0),
                total_cost_in_cents: Some(36.0),
                cost_in_cents_for_last_block: None,
                ..placeholder_usage_info()
            },
            DisplayMode::Footer,
            None,
            MouseStateHandle::default(),
        );

        app.read(|ctx| {
            let text = view
                .render_unified_layout(ctx)
                .debug_text_content()
                .unwrap_or_default();
            assert!(text.contains("Usage charged (total)"), "got {text:?}");
            assert!(text.contains("$0.36"), "got {text:?}");
            assert!(
                text.contains("Credits spent (last response)"),
                "got {text:?}"
            );
            assert!(text.contains("5 credits"), "got {text:?}");
        });
    });
}

#[test]
fn show_all_agent_rows_is_independent_of_details_expanded() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        // `ShowAllAgentRows` on its own should flip `show_all_clicked`
        // even when the user hasn't expanded the breakdown yet (the
        // render path won't show rows until expanded, but the handler
        // itself shouldn't care about ordering).
        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ShowAllAgentRows, ctx);
        });
        view.read(&app, |view, _| {
            assert!(
                view.show_all_clicked,
                "ShowAllAgentRows should flip show_all_clicked regardless of expanded state"
            );
            assert!(
                !view.details_expanded,
                "ShowAllAgentRows must not implicitly expand details"
            );
        });
    });
}
