use std::future::Future;

use warp_util::sync::Condition;
use warpui::{Entity, ModelContext, SingletonEntity};

/// Represents whether the client is connected to the network.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum NetworkStatusKind {
    /// The client is successfully connected to the network.
    /// When the app starts, we assume the client is connected to the network.
    /// If they actually weren't, we'd emit a network changed event immediately.
    /// If they were indeed connected, we _wouldn't_ emit an event (because there isn't a new status).
    #[default]
    Online,

    /// The client is not connected to the network.
    Offline,
}

/// Model that tracks the client's network connectivity status.
///
/// This model emits `NetworkStatusEvent::NetworkStatusChanged` when the network status changes.
/// It also provides `wait_until_online()` to asynchronously wait for network connectivity.
pub struct NetworkStatus {
    status: NetworkStatusKind,
    /// A condition that resolves when the client regains connectivity.
    /// This is `Some` when offline, and `None` when online.
    pending_reconnect: Option<Condition>,
}

impl NetworkStatus {
    pub fn new() -> Self {
        Self {
            status: NetworkStatusKind::Online,
            pending_reconnect: None,
        }
    }

    /// Returns the current network status.
    pub fn status(&self) -> NetworkStatusKind {
        self.status
    }

    /// Returns true if the client is currently online.
    pub fn is_online(&self) -> bool {
        self.status == NetworkStatusKind::Online
    }

    pub fn reachability_changed(&mut self, reachable: bool, ctx: &mut ModelContext<Self>) {
        let old_status = self.status;
        let new_status = if reachable {
            NetworkStatusKind::Online
        } else {
            NetworkStatusKind::Offline
        };

        if old_status != new_status {
            self.status = new_status;

            match new_status {
                NetworkStatusKind::Online => {
                    // Wake up anyone waiting for reconnection.
                    if let Some(condition) = self.pending_reconnect.take() {
                        condition.set();
                    }
                }
                NetworkStatusKind::Offline => {
                    // Create a fresh condition for the next reconnect.
                    self.pending_reconnect = Some(Condition::new());
                }
            }

            ctx.emit(NetworkStatusEvent::NetworkStatusChanged { new_status });
        }
    }

    /// Returns a future that resolves immediately if online, or waits until the next online
    /// transition if currently offline.
    pub fn wait_until_online(&self) -> impl Future<Output = ()> {
        let condition = self.pending_reconnect.clone();
        async move {
            if let Some(cond) = condition {
                cond.wait().await;
            }
            // If None, we're already online — return immediately.
        }
    }
}

impl Default for NetworkStatus {
    fn default() -> Self {
        Self::new()
    }
}

pub enum NetworkStatusEvent {
    NetworkStatusChanged { new_status: NetworkStatusKind },
}

impl Entity for NetworkStatus {
    type Event = NetworkStatusEvent;
}

impl SingletonEntity for NetworkStatus {}
