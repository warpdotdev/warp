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
    }
}

/// Registers the singletons that the view touches when constructed and
/// when `ctx.notify()` runs (theme lookups, etc.). Keep this minimal: the
/// goal is to satisfy the runtime, not to mirror the full production app.
fn initialize_test_app(app: &mut App) {
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
fn toggle_credits_details_expanded_flips_state_and_resets_show_all_on_collapse() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        // `add_window` registers the root view via `add_typed_action_view`
        // internally, so simply standing up the window proves
        // `ConversationUsageView: TypedActionView` is wired correctly.
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        view.read(&app, |view, _| {
            assert!(
                !view.credits_details_expanded,
                "view starts collapsed before any action is dispatched"
            );
            assert!(
                !view.credits_show_all_clicked,
                "credits_show_all_clicked starts false before any action is dispatched"
            );
        });

        // Expand the breakdown.
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &ConversationUsageViewAction::ToggleCreditsDetailsExpanded,
                ctx,
            );
        });
        view.read(&app, |view, _| {
            assert!(
                view.credits_details_expanded,
                "ToggleCreditsDetailsExpanded should expand the breakdown"
            );
        });

        // Reveal-more should set the flag while keeping the view expanded.
        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ShowAllCreditsAgentRows, ctx);
        });
        view.read(&app, |view, _| {
            assert!(
                view.credits_details_expanded,
                "still expanded after Show N more"
            );
            assert!(
                view.credits_show_all_clicked,
                "Show N more should set credits_show_all_clicked"
            );
        });

        // Toggling collapse should both flip the expanded flag and reset
        // the show-all state so the next expand lands on the truncated
        // list.
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &ConversationUsageViewAction::ToggleCreditsDetailsExpanded,
                ctx,
            );
        });
        view.read(&app, |view, _| {
            assert!(
                !view.credits_details_expanded,
                "collapsing should toggle credits_details_expanded back off"
            );
            assert!(
                !view.credits_show_all_clicked,
                "collapsing should reset credits_show_all_clicked"
            );
        });
    });
}

#[test]
fn toggle_diffs_details_expanded_is_independent_of_credits() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        // Expanding the diffs breakdown must not affect the credits
        // breakdown's state, and vice versa — the two disclosures are
        // independent per-row toggles.
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &ConversationUsageViewAction::ToggleDiffsDetailsExpanded,
                ctx,
            );
        });
        view.read(&app, |view, _| {
            assert!(
                view.diffs_details_expanded,
                "ToggleDiffsDetailsExpanded should expand the diffs breakdown"
            );
            assert!(
                !view.credits_details_expanded,
                "the credits breakdown must stay collapsed"
            );
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ShowAllDiffsAgentRows, ctx);
        });
        view.read(&app, |view, _| {
            assert!(view.diffs_show_all_clicked);
            assert!(!view.credits_show_all_clicked);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &ConversationUsageViewAction::ToggleDiffsDetailsExpanded,
                ctx,
            );
        });
        view.read(&app, |view, _| {
            assert!(
                !view.diffs_details_expanded,
                "collapsing should toggle diffs_details_expanded back off"
            );
            assert!(
                !view.diffs_show_all_clicked,
                "collapsing should reset diffs_show_all_clicked"
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

#[test]
fn show_all_credits_agent_rows_is_independent_of_details_expanded() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        // `ShowAllCreditsAgentRows` on its own should flip
        // `credits_show_all_clicked` even when the user hasn't expanded the
        // breakdown yet (the render path won't show rows until expanded,
        // but the handler itself shouldn't care about ordering).
        view.update(&mut app, |view, ctx| {
            view.handle_action(&ConversationUsageViewAction::ShowAllCreditsAgentRows, ctx);
        });
        view.read(&app, |view, _| {
            assert!(
                view.credits_show_all_clicked,
                "ShowAllCreditsAgentRows should flip credits_show_all_clicked regardless of expanded state"
            );
            assert!(
                !view.credits_details_expanded,
                "ShowAllCreditsAgentRows must not implicitly expand details"
            );
        });
    });
}
