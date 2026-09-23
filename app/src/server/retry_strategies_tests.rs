use futures::executor::block_on;

use super::*;

#[test]
fn with_retry_succeeds_on_first_attempt() {
    let mut attempts = 0usize;
    let result: Result<u32, &str> = block_on(with_retry(
        "test operation",
        || {
            attempts += 1;
            std::future::ready(Ok(attempts as u32))
        },
        |_err: &&str| true,
        |_delay| std::future::ready(()),
        |_attempts_made| Some(Duration::from_millis(1)),
    ));

    assert_eq!(result, Ok(1));
    assert_eq!(attempts, 1);
}

#[test]
fn with_retry_retries_transient_failures_on_backoff_schedule_then_succeeds() {
    let backoff_schedule = [Duration::from_millis(1), Duration::from_millis(2)];
    let mut attempts = 0usize;
    let mut delays_used = Vec::new();
    let result: Result<u32, &str> = block_on(with_retry(
        "test operation",
        || {
            attempts += 1;
            std::future::ready(if attempts < 3 {
                Err("transient failure")
            } else {
                Ok(attempts as u32)
            })
        },
        |_err: &&str| true,
        |delay| {
            delays_used.push(delay);
            std::future::ready(())
        },
        |attempts_made| backoff_schedule.get(attempts_made).copied(),
    ));

    assert_eq!(result, Ok(3));
    assert_eq!(attempts, 3);
    assert_eq!(delays_used, backoff_schedule);
}

#[test]
fn with_retry_stops_once_backoff_schedule_is_exhausted() {
    let backoff_schedule = [Duration::from_millis(1), Duration::from_millis(2)];
    let mut attempts = 0usize;
    let result: Result<(), &str> = block_on(with_retry(
        "test operation",
        || {
            attempts += 1;
            std::future::ready(Err("persistent failure"))
        },
        |_err: &&str| true,
        |_delay| std::future::ready(()),
        |attempts_made| backoff_schedule.get(attempts_made).copied(),
    ));

    assert_eq!(result, Err("persistent failure"));
    // One initial attempt plus one retry per scheduled backoff delay.
    assert_eq!(attempts, backoff_schedule.len() + 1);
}

#[test]
fn with_retry_fails_fast_when_error_is_not_retryable() {
    let mut attempts = 0usize;
    let result: Result<(), &str> = block_on(with_retry(
        "test operation",
        || {
            attempts += 1;
            std::future::ready(Err("permanent failure"))
        },
        |_err: &&str| false,
        |_delay| std::future::ready(()),
        |_attempts_made| Some(Duration::from_millis(1)),
    ));

    assert_eq!(result, Err("permanent failure"));
    assert_eq!(attempts, 1, "non-retryable errors must not be retried");
}
