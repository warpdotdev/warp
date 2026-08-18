use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use warp::tui_export::register_tui_session_view_test_singletons;
use warp::{TuiLoginModel, TuiLoginPhase};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, SingletonEntity, UpdateModel};
use warpui_core::{App, TuiView as _, TypedActionView as _, WindowId};

use super::{RootTuiAction, RootTuiView};
use crate::cloud_run::TuiCloudRunState;
use crate::session_registry::{TuiSessions, TuiSessionsEvent};
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
fn start_device_login_action_retries_from_failure() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.add_singleton_model(|_| {
            TuiLoginModel::failed_for_test("Failed to generate device code")
        });
        let (_, root) = add_root(&mut app);

        root.update(&mut app, |root, ctx| {
            root.login_copy_hint.show_success(
                "Login URL copied to clipboard".to_owned(),
                ctx,
                |view| &mut view.login_copy_hint,
            );
            root.handle_action(&RootTuiAction::StartDeviceLogin, ctx);
        });

        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin { browser_url: None }
            ));
            assert!(!root.as_ref(ctx).copy_login_url_when_available);
            assert!(root.as_ref(ctx).login_copy_hint.current().is_none());
        });
    });
}

#[test]
fn terminal_root_focus_delegates_to_the_selected_session() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        add_test_semantic_selection(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        let (window_id, root) = add_root(&mut app);
        let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
        let (foreground, foreground_manager) = add_test_terminal_session(&mut app, window_id);
        let foreground_id = app.update(|ctx| {
            TuiSessions::register_session(
                &sessions,
                foreground.clone(),
                foreground_manager,
                true,
                ctx,
            )
        });
        let (background, background_manager) = add_test_terminal_session(&mut app, window_id);
        app.update(|ctx| {
            TuiSessions::register_session(
                &sessions,
                background.clone(),
                background_manager,
                false,
                ctx,
            );
        });
        root.update(&mut app, |root, ctx| root.show_terminal(ctx));

        background.update(&mut app, |background, ctx| background.activate(ctx));
        assert!(app.read(|ctx| { ctx.check_view_or_child_focused(window_id, &background.id()) }));
        assert_eq!(
            app.read(|ctx| TuiSessions::as_ref(ctx).focused_session_id()),
            Some(foreground_id)
        );

        root.update(&mut app, |_, ctx| ctx.focus_self());

        assert!(app.read(|ctx| { ctx.check_view_or_child_focused(window_id, &foreground.id()) }));
    });
}
#[test]
fn pending_copy_clears_on_failure_before_url_generation() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.add_singleton_model(|_| {
            TuiLoginModel::failed_for_test("Failed to generate device code")
        });
        let (_, root) = add_root(&mut app);

        root.update(&mut app, |root, ctx| {
            root.copy_login_url_when_available = true;
            root.handle_login_phase_changed(ctx, |_| -> Result<()> {
                panic!("failure has no URL to copy")
            });
        });
        app.read(|ctx| {
            assert!(!root.as_ref(ctx).copy_login_url_when_available);
        });
    });
}

#[test]
fn start_and_copy_action_waits_for_generated_url() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.add_singleton_model(|_| TuiLoginModel::signed_out_for_test());
        let (_, root) = add_root(&mut app);

        root.update(&mut app, |root, ctx| {
            root.handle_action(&RootTuiAction::StartDeviceLoginAndCopyUrl, ctx);
        });

        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin { browser_url: None }
            ));
            assert!(root.as_ref(ctx).copy_login_url_when_available);
        });
    });
}

#[test]
fn pending_copy_consumes_exact_generated_url_once() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        let browser_url =
            "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli".to_owned();
        app.add_singleton_model({
            let browser_url = browser_url.clone();
            move |_| TuiLoginModel::awaiting_login_for_test(Some(browser_url))
        });
        let (_, root) = add_root(&mut app);
        let copied = Rc::new(RefCell::new(None));
        let copied_for_action = copied.clone();

        root.update(&mut app, |root, ctx| {
            root.copy_login_url_when_available = true;
            root.handle_login_phase_changed(ctx, move |url| {
                copied_for_action.replace(Some(url.to_owned()));
                Ok(())
            });
        });

        assert_eq!(copied.borrow().as_deref(), Some(browser_url.as_str()));
        app.read(|ctx| {
            assert!(!root.as_ref(ctx).copy_login_url_when_available);
        });
        root.update(&mut app, |root, ctx| {
            root.handle_login_phase_changed(ctx, |_| -> Result<()> {
                panic!("generated URL must only be copied once")
            });
        });
    });
}

