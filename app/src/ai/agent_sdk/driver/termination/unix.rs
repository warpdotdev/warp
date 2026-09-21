use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use futures::channel::oneshot;
use futures::{StreamExt as _, future};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::{SigId, flag};
use signal_hook_tokio::Signals;
use warpui::r#async::executor::{Background, BackgroundTask};

use super::Interrupt;

/// Owns the Unix signal registrations and the background task waiting for their delivery.
pub(super) struct InterruptWatch {
    signal_rx: oneshot::Receiver<Interrupt>,
    _handlers: RegisteredHandlers,
    handle: signal_hook_tokio::Handle,
    task: BackgroundTask,
}

impl InterruptWatch {
    pub(super) async fn register(background: &Background) -> std::io::Result<Self> {
        let mut handlers = RegisteredHandlers::default();
        for signal in [SIGTERM, SIGINT] {
            let shutdown_armed = Arc::new(AtomicBool::new(false));
            // Registered before the flag-setting handler below so that the second delivery,
            // and only the second delivery of this signal, sees `shutdown_armed` already set
            // and terminates.
            handlers.push(flag::register_conditional_default(
                signal,
                Arc::clone(&shutdown_armed),
            )?);
            handlers.push(flag::register(signal, Arc::clone(&shutdown_armed))?);
        }

        // `Signals` registers with the Tokio reactor, so it has to be built and polled on the
        // background runtime rather than on the driver's own executor.
        let (ready_tx, ready_rx) = oneshot::channel();
        let (signal_tx, signal_rx) = oneshot::channel();
        let task = background.spawn(async move {
            let mut signals = match Signals::new([SIGTERM, SIGINT]) {
                Ok(signals) => signals,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            if ready_tx.send(Ok(signals.handle())).is_err() {
                return;
            }
            // Only the signals registered above are delivered, so the first item maps to one
            // of them. The stream itself only ends once the watch is disarmed, at which point
            // nothing is waiting on `signal_tx` any more.
            if let Some(signal) = signals.next().await.and_then(Interrupt::from_raw) {
                log::warn!("Received Unix signal {signal:?}");
                tracing::warn!(tags.cloud_agent = true, signal = ?signal, "received unix signal");
                let _ = signal_tx.send(signal);
            }
        });
        let handle = ready_rx
            .await
            .map_err(|_| std::io::Error::other("Unix signal watcher task did not start"))??;

        Ok(Self {
            signal_rx,
            _handlers: handlers,
            handle,
            task,
        })
    }

    pub(super) async fn wait(&mut self) -> Interrupt {
        match (&mut self.signal_rx).await {
            Ok(interrupt) => interrupt,
            Err(_) => future::pending().await,
        }
    }

    pub(super) fn disarm(self) {
        self.handle.close();
        self.task.abort();
        // Dropping self._handlers will unregister the fallback signal
        // handlers for double ctrl-c.
    }

    pub(super) fn terminate(self, interrupt: Interrupt) -> ! {
        let _watch = self;
        let _ = signal_hook::low_level::emulate_default_handler(interrupt.as_raw());
        signal_hook::low_level::abort();
    }
}

impl Interrupt {
    fn as_raw(self) -> libc::c_int {
        match self {
            Self::Terminate => SIGTERM,
            Self::Interrupt => SIGINT,
        }
    }

    fn from_raw(raw: libc::c_int) -> Option<Self> {
        match raw {
            SIGTERM => Some(Self::Terminate),
            SIGINT => Some(Self::Interrupt),
            _ => None,
        }
    }
}

/// Unregisters the handlers it holds when dropped, so a registration sequence that fails
/// partway through cannot leave handlers installed with nothing watching them.
#[derive(Default)]
struct RegisteredHandlers(Vec<SigId>);

impl RegisteredHandlers {
    fn push(&mut self, id: SigId) {
        self.0.push(id);
    }
}

impl Drop for RegisteredHandlers {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;
