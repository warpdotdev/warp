use warpui_core::App;

use super::{
    StartListeningError, VoiceInput, VoiceInputLifecycle, VoiceInputLifecycleState,
    VoiceInputState, VoiceInputToggledFrom,
};

#[test]
fn lifecycle_rejects_overlapping_sessions() {
    let mut lifecycle = VoiceInputLifecycle::default();
    assert!(lifecycle.start());

    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Listening);
    assert!(!lifecycle.start());
    assert!(lifecycle.begin_transcribing());
    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Transcribing);
    assert!(!lifecycle.start());
}

#[test]
fn lifecycle_rejects_invalid_transitions() {
    let mut lifecycle = VoiceInputLifecycle::default();
    assert!(!lifecycle.begin_transcribing());
    assert!(!lifecycle.complete());
    assert!(!lifecycle.fail());
    assert!(lifecycle.start());
    assert!(!lifecycle.complete());
    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Listening);
}

#[test]
fn lifecycle_cancellation_returns_to_idle() {
    let mut lifecycle = VoiceInputLifecycle::default();
    assert!(lifecycle.start());
    assert!(lifecycle.begin_transcribing());
    assert!(lifecycle.cancel());

    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Idle);
    assert!(!lifecycle.complete());
    assert!(!lifecycle.fail());
    assert!(!lifecycle.cancel());
}

#[test]
fn recorder_rejects_a_new_session_while_transcribing() {
    App::test((), |mut app| async move {
        let voice_input = app.add_model(VoiceInput::new);
        voice_input.update(&mut app, |voice_input, ctx| {
            voice_input.state = VoiceInputState::Transcribing;
            assert!(matches!(
                voice_input.start_listening(ctx, VoiceInputToggledFrom::Button),
                Err(StartListeningError::AlreadyRunning)
            ));
        });
    });
}
