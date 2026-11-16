use std::cell::RefCell;
use std::rc::Rc;

use warp::settings::{TuiStatuslineConfig, TuiStatuslineItem};
use warp::tui_export::Appearance;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App};
use warpui_core::TypedActionView as _;

use super::{TuiStatuslineConfigAction, TuiStatuslineConfigEvent, TuiStatuslineConfigView};

#[test]
fn default_picker_preserves_figma_selection_and_full_catalog_order() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let view = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| TuiStatuslineConfigView::new(TuiStatuslineConfig::default(), ctx),
            )
            .1
        });

        assert_eq!(
            app.read(|ctx| view.as_ref(ctx).current_config(ctx)),
            TuiStatuslineConfig::default()
        );
    });
}

#[test]
fn toggle_and_reorder_are_reflected_in_saved_config() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let view = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| TuiStatuslineConfigView::new(TuiStatuslineConfig::default(), ctx),
            )
            .1
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let events_for_subscription = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| {
                events_for_subscription.borrow_mut().push(event.clone());
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiStatuslineConfigAction::Toggle, ctx);
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiStatuslineConfigAction::MoveForward, ctx);
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiStatuslineConfigAction::Save, ctx);
        });

        let saved = events
            .borrow()
            .iter()
            .find_map(|event| match event {
                TuiStatuslineConfigEvent::Saved(config) => Some(config.clone()),
                TuiStatuslineConfigEvent::Cancelled | TuiStatuslineConfigEvent::LayoutChanged => {
                    None
                }
            })
            .expect("save emits a config");
        assert_eq!(
            saved.order,
            [
                // VimModeIndicator is a default-on statusline item, so it is expected
                // in the saved order at its position in TuiStatuslineItem::ALL.
                TuiStatuslineItem::VimModeIndicator,
                TuiStatuslineItem::AutoApprove,
                TuiStatuslineItem::Model,
                TuiStatuslineItem::Team,
                TuiStatuslineItem::WorkingDirectory,
                TuiStatuslineItem::GitBranch,
                TuiStatuslineItem::GitBranchStatus,
                TuiStatuslineItem::GitDiffStatus,
                TuiStatuslineItem::GitHubPullRequest,
                TuiStatuslineItem::CreditUsage,
                TuiStatuslineItem::ContextWindowUsage,
                TuiStatuslineItem::Date,
                TuiStatuslineItem::Time12Hour,
                TuiStatuslineItem::Time24Hour,
                TuiStatuslineItem::AgentTodoList,
                TuiStatuslineItem::VoiceInput,
            ]
        );
        assert_eq!(
            saved.enabled,
            [
                TuiStatuslineItem::VimModeIndicator,
                TuiStatuslineItem::Model,
                TuiStatuslineItem::WorkingDirectory,
                TuiStatuslineItem::GitBranch,
                TuiStatuslineItem::GitDiffStatus,
            ]
        );
    });
}
