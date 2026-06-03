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
        "3 毫秒".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(10)),
        "10 毫秒".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(3141)),
        "3.14 秒".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::milliseconds(19961)),
        "20.0 秒".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::seconds(61)),
        "1.02 分钟".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::minutes(930)),
        "15.5 小时".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::hours(46)),
        "1.92 天".to_owned()
    );
    assert_eq!(
        human_readable_precise_duration(Duration::weeks(2)),
        ">1 周".to_owned()
    );
}

#[test]
fn test_human_readable_approx_duration() {
    assert_eq!(
        human_readable_approx_duration(Duration::milliseconds(2), false),
        "刚刚".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(2), false),
        "刚刚".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::milliseconds(2), true),
        "刚刚".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(2), true),
        "刚刚".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::seconds(90), false),
        "1 分钟前".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::minutes(100), false),
        "1 小时前".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::minutes(130), false),
        "2 小时前".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::days(4), false),
        "4 天前".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(1), false),
        "1 周前".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(15), false),
        "3 个月前".to_owned()
    );
    assert_eq!(
        human_readable_approx_duration(Duration::weeks(520), false),
        "9 年前".to_owned()
    );
}
