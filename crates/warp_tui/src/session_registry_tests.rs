use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::register_tui_session_view_test_singletons;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ReadModel, SingletonEntity as _, UpdateModel};
use warpui_core::App;

use super::{TuiSessions, TuiSessionsEvent};
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session, TestHostView};

type CapturedEvents = Rc<RefCell<Vec<TuiSessionsEvent>>>;

fn capture_events(app: &mut App) -> CapturedEvents {
    let events: CapturedEvents = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    app.update(|ctx| {
        let sessions = TuiSessions::handle(ctx);
        ctx.subscribe_to_model(&sessions, move |_, event, _| {
            captured.borrow_mut().push(*event);
        });
    });
    events
}

#[test]
fn focus_drives_events() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        add_test_semantic_selection(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        let window_id = app.update(|ctx| {
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |_| TestHostView,
            )
            .0
        });
        let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
        let events = capture_events(&mut app);

        let (first, first_manager) = add_test_terminal_session(&mut app, window_id);
        let first_view_id = first.id();

        let first_id = app.update(|ctx| {
            TuiSessions::register_session(&sessions, first, first_manager, true, ctx)
        });
        assert_eq!(first_id.surface_id(), first_view_id);
        assert!(app.read(|ctx| { ctx.check_view_or_child_focused(window_id, &first_view_id) }));
        assert_eq!(
            std::mem::take(&mut *events.borrow_mut()),
            vec![TuiSessionsEvent::FocusChanged(first_id)],
        );
        let first_focused_view_id = app.read(|ctx| ctx.focused_view_id(window_id));
        let (second, second_manager) = add_test_terminal_session(&mut app, window_id);
        let second_view_id = second.id();
        assert_eq!(
            app.read(|ctx| ctx.focused_view_id(window_id)),
            first_focused_view_id,
        );

        let second_id = app.update(|ctx| {
            TuiSessions::register_session(&sessions, second, second_manager, false, ctx)
        });
        assert_eq!(second_id.surface_id(), second_view_id);
        assert!(std::mem::take(&mut *events.borrow_mut()).is_empty());
        assert_eq!(
            app.read_model(&sessions, |sessions, _| sessions.focused_session_id()),
            Some(first_id),
        );

        app.update_model(&sessions, |sessions, ctx| {
            assert!(sessions.focus_session(second_id, ctx));
            assert!(!sessions.focus_session(second_id, ctx));
        });
        assert!(app.read(|ctx| { ctx.check_view_or_child_focused(window_id, &second_view_id) }));
        assert_ne!(
            app.read(|ctx| ctx.focused_view_id(window_id)),
            first_focused_view_id,
        );
        assert_eq!(
            std::mem::take(&mut *events.borrow_mut()),
            vec![TuiSessionsEvent::FocusChanged(second_id)],
        );

        app.update_model(&sessions, |sessions, ctx| {
            assert!(sessions.focus_session(first_id, ctx));
        });
        assert!(app.read(|ctx| { ctx.check_view_or_child_focused(window_id, &first_view_id) }));
        assert_eq!(
            app.read(|ctx| ctx.focused_view_id(window_id)),
            first_focused_view_id,
        );
        assert_eq!(
            std::mem::take(&mut *events.borrow_mut()),
            vec![TuiSessionsEvent::FocusChanged(first_id)],
        );
    });
}
