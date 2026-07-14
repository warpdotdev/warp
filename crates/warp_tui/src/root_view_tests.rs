use warp::tui_export::register_tui_session_view_test_singletons;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, UpdateModel};
use warpui_core::{App, TuiView as _, WindowId};

use super::RootTuiView;
use crate::sessions::TuiSessions;
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session};

fn add_root(app: &mut App) -> (WindowId, warpui_core::ViewHandle<RootTuiView>) {
    app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| RootTuiView::new(),
        )
    })
}

#[test]
fn root_projects_only_the_focused_retained_session_view() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        add_test_semantic_selection(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        let (window_id, root) = add_root(&mut app);
        let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test(window_id));
        root.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sessions, |_, _, _, ctx| ctx.notify());
        });

        let (first, first_manager) = add_test_terminal_session(&mut app, window_id);
        let (second, second_manager) = add_test_terminal_session(&mut app, window_id);
        let first_view_id = first.id();
        let second_view_id = second.id();
        let first_id = app.update_model(&sessions, |sessions, ctx| {
            sessions.add_session(first, first_manager, true, ctx)
        });
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![first_view_id]);
        });
        let focused_window_view = app.read(|ctx| ctx.focused_view_id(window_id));

        let second_id = app.update_model(&sessions, |sessions, ctx| {
            sessions.add_session(second, second_manager, false, ctx)
        });
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![first_view_id]);
            assert_eq!(ctx.focused_view_id(window_id), focused_window_view);
        });

        app.update_model(&sessions, |sessions, ctx| {
            sessions.focus_session(second_id, ctx);
        });
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![second_view_id]);
        });
        app.update_model(&sessions, |sessions, ctx| {
            sessions.focus_session(first_id, ctx);
        });
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![first_view_id]);
        });
    });
}
