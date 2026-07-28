use chrono::TimeZone as _;

use super::*;

fn parse(json: &str) -> ClaudeUsageSnapshot {
    serde_json::from_str::<UsageResponse>(json)
        .expect("usage response should parse")
        .into()
}

#[test]
fn test_parses_a_full_usage_response() {
    // Trimmed copy of a real response from the usage endpoint.
    let snapshot = parse(
        r#"{
            "five_hour": {
                "utilization": 23.0,
                "resets_at": "2026-07-25T13:49:59.320166+00:00"
            },
            "seven_day": {
                "utilization": 73.0,
                "resets_at": "2026-07-27T01:59:59.320200+00:00"
            },
            "seven_day_opus": null,
            "extra_usage": {
                "is_enabled": true,
                "monthly_limit": 5000.0,
                "used_credits": 1250.0,
                "utilization": 25.0,
                "currency": "USD"
            }
        }"#,
    );

    assert_eq!(23., snapshot.session_percent);
    assert_eq!(
        Some(
            Utc.with_ymd_and_hms(2026, 7, 25, 13, 49, 59)
                .unwrap()
                .timestamp()
        ),
        snapshot.session_resets_at.map(|at| at.timestamp())
    );
    assert_eq!(Some(73.), snapshot.weekly_percent);
    assert_eq!(
        Some(ClaudeExtraUsage {
            used_credits: 1250.,
            monthly_limit: 5000.,
        }),
        snapshot.extra_usage
    );
}

#[test]
fn test_parses_a_response_with_no_limits_reported() {
    let snapshot = parse(r#"{"five_hour": null, "seven_day": null, "extra_usage": null}"#);

    assert_eq!(0., snapshot.session_percent);
    assert_eq!(None, snapshot.session_resets_at);
    assert_eq!(None, snapshot.weekly_percent);
    assert_eq!(None, snapshot.extra_usage);
}

#[test]
fn test_parses_a_response_missing_optional_fields_entirely() {
    let snapshot = parse(r#"{"five_hour": {"utilization": 40.0}}"#);

    assert_eq!(40., snapshot.session_percent);
    assert_eq!(None, snapshot.weekly_percent);
}

#[test]
fn test_disabled_extra_usage_is_dropped() {
    let snapshot = parse(
        r#"{
            "extra_usage": {
                "is_enabled": false,
                "monthly_limit": 5000.0,
                "used_credits": 1250.0
            }
        }"#,
    );

    assert_eq!(None, snapshot.extra_usage);
}

#[test]
fn test_usage_levels_follow_the_display_thresholds() {
    assert_eq!(ClaudeUsageLevel::Normal, ClaudeUsageLevel::from_percent(0.));
    assert_eq!(
        ClaudeUsageLevel::Normal,
        ClaudeUsageLevel::from_percent(49.9)
    );
    assert_eq!(
        ClaudeUsageLevel::Elevated,
        ClaudeUsageLevel::from_percent(50.)
    );
    assert_eq!(ClaudeUsageLevel::High, ClaudeUsageLevel::from_percent(80.));
    assert_eq!(
        ClaudeUsageLevel::Critical,
        ClaudeUsageLevel::from_percent(95.)
    );
    assert_eq!(
        ClaudeUsageLevel::Critical,
        ClaudeUsageLevel::from_percent(120.)
    );
}

#[test]
fn test_percent_is_clamped_and_rounded_for_display() {
    let snapshot = |percent| ClaudeUsageSnapshot {
        session_percent: percent,
        session_resets_at: None,
        weekly_percent: None,
        extra_usage: None,
    };

    assert_eq!(23, snapshot(22.6).session_percent_rounded());
    assert_eq!(100, snapshot(140.).session_percent_rounded());
    assert_eq!(0, snapshot(-5.).session_percent_rounded());
}

#[test]
fn test_countdown_to_the_session_reset() {
    let now = Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap();
    let snapshot = |resets_at| ClaudeUsageSnapshot {
        session_percent: 10.,
        session_resets_at: Some(resets_at),
        weekly_percent: None,
        extra_usage: None,
    };

    assert_eq!(
        Some("2h 18m".to_string()),
        snapshot(now + chrono::TimeDelta::minutes(138)).time_until_session_reset(now)
    );
    assert_eq!(
        Some("45m".to_string()),
        snapshot(now + chrono::TimeDelta::minutes(45)).time_until_session_reset(now)
    );
    assert_eq!(
        Some("1d 2h".to_string()),
        snapshot(now + chrono::TimeDelta::hours(26)).time_until_session_reset(now)
    );
    // A reset time that has already passed reads as an imminent reset.
    assert_eq!(
        Some("now".to_string()),
        snapshot(now - chrono::TimeDelta::minutes(5)).time_until_session_reset(now)
    );
}
