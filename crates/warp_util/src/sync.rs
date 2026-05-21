//! Synchronization utilities.

use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use event_listener::Event;

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;

/// A set-once asynchronous condition variable.
///
/// Generally, a [condition variable](http://www.cs.cornell.edu/courses/cs3110/2012fa/recitations/rec16.html)
/// lets tasks wait until some condition becomes true (for example, we might want to wait for the
/// user to have logged in, or for the initial load of Warp Drive objects to have finished). When
/// the condition becomes true, one or all of the waiting tasks can wake up and do their work.
///
/// This [`Condition`] implementation models the simpler case where a condition becomes true and
/// is then *always* true (unless reset explicitly). This allows waiting for something to happen at
/// least once. If a task starts waiting before the condition is met, it will block, but if the
/// condition is already true, it continues immediately.
///
/// This is useful when regular UI framework events are not suitable because they do not tell us if
/// the event had *already* happened - a task that subscribed too late would block forever.
///
/// Also see [`std::sync::Condvar`].
#[derive(Debug, Clone)]
pub struct Condition {
    // This is more or less the reference example for event-listener:
    // https://github.com/smol-rs/event-listener
    flag: Arc<AtomicBool>,
    event: Arc<Event>,
}

impl Condition {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            event: Arc::new(Event::new()),
        }
    }

    /// Mark the condition as true.
    pub fn set(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.event.notify(usize::MAX);
    }

    /// Reset the condition to false so that future [`wait`](Self::wait) calls
    /// will block until [`set`](Self::set) is called again.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
    }

    /// Returns `true` if the condition has already been set.
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Asynchronously wait for the condition to be true.
    pub fn wait(&self) -> impl Future<Output = ()> {
        let flag = self.flag.clone();
        let event = self.event.clone();
        async move {
            // Loop in case of spurious wakeups.
            loop {
                // Check if the condition has already been set.
                if flag.load(Ordering::SeqCst) {
                    break;
                }

                let listener = event.listen();

                // Check the flag again after creating the listener, in case it was set while we
                // started listening.
                if flag.load(Ordering::SeqCst) {
                    break;
                }

                listener.await;
            }
        }
    }
}

impl Default for Condition {
    fn default() -> Self {
        Self::new()
    }
}
