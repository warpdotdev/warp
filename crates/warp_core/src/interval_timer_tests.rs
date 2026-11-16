use std::thread;
use std::time::Duration;

use super::*;

#[test]
fn test_timing_info() {
    let mut timer = IntervalTimer::new();
    let ten_ms = Duration::from_millis(10);
    thread::sleep(ten_ms);
    timer.mark_interval_end("a");
    thread::sleep(ten_ms);
    timer.mark_interval_end("b");

    let stats = timer.compute_stats();
    assert_eq!(stats.len(), 2);

    assert_eq!(stats[0].name, "a");
    assert_eq!(
        stats[0].cumulative_duration_ms,
        stats[0].marginal_duration_ms
    );
    assert!(stats[0].marginal_duration_ms >= 10);

    assert_eq!(stats[1].name, "b");
    assert_eq!(
        stats[1].cumulative_duration_ms,
        stats[1].marginal_duration_ms + stats[0].marginal_duration_ms
    );
    assert!(stats[1].marginal_duration_ms >= 10);
}
