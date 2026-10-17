use std::time::Duration;

use warpui::App;
use warpui::r#async::Timer;

use super::*;
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::{AnsiHandlerEvent, ModelEvent, ModelEventDispatcher};

#[test]
fn precmd_activates_line_editor_before_session_is_registered() {
    App::test((), |mut app| async move {
        let (_model_events_tx, model_events_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        // Precmd updates active_session_id before LineEditorStatus sees the event, but the
        // session object may not exist in Sessions yet (Bootstrapped lands later).
        model_events.update(&mut app, |dispatcher, _| {
            dispatcher.set_active_session_id(crate::terminal::model::session::SessionId::from(1));
        });
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events, sessions, ctx));

        line_editor_status.update(&mut app, |status, ctx| {
            status.handle_model_event(&ModelEvent::Handler(AnsiHandlerEvent::Precmd), ctx);
        });

        Timer::after(LINE_EDITOR_ACTIVATION_DELAY + Duration::from_millis(20)).await;

        line_editor_status.read(&app, |status, _| {
            assert!(
                status.is_line_editor_active(),
                "Precmd before session registration must activate the line editor for non-zsh shells"
            );
        });
    });
}

#[test]
fn precmd_activates_line_editor_when_active_session_id_is_unset() {
    App::test((), |mut app| async move {
        let (_model_events_tx, model_events_rx) = async_channel::unbounded();
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let model_events =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        let line_editor_status =
            app.add_model(|ctx| LineEditorStatus::new(model_events, sessions, ctx));

        line_editor_status.update(&mut app, |status, ctx| {
            status.handle_model_event(&ModelEvent::Handler(AnsiHandlerEvent::Precmd), ctx);
        });

        Timer::after(LINE_EDITOR_ACTIVATION_DELAY + Duration::from_millis(20)).await;

        line_editor_status.read(&app, |status, _| {
            assert!(status.is_line_editor_active());
        });
    });
}
