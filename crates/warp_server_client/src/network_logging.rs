use std::fmt;

use bounded_vec_deque::BoundedVecDeque;
use chrono::{DateTime, FixedOffset};
use warp_core::report_error;
use warpui_core::{Entity, ModelContext, SingletonEntity};

/// Maximum number of network log items retained in memory. Matches the
/// previous file-rotation threshold so the pane surface behaves consistently
/// with historical expectations.
const NETWORK_LOGGING_MAX_ITEMS: usize = 50;

/// Upper bound on the bounded async channel between the HTTP client hooks and
/// the in-memory model. Keeps a small backlog to tolerate bursts without
/// blocking the request thread.
const NETWORK_LOGGING_MAX_QUEUE_SIZE: usize = 100;

/// In-memory store of the most recent network log items. Populated by
/// [`Self::install_on_clients`] and read by the network log pane. Holds at most
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

    /// Installs network logging hooks that listen for requests that pass
    /// through the provided HTTP clients and forward them to this model.
    ///
    /// The logging happens via an async channel so that request hooks never
    /// block on the main thread. Items are delivered to the model on the main
    /// thread through [`ModelContext::spawn_stream_local`].
    pub fn install_on_clients<'a>(
        &mut self,
        http_clients: impl IntoIterator<Item = &'a mut http_client::Client>,
        ctx: &mut ModelContext<Self>,
    ) {
        let (tx, rx) = async_channel::bounded::<NetworkLogItem>(NETWORK_LOGGING_MAX_QUEUE_SIZE);
        ctx.spawn_stream_local(rx, |model, item, ctx| model.push(item, ctx), |_, _| {});

        for client in http_clients {
            let request_tx = tx.clone();
            client.set_before_request_fn(Box::new(move |request, serialized_payload| {
                if !request_tx.is_closed()
                    && let Err(error) = request_tx.try_send(NetworkLogItem::request(
                        request,
                        serialized_payload.clone(),
                        chrono::Local::now().fixed_offset(),
                    ))
                {
                    report_error!(
                        anyhow::Error::new(error)
                            .context("Error sending request from HTTP client to logging task")
                    );
                }
            }));

            let response_tx = tx.clone();
            client.set_after_response_fn(Box::new(move |response| {
                if !response_tx.is_closed()
                    && let Err(error) = response_tx.try_send(NetworkLogItem::response(
                        response,
                        chrono::Local::now().fixed_offset(),
                    ))
                {
                    report_error!(
                        anyhow::Error::new(error)
                            .context("Error sending response from HTTP client to logging task")
                    );
                }
            }));
        }
    }

    /// Number of items currently retained. Exposed for tests.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.items.len()
    }
}

impl Entity for NetworkLogModel {
    type Event = ();
}

impl SingletonEntity for NetworkLogModel {}

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
