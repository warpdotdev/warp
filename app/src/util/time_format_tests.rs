use super::*;

#[test]
fn test_format_sigfigs() {
    assert_eq!(format_sigfigs(0.000456, 2,), "0.00046");
    assert_eq!(format_sigfigs(0.043256, 3,), "0.0433");
    assert_eq!(format_sigfigs(0.01, 2,), "0.010");
    assert_eq!(format_sigfigs(10., 3,), "10.0");
    assert_eq!(format_sigfigs(456.719, 4,), "456.7");
    assert_eq!(format_sigfigs(10., 2,), "10");
}

#[test]
fn test_human_readable_precise_duration() {
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(3)),
        "3 ms".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(10)),
        "10 ms".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(3141)),
        "3.14 sec".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(19961)),
        "20.0 sec".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::seconds(61)),
        "1.02 min".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::minutes(930)),
        "15.5 hours".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::hours(46)),
        "1.92 days".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::weeks(2)),
        ">1 week".to_owned()
    );
}

#[test]
fn format_elapsed_seconds_pluralizes_and_truncates() {
    assert_eq!(
        format_elapsed_seconds(StdDuration::from_secs(0)),
        "0 seconds"
    );
    assert_eq!(
        format_elapsed_seconds(StdDuration::from_secs(1)),
        "1 second"
    );
    assert_eq!(
        format_elapsed_seconds(StdDuration::from_secs(15)),
        "15 seconds"
    );
    // Subsecond precision is truncated, not rounded.
    assert_eq!(
        format_elapsed_seconds(StdDuration::from_millis(1999)),
        "1 second"
    );
}

#[test]
fn test_human_readable_approx_duration() {
    assert_eq!(
        human_readable_approx_duration(Duration::milliseconds(2), false),
        "just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(2), false),
        "just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::milliseconds(2), true),
        "Just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(2), true),
        "Just now".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(90), false),
        "1 min ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::minutes(100), false),
        "1 hour ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::minutes(130), false),
        "2 hours ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::days(4), false),
        "4 days ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(1), false),
        "1 week ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(15), false),
        "3 months ago".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(520), false),
        "9 years ago".to_owned()
    );
}
