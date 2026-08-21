use warpui_core::App;

use super::{
    InputSampleFormat, StartListeningError, VoiceInput, VoiceInputLifecycle,
    VoiceInputLifecycleState, VoiceInputState, VoiceInputToggledFrom, normalize_audio_frame,
    send_audio_frame,
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

#[test]
fn mono_f32_samples_preserve_values_and_frame_count() {
    let samples = [-1.0, -0.25, 0.0, 0.5, 1.0];

    let normalized = normalize_audio_frame(&samples, 1).unwrap();

    assert_eq!(normalized, samples);
}

#[test]
fn signed_integer_samples_convert_to_normalized_f32() {
    let samples = [i16::MIN, 0, i16::MAX];

    let normalized = normalize_audio_frame(&samples, 1).unwrap();

    assert_eq!(normalized, [-1.0, 0.0, 0.9999695]);
}

#[test]
fn unsigned_integer_samples_convert_to_normalized_f32() {
    let samples = [u16::MIN, 32768, u16::MAX];

    let normalized = normalize_audio_frame(&samples, 1).unwrap();

    assert_eq!(normalized, [-1.0, 0.0, 0.9999695]);
}

#[test]
fn stereo_samples_are_averaged_in_frame_order() {
    let samples = [1.0, 0.5, -1.0, 0.0, 0.25, 0.75];

    let normalized = normalize_audio_frame(&samples, 2).unwrap();

    assert_eq!(normalized, [0.75, -0.5, 0.5]);
}

#[test]
fn empty_input_produces_an_empty_frame() {
    let normalized = normalize_audio_frame::<f32>(&[], 1).unwrap();

    assert!(normalized.is_empty());
}

#[test]
fn zero_channels_returns_a_recoverable_error() {
    let error = normalize_audio_frame(&[0.5], 0).unwrap_err();

    assert_eq!(error.to_string(), "Input stream reported zero channels");
}

#[test]
fn signed_integer_frames_are_forwarded_as_normalized_mono_samples() {
    let (audio_frame_tx, audio_frame_rx) = async_channel::unbounded();

    send_audio_frame(&[i16::MIN, i16::MAX], 2, &audio_frame_tx);

    assert_eq!(audio_frame_rx.try_recv().unwrap(), [-0.000015258789]);
}

#[test]
fn non_f32_sample_format_selects_its_typed_input_format() {
    let sample_format = InputSampleFormat::try_from(cpal::SampleFormat::I16).unwrap();

    assert_eq!(sample_format, InputSampleFormat::I16);
}
