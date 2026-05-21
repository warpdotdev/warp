use std::fmt;

use bounded_vec_deque::BoundedVecDeque;
use chrono::{DateTime, FixedOffset};
use enclose::enclose;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::server::datetime_ext::DateTimeExt;
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

/// Human-friendly snapshot plus a plain-text export for copying/sharing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkLogSnapshot {
    pub display_text: String,
    pub plain_text: String,
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

    /// Returns both a human-friendly snapshot for the pane and a plain-text
    /// export suitable for copying into another editor.
    pub fn snapshot(&self) -> NetworkLogSnapshot {
        if self.items.is_empty() {
            return NetworkLogSnapshot {
                display_text: String::new(),
                plain_text: String::new(),
            };
        }

        let mut display_text = String::new();
        let mut plain_text = String::new();

        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                display_text.push_str("\n\n");
                plain_text.push_str("\n\n");
            }
            display_text.push_str(&item.display_text);
            plain_text.push_str(&item.plain_text);
        }

        NetworkLogSnapshot {
            display_text,
            plain_text,
        }
    }

    /// Returns the current display snapshot as a single string.
    pub fn snapshot_text(&self) -> String {
        self.snapshot().display_text
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
                    DateTime::now(),
                )) {
                    log::error!(
                        "Error sending request from http client to logging task: {e}"
                    );
                }
            }
        })));

        client.set_after_response_fn(Box::new(enclose!((tx) move |response| {
            if !tx.is_closed() {
                if let Err(e) = tx.try_send(NetworkLogItem::response(response, DateTime::now())) {
                    log::error!("Error sending request from http client to logging task: {e}");
                }
            }
        })));
    }
}

/// Represents an item (either a request or response) captured for the network
/// activity log.
#[derive(Clone, Debug)]
pub struct NetworkLogItem {
    display_text: String,
    plain_text: String,
}

impl NetworkLogItem {
    pub fn request(
        request: &reqwest::Request,
        serialized_payload: Option<String>,
        timestamp: DateTime<FixedOffset>,
    ) -> Self {
        let timestamp = timestamp.format("%Y-%m-%d %H:%M:%S,%3f").to_string();
        let request_debug = format!("{:?}", request);
        let body_suffix = serialized_payload
            .as_ref()
            .map_or(String::new(), |payload| format!("\nBody {payload}"));
        let plain_text = format!("[{timestamp}]: {request_debug}{body_suffix}");
        let display_text =
            format_request_for_display(&timestamp, request, serialized_payload.as_deref());
        Self {
            display_text,
            plain_text,
        }
    }

    pub fn response(response: &reqwest::Response, timestamp: DateTime<FixedOffset>) -> Self {
        let timestamp = timestamp.format("%Y-%m-%d %H:%M:%S,%3f").to_string();
        let response_debug = format!("{:?}", response);
        let plain_text = format!("[{timestamp}]: {response_debug}");
        let display_text = format_response_for_display(&timestamp, response);
        Self {
            display_text,
            plain_text,
        }
    }

    /// Constructs a log item directly from a pre-formatted string. Used in
    /// tests where we don't have a real `reqwest` request/response handy.
    #[cfg(test)]
    pub fn from_string(s: impl Into<String>) -> Self {
        let s = s.into();
        Self {
            display_text: s.clone(),
            plain_text: s,
        }
    }
}

impl fmt::Display for NetworkLogItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.plain_text)
    }
}

fn format_request_for_display(
    timestamp: &str,
    request: &reqwest::Request,
    serialized_payload: Option<&str>,
) -> String {
    let method = request.method();
    let url = request.url();
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let query = url
        .query()
        .map_or(String::new(), |query| format!("?{query}"));

    let mut lines = vec![
        format!("[{timestamp}] Request"),
        format!("Method: {method}"),
        format!("Host: {}", url.host_str().unwrap_or("")),
        format!("Path: {path}{query}"),
        format!("URL: {url}"),
    ];

    if let Some(payload) = serialized_payload {
        lines.push(format!("Body: {payload}"));
    }

    lines.join("\n")
}

fn format_response_for_display(timestamp: &str, response: &reqwest::Response) -> String {
    let status = response.status();
    let url = response.url();
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let query = url
        .query()
        .map_or(String::new(), |query| format!("?{query}"));

    [
        format!("[{timestamp}] Response"),
        format!("Status: {status}"),
        format!("Host: {}", url.host_str().unwrap_or("")),
        format!("Path: {path}{query}"),
        format!("URL: {url}"),
    ]
    .join("\n")
}

#[cfg(test)]
#[path = "network_logging_tests.rs"]
mod tests;
