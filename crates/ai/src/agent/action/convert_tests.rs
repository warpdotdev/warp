use computer_use::Target;
use warp_multi_agent_api as api;
use warp_multi_agent_api::message::tool_call::start_recording::AttachmentSetting;

use crate::agent::action::AIAgentActionType;
use crate::agent::convert::ToolToAIAgentActionError;

fn start_recording(
    target: Option<api::message::tool_call::ComputerUseTarget>,
) -> api::message::tool_call::StartRecording {
    start_recording_with_setting(target, AttachmentSetting::Unspecified)
}

fn start_recording_with_setting(
    target: Option<api::message::tool_call::ComputerUseTarget>,
    attachment_setting: AttachmentSetting,
) -> api::message::tool_call::StartRecording {
    api::message::tool_call::StartRecording {
        frame_rate: 15,
        limits: None,
        summary: String::new(),
        playback_speed_multiplier: 0,
        target,
        description: String::new(),
        attachment_setting: attachment_setting as i32,
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

#[test]
fn start_recording_captures_thumbnail_only_for_embed() {
    for (setting, expected) in [
        (AttachmentSetting::Unspecified, false),
        (AttachmentSetting::Disabled, false),
        (AttachmentSetting::LinkOnly, false),
        (AttachmentSetting::Embed, true),
    ] {
        let action = AIAgentActionType::try_from(start_recording_with_setting(None, setting))
            .expect("recording should convert");
        let AIAgentActionType::StartRecording {
            capture_thumbnail, ..
        } = action
        else {
            panic!("expected a StartRecording action, got {action:?}");
        };
        assert_eq!(
            capture_thumbnail, expected,
            "capture_thumbnail for {setting:?} should be {expected}"
        );
    }
}
