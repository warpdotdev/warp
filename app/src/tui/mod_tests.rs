use std::cell::Cell;
use std::rc::Rc;

use warp_core::channel::ChannelState;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui::{App, SingletonEntity};

use super::telemetry::TuiOnboardingTelemetry;
use super::{
    TuiAuthBrowserFlow, TuiLoginEvent, TuiLoginModel, TuiLoginPhase, handle_auth_manager_event,
    has_validated_identity, initial_login_phase, set_logged_out_phase, set_login_phase,
    start_tui_device_login, tui_verification_url,
};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::auth_state::AuthState;
use crate::auth::credentials::Credentials;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::auth::UserAuthenticationError;
fn login_model(phase: TuiLoginPhase) -> TuiLoginModel {
    let logged_in = matches!(phase, TuiLoginPhase::LoggedIn);
    TuiLoginModel {
        phase,
        browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
        telemetry: TuiOnboardingTelemetry::new(logged_in),
    }
}

#[test]
fn credential_only_auth_stays_on_signed_out_welcome() {
    let auth_state = AuthState::new_logged_out_for_test();
    auth_state.set_credentials(Some(Credentials::ApiKey {
        key: "wk-api-test".to_owned(),
        owner_type: None,
    }));

    assert!(!has_validated_identity(&auth_state));
    assert!(matches!(
        initial_login_phase(&auth_state),
        TuiLoginPhase::SignedOutWelcome
    ));
}

#[test]
fn credentials_with_user_identity_start_logged_in() {
    let auth_state = AuthState::new_for_test();

    assert!(has_validated_identity(&auth_state));
    assert!(matches!(
        initial_login_phase(&auth_state),
        TuiLoginPhase::LoggedIn
    ));
}

#[test]
fn missing_credentials_and_identity_start_signed_out() {
    let auth_state = AuthState::new_logged_out_for_test();

    assert!(!has_validated_identity(&auth_state));
    assert!(matches!(
        initial_login_phase(&auth_state),
        TuiLoginPhase::SignedOutWelcome
    ));
}

#[test]
fn tags_tui_verification_url_without_losing_existing_query_parameters() {
    let url = tui_verification_url(
        "https://app.warp.dev/device?user_code=ABCD-EFGH&existing=value#fragment",
        "ABCD-EFGH",
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
    assert_eq!(
        tui_verification_url("not a URL", "ABCD-EFGH"),
        "not a URL".to_owned()
    );
}

#[test]
fn adds_user_code_when_complete_verification_url_is_unavailable() {
    let url = tui_verification_url("https://app.warp.dev/device", "ABCD-EFGH");
    let url = url::Url::parse(&url).unwrap();

    assert_eq!(
        url.query_pairs()
            .find(|(key, _)| key == "user_code")
            .map(|(_, value)| value.into_owned()),
        Some("ABCD-EFGH".to_owned())
    );

    assert_eq!(
        url.query_pairs()
            .filter(|(key, _)| key == "return_to")
            .count(),
        0
    );
}

#[test]
fn explicit_start_device_login_preserves_pending_logout_on_retry() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| TuiLoginModel::signed_out_for_test());
        app.update(MockTelemetryContextProvider::register);

        let phase_changed_events = Rc::new(Cell::new(0));
        let phase_changed_events_for_subscription = phase_changed_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&TuiLoginModel::handle(ctx), move |_, event, _| {
                if matches!(event, TuiLoginEvent::PhaseChanged) {
                    phase_changed_events_for_subscription
                        .set(phase_changed_events_for_subscription.get() + 1);
                }
            });
        });

        app.update(start_tui_device_login);
        app.update(start_tui_device_login);

        assert_eq!(phase_changed_events.get(), 1);
        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin { browser_url: None }
            ));
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).browser_flow,
                TuiAuthBrowserFlow::DirectDeviceAuthorization
            ));
        });

        app.update(|ctx| {
            TuiLoginModel::handle(ctx).update(ctx, |model, _| {
                model.phase = TuiLoginPhase::Failed {
                    message: "Unable to open logout URL".to_owned(),
                };
                model.browser_flow = TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending;
            });
        });
        app.update(start_tui_device_login);
        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin { browser_url: None }
            ));
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).browser_flow,
                TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending
            ));
        });
    });
}
#[test]
fn stores_device_fallback_before_opening_browser() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            login_model(TuiLoginPhase::AwaitingLogin { browser_url: None })
        });

        let browser_opened = Rc::new(Cell::new(false));
        let browser_opened_for_callback = browser_opened.clone();
        app.update(|ctx| {
            ctx.set_before_open_url(move |url, ctx| {
                assert!(matches!(
                    TuiLoginModel::as_ref(ctx).phase(),
                    TuiLoginPhase::AwaitingLogin {
                        browser_url: Some(browser_url),
                    } if browser_url == url
                ));
                browser_opened_for_callback.set(true);
                url.to_owned()
            });
            let verification_url = format!(
                "{}/device",
                ChannelState::server_root_url().trim_end_matches('/')
            );
            handle_auth_manager_event(
                &AuthManagerEvent::ReceivedDeviceAuthorizationCode {
                    verification_url,
                    verification_url_complete: None,
                    user_code: "ABCD-EFGH".to_owned(),
                },
                ctx,
            );
        });

        assert!(browser_opened.get());
    });
}

