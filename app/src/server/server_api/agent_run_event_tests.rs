use serde_json::json;

use super::{AgentRunEvent, AgentRunEventKind};

#[test]
fn lifecycle_details_roundtrip_in_the_existing_wire_shape() {
    for (wire, expected) in [
        (
            json!({"event_type":"run_failed","detail":{"reason":"setup_failed","error_message":"Repository unavailable"}}),
            AgentRunEventKind::Failed {
                reason: Some("setup_failed".into()),
                error_message: Some("Repository unavailable".into()),
            },
        ),
        (
            json!({"event_type":"run_errored","detail":{"reason":"process_failed","error_message":"Process exited"}}),
            AgentRunEventKind::Errored {
                reason: Some("process_failed".into()),
                error_message: Some("Process exited".into()),
            },
        ),
        (
            json!({"event_type":"run_blocked","detail":{"blocked_action":"Approval required"}}),
            AgentRunEventKind::Blocked {
                blocked_action: Some("Approval required".into()),
            },
        ),
        (
            json!({"event_type":"run_cancelled","detail":{"reason":"Requester cancelled"}}),
            AgentRunEventKind::Cancelled {
                reason: Some("Requester cancelled".into()),
            },
        ),
    ] {
        let decoded: AgentRunEventKind = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }
}

#[test]
fn historical_lifecycle_details_remain_optional() {
    for wire in [
        json!({"event_type":"run_failed"}),
        json!({"event_type":"run_failed","detail":null}),
        json!({"event_type":"run_failed","detail":{}}),
        json!({"event_type":"run_failed","detail":{"reason":null,"error_message":null,"future":"ignored"}}),
    ] {
        let decoded: AgentRunEventKind = serde_json::from_value(wire).unwrap();
        assert_eq!(
            decoded,
            AgentRunEventKind::Failed {
                reason: None,
                error_message: None,
            }
        );
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            json!({"event_type":"run_failed"})
        );
    }
}

#[test]
fn malformed_known_details_do_not_become_unknown_events() {
    for wire in [
        json!({"event_type":"run_failed","detail":[]}),
        json!({"event_type":"run_errored","detail":{"error_message":134}}),
    ] {
        assert!(serde_json::from_value::<AgentRunEventKind>(wire).is_err());
    }
}

#[test]
fn flattened_events_keep_unknown_types_and_non_lifecycle_references() {
    let events: Vec<AgentRunEvent> = serde_json::from_value(json!([
        {"event_type":"future_event","detail":{"future":true},"run_id":"child","sequence":1,"occurred_at":"2026-01-01T00:00:00Z"},
        {"event_type":"new_message","ref_id":"message","run_id":"parent","sequence":2,"occurred_at":"2026-01-01T00:00:00Z"},
        {"event_type":"child_agent_started","ref_id":"child","run_id":"parent","sequence":4,"occurred_at":"2026-01-01T00:00:00Z"},
        {"event_type":"run_session_linked","ref_id":"session","run_id":"child","sequence":5,"occurred_at":"2026-01-01T00:00:00Z"}
    ])).unwrap();
    assert_eq!(events[0].kind, AgentRunEventKind::Unknown);
    assert_eq!(
        serde_json::to_value(&events[0].kind).unwrap(),
        json!({"event_type":"unknown"})
    );
    assert_eq!(events[1].kind, AgentRunEventKind::NewMessage);
    assert_eq!(events[1].ref_id.as_deref(), Some("message"));
    assert_eq!(events[2].kind, AgentRunEventKind::ChildAgentStarted);
    assert_eq!(events[2].ref_id.as_deref(), Some("child"));
    assert_eq!(events[3].kind, AgentRunEventKind::RunSessionLinked);
    assert_eq!(events[3].ref_id.as_deref(), Some("session"));
}
