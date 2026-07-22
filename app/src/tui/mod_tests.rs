use std::cell::Cell;
use std::rc::Rc;

use warpui::{App, SingletonEntity};

use super::{
    TuiLoginEvent, TuiLoginModel, TuiLoginPhase, handle_received_device_authorization_code,
    set_login_phase,
};

#[test]
fn emits_logged_in_event_when_login_completes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::AwaitingLogin {
                verification_uri: None,
                user_code: None,
            },
        });

        let logged_in_events = Rc::new(Cell::new(0));
        let logged_in_events_for_subscription = logged_in_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &TuiLoginModel::handle(ctx),
                move |_, event, _| match event {
                    TuiLoginEvent::LoggedIn => {
                        logged_in_events_for_subscription
                            .set(logged_in_events_for_subscription.get() + 1);
                    }
                    TuiLoginEvent::LoggedOut => {}
                },
            );
        });
        app.update(|ctx| {
            set_login_phase(
                ctx,
                TuiLoginPhase::AwaitingLogin {
                    verification_uri: Some("https://example.com".to_owned()),
                    user_code: Some("CODE".to_owned()),
                },
            );
        });
        assert_eq!(logged_in_events.get(), 0);

        app.update(|ctx| set_login_phase(ctx, TuiLoginPhase::LoggedIn));
        assert_eq!(logged_in_events.get(), 1);
    });
}

#[test]
fn emits_logged_out_event_when_login_phase_returns_to_auth() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::LoggedIn,
        });

        let logged_out_events = Rc::new(Cell::new(0));
        let logged_out_events_for_subscription = logged_out_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &TuiLoginModel::handle(ctx),
                move |_, event, _| match event {
                    TuiLoginEvent::LoggedOut => {
                        logged_out_events_for_subscription
                            .set(logged_out_events_for_subscription.get() + 1);
                    }
                    TuiLoginEvent::LoggedIn => {}
                },
            );
        });
        app.update(|ctx| {
            set_login_phase(
                ctx,
                TuiLoginPhase::AwaitingLogin {
                    verification_uri: None,
                    user_code: None,
                },
            );
        });

        assert_eq!(logged_out_events.get(), 1);
    });
}

#[test]
fn ignores_late_device_code_when_login_is_already_complete() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::LoggedIn,
        });

        app.update(|ctx| {
            handle_received_device_authorization_code(
                "https://example.com",
                Some("https://example.com?code=CODE"),
                "CODE",
                ctx,
            );
        });

        app.update(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::LoggedIn
            ));
        });
    });
}
