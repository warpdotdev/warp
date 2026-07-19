use std::time::Duration;

use warp::tui_export::{LongRunningCommandControlState, UserTakeOverReason};

use super::{
    format_next_check_remaining, remaining_for_fixed_delay, resolve_latest_instruction,
    terminal_use_status_text,
};

#[test]
fn terminal_use_status_covers_control_and_lifecycle_states() {
    let agent = LongRunningCommandControlState::Agent {
        is_blocked: false,
        should_hide_responses: false,
    };
    assert_eq!(
        terminal_use_status_text(&agent, false, true),
        "Agent is monitoring command · ctrl-c to take control"
    );
    assert_eq!(
        terminal_use_status_text(&agent, false, false),
        "Agent waiting for instructions · ctrl-c to take control"
    );
    assert_eq!(
        terminal_use_status_text(&agent, true, true),
        "Command finished"
    );

    let blocked = LongRunningCommandControlState::Agent {
        is_blocked: true,
        should_hide_responses: false,
    };
    assert_eq!(
        terminal_use_status_text(&blocked, false, true),
        "Agent needs your input · ctrl-c to take control"
    );

    let manual = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::Manual,
    };
    assert_eq!(
        terminal_use_status_text(&manual, false, false),
        "User is in control · ctrl-g to hand back"
    );

    let stopped = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::Stop {
            should_auto_resume: true,
        },
    };
    assert_eq!(
        terminal_use_status_text(&stopped, false, false),
        "Agent paused · user is in control · ctrl-g to hand back"
    );

    let transferred = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::TransferFromAgent {
            reason: "enter password".to_owned(),
        },
    };
    assert_eq!(
        terminal_use_status_text(&transferred, false, false),
        "Agent handed control to you · ctrl-g to hand back"
    );
}

#[test]
fn controller_instruction_precedes_stale_exchange_input() {
    assert_eq!(
        resolve_latest_instruction(
            Some("new instruction".to_owned()),
            Some("old instruction".to_owned())
        ),
        Some("new instruction".to_owned())
    );
}

#[test]
fn next_check_countdown_decreases_and_expires() {
    assert_eq!(
        remaining_for_fixed_delay(Duration::from_secs(10), Duration::from_secs(3)),
        Some(Duration::from_secs(7))
    );
    assert_eq!(
        remaining_for_fixed_delay(Duration::from_secs(10), Duration::from_secs(10)),
        None
    );
}

#[test]
fn next_check_countdown_formats_seconds_and_minutes() {
    assert_eq!(
        format_next_check_remaining(Duration::from_secs(12)),
        " · Check in 12s"
    );
    assert_eq!(
        format_next_check_remaining(Duration::from_secs(65)),
        " · Check in 1m"
    );
}
