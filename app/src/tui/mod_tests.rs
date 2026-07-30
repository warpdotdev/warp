use std::cell::Cell;
use std::rc::Rc;

use warpui::{App, SingletonEntity};

use super::{
    TuiLoginEvent, TuiLoginModel, TuiLoginPhase, handle_auth_manager_event, set_logged_out_phase,
    set_login_phase, tui_verification_url,
};
use crate::auth::auth_manager::AuthManagerEvent;
use crate::server::server_api::auth::UserAuthenticationError;

#[test]
fn tags_tui_verification_url_without_losing_existing_query_parameters() {
    let url = tui_verification_url(
        "https://app.warp.dev/device?user_code=ABCD-EFGH&existing=value#fragment",
    );
    let url = url::Url::parse(&url).unwrap();

    assert_eq!(url.fragment(), Some("fragment"));
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        vec![
            ("user_code".into(), "ABCD-EFGH".into()),
            ("existing".into(), "value".into()),
            ("source".into(), "warp-agent-cli".into()),
        ]
    );
}

#[test]
fn leaves_invalid_verification_url_unchanged() {
    assert_eq!(tui_verification_url("not a URL"), "not a URL".to_owned());
}

#[test]
fn stores_device_fallback_before_opening_browser() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::AwaitingLogin {
                verification_uri: None,
                user_code: None,
            },
        });

        let browser_opened = Rc::new(Cell::new(false));
        let browser_opened_for_callback = browser_opened.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, ctx| {
                assert!(matches!(
                    TuiLoginModel::as_ref(ctx).phase(),
                    TuiLoginPhase::AwaitingLogin {
                        verification_uri: Some(verification_uri),
                        user_code: Some(user_code),
                    } if verification_uri == url && user_code == "ABCD-EFGH"
                ));
                browser_opened_for_callback.set(true);
                url.to_owned()
            });
            handle_auth_manager_event(
                &AuthManagerEvent::ReceivedDeviceAuthorizationCode {
                    verification_url: "https://app.warp.dev/device".to_owned(),
                    verification_url_complete: Some(
                        "https://app.warp.dev/device?user_code=ABCD-EFGH".to_owned(),
                    ),
                    user_code: "ABCD-EFGH".to_owned(),
                },
                ctx,
            );
        });

        assert!(browser_opened.get());
    });
}

#[test]
fn renders_device_code_request_timeout_without_id_token_prefix() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::AwaitingLogin {
                verification_uri: None,
                user_code: None,
            },
        });

        app.update(|ctx| {
            handle_auth_manager_event(
                &AuthManagerEvent::AuthFailed(UserAuthenticationError::DeviceCodeRequestTimedOut {
                    attempts: 2,
                }),
                ctx,
            );
        });

        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::Failed { message }
                    if message == "Timed out requesting a sign-in link after 2 attempts"
                        && !message.contains("ID token")
            ));
        });
    });
}
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
        let phase_changed_events = Rc::new(Cell::new(0));
        let phase_changed_events_for_subscription = phase_changed_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &TuiLoginModel::handle(ctx),
                move |_, event, _| match event {
                    TuiLoginEvent::PhaseChanged => {
                        phase_changed_events_for_subscription
                            .set(phase_changed_events_for_subscription.get() + 1);
                    }
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
        assert_eq!(phase_changed_events.get(), 1);

        app.update(|ctx| set_login_phase(ctx, TuiLoginPhase::LoggedIn));
        assert_eq!(logged_in_events.get(), 1);
        assert_eq!(phase_changed_events.get(), 2);
    });
}

#[test]
fn emits_logged_out_event_and_resets_login_details() {
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
                    TuiLoginEvent::PhaseChanged => {}
                    TuiLoginEvent::LoggedIn => {}
                    TuiLoginEvent::LoggedOut => {
                        logged_out_events_for_subscription
                            .set(logged_out_events_for_subscription.get() + 1);
                    }
                },
            );
        });

        app.update(set_logged_out_phase);

        assert_eq!(logged_out_events.get(), 1);
        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin {
                    verification_uri: None,
                    user_code: None,
                }
            ));
        });
    });
}
