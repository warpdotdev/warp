use std::cell::Cell;
use std::rc::Rc;

use warpui::{App, SingletonEntity};

use super::{
    TuiLoginEvent, TuiLoginModel, TuiLoginPhase, set_logged_out_phase, set_login_phase,
    tui_verification_url,
};

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
