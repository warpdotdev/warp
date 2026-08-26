use std::any::Any;
use std::sync::Arc;

use async_channel::Sender;

use crate::event::Event as TerminalEvent;

type AppEvent = Box<dyn Any + Send>;
type SendTerminalEvent = Arc<dyn Fn(TerminalEvent) -> bool + Send + Sync>;
type SendAppEvent = Arc<dyn Fn(AppEvent) -> bool + Send + Sync>;
type IsTerminalEventQueueEmpty = Arc<dyn Fn() -> bool + Send + Sync>;

/// A wrapper struct that emits events which originate from the PTY event loop.
/// Instead of passing individual senders, we can pass through this struct
/// so that users have access to all of the senders in one nicely wrapped struct.
#[derive(Clone)]
pub struct ChannelEventListener {
    /// We have a dedicated channel for "wakeup"s because we throttle the receiver
    /// so that we can coalesce successive wakeup events during situations of high
    /// throughput (e.g. running `yes`).
    wakeups_tx: Sender<()>,
    send_terminal_event: SendTerminalEvent,
    send_app_event: SendAppEvent,
    #[cfg_attr(not(any(test, feature = "integration_tests")), allow(dead_code))]
    is_terminal_event_queue_empty: IsTerminalEventQueueEmpty,
    pty_reads_tx: async_broadcast::Sender<Arc<Vec<u8>>>,
}

impl ChannelEventListener {
    pub fn new<E>(
        wakeups_tx: Sender<()>,
        terminal_events_tx: Sender<E>,
        pty_reads_tx: async_broadcast::Sender<Arc<Vec<u8>>>,
    ) -> Self
    where
        E: From<TerminalEvent> + Send + 'static,
    {
        let runtime_tx = terminal_events_tx.clone();
        let send_terminal_event: SendTerminalEvent =
            Arc::new(move |event| runtime_tx.try_send(event.into()).is_ok());
        let app_tx = terminal_events_tx.clone();
        let send_app_event: SendAppEvent = Arc::new(move |event: AppEvent| {
            let Ok(event) = event.downcast::<E>() else {
                return false;
            };
            app_tx.try_send(*event).is_ok()
        });
        let is_terminal_event_queue_empty: IsTerminalEventQueueEmpty =
            Arc::new(move || terminal_events_tx.is_empty());
        ChannelEventListener {
            wakeups_tx,
            send_terminal_event,
            send_app_event,
            is_terminal_event_queue_empty,
            pty_reads_tx,
        }
    }

    #[cfg(any(test, feature = "integration_tests", feature = "test-util"))]
    pub fn are_any_events_pending(&self) -> bool {
        !self.wakeups_tx.is_empty()
            || !(self.is_terminal_event_queue_empty)()
            || !self.pty_reads_tx.is_empty()
    }

    pub fn send_wakeup_event(&self) {
        if let Err(e) = self.wakeups_tx.try_send(()) {
            log::warn!("Failed to send Wakeup event: {e:?}");
        }
    }

    pub fn send_terminal_event(&self, event: TerminalEvent) {
        if !(self.send_terminal_event)(event) {
            log::warn!("Failed to send terminal runtime event");
        }
    }

    pub fn send_app_event<E>(&self, event: E)
    where
        E: Send + 'static,
    {
        if !(self.send_app_event)(Box::new(event)) {
            log::warn!("Failed to send application terminal event");
        }
    }

    pub fn send_pty_read_event(&self, bytes: &[u8]) {
        // Don't bother sending the event if there aren't any
        // active receivers. This avoids an unnecessary allocation of the bytes vector.
        // Note that we don't simply close the sending side since receivers
        // might come alive at some point in the future.
        if self.pty_reads_tx.receiver_count() > 0
            && let Err(e) = self.pty_reads_tx.try_broadcast(Arc::new(bytes.to_vec()))
        {
            log::warn!("Failed to send pty read event: {e:?}");
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
mod testing;
#[cfg(any(test, feature = "test-util"))]
pub use testing::*;
