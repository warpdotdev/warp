use std::time::Duration;

use settings::Setting;
use warpui::{App, SingletonEntity};

use super::{UndoCloseSettings, UndoCloseStack};
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::pane_group::PaneGroup;
use crate::tab::TabData;
use crate::test_util::assert_eventually;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

#[test]
fn discard_closed_tab_skips_unavailable_pane_group() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let pane_group = workspace.read(&app, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        let terminal_view = pane_group.read(&app, |pane_group, ctx| {
            pane_group.active_session_view(ctx).unwrap()
        });
        let stack = UndoCloseStack::handle(&app);
        stack.update(&mut app, |stack, ctx| {
            stack.handle_tab_closed(
                workspace.downgrade(),
                0,
                TabData::new(pane_group.clone()),
                ctx,
            );
        });

        pane_group.update(&mut app, |_, ctx| {
            assert!(ctx.is_window_open(ctx.window_id()));
            assert!(
                ctx.view_with_id::<PaneGroup>(ctx.window_id(), pane_group.id())
                    .is_none()
            );

            stack.update(ctx, |stack, ctx| {
                let removed_item = stack.stack.pop().unwrap();
                removed_item.closed_item.discard(ctx);
                assert!(stack.is_empty());
            });
        });

        app.read(|ctx| {
            assert!(
                ctx.view_with_id::<PaneGroup>(pane_group.window_id(ctx), pane_group.id())
                    .is_some()
            );
            assert!(
                ActiveAgentViewsModel::as_ref(ctx)
                    .is_terminal_view_attached(terminal_view.id(), ctx)
            );
        });
    });
}

#[test]
fn expired_closed_tab_cleans_up_live_panes() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        UndoCloseSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .grace_period
                .set_value(Duration::ZERO, ctx)
                .unwrap();
        });
        let workspace = mock_workspace(&mut app);
        let pane_group = workspace.read(&app, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        let terminal_view = pane_group.read(&app, |pane_group, ctx| {
            pane_group.active_session_view(ctx).unwrap()
        });
        app.read(|ctx| {
            assert!(
                ActiveAgentViewsModel::as_ref(ctx)
                    .is_terminal_view_attached(terminal_view.id(), ctx)
            );
        });

        let stack = UndoCloseStack::handle(&app);
        stack.update(&mut app, |stack, ctx| {
            stack.handle_tab_closed(
                workspace.downgrade(),
                0,
                TabData::new(pane_group.clone()),
                ctx,
            );
        });

        assert_eventually!(
            stack.read(&app, |stack, _| stack.is_empty()),
            "closed tab should expire"
        );
        app.read(|ctx| {
            assert!(
                !ActiveAgentViewsModel::as_ref(ctx)
                    .is_terminal_view_attached(terminal_view.id(), ctx)
            );
        });
    });
}
