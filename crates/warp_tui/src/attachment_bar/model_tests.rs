use super::{AttachmentModeTransition, attachment_mode_transition, reconciled_selected_index};

#[test]
fn selection_tracks_newest_and_clamps_after_removal() {
    assert_eq!(reconciled_selected_index(0, 2, None), Some(1));
    assert_eq!(reconciled_selected_index(2, 1, Some(1)), Some(0));
    assert_eq!(reconciled_selected_index(1, 0, Some(0)), None);
}

#[test]
fn attachment_transitions_lock_and_restore_nld() {
    assert_eq!(
        attachment_mode_transition(false, true, true, false),
        AttachmentModeTransition::LockAgent
    );
    assert_eq!(
        attachment_mode_transition(true, true, true, false),
        AttachmentModeTransition::None
    );
    assert_eq!(
        attachment_mode_transition(true, false, true, false),
        AttachmentModeTransition::RestoreAgent {
            request_detection: true
        }
    );
    assert_eq!(
        attachment_mode_transition(true, false, true, true),
        AttachmentModeTransition::RestoreAgent {
            request_detection: false
        }
    );
    assert_eq!(
        attachment_mode_transition(true, false, false, false),
        AttachmentModeTransition::RestoreAgent {
            request_detection: false
        }
    );
}
