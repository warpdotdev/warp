use std::cell::Cell;
use std::rc::Rc;

use warpui::{App, SingletonEntity};

use super::{TuiLoginEvent, TuiLoginModel, TuiLoginPhase, set_login_phase};

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
