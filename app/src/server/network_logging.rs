use std::fmt;

use bounded_vec_deque::BoundedVecDeque;
use chrono::{DateTime, FixedOffset};
use enclose::enclose;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::server::server_api::ServerApiProvider;

/// Maximum number of network log items retained in memory. Matches the
/// previous file-rotation threshold so the pane surface behaves consistently
/// with historical expectations.
const NETWORK_LOGGING_MAX_ITEMS: usize = 50;

/// Upper bound on the bounded async channel between the HTTP client hooks and
/// the in-memory model. Keeps a small backlog to tolerate bursts without
/// blocking the request thread.
const NETWORK_LOGGING_MAX_QUEUE_SIZE: usize = 100;

/// In-memory store of the most recent network log items. Populated by
/// [`init`] and read by the network log pane. Holds at most
/// [`NETWORK_LOGGING_MAX_ITEMS`] entries; older entries are dropped when new
/// ones arrive.
pub struct NetworkLogModel {
    items: BoundedVecDeque<NetworkLogItem>,
}

impl Default for NetworkLogModel {
    fn default() -> Self {
        Self {
            items: BoundedVecDeque::new(NETWORK_LOGGING_MAX_ITEMS),
        }
    }
}

impl NetworkLogModel {
    /// Appends a new log item, evicting the oldest if at capacity.
    pub fn push(&mut self, item: NetworkLogItem, ctx: &mut ModelContext<Self>) {
        // `BoundedVecDeque::push_back` returns the evicted item when the
        // store is at capacity; we discard it since the pane only needs the
        // most recent entries.
        let _evicted = self.items.push_back(item);
        ctx.notify();
    }

    /// Returns the current snapshot as a single string with one item per line,
    /// in chronological order. Returns an empty string when no items have been
    /// captured.
    pub fn snapshot_text(&self) -> String {
        let mut out = String::new();
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&item.0);
        }
        out
    }

    /// Number of items currently retained. Exposed for tests.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl Entity for NetworkLogModel {
    type Event = ();
}

impl SingletonEntity for NetworkLogModel {}

/// Initializes a network logging task that listens for requests that pass
/// through the provided HTTP clients and forwards them to the in-memory
/// [`NetworkLogModel`].
///
/// The logging happens via an async channel so that request hooks never block
/// on the main thread. Items are delivered to the model on the main thread via
/// [`ModelContext::spawn_stream_local`], mirroring how `ServerApiProvider`
/// consumes its own event stream.
pub(super) fn init<'a>(
    http_clients: impl IntoIterator<Item = &'a mut http_client::Client>,
    ctx: &mut ModelContext<ServerApiProvider>,
) {
    let (tx, rx) = async_channel::bounded::<NetworkLogItem>(NETWORK_LOGGING_MAX_QUEUE_SIZE);

    ctx.spawn_stream_local(
        rx,
        move |_, item, ctx| {
            NetworkLogModel::handle(ctx).update(ctx, |model, ctx| {
                model.push(item, ctx);
            });
        },
        |_, _| {},
    );

    for client in http_clients.into_iter() {
        client.set_before_request_fn(Box::new(enclose!((tx) move |request, serialized_payload| {
            if !tx.is_closed() {
                if let Err(e) = tx.try_send(NetworkLogItem::request(
                    request,
                    serialized_payload.clone(),
                    chrono::Local::now().fixed_offset(),
                )) {
                    log::error!(
                        "Error sending request from http client to logging task: {e}"
                    );
                }
            }
        })));

        client.set_after_response_fn(Box::new(enclose!((tx) move |response| {
            if !tx.is_closed() {
                if let Err(e) = tx.try_send(NetworkLogItem::response(response, chrono::Local::now().fixed_offset())) {
                    log::error!("Error sending request from http client to logging task: {e}");
                }
            }
        })));
    }
}

/// Represents an item (either a request or response) captured for the network
/// activity log. The inner string contains a timestamp and the
/// [`Debug`]-formatted representation of the request or response, matching the
/// format previously written to `warp_network.log`.
#[derive(Clone, Debug)]
pub struct NetworkLogItem(String);

impl NetworkLogItem {
    pub fn request(
        request: &reqwest::Request,
        serialized_payload: Option<String>,
        timestamp: DateTime<FixedOffset>,
    ) -> Self {
        Self(format!(
            "[{}]: {:?}{}",
            timestamp.format("%Y-%m-%d %H:%M:%S,%3f"),
            request,
            serialized_payload.map_or("".to_owned(), |payload| format!("\nBody {payload}"))
        ))
    }

    pub fn response(response: &reqwest::Response, timestamp: DateTime<FixedOffset>) -> Self {
        Self(format!(
            "[{}]: {:?}",
            timestamp.format("%Y-%m-%d %H:%M:%S,%3f"),
            response
        ))
    }

    /// Constructs a log item directly from a pre-formatted string. Used in
    /// tests where we don't have a real `reqwest` request/response handy.
    #[cfg(test)]
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl fmt::Display for NetworkLogItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
#[path = "network_logging_tests.rs"]
mod tests;
