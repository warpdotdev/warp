use std::time::Duration;

use futures::stream::AbortHandle;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext};

const DEFAULT_PING_FREQUENCY: Duration = Duration::from_secs(5);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// A simple heartbeat mechanism to trigger ping notifications
/// on some cadence and to maintain a idle timer.
pub struct Heartbeat {
    /// How often we want to trigger a [`Event::SendPing`] event.
    ping_frequency: Duration,

    /// The duration that we want to wait before firing a [`Event::Idle`] event.
    /// To extend the timer by this duration, use [`Self::reset_idle_timeout`].
    idle_timeout: Duration,

    idle_timeout_abort_handle: Option<AbortHandle>,
    periodic_ping_abort_handle: Option<AbortHandle>,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self {
            idle_timeout_abort_handle: None,
            periodic_ping_abort_handle: None,
            ping_frequency: DEFAULT_PING_FREQUENCY,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

impl Heartbeat {
    pub fn with_idle_timeout(mut self, idle_timeout: Duration) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    pub fn with_ping_frequency(mut self, ping_frequency: Duration) -> Self {
        self.ping_frequency = ping_frequency;
        self
    }

    /// Starts the periodic ping and the idle timeout tracker.
    pub fn start(&mut self, ctx: &mut ModelContext<Self>) {
        self.reset_idle_timeout(ctx);
        self.periodic_ping(ctx);
    }
    /// Cancels periodic pings and the idle timeout for a closed websocket attempt.
    pub fn stop(&mut self) {
        if let Some(handle) = self.idle_timeout_abort_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.periodic_ping_abort_handle.take() {
            handle.abort();
        }
    }

    /// Resets the idle timeout to expire after [`Self::idle_timeout`] from now.
    pub fn reset_idle_timeout(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.idle_timeout_abort_handle.take() {
            handle.abort();
        }

        let idle_timeout = self.idle_timeout;
        let handle = ctx.spawn(
            async move { Timer::after(idle_timeout).await },
            |me, _, ctx| {
                // If the heartbeat has become idle, then don't ping anymore.
                me.stop();
                ctx.emit(Event::Idle);
            },
        );
        self.idle_timeout_abort_handle = Some(handle.abort_handle());
    }

    /// Emits a [`Event::SendPing`] event based on [`Self::ping_frequency`].
    fn periodic_ping(&mut self, ctx: &mut ModelContext<Self>) {
        // TODO: this would be simpler with a `spawn_stream_local`
        // if our async timer supported an [`interval` API](https://docs.rs/async-io/latest/async_io/struct.Timer.html#method.interval).
        let ping_frequency = self.ping_frequency;
        let handle = ctx.spawn(
            async move { Timer::after(ping_frequency).await },
            |me, _, ctx| {
                ctx.emit(Event::Ping);
                me.periodic_ping(ctx);
            },
        );
        self.periodic_ping_abort_handle = Some(handle.abort_handle());
    }
}

pub enum Event {
    Ping,
    Idle,
}

impl Entity for Heartbeat {
    type Event = Event;
}

#[cfg(test)]
#[path = "heartbeat_tests.rs"]
mod tests;
