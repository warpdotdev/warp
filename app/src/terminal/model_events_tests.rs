use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use warpui::App;

use super::{AnsiHandlerEvent, ModelEvent, ModelEventDispatcher};
use crate::terminal::event::Event;
use crate::terminal::model::session::Sessions;
use crate::terminal::model::terminal_model::{HandlerEvent, PrecmdDisposition};

#[test]
fn refreshed_precmd_updates_active_session_without_emitting_public_precmd_event() {
    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_events_tx, events_rx) = async_channel::unbounded();
        let dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));

        let precmd_event_count = Arc::new(AtomicUsize::new(0));
        let precmd_event_count_for_subscription = precmd_event_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&dispatcher, move |_, event, _| {
                if matches!(event, ModelEvent::Handler(AnsiHandlerEvent::Precmd)) {
                    precmd_event_count_for_subscription.fetch_add(1, Ordering::Relaxed);
                }
            });
        });

        let fresh_session_id = 123.into();
        dispatcher.update(&mut app, |dispatcher, ctx| {
            dispatcher.handle_terminal_model_event(
                Event::Handler(HandlerEvent::Precmd {
                    session_id: Some(fresh_session_id),
                    handled_after_inband: true,
                    env_vars: HashMap::new(),
                    disposition: PrecmdDisposition::AppliedToFreshBlock,
                }),
                ctx,
            );
        });
        assert_eq!(precmd_event_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            dispatcher.read(&app, |dispatcher, _| dispatcher.active_session_id()),
            Some(fresh_session_id)
        );

        let refreshed_session_id = 456.into();
        dispatcher.update(&mut app, |dispatcher, ctx| {
            dispatcher.handle_terminal_model_event(
                Event::Handler(HandlerEvent::Precmd {
                    session_id: Some(refreshed_session_id),
                    handled_after_inband: true,
                    env_vars: HashMap::new(),
                    disposition: PrecmdDisposition::RefreshedActivePrompt,
                }),
                ctx,
            );
        });
        assert_eq!(precmd_event_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            dispatcher.read(&app, |dispatcher, _| dispatcher.active_session_id()),
            Some(refreshed_session_id)
        );
    });
}
