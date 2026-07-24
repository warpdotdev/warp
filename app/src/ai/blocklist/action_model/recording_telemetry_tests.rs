use serde_json::json;
use warp_core::telemetry::TelemetryEvent;

use super::RecordingTelemetryEvent;

#[test]
fn started_payload_shape() {
    let event = RecordingTelemetryEvent::Started {
        recording_id: "11111111-2222-3333-4444-555555555555".to_string(),
        capture_target: "window".to_string(),
    };

    assert_eq!(event.name(), "Recording.Started");
    let payload = event.payload().expect("Started event has a payload");
    assert_eq!(
        payload,
        json!({
            "recording_id": "11111111-2222-3333-4444-555555555555",
            "capture_target": "window",
        })
    );
}

#[test]
fn stopped_success_payload_shape() {
    let event = RecordingTelemetryEvent::Stopped {
        recording_id: "11111111-2222-3333-4444-555555555555".to_string(),
        outcome: "success".to_string(),
        duration_secs: Some(12.5),
        size_bytes: Some(1_048_576),
        termination_reason: "agent_stopped".to_string(),
    };

    assert_eq!(event.name(), "Recording.Stopped");
    let payload = event.payload().expect("Stopped event has a payload");
    assert_eq!(
        payload,
        json!({
            "recording_id": "11111111-2222-3333-4444-555555555555",
            "outcome": "success",
            "duration_secs": 12.5,
            "size_bytes": 1_048_576,
            "termination_reason": "agent_stopped",
        })
    );
}

#[test]
fn stopped_error_payload_allows_missing_metadata() {
    // An error/cancel/discard outcome carries no capture metadata.
    let event = RecordingTelemetryEvent::Stopped {
        recording_id: "deadbeef".to_string(),
        outcome: "error".to_string(),
        duration_secs: None,
        size_bytes: None,
        termination_reason: "encoding_failed".to_string(),
    };

    let payload = event.payload().expect("Stopped event has a payload");
    assert_eq!(payload["outcome"], json!("error"));
    assert_eq!(payload["duration_secs"], json!(null));
    assert_eq!(payload["size_bytes"], json!(null));
    assert_eq!(payload["termination_reason"], json!("encoding_failed"));
}

#[test]
fn contains_no_user_generated_content() {
    let started = RecordingTelemetryEvent::Started {
        recording_id: "rec".to_string(),
        capture_target: "screen".to_string(),
    };
    let stopped = RecordingTelemetryEvent::Stopped {
        recording_id: "rec".to_string(),
        outcome: "success".to_string(),
        duration_secs: None,
        size_bytes: None,
        termination_reason: "agent_stopped".to_string(),
    };
    assert!(!started.contains_ugc());
    assert!(!stopped.contains_ugc());
}
