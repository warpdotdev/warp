use chrono::TimeZone;

use super::*;

fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
    Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

fn input(
    now: DateTime<Local>,
    schedule: &str,
    last_fire: Option<DateTime<Local>>,
) -> ScheduleEvalInput {
    ScheduleEvalInput {
        now,
        enabled: true,
        schedule_raw: schedule.to_string(),
        last_scheduled_fire_at: last_fire,
        last_missed_at: None,
        in_flight_since: None,
    }
}

#[test]
fn normalize_presets() {
    assert_eq!(normalize_schedule("@hourly").unwrap(), "0 * * * *");
    assert_eq!(normalize_schedule("@daily").unwrap(), "0 0 * * *");
    assert_eq!(normalize_schedule("@weekly").unwrap(), "0 0 * * 0");
    assert_eq!(normalize_schedule("@monthly").unwrap(), "0 0 1 * *");
    assert_eq!(normalize_schedule("@yearly").unwrap(), "0 0 1 1 *");
    assert_eq!(normalize_schedule("@annually").unwrap(), "0 0 1 1 *");
    assert_eq!(normalize_schedule("0 9 * * 1-5").unwrap(), "0 9 * * 1-5");
    assert!(matches!(
        normalize_schedule("  "),
        Err(ScheduleError::Empty)
    ));
}

#[test]
fn parse_valid_and_invalid() {
    assert!(parse_schedule("0 9 * * 1-5").is_ok());
    assert!(parse_schedule("@daily").is_ok());
    assert!(parse_schedule("not a cron").is_err());
    assert!(parse_schedule("").is_err());
}

#[test]
fn next_fire_after_daily() {
    let schedule = parse_schedule("@daily").unwrap();
    // Just after midnight local
    let after = at(2026, 7, 22, 0, 1);
    let next = next_fire_after(&schedule, after).unwrap();
    assert_eq!(next, at(2026, 7, 23, 0, 0));
}

#[test]
fn fire_when_due_within_window() {
    // Hourly; last fire 10:00; now 11:05 → due 11:00 within 6h → Fire
    let now = at(2026, 7, 22, 11, 5);
    let last = at(2026, 7, 22, 10, 0);
    let decision = decide_schedule(&input(now, "0 * * * *", Some(last)));
    assert_eq!(
        decision,
        ScheduleDecision::Fire {
            due_at: at(2026, 7, 22, 11, 0)
        }
    );
}

#[test]
fn coalesce_multiple_missed_ticks_to_latest() {
    // Last fire 08:00; now 11:05 hourly → due should be 11:00 only (one fire)
    let now = at(2026, 7, 22, 11, 5);
    let last = at(2026, 7, 22, 8, 0);
    let decision = decide_schedule(&input(now, "0 * * * *", Some(last)));
    assert_eq!(
        decision,
        ScheduleDecision::Fire {
            due_at: at(2026, 7, 22, 11, 0)
        }
    );
}

#[test]
fn missed_when_outside_catch_up_window() {
    // Last fire 8h before a due tick that is now 8h old
    let now = at(2026, 7, 22, 18, 0);
    let last = at(2026, 7, 22, 9, 0);
    // Daily at 10:00 → due 10:00 is 8h ago → Missed
    let decision = decide_schedule(&input(now, "0 10 * * *", Some(last)));
    assert_eq!(
        decision,
        ScheduleDecision::Missed {
            due_at: at(2026, 7, 22, 10, 0)
        }
    );
}

#[test]
fn wait_when_nothing_due() {
    let now = at(2026, 7, 22, 9, 30);
    let last = at(2026, 7, 22, 9, 0);
    // Hourly: next is 10:00
    let decision = decide_schedule(&input(now, "0 * * * *", Some(last)));
    match decision {
        ScheduleDecision::Wait { next_at } => {
            assert_eq!(next_at, Some(at(2026, 7, 22, 10, 0)));
        }
        other => panic!("expected Wait, got {other:?}"),
    }
}

#[test]
fn skip_disabled_and_in_flight() {
    let now = at(2026, 7, 22, 11, 5);
    let mut inp = input(now, "0 * * * *", Some(at(2026, 7, 22, 10, 0)));
    inp.enabled = false;
    assert_eq!(decide_schedule(&inp), ScheduleDecision::SkipDisabled);

    inp.enabled = true;
    inp.in_flight_since = Some(now - Duration::minutes(1));
    assert_eq!(decide_schedule(&inp), ScheduleDecision::SkipInFlight);
}

#[test]
fn in_flight_safety_timeout_expires() {
    let now = at(2026, 7, 22, 11, 5);
    let mut inp = input(now, "0 * * * *", Some(at(2026, 7, 22, 10, 0)));
    inp.in_flight_since = Some(now - Duration::minutes(10));
    assert_eq!(
        decide_schedule(&inp),
        ScheduleDecision::Fire {
            due_at: at(2026, 7, 22, 11, 0)
        }
    );
}

#[test]
fn invalid_schedule_decision() {
    let now = at(2026, 7, 22, 11, 5);
    assert_eq!(
        decide_schedule(&input(now, "not-cron", None)),
        ScheduleDecision::Invalid
    );
}

#[test]
fn already_recorded_miss_does_not_reemit() {
    let now = at(2026, 7, 22, 18, 0);
    let due = at(2026, 7, 22, 10, 0);
    let mut inp = input(now, "0 10 * * *", Some(at(2026, 7, 22, 9, 0)));
    inp.last_missed_at = Some(due);
    match decide_schedule(&inp) {
        ScheduleDecision::Wait { .. } => {}
        other => panic!("expected Wait after recorded miss, got {other:?}"),
    }
}

#[test]
fn first_seen_can_catch_up_recent_tick() {
    // Never fired; daily at 09:00; now 09:30 → Fire 09:00
    let now = at(2026, 7, 22, 9, 30);
    let decision = decide_schedule(&input(now, "0 9 * * *", None));
    assert_eq!(
        decision,
        ScheduleDecision::Fire {
            due_at: at(2026, 7, 22, 9, 0)
        }
    );
}

#[test]
fn subtitle_fragment_missed_and_next() {
    let status = AutomationScheduleStatus {
        next_at: Some(at(2026, 7, 23, 9, 0)),
        last_scheduled_fire_at: Some(at(2026, 7, 22, 9, 0)),
        missed: true,
        invalid_schedule: false,
        disabled: false,
    };
    let frag = status.subtitle_fragment().unwrap();
    assert!(frag.contains("Missed while Warp was closed"));
    assert!(frag.contains("Next"));
    assert!(frag.contains("Last ran"));
}
