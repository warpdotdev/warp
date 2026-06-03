use std::ops::Sub;

use chrono::{DateTime, Duration, Local, Utc};

// Some conversion ratios for time units.
const SEC_TO_MS: f64 = 1000.;
const MIN_TO_MS: f64 = 60. * SEC_TO_MS;
const HOUR_TO_MS: f64 = 60. * MIN_TO_MS;
const DAY_TO_MS: f64 = 24. * HOUR_TO_MS;
const WEEK_TO_MS: f64 = 7. * DAY_TO_MS;
const MONTH_TO_MS: f64 = 30.44 * DAY_TO_MS;
const YEAR_TO_MS: f64 = 365.25 * DAY_TO_MS;

/// Subtract a given DateTime from now and format the duration is a concise, approximated,
/// human-readable form. e.g. "just now"
pub fn format_approx_duration_from_now(datetime: DateTime<Local>) -> String {
    human_readable_approx_duration(Local::now().sub(datetime), false)
}

/// Subtract a given DateTime from now and format the duration is a concise, approximated,
/// human-readable form. e.g. "Just now"
pub fn format_approx_duration_from_now_sentence_case(datetime: DateTime<Local>) -> String {
    human_readable_approx_duration(Local::now().sub(datetime), true)
}

/// Takes a time in UTC and determines roughly how long ago it occurred.
pub fn format_approx_duration_from_now_utc(datetime: DateTime<Utc>) -> String {
    human_readable_approx_duration(Utc::now().sub(datetime), false)
}

/// Format a duration into a human-readable string, e.g. "3.14 sec".
/// Compared to [`human_readable_approx_duration`], this method is for higher-precision, smaller
/// values.
pub fn human_readable_precise_duration(duration: Duration) -> String {
    let ms = duration.num_milliseconds() as f64;
    let weeks = ms / WEEK_TO_MS;
    if weeks >= 1. {
        return i18n::t("util.time.precise.more_than_one_week");
    }
    let days = ms / DAY_TO_MS;
    if days >= 1. {
        return t_count("util.time.precise.days", format_sigfigs(days, 3));
    }
    let hours = ms / HOUR_TO_MS;
    if hours >= 1. {
        return t_count("util.time.precise.hours", format_sigfigs(hours, 3));
    }
    let minutes = ms / MIN_TO_MS;
    if minutes >= 1. {
        return t_count("util.time.precise.minutes", format_sigfigs(minutes, 3));
    }
    let seconds = ms / SEC_TO_MS;
    if seconds >= 1. {
        return t_count("util.time.precise.seconds", format_sigfigs(seconds, 3));
    }
    t_count(
        "util.time.precise.milliseconds",
        duration.num_milliseconds(),
    )
}

fn format_sigfigs(num: f64, sigfigs: usize) -> String {
    let a = num.abs();
    let precision = if a > 1. {
        let n = (1. + a.log10().floor()) as usize;
        sigfigs.saturating_sub(n)
    } else if a > 0. {
        let n = -(1. + a.log10().floor()) as usize;
        sigfigs + n
    } else {
        0
    };
    format!("{num:.precision$}")
}

/// Format an approximated duration into a human-readable string, e.g. "2 days ago".
/// Precision is limited to the most significant unit, i.e. 2 days and _n_ hours always displays
/// simply as "2 days ago".
pub fn human_readable_approx_duration(duration: Duration, sentence_case: bool) -> String {
    let ms = duration.num_milliseconds() as f64;
    let years = ms / YEAR_TO_MS;
    if years >= 1. {
        return truncated_quantity_with_unit(
            years,
            "util.time.approx.year_ago",
            "util.time.approx.years_ago",
        );
    }
    let months = ms / MONTH_TO_MS;
    if months >= 1. {
        return truncated_quantity_with_unit(
            months,
            "util.time.approx.month_ago",
            "util.time.approx.months_ago",
        );
    }
    let weeks = ms / WEEK_TO_MS;
    if weeks >= 1. {
        return truncated_quantity_with_unit(
            weeks,
            "util.time.approx.week_ago",
            "util.time.approx.weeks_ago",
        );
    }
    let days = ms / DAY_TO_MS;
    if days >= 1. {
        return truncated_quantity_with_unit(
            days,
            "util.time.approx.day_ago",
            "util.time.approx.days_ago",
        );
    }
    let hours = ms / HOUR_TO_MS;
    if hours >= 1. {
        return truncated_quantity_with_unit(
            hours,
            "util.time.approx.hour_ago",
            "util.time.approx.hours_ago",
        );
    }
    // Minutes and seconds are both abbreviated, so skip pluralization.
    let minutes = ms / MIN_TO_MS;
    if minutes >= 1. {
        return t_count("util.time.approx.minutes_ago_short", minutes as i32);
    }
    if sentence_case {
        i18n::t("util.time.approx.just_now_sentence")
    } else {
        i18n::t("util.time.approx.just_now")
    }
}

/// Provided a value and a unit, this will format the quantity as an integer number with the
/// unit pluralized if the value is not 1.
fn truncated_quantity_with_unit(num: f64, singular_key: &str, plural_key: &str) -> String {
    let truncated_int = num as i32;
    if truncated_int == 1 {
        t_count(singular_key, truncated_int)
    } else {
        t_count(plural_key, truncated_int)
    }
}

fn t_count(key: &str, count: impl ToString) -> String {
    i18n::t(key).replace("{count}", &count.to_string())
}

/// Formats a monotonic `Instant` as a human-readable relative timestamp.
/// (Uses `Instant` rather than wall-clock `DateTime` for elapsed-time display.)
pub fn format_elapsed_since(created_at: instant::Instant) -> String {
    let secs = created_at.elapsed().as_secs();

    if secs < 60 {
        i18n::t("util.time.approx.just_now_sentence")
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            t_count("util.time.elapsed.minute_ago", mins)
        } else {
            t_count("util.time.elapsed.minutes_ago", mins)
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            t_count("util.time.approx.hour_ago", hours)
        } else {
            t_count("util.time.approx.hours_ago", hours)
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            t_count("util.time.approx.day_ago", days)
        } else {
            t_count("util.time.approx.days_ago", days)
        }
    }
}

#[cfg(test)]
#[path = "time_format_tests.rs"]
mod tests;
