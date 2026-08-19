use computer_use::Target;
use warp_multi_agent_api as api;

use crate::agent::action::AIAgentActionType;
use crate::agent::convert::ToolToAIAgentActionError;

// The deprecated legacy field must still be set to construct this struct
// literal (it is a plain, non-optional proto3 field); tests exercise the
// current `playback_speed_kind` oneof instead.
#[allow(deprecated)]
fn start_recording(
    target: Option<api::message::tool_call::ComputerUseTarget>,
) -> api::message::tool_call::StartRecording {
    api::message::tool_call::StartRecording {
        frame_rate: 15,
        limits: None,
        summary: String::new(),
        playback_speed_multiplier: 0,
        target,
        description: String::new(),
        playback_speed_kind: None,
    }
}

fn window_target(window_id: &str) -> api::message::tool_call::ComputerUseTarget {
    use api::message::tool_call::computer_use_target::{Target as ApiTarget, Window};
    api::message::tool_call::ComputerUseTarget {
        target: Some(ApiTarget::Window(Window {
            window_id: window_id.to_string(),
            pid: 7,
        })),
    }
}

#[test]
fn start_recording_parses_valid_window_target() {
    let action = AIAgentActionType::try_from(start_recording(Some(window_target("12345"))))
        .expect("valid window id should convert");
    match action {
        AIAgentActionType::StartRecording {
            window: Some(Target::Window { window_id, pid }),
            ..
        } => {
            assert_eq!(window_id, 12345);
            assert_eq!(pid, 7);
        }
        other => panic!("expected a window recording target, got {other:?}"),
    }
}

#[test]
fn start_recording_rejects_unparseable_window_id() {
    // A malformed window id must error before capture starts rather than silently
    // falling back to whole-screen recording.
    let err = AIAgentActionType::try_from(start_recording(Some(window_target("not-a-window"))))
        .expect_err("an unparseable window id should be rejected");
    assert!(
        matches!(&err, ToolToAIAgentActionError::InvalidRecordingWindowId(id) if id == "not-a-window"),
        "expected InvalidRecordingWindowId, got {err:?}"
    );
}

#[test]
fn start_recording_without_target_records_whole_screen() {
    let action =
        AIAgentActionType::try_from(start_recording(None)).expect("absent target should convert");
    assert!(matches!(
        action,
        AIAgentActionType::StartRecording { window: None, .. }
    ));
}

/// Sets the tool call's `playback_speed` to an explicit value via the
/// presence-bearing oneof (as a real server would), rather than assuming any
/// particular representation for "unset".
fn with_playback_speed(
    mut tool_call: api::message::tool_call::StartRecording,
    speed: f32,
) -> api::message::tool_call::StartRecording {
    use api::message::tool_call::start_recording::PlaybackSpeedKind;
    tool_call.playback_speed_kind = Some(PlaybackSpeedKind::PlaybackSpeed(speed));
    tool_call
}

#[test]
fn start_recording_absent_playback_speed_converts_to_none() {
    // The server never set the field at all (e.g. an old server build); the
    // client must fall back to its own default rather than assuming
    // real-time or any other specific value.
    let action = AIAgentActionType::try_from(start_recording(None))
        .expect("valid start_recording should convert");
    match action {
        AIAgentActionType::StartRecording {
            playback_speed_multiplier,
            ..
        } => assert_eq!(
            playback_speed_multiplier, None,
            "an absent playback_speed field should convert to None"
        ),
        other => panic!("expected StartRecording, got {other:?}"),
    }
}

#[test]
fn start_recording_carries_fractional_playback_speed_above_one() {
    let tool_call = with_playback_speed(start_recording(None), 1.5);
    let action =
        AIAgentActionType::try_from(tool_call).expect("valid start_recording should convert");
    match action {
        AIAgentActionType::StartRecording {
            playback_speed_multiplier,
            ..
        } => assert_eq!(playback_speed_multiplier, Some(1.5)),
        other => panic!("expected StartRecording, got {other:?}"),
    }
}

#[test]
fn start_recording_preserves_explicit_real_time_values_as_present() {
    // A server that explicitly sends 0 or 1 is asking for real-time, which
    // must be distinguishable from "the server said nothing at all" (see
    // `start_recording_absent_playback_speed_converts_to_none`). Presence
    // must therefore survive the conversion even though the raw value is
    // <= 1.0; only the executor's resolution step (not this conversion)
    // interprets what to do with an explicit <= 1.0 value.
    for speed in [0.0, 1.0] {
        let tool_call = with_playback_speed(start_recording(None), speed);
        let action =
            AIAgentActionType::try_from(tool_call).expect("valid start_recording should convert");
        match action {
            AIAgentActionType::StartRecording {
                playback_speed_multiplier,
                ..
            } => assert_eq!(
                playback_speed_multiplier,
                Some(speed),
                "explicit speed {speed} should remain present (Some), not collapse to None"
            ),
            other => panic!("expected StartRecording, got {other:?}"),
        }
    }
}
