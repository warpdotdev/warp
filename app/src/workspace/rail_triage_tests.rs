use std::time::Duration;

use super::*;

fn blocked(secs: u64) -> TaskTriage {
    TaskTriage {
        status: Some(ConversationStatus::Blocked {
            blocked_action: "Wants to run bash: ls".to_owned(),
        }),
        blocked_for: Some(Duration::from_secs(secs)),
        has_unseen_success: false,
        marked_unread: false,
    }
}

fn running() -> TaskTriage {
    TaskTriage {
        status: Some(ConversationStatus::InProgress),
        ..Default::default()
    }
}

fn success(seen: bool) -> TaskTriage {
    TaskTriage {
        status: Some(ConversationStatus::Success),
        has_unseen_success: !seen,
        ..Default::default()
    }
}

fn task(tab_index: usize, urgency: Option<RailUrgency>) -> RailTask {
    RailTask { tab_index, urgency }
}

// -- manual unread -----------------------------------------------------------

/// A manual unread mark is the one state that needs no agent status at all:
/// it is the only way a dormant row (no live session) can be green, and it
/// outranks every live status including a fresh block.
#[test]
fn marked_unread_is_green_for_any_status_and_none() {
    for status in [
        None,
        Some(ConversationStatus::InProgress),
        Some(ConversationStatus::Success),
        Some(ConversationStatus::Blocked {
            blocked_action: "Wants to run bash: ls".to_owned(),
        }),
        Some(ConversationStatus::Error),
        Some(ConversationStatus::Cancelled),
    ] {
        let triage = TaskTriage {
            status,
            marked_unread: true,
            ..Default::default()
        };
        assert_eq!(
            triage.urgency(),
            Some(RailUrgency::Unseen),
            "marked_unread must pin the row green"
        );
    }
}

/// Clearing the manual mark hands the row back to the ordinary rules.
#[test]
fn cleared_unread_mark_restores_status_driven_triage() {
    let mut triage = TaskTriage {
        status: Some(ConversationStatus::InProgress),
        marked_unread: true,
        ..Default::default()
    };
    triage.marked_unread = false;
    assert_eq!(triage.urgency(), Some(RailUrgency::Running));

    let mut triage = TaskTriage {
        status: None,
        marked_unread: true,
        ..Default::default()
    };
    triage.marked_unread = false;
    assert_eq!(triage.urgency(), None);
}

// -- threshold math -------------------------------------------------------

/// The escalation is a `>=` on the threshold, so the boundary second itself is
/// already red. Pinned because "5 minutes" is a promise to the user: at 4:59
/// the row must still be orange, at 5:00 it must be red.
#[test]
fn rail_triage_orange_until_the_threshold_then_red() {
    let threshold = BLOCKED_ESCALATION_THRESHOLD.as_secs();

    assert_eq!(blocked(0).urgency(), Some(RailUrgency::Waiting));
    assert_eq!(
        blocked(threshold - 1).urgency(),
        Some(RailUrgency::Waiting),
        "one second before the threshold escalated early"
    );
    assert_eq!(
        blocked(threshold).urgency(),
        Some(RailUrgency::Overdue),
        "the threshold second itself must already be red"
    );
    assert_eq!(
        blocked(threshold * 10).urgency(),
        Some(RailUrgency::Overdue)
    );
}

/// A blocked row whose session carries no stamp still goes orange — it just
/// cannot escalate. Degrading to plain orange beats inventing an age.
#[test]
fn rail_triage_blocked_without_a_stamp_is_orange_and_ageless() {
    let unstamped = TaskTriage {
        blocked_for: None,
        ..blocked(0)
    };
    assert_eq!(unstamped.urgency(), Some(RailUrgency::Waiting));
    assert_eq!(unstamped.wait_age(), None);
}

#[test]
fn rail_triage_wait_age_is_whole_minutes_then_hours() {
    // Under a minute there is nothing to show: the refresh is 30s coarse.
    assert_eq!(format_wait_age(Duration::from_secs(0)), None);
    assert_eq!(format_wait_age(Duration::from_secs(59)), None);
    assert_eq!(
        format_wait_age(Duration::from_secs(60)).as_deref(),
        Some("1m")
    );
    assert_eq!(
        format_wait_age(Duration::from_secs(7 * 60 + 45)).as_deref(),
        Some("7m"),
        "the age must truncate, not round up past the real wait"
    );
    assert_eq!(
        format_wait_age(Duration::from_secs(59 * 60)).as_deref(),
        Some("59m")
    );
    assert_eq!(
        format_wait_age(Duration::from_secs(60 * 60)).as_deref(),
        Some("1h")
    );
    assert_eq!(
        format_wait_age(Duration::from_secs(150 * 60)).as_deref(),
        Some("2h")
    );
}

