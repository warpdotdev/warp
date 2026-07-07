use std::borrow::Cow;

use bounded_vec_deque::BoundedVecDeque;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::time::get_current_time;

const MAX_BUFFER_SIZE: usize = 1024;

/// The length of time between events used to determine a 'session' boundary. In other words, if
/// new_event occurs >SESSION_CONTINUATION_THRESHOLD_SECONDS after old_event, new_event is
/// associated with a new session.
pub(super) const SESSION_CONTINUATION_THRESHOLD_SECONDS: u64 = 5 * 60;

/// A data store for telemetry events. This is a thin wrapper around a [`BoundedVecDeque`] with
/// APIs for domain-specific APIs for recording events (appending to the deque).
pub(super) struct EventStore {
    // Bounded for now to save memory.
    // TODO: write to disk periodically
    pub(super) events: BoundedVecDeque<Event>,
    current_session_created_at: DateTime<Utc>,
    last_event_timestamp_seen: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct Event {
    /// The type of the event and its payload.
    pub payload: EventPayload,

    // We are using the session creation time as the identifier for the session.
    // Some metrics platforms (e.g. Amplitude) expect this.
    pub session_created_at: DateTime<Utc>,

    /// The time at which the event occurred.
    pub timestamp: DateTime<Utc>,

    /// Whether the event contains user-generated content.
    pub contains_ugc: bool,
}

/// Represents the type of telemetry event and its contents.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventPayload {
    IdentifyUser {
        user_id: String,
        anonymous_id: String,
    },
    AppActive {
        user_id: Option<String>,
        anonymous_id: String,
    },
    NamedEvent {
        user_id: Option<String>,
        anonymous_id: String,
        name: Cow<'static, str>,
        value: Option<Value>,
    },
}

impl EventStore {
    pub(super) fn new() -> Self {
        let initial_timestamp = get_current_time();
        Self {
            events: BoundedVecDeque::new(MAX_BUFFER_SIZE),
            current_session_created_at: initial_timestamp,
            last_event_timestamp_seen: initial_timestamp,
        }
    }

    // Register a named telemetry event
    // Create a new session if the session is stale
    pub(super) fn record_event(
        &mut self,
        user_id: Option<String>,
        anonymous_id: String,
        name: Cow<'static, str>,
        payload: Option<Value>,
        contains_ugc: bool,
        timestamp: DateTime<Utc>,
    ) {
        let event = self.create_event(
            user_id,
            anonymous_id,
            name,
            payload,
            contains_ugc,
            timestamp,
        );

        #[cfg(feature = "log_named_telemetry_events")]
        log::info!("Recorded telemetry event: {event:#?}");

        self.events.push_back(event);
    }

    // Register an Identify User telemetry event
    // Create a new session if the session is stale
    pub(super) fn record_identify_user_event(
        &mut self,
        user_id: String,
        anonymous_id: String,
        timestamp: DateTime<Utc>,
    ) {
        let session_created_at = if self.is_session_stale(timestamp) {
            self.current_session_created_at = timestamp;
            timestamp
        } else {
            self.current_session_created_at
        };
        self.last_event_timestamp_seen = self.last_event_timestamp_seen.max(timestamp);
        self.events.push_back(Event {
            session_created_at,
            payload: EventPayload::IdentifyUser {
                user_id,
                anonymous_id,
            },
            timestamp,
            contains_ugc: false,
        });
    }

    // Called every time app is active
    // If session is fresh and the last event on the queue is an App Active event, collapse them
    // Else, it behaves like `record_event`
    pub(super) fn record_app_active(
        &mut self,
        user_id: Option<String>,
        anonymous_id: String,
        timestamp: DateTime<Utc>,
    ) {
        if !self.is_session_stale(timestamp) {
            if let Some(last_event) = self.events.back_mut().filter(|event| {
                event.payload
                    == EventPayload::AppActive {
                        user_id: user_id.clone(),
                        anonymous_id: anonymous_id.clone(),
                    }
            }) {
                last_event.timestamp = timestamp;
                self.last_event_timestamp_seen = self.last_event_timestamp_seen.max(timestamp);
                return;
            }
        }

        let session_created_at = if self.is_session_stale(timestamp) {
            self.current_session_created_at = timestamp;
            timestamp
        } else {
            self.current_session_created_at
        };
        self.last_event_timestamp_seen = self.last_event_timestamp_seen.max(timestamp);
        self.events.push_back(Event {
            session_created_at,
            payload: EventPayload::AppActive {
                user_id,
                anonymous_id,
            },
            timestamp,
            contains_ugc: false,
        });
    }

    /// Returns a newly [`Event`], while also updating `Self::last_event_timestamp_seen` and
    /// `Self::current_session_created_at`, if necessary.
    pub(super) fn create_event(
        &mut self,
        user_id: Option<String>,
        anonymous_id: String,
        name: Cow<'static, str>,
        payload: Option<Value>,
        contains_ugc: bool,
        timestamp: DateTime<Utc>,
    ) -> Event {
        let session_created_at = if self.is_session_stale(timestamp) {
            self.current_session_created_at = timestamp;
            timestamp
        } else {
            self.current_session_created_at
        };
        self.last_event_timestamp_seen = self.last_event_timestamp_seen.max(timestamp);
        Event {
            session_created_at,
            payload: EventPayload::NamedEvent {
                user_id,
                anonymous_id,
                name,
                value: payload,
            },
            timestamp,
            contains_ugc,
        }
    }

    fn is_session_stale(&self, now: DateTime<Utc>) -> bool {
        let session_freshness_threshold = self.last_event_timestamp_seen
            + Duration::seconds(SESSION_CONTINUATION_THRESHOLD_SECONDS as i64);
        now > session_freshness_threshold
    }
}

#[cfg(test)]
#[path = "event_store_tests.rs"]
mod tests;
