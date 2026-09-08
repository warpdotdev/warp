use std::rc::Rc;

use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    BlocklistAIInputModel, ConversationSelectionEvent, InputConfig, InputModePolicy, InputType,
    PolicyConfigUpdate, UserWorkspaces, VoiceInput,
};
use warpui_core::event::KeyState;
use warpui_core::platform::keyboard::KeyCode;
use warpui_core::{App, AppContext, ModelHandle};

use super::{TuiVoiceInputModel, TuiVoiceInputState, VoiceInputStartSource};

struct TestInputModePolicy;

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        InputConfig {
            input_type: InputType::AI,
            is_locked: true,
        }
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        false
    }

    fn config_on_conversation_selection_changed(
        &self,
        _event: &ConversationSelectionEvent,
        _current: InputConfig,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }

    fn config_on_ai_settings_changed(
        &self,
        _event: &AISettingsChangedEvent,
        _current: InputConfig,
        _is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
}

fn add_voice_model(app: &mut App) -> ModelHandle<TuiVoiceInputModel> {
    let input_mode =
        app.update(|ctx| BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx));
    app.add_model(|ctx| {
        TuiVoiceInputModel::new(
            input_mode,
            UserWorkspaces::teamless_context_resolver_for_test(),
            ctx,
        )
    })
}

#[test]
fn start_does_not_replace_an_active_session() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = add_voice_model(&mut app);
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
        let model = add_voice_model(&mut app);
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
        let model = add_voice_model(&mut app);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Transcribing, ctx);
            voice.cancel(ctx);
            assert_eq!(voice.state(), TuiVoiceInputState::Idle);
        });
    });
}

#[test]
fn hold_release_stops_only_a_recording_the_hold_started() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = add_voice_model(&mut app);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            voice.set_hold_key_for_test(Some(KeyCode::ControlLeft));
            voice.handle_hold_key(KeyCode::ControlLeft, KeyState::Released, false, ctx);
            assert_eq!(voice.state(), TuiVoiceInputState::Transcribing);
            assert_eq!(voice.hold_key(), None);

            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            voice.handle_hold_key(KeyCode::ControlLeft, KeyState::Released, false, ctx);
            assert_eq!(
                voice.state(),
                TuiVoiceInputState::Listening,
                "a release without a successful hold-key press must not stop voice"
            );
            voice.handle_hold_key(KeyCode::ControlLeft, KeyState::Pressed, false, ctx);
            assert_eq!(
                voice.hold_key(),
                None,
                "pressing the hold key while voice is already active must not arm its release"
            );
        });
    });
}

#[test]
fn stop_hold_only_ends_a_held_recording() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = add_voice_model(&mut app);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            voice.stop_hold(ctx);
            assert_eq!(
                voice.state(),
                TuiVoiceInputState::Listening,
                "a recording started by another entry point must keep running"
            );

            voice.set_hold_key_for_test(Some(KeyCode::ControlLeft));
            voice.stop_hold(ctx);
            assert_eq!(voice.state(), TuiVoiceInputState::Transcribing);
            assert_eq!(voice.hold_key(), None);
        });
    });
}

#[test]
fn the_hold_key_clears_when_its_recording_ends_by_another_path() {
    App::test((), |mut app| async move {
        app.add_singleton_model(VoiceInput::new);
        let model = add_voice_model(&mut app);
        model.update(&mut app, |voice, ctx| {
            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            voice.set_hold_key_for_test(Some(KeyCode::ControlLeft));
            voice.cancel(ctx);
            assert_eq!(voice.hold_key(), None);

            voice.set_state_for_test(TuiVoiceInputState::Listening, ctx);
            voice.handle_hold_key(KeyCode::ControlLeft, KeyState::Released, false, ctx);
            assert_eq!(
                voice.state(),
                TuiVoiceInputState::Listening,
                "releasing a cancelled hold must not stop a later ctrl-s recording"
            );
        });
    });
}