#[test]
fn opens_only_the_current_retained_url() {
    App::test((), |mut app| async move {
        let browser_url =
            "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli".to_owned();
        app.add_singleton_model({
            let browser_url = browser_url.clone();
            move |_| login_model(TuiLoginPhase::BrowserOpenFailed { browser_url })
        });
        let browser_opened = Rc::new(Cell::new(false));
        let browser_opened_for_callback = browser_opened.clone();
        app.update(|ctx| {
            let expected_url = browser_url.clone();
            ctx.set_before_open_url(move |url, _| {
                assert_eq!(url, expected_url);
                browser_opened_for_callback.set(true);
                url.to_owned()
            });
            TuiLoginModel::open_login_url("https://example.com/wrong", ctx);
            TuiLoginModel::open_login_url(&browser_url, ctx);
        });

        assert!(browser_opened.get());
        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::AwaitingLogin {
                    browser_url: Some(current_url),
                } if current_url == &browser_url
            ));
        });
    });
}

#[test]
fn post_logout_device_auth_opens_logout_with_device_continuation() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::AwaitingLogin { browser_url: None },
            browser_flow: TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending,
            telemetry: TuiOnboardingTelemetry::new(true),
        });

        app.update(|ctx| {
            ctx.set_before_open_url(|url, ctx| {
                let logout_url = url::Url::parse(url).unwrap();
                assert_eq!(logout_url.path(), "/logout");
                let continuation = logout_url
                    .query_pairs()
                    .find(|(key, _)| key == "continue")
                    .map(|(_, value)| value.into_owned())
                    .unwrap();
                let continuation = url::Url::parse(&continuation).unwrap();
                assert_eq!(continuation.path(), "/device");
                assert_eq!(
                    continuation
                        .query_pairs()
                        .find(|(key, _)| key == "source")
                        .map(|(_, value)| value.into_owned()),
                    Some("warp-agent-cli".to_owned())
                );
                assert!(matches!(
                    TuiLoginModel::as_ref(ctx).browser_flow,
                    TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationOpened
                ));
                url.to_owned()
            });
            let verification_url = format!(
                "{}/device",
                ChannelState::server_root_url().trim_end_matches('/')
            );
            handle_auth_manager_event(
                &AuthManagerEvent::ReceivedDeviceAuthorizationCode {
                    verification_url,
                    verification_url_complete: None,
                    user_code: "ABCD-EFGH".to_owned(),
                },
                ctx,
            );
        });
    });
}
#[test]
fn renders_device_code_request_timeout_without_id_token_prefix() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            login_model(TuiLoginPhase::AwaitingLogin { browser_url: None })
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
fn credential_validation_failure_stays_on_auth_flow() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel::signed_out_for_test());

        app.update(|ctx| {
            handle_auth_manager_event(
                &AuthManagerEvent::AuthFailed(UserAuthenticationError::Unexpected(
                    anyhow::anyhow!("API key rejected"),
                )),
                ctx,
            );
        });

        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).phase(),
                TuiLoginPhase::Failed { message } if message.contains("API key rejected")
            ));
        });
    });
}
#[test]
fn post_logout_device_code_failure_still_opens_web_logout() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| TuiLoginModel {
            phase: TuiLoginPhase::AwaitingLogin { browser_url: None },
            browser_flow: TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending,
            telemetry: TuiOnboardingTelemetry::new(true),
        });
        let browser_opened = Rc::new(Cell::new(false));
        let browser_opened_for_callback = browser_opened.clone();

        app.update(|ctx| {
            ctx.set_before_open_url(move |url, _| {
                assert_eq!(url::Url::parse(url).unwrap().path(), "/logout");
                browser_opened_for_callback.set(true);
                url.to_owned()
            });
            handle_auth_manager_event(
                &AuthManagerEvent::AuthFailed(UserAuthenticationError::DeviceCodeRequestTimedOut {
                    attempts: 2,
                }),
                ctx,
            );
        });

        assert!(browser_opened.get());
        app.read(|ctx| {
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).browser_flow,
                TuiAuthBrowserFlow::DirectDeviceAuthorization
            ));
        });
    });
}
#[test]
fn emits_logged_in_event_when_login_completes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| {
            login_model(TuiLoginPhase::AwaitingLogin { browser_url: None })
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
                    browser_url: Some("https://example.com".to_owned()),
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
        app.add_singleton_model(|_| login_model(TuiLoginPhase::LoggedIn));
        app.update(MockTelemetryContextProvider::register);

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
                TuiLoginPhase::AwaitingLogin { browser_url: None }
            ));
            assert!(matches!(
                TuiLoginModel::as_ref(ctx).browser_flow,
                TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending
            ));
        });
    });
}
