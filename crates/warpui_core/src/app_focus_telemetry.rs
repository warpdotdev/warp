use crate::time::get_current_time;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

// `DailyAppFocusDuration` records the cumulative duration for focus events that end on each day.
struct DailyAppFocusDuration {
    duration: Duration,
    last_synced_time: DateTime<Utc>,
}

impl DailyAppFocusDuration {
    // If calendar date has advanced since the last sync, record the
    // Daily App Focus event with the current duration.
    #[allow(deprecated)]
    fn try_record(&mut self, user_id: Option<String>, anonymous_id: String) {
        if get_current_time().date_naive() > self.last_synced_time.date_naive() {
            let daily_app_focus_duration_seconds =
                json!(self.duration.num_milliseconds() as f64 / 1000.);
            crate::telemetry::record_event(
                user_id,
                anonymous_id,
                "Daily App Focus Duration (seconds)".into(),
                Some(daily_app_focus_duration_seconds),
                false, /* contains_ugc */
                self.last_synced_time.date().and_hms(0, 0, 0),
            );
            self.reset();
        }
    }

    fn reset(&mut self) {
        self.duration = Duration::seconds(0);
        self.last_synced_time = get_current_time();
    }

    fn add_duration(&mut self, duration: Duration, user_id: Option<String>, anonymous_id: String) {
        self.try_record(user_id, anonymous_id);
        if let Some(new_duration) = self.duration.checked_add(&duration) {
            self.duration = new_duration;
        } else {
            log::info!("Unable to increase the running total daily app focus duration.");
        }
    }
}

pub struct AppFocusInfo {
    last_time_app_focused: DateTime<Utc>,
    daily_app_focus_duration: DailyAppFocusDuration,
}

impl AppFocusInfo {
    pub fn new() -> Self {
        let now = get_current_time();
        Self {
            last_time_app_focused: now,
            daily_app_focus_duration: DailyAppFocusDuration {
                duration: Duration::seconds(0),
                last_synced_time: now,
            },
        }
    }

    pub fn record_app_focus(&mut self, user_id: Option<String>, anonymous_id: String) {
        self.last_time_app_focused = get_current_time();
        self.try_record_daily_app_focus_duration(user_id, anonymous_id);
    }

    pub fn try_record_daily_app_focus_duration(
        &mut self,
        user_id: Option<String>,
        anonymous_id: String,
    ) {
        self.daily_app_focus_duration
            .try_record(user_id, anonymous_id);
    }

    pub fn record_app_blur(&mut self, user_id: Option<String>, anonymous_id: String) {
        let app_focus_duration =
            get_current_time().signed_duration_since(self.last_time_app_focused);
        self.daily_app_focus_duration
            .add_duration(app_focus_duration, user_id, anonymous_id);
    }
}

#[cfg(test)]
#[path = "app_focus_telemetry_tests.rs"]
mod tests;
