#[cfg(test)]
use std::sync::atomic::{AtomicI64, Ordering};

#[cfg(test)]
use chrono::TimeZone;
use chrono::{DateTime, Utc};

#[cfg(not(test))]
pub fn get_current_time() -> DateTime<Utc> {
    Utc::now()
}

thread_local! {
    #[cfg(test)]
    static CURRENT_SECS: AtomicI64 = const { AtomicI64::new(0) };
}
#[cfg(test)]
pub fn get_current_time() -> DateTime<Utc> {
    CURRENT_SECS.with(|current_secs| {
        Utc.timestamp_opt(current_secs.load(Ordering::SeqCst), 0)
            .unwrap()
    })
}
#[cfg(test)]
pub fn test_offset_time(offset_secs: i64) {
    CURRENT_SECS.with(|current_secs| {
        current_secs.fetch_add(offset_secs, Ordering::SeqCst);
    })
}