#[test]
fn pending_copy_waits_without_url() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.add_singleton_model(|_| TuiLoginModel::awaiting_login_for_test(None));
        let (_, root) = add_root(&mut app);

        root.update(&mut app, |root, ctx| {
            root.copy_login_url_when_available = true;
            root.handle_login_phase_changed(ctx, |_| -> Result<()> {
                panic!("copy must wait until an exact URL exists")
            });
        });
        app.read(|ctx| {
            assert!(root.as_ref(ctx).copy_login_url_when_available);
        });
    });
}

#[test]
fn start_device_login_action_transitions_from_welcome() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        app.add_singleton_model(|_| TuiLoginModel::signed_out_for_test());
        let (_, root) = add_root(&mut app);

        root.update(&mut app, |root, ctx| {
            root.handle_action(&RootTuiAction::StartDeviceLogin, ctx);
        });

        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin { browser_url: None }
            ));
        });
    });
}

#[test]
fn root_projects_only_the_focused_retained_session_view() {
    App::test((), |mut app| async move {
        register_tui_session_view_test_singletons(&mut app);
        add_test_semantic_selection(&mut app);
        app.update(crate::autoupdate::TuiAutoupdater::register);
        let (window_id, root) = add_root(&mut app);
        let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
        root.update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&sessions, |_, _, event, ctx| match event {
                TuiSessionsEvent::SessionRemoved(_) => ctx.notify(),
                TuiSessionsEvent::FocusChanged(_) => ctx.notify(),
            });
        });
        app.read(|ctx| {
            assert!(root.as_ref(ctx).child_view_ids(ctx).is_empty());
        });

        let (first, first_manager) = add_test_terminal_session(&mut app, window_id);
        let first_view_id = first.id();
        let first_id = app.update(|ctx| {
            TuiSessions::register_session(&sessions, first, first_manager, true, ctx)
        });
        app.read(|ctx| {
            assert!(root.as_ref(ctx).child_view_ids(ctx).is_empty());
        });
        root.update(&mut app, |root, ctx| root.show_terminal(ctx));
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![first_view_id]);
            assert!(ctx.check_view_or_child_focused(window_id, &first_view_id));
        });
        let focused_window_view = app.read(|ctx| ctx.focused_view_id(window_id));
        let (second, second_manager) = add_test_terminal_session(&mut app, window_id);
        let second_view_id = second.id();

        let second_id = app.update(|ctx| {
            TuiSessions::register_session(&sessions, second, second_manager, false, ctx)
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
            assert!(ctx.check_view_or_child_focused(window_id, &second_view_id));
            assert_ne!(ctx.focused_view_id(window_id), focused_window_view);
        });
        app.update_model(&sessions, |sessions, ctx| {
            sessions.focus_session(first_id, ctx);
        });
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![first_view_id]);
            assert!(ctx.check_view_or_child_focused(window_id, &first_view_id));
            assert_eq!(ctx.focused_view_id(window_id), focused_window_view);
        });

        let cloud_state = app.add_model(|_| TuiCloudRunState::new());
        let (cloud_id, cloud_view) = app.update(|ctx| {
            TuiSessions::create_cloud_run_session(&sessions, window_id, cloud_state, false, ctx)
        });
        app.update_model(&sessions, |sessions, ctx| {
            sessions.focus_session(cloud_id, ctx);
        });
        app.read(|ctx| {
            assert_eq!(root.as_ref(ctx).child_view_ids(ctx), vec![cloud_view.id()]);
            assert!(ctx.check_view_or_child_focused(window_id, &cloud_view.id()));
        });
        root.update(&mut app, |root, ctx| root.show_auth(ctx));
        app.read(|ctx| {
            assert!(root.as_ref(ctx).child_view_ids(ctx).is_empty());
            assert!(ctx.check_view_or_child_focused(window_id, &root.id()));
        });
    });
}
