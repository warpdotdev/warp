use std::time::Duration;

use super::format_elapsed_seconds;

#[test]
fn pluralizes_seconds() {
    assert_eq!(format_elapsed_seconds(Duration::from_secs(0)), "0 seconds");
    assert_eq!(format_elapsed_seconds(Duration::from_secs(1)), "1 second");
    assert_eq!(
        format_elapsed_seconds(Duration::from_secs(15)),
        "15 seconds"
    );
}

#[test]
fn truncates_subsecond_precision() {
    assert_eq!(
        format_elapsed_seconds(Duration::from_millis(1999)),
        "1 second"
    );
}
