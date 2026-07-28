use warp::tui_export::VoiceInput;
use warpui_core::App;

use super::{TuiVoiceInputModel, TuiVoiceInputState, VoiceInputStartSource};

#[test]
fn start_does_not_replace_an_active_session() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = app.add_model(TuiVoiceInputModel::new);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            assert!(!voice.start(false, VoiceInputStartSource::Keybinding, ctx));
            assert_eq!(voice.state(), TuiVoiceInputState::Listening);
        });
    });
}

#[test]
fn stop_transitions_the_model_to_transcribing() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = app.add_model(TuiVoiceInputModel::new);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            voice.stop(ctx);
            assert_eq!(voice.state(), TuiVoiceInputState::Transcribing);
        });
    });
}

#[test]
fn cancel_returns_the_model_to_idle() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = app.add_model(TuiVoiceInputModel::new);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Transcribing, ctx);
            voice.cancel(ctx);
            assert_eq!(voice.state(), TuiVoiceInputState::Idle);
        });
    });
}