/// Only the waiting states carry an age; a working or finished row has no
/// wait to report even though time has passed.
#[test]
fn rail_triage_only_waiting_rows_show_a_wait_age() {
    assert_eq!(blocked(7 * 60).wait_age().as_deref(), Some("7m"));
    assert_eq!(running().wait_age(), None);
    assert_eq!(success(false).wait_age(), None);
}

// -- success_seen ---------------------------------------------------------

/// Green is exactly "finished AND unseen": acknowledging it returns the row to
/// neutral, and the bit means nothing at all outside `Success`.
#[test]
fn rail_triage_green_needs_success_and_unseen() {
    assert_eq!(success(false).urgency(), Some(RailUrgency::Unseen));
    assert_eq!(
        success(true).urgency(),
        None,
        "a seen result must render neutral, not green"
    );

    // The bit set on a non-Success row must never leak a green.
    for status in [
        ConversationStatus::InProgress,
        ConversationStatus::Error,
        ConversationStatus::TransientError,
        ConversationStatus::Cancelled,
        ConversationStatus::WaitingForEvents,
        ConversationStatus::Blocked {
            blocked_action: String::new(),
        },
    ] {
        let triage = TaskTriage {
            status: Some(status.clone()),
            blocked_for: None,
            has_unseen_success: true,
            marked_unread: false,
        };
        assert_ne!(
            triage.urgency(),
            Some(RailUrgency::Unseen),
            "{status:?} rendered as an unseen result"
        );
    }
}

/// Errors are not triage: a failed turn is not time-sensitive and must not
/// compete with an agent that is waiting right now.
#[test]
fn rail_triage_terminal_failures_are_neutral() {
    for status in [
        ConversationStatus::Error,
        ConversationStatus::TransientError,
        ConversationStatus::Cancelled,
    ] {
        let triage = TaskTriage {
            status: Some(status.clone()),
            ..Default::default()
        };
        assert_eq!(triage.urgency(), None, "{status:?} claimed an urgency");
    }
    assert_eq!(TaskTriage::default().urgency(), None);
}

// -- project aggregate ----------------------------------------------------

/// red > orange > green > running, whatever order the tasks arrive in.
#[test]
fn rail_triage_project_header_inherits_the_most_urgent_child() {
    let overdue = blocked(BLOCKED_ESCALATION_THRESHOLD.as_secs() + 1);
    let waiting = blocked(30);

    let all = [running(), success(false), waiting.clone(), overdue.clone()];
    assert_eq!(project_triage(&all).urgency, Some(RailUrgency::Overdue));

    // Reversed input, same winner: precedence is `Ord`, not arrival order.
    let mut reversed = all.clone();
    reversed.reverse();
    assert_eq!(
        project_triage(&reversed).urgency,
        Some(RailUrgency::Overdue)
    );

    assert_eq!(
        project_triage(&[running(), success(false), waiting]).urgency,
        Some(RailUrgency::Waiting)
    );
    assert_eq!(
        project_triage(&[running(), success(false)]).urgency,
        Some(RailUrgency::Unseen)
    );
    assert_eq!(
        project_triage(&[running()]).urgency,
        Some(RailUrgency::Running)
    );
    // Only untriageable children, and none at all, both read as no badge.
    assert_eq!(project_triage(&[success(true)]).urgency, None);
    assert_eq!(project_triage(&[]), ProjectTriage::default());
}

/// The header reports the *oldest* fire, not the winning task's own age —
/// with two red children the user needs to know how bad the worst one is.
#[test]
fn rail_triage_project_header_reports_the_worst_wait() {
    let aggregate = project_triage(&[blocked(9 * 60), running(), blocked(2 * 60)]);
    assert_eq!(aggregate.urgency, Some(RailUrgency::Overdue));
    assert_eq!(
        aggregate.worst_blocked_for,
        Some(Duration::from_secs(9 * 60))
    );
    assert_eq!(aggregate.wait_age().as_deref(), Some("9m"));

    // A green/running project has no wait to report at all.
    assert_eq!(project_triage(&[success(false)]).worst_blocked_for, None);
    assert_eq!(project_triage(&[success(false)]).wait_age(), None);
}

/// The badge and the tint come from the same child, so a header can never show
/// a green check while tinted red.
#[test]
fn rail_triage_project_header_status_is_the_winning_child() {
    let aggregate = project_triage(&[success(false), blocked(30)]);
    assert_eq!(aggregate.urgency, Some(RailUrgency::Waiting));
    assert!(matches!(
        aggregate.status,
        Some(ConversationStatus::Blocked { .. })
    ));
}

