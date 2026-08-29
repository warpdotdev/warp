use super::{LongRunningCommandControlState, UserTakeOverReason};

// Blocks persisted before `should_auto_resume` stored `Stop` as a bare unit variant. These must
// still deserialize (with resume disabled) so restoring an older session doesn't drop the block's
// AI metadata wholesale.
#[test]
fn legacy_stop_reason_deserializes_with_resume_disabled() {
    let reason: UserTakeOverReason = serde_json::from_str("\"Stop\"").unwrap();
    assert_eq!(
        reason,
        UserTakeOverReason::Stop {
            should_auto_resume: false
        }
    );

    let state: LongRunningCommandControlState =
        serde_json::from_str(r#"{"User":{"reason":"Stop"}}"#).unwrap();
    assert_eq!(
        state,
        LongRunningCommandControlState::User {
            reason: UserTakeOverReason::Stop {
                should_auto_resume: false
            }
        }
    );
}

#[test]
fn stop_reason_round_trips() {
    for should_auto_resume in [true, false] {
        let reason = UserTakeOverReason::Stop { should_auto_resume };
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(
            serde_json::from_str::<UserTakeOverReason>(&json).unwrap(),
            reason
        );
    }
}
