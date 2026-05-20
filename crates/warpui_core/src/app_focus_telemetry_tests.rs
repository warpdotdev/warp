use crate::app_focus_telemetry::AppFocusInfo;
use crate::time::test_offset_time;
use chrono::Duration;

#[test]
fn test_daily_app_focus_duration_increase() {
    let mut app_focus_info = AppFocusInfo::new();
    let user_id = Some("user123".to_string());
    let anonymous_id = "anon-user-xyz".to_string();

    // When app blurs, the daily focus duration increases if date is the same
    let focus_duration_0 = app_focus_info.daily_app_focus_duration.duration;
    let last_synced_date_0 = app_focus_info
        .daily_app_focus_duration
        .last_synced_time
        .date_naive();
    app_focus_info.record_app_focus(user_id.clone(), anonymous_id.clone());
    test_offset_time(10);
    app_focus_info.record_app_blur(user_id.clone(), anonymous_id.clone());
    let focus_duration_1 = app_focus_info.daily_app_focus_duration.duration;
    let last_synced_date_1 = app_focus_info
        .daily_app_focus_duration
        .last_synced_time
        .date_naive();
    assert_eq!(focus_duration_1 - focus_duration_0, Duration::seconds(10));
    assert_eq!(last_synced_date_1, last_synced_date_0);

    // If date is the next day, the running total would be counted for the new day
    app_focus_info.record_app_focus(user_id.clone(), anonymous_id.clone());
    let one_day_seconds = 24 * 60 * 60;
    test_offset_time(one_day_seconds);
    app_focus_info.record_app_blur(user_id, anonymous_id);
    let focus_duration_2 = app_focus_info.daily_app_focus_duration.duration;
    let last_synced_date_2 = app_focus_info
        .daily_app_focus_duration
        .last_synced_time
        .date_naive();
    assert_eq!(focus_duration_2, Duration::seconds(one_day_seconds));
    assert_eq!(last_synced_date_2 - last_synced_date_1, Duration::days(1));
}
