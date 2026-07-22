//! Cron parsing and schedule fire/miss decisions for local automations.
//!
//! Pure logic (no WarpUI). Unit-tested with fixed timestamps.

use std::str::FromStr;

use chrono::{DateTime, Duration, Local, TimeZone};
use cron::Schedule;

/// How far back a missed tick may still trigger a single catch-up run.
pub const CATCH_UP_WINDOW: Duration = Duration::hours(6);

/// How long a scheduled start may stay "in flight" before the scheduler
/// treats the spawn as failed and allows another attempt.
pub const IN_FLIGHT_SAFETY_TIMEOUT: Duration = Duration::minutes(5);

/// Interval between scheduler ticks while Warp is running.
pub const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    Empty,
    Invalid(String),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::Empty => write!(f, "schedule must not be empty"),
            ScheduleError::Invalid(msg) => write!(f, "invalid schedule: {msg}"),
        }
    }
}

/// Expands well-known presets into 5-field cron, or returns the trimmed raw
/// expression. Does not validate cron syntax.
pub fn normalize_schedule(raw: &str) -> Result<String, ScheduleError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ScheduleError::Empty);
    }
    let normalized = match trimmed.to_ascii_lowercase().as_str() {
        "@hourly" => "0 * * * *",
        "@daily" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        _ => trimmed,
    };
    Ok(normalized.to_string())
}

/// Parses a user schedule string (preset or 5-field cron) into a `cron::Schedule`.
///
/// The `cron` crate expects a 6-field expression with seconds first. We accept
/// the user-facing 5-field form and prepend `0` seconds.
pub fn parse_schedule(raw: &str) -> Result<Schedule, ScheduleError> {
    let normalized = normalize_schedule(raw)?;
    let with_seconds = if normalized.split_whitespace().count() == 5 {
        format!("0 {normalized}")
    } else {
        normalized
    };
    Schedule::from_str(&with_seconds).map_err(|e| ScheduleError::Invalid(e.to_string()))
}

/// Next fire strictly after `after` in local time.
///
/// Passes `DateTime<Local>` into the cron iterator so field matching uses the
/// machine's local timezone (not UTC).
pub fn next_fire_after(schedule: &Schedule, after: DateTime<Local>) -> Option<DateTime<Local>> {
    schedule.after(&after).next()
}

/// Most recent fire time at or before `now` that is strictly after `after`
/// (exclusive lower bound). Returns `None` if no fire falls in `(after, now]`.
pub fn latest_due_fire(
    schedule: &Schedule,
    after: DateTime<Local>,
    now: DateTime<Local>,
) -> Option<DateTime<Local>> {
    if now <= after {
        return None;
    }
    // Walk upcoming fires from `after` and keep the last one that is still <= now.
    let mut latest = None;
    for local in schedule.after(&after) {
        if local > now {
            break;
        }
        latest = Some(local);
    }
    latest
}

/// Scheduler decision for one automation at `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleDecision {
    /// Start a scheduled run for `due_at` (live tick or catch-up).
    Fire {
        due_at: DateTime<Local>,
    },
    /// Record a miss; do not auto-start (outside catch-up window).
    Missed {
        due_at: DateTime<Local>,
    },
    /// Nothing due; next fire is at `next_at` if known.
    Wait {
        next_at: Option<DateTime<Local>>,
    },
    SkipDisabled,
    SkipInFlight,
    Invalid,
}

/// Inputs for [`decide_schedule`].
#[derive(Debug, Clone)]
pub struct ScheduleEvalInput {
    pub now: DateTime<Local>,
    pub enabled: bool,
    pub schedule_raw: String,
    pub last_scheduled_fire_at: Option<DateTime<Local>>,
    pub last_missed_at: Option<DateTime<Local>>,
    pub in_flight_since: Option<DateTime<Local>>,
}

/// Decide whether to fire, mark missed, wait, or skip.
pub fn decide_schedule(input: &ScheduleEvalInput) -> ScheduleDecision {
    if !input.enabled {
        return ScheduleDecision::SkipDisabled;
    }

    if let Some(since) = input.in_flight_since {
        if input.now - since < IN_FLIGHT_SAFETY_TIMEOUT {
            return ScheduleDecision::SkipInFlight;
        }
        // Safety timeout expired: treat as not in flight and continue.
    }

    let schedule = match parse_schedule(&input.schedule_raw) {
        Ok(s) => s,
        Err(_) => return ScheduleDecision::Invalid,
    };

    // Lower bound for "due since last fire": just after the last scheduled fire,
    // or far in the past on first sight so the most recent tick can catch up.
    let after = input
        .last_scheduled_fire_at
        .unwrap_or_else(|| Local.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap());

    let Some(due_at) = latest_due_fire(&schedule, after, input.now) else {
        let next_at = next_fire_after(&schedule, input.now);
        return ScheduleDecision::Wait { next_at };
    };

    // Already recorded this exact miss: do not re-bump; wait for the next tick.
    if input.last_missed_at == Some(due_at)
        && input
            .last_scheduled_fire_at
            .map_or(true, |last| last < due_at)
    {
        // If we marked missed for this due_at and never fired it, stay waiting
        // for a *future* tick rather than re-emitting Missed every 30s.
        let next_at = next_fire_after(&schedule, input.now);
        return ScheduleDecision::Wait { next_at };
    }

    let age = input.now - due_at;
    if age <= CATCH_UP_WINDOW {
        ScheduleDecision::Fire { due_at }
    } else {
        ScheduleDecision::Missed { due_at }
    }
}

/// Human-facing next/last/missed status for the list UI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationScheduleStatus {
    pub next_at: Option<DateTime<Local>>,
    pub last_scheduled_fire_at: Option<DateTime<Local>>,
    pub missed: bool,
    pub invalid_schedule: bool,
    pub disabled: bool,
}

impl AutomationScheduleStatus {
    /// Compact subtitle fragment (without leading separator).
    pub fn subtitle_fragment(&self) -> Option<String> {
        if self.disabled {
            return None; // caller already shows "disabled"
        }
        if self.invalid_schedule {
            return Some("invalid schedule".to_string());
        }
        let mut parts = Vec::new();
        if self.missed {
            parts.push("Missed while Warp was closed".to_string());
        }
        if let Some(next) = self.next_at {
            parts.push(format!("Next {}", format_local_short(next)));
        }
        if let Some(last) = self.last_scheduled_fire_at {
            parts.push(format!("Last ran {}", format_local_short(last)));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }
}

fn format_local_short(dt: DateTime<Local>) -> String {
    dt.format("%b %-d %-I:%M%P").to_string()
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;
