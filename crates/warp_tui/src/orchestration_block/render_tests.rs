use warp::tui_export::AIActionStatus;

use super::is_orphaned_by_finished_output;

/// A `RunAgents` tool call that never entered the action queue has no
/// status of its own; once the block backing it has already finished
/// unsuccessfully (cancelled or failed), the card can never reach a real
/// outcome and must render as cancelled.
#[test]
fn statusless_action_on_finished_block_is_orphaned() {
    assert!(is_orphaned_by_finished_output(None, true));
}

/// A status-less action on a block that hasn't finished unsuccessfully
/// (still streaming, or completed successfully) is still awaiting its
/// first status and must not be treated as orphaned.
#[test]
fn statusless_action_on_unfinished_or_successful_block_is_not_orphaned() {
    assert!(!is_orphaned_by_finished_output(None, false));
}

/// An action that reached the queue gets a real status of its own even
/// after the conversation is cancelled, so its own status must keep
/// driving the card instead of the orphan fallback.
#[test]
fn action_with_status_on_finished_block_is_not_orphaned() {
    for status in [
        AIActionStatus::Preprocessing,
        AIActionStatus::Queued,
        AIActionStatus::Blocked,
        AIActionStatus::RunningAsync,
    ] {
        assert!(
            !is_orphaned_by_finished_output(Some(&status), true),
            "{status:?} should not orphan the card"
        );
    }
}
