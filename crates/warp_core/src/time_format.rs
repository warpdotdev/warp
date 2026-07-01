//! Human-readable formatting for elapsed durations.

use std::time::Duration;

/// Formats elapsed time as a whole-seconds string with proper singular/plural
/// (e.g. "1 second", "15 seconds").
pub fn format_elapsed_seconds(elapsed: Duration) -> String {
    let total_seconds = elapsed.as_secs();
    if total_seconds == 1 {
        "1 second".to_owned()
    } else {
        format!("{total_seconds} seconds")
    }
}

#[cfg(test)]
#[path = "time_format_tests.rs"]
mod tests;