// -- chips ----------------------------------------------------------------

#[test]
fn rail_triage_chip_counts_span_both_bands() {
    // Two ranked projects and one unranked one, with the divider between.
    let rows = [
        RailProjectRow::Project {
            index: 0,
            rank: Some(0),
        },
        RailProjectRow::Project {
            index: 1,
            rank: Some(1),
        },
        RailProjectRow::UnrankedDivider,
        RailProjectRow::Project {
            index: 2,
            rank: None,
        },
    ];
    let tasks_by_project = vec![
        vec![
            task(10, Some(RailUrgency::Overdue)),
            task(11, Some(RailUrgency::Running)),
        ],
        vec![task(20, Some(RailUrgency::Unseen))],
        // The unranked band still counts: the chips are the escape hatch for
        // "the rank-8 project is on fire".
        vec![
            task(30, Some(RailUrgency::Waiting)),
            task(31, Some(RailUrgency::Unseen)),
            task(32, None),
        ],
    ];

    let order = rail_task_order(&rows, &tasks_by_project);
    assert_eq!(
        order.iter().map(|t| t.tab_index).collect::<Vec<_>>(),
        vec![10, 11, 20, 30, 31, 32],
        "the divider must not contribute a task, and the bands must not interleave"
    );

    let counts = chip_counts(&order);
    assert_eq!(
        counts.blocked, 2,
        "orange and red are one gradient, both count"
    );
    assert_eq!(counts.unseen, 2);
    assert_eq!(counts.get(RailChip::Blocked), 2);
    assert_eq!(counts.get(RailChip::Unseen), 2);

    // Nothing urgent anywhere: both chips are hidden.
    let quiet = rail_task_order(
        &rows,
        &[vec![task(10, Some(RailUrgency::Running))], vec![], vec![]],
    );
    assert_eq!(chip_counts(&quiet), ChipCounts::default());
}

/// A project the rows do not mention contributes nothing, and a row pointing
/// past the end of the task lists is skipped rather than panicking.
#[test]
fn rail_triage_task_order_tolerates_a_missing_project() {
    let rows = [RailProjectRow::Project {
        index: 7,
        rank: None,
    }];
    assert!(rail_task_order(&rows, &[vec![task(1, None)]]).is_empty());
}

/// Clicking the chip walks down the rail and wraps — ranked band first, so a
/// rank-1 fire is always reached before a rank-8 one.
#[test]
fn rail_triage_chip_cycles_in_rank_order_and_wraps() {
    let tasks = [
        task(10, Some(RailUrgency::Overdue)), // ranked project 1
        task(11, Some(RailUrgency::Running)),
        task(20, Some(RailUrgency::Waiting)), // ranked project 2
        task(30, Some(RailUrgency::Unseen)),  // unranked
        task(31, Some(RailUrgency::Waiting)),
    ];

    // No active tab (or an active tab that is not a rail task): start at the top.
    assert_eq!(next_chip_target(&tasks, RailChip::Blocked, None), Some(10));
    assert_eq!(
        next_chip_target(&tasks, RailChip::Blocked, Some(999)),
        Some(10)
    );

    // Walking the whole cycle: 10 → 20 → 31 → back to 10.
    assert_eq!(
        next_chip_target(&tasks, RailChip::Blocked, Some(10)),
        Some(20)
    );
    assert_eq!(
        next_chip_target(&tasks, RailChip::Blocked, Some(20)),
        Some(31)
    );
    assert_eq!(
        next_chip_target(&tasks, RailChip::Blocked, Some(31)),
        Some(10),
        "the last blocked task must wrap to the first"
    );

    // Standing on a non-target row still advances to the next target below it.
    assert_eq!(
        next_chip_target(&tasks, RailChip::Blocked, Some(11)),
        Some(20)
    );

    // The two chips walk disjoint sets.
    assert_eq!(next_chip_target(&tasks, RailChip::Unseen, None), Some(30));
    assert_eq!(
        next_chip_target(&tasks, RailChip::Unseen, Some(30)),
        Some(30),
        "a lone target must cycle back to itself rather than vanish"
    );
}

#[test]
fn rail_triage_chip_with_no_targets_has_nowhere_to_jump() {
    assert_eq!(next_chip_target(&[], RailChip::Blocked, None), None);
    assert_eq!(next_chip_target(&[], RailChip::Blocked, Some(3)), None);
    let only_running = [task(10, Some(RailUrgency::Running)), task(11, None)];
    assert_eq!(
        next_chip_target(&only_running, RailChip::Blocked, Some(10)),
        None
    );
    assert_eq!(
        next_chip_target(&only_running, RailChip::Unseen, None),
        None
    );
}
