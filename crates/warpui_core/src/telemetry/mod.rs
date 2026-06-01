mod event_store;

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use event_store::*;
pub use event_store::{Event, EventPayload};
use lazy_static::lazy_static;
use parking_lot::Mutex;
use serde_json::Value;

lazy_static! {
    static ref TELEMETRY: Mutex<EventStore> = Mutex::new(EventStore::new());
}

#[macro_export]
macro_rules! record_telemetry_from_ctx {
    ($user_id: expr, $anonymous_id: expr, $name:expr, $payload: expr, $contains_ugc: expr, $ctx: expr) => {{
        let timestamp = $crate::time::get_current_time();
        $ctx.background_executor()
            .spawn(async move {
                $crate::telemetry::record_event(
                    $user_id,
                    $anonymous_id,
                    $name,
                    $payload,
                    $contains_ugc,
                    timestamp,
                )
            })
            .detach();
    }};
}

#[macro_export]
macro_rules! record_telemetry_on_executor {
    ($user_id: expr, $anonymous_id: expr, $name:expr, $payload: expr, $contains_ugc: expr, $executor: expr) => {{
        let timestamp = $crate::time::get_current_time();
        let _ = $executor
            .spawn(async move {
                $crate::telemetry::record_event(
                    $user_id,
                    $anonymous_id,
                    $name,
                    $payload,
                    $contains_ugc,
                    timestamp,
                )
            })
            .detach();
    }};
}

/// Creates a new `Event`, but does not record it. It is up to the caller to determine when, and
/// how, the event should be recorded.
pub fn create_event(
    user_id: Option<String>,
    anonymous_id: String,
    name: Cow<'static, str>,
    payload: Option<Value>,
    contains_ugc: bool,
    timestamp: DateTime<Utc>,
) -> Event {
    let mut telemetry = TELEMETRY.lock();
    telemetry.create_event(
        user_id,
        anonymous_id,
        name,
        payload,
        contains_ugc,
        timestamp,
    )
}

pub fn record_event(
    user_id: Option<String>,
    anonymous_id: String,
    name: Cow<'static, str>,
    payload: Option<Value>,
    contains_ugc: bool,
    timestamp: DateTime<Utc>,
) {
    let mut telemetry = TELEMETRY.lock();
    telemetry.record_event(
        user_id,
        anonymous_id,
        name,
        payload,
        contains_ugc,
        timestamp,
    );
}

pub fn record_identify_user_event(user_id: String, anonymous_id: String, timestamp: DateTime<Utc>) {
    let mut telemetry = TELEMETRY.lock();
    telemetry.record_identify_user_event(user_id, anonymous_id, timestamp);
}

/// Adds a 'App Active' event to the global event queue.  This should only be called in an async
/// context.
pub fn record_app_active_event(
    user_id: Option<String>,
    anonymous_id: String,
    timestamp: DateTime<Utc>,
) {
    let mut telemetry = TELEMETRY.lock();
    telemetry.record_app_active(user_id, anonymous_id, timestamp);
}

pub fn flush_events() -> Vec<Event> {
    TELEMETRY.lock().events.drain(..).collect()
}
