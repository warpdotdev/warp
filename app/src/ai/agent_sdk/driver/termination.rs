//! Termination plumbing for [`AgentDriver`](super::AgentDriver): naming the reason a run
//! stopped, and watching for the Unix interrupts that abort an in-progress run so a
//! handoff snapshot can be saved before the process dies.

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::AtomicBool;

#[cfg(unix)]
use futures::StreamExt as _;
#[cfg(unix)]
use futures::channel::oneshot;
#[cfg(unix)]
use signal_hook::consts::{SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::{SigId, flag};
#[cfg(unix)]
use signal_hook_tokio::Signals;
#[cfg(unix)]
use warpui::r#async::executor::{Background, BackgroundTask};

/// Unix signal that aborts an in-progress agent run so a handoff snapshot can be
/// saved before the default terminate disposition is restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InterruptSignal {
    /// SIGTERM from instance teardown, container stop, or worker termination.
    #[cfg_attr(not(unix), allow(dead_code))]
    Term,
    /// SIGINT from Ctrl-C.
    #[cfg_attr(not(unix), allow(dead_code))]
    Int,
}

#[cfg(unix)]
impl InterruptSignal {
    fn as_raw(self) -> libc::c_int {
        match self {
            Self::Term => SIGTERM,
            Self::Int => SIGINT,
        }
    }

    fn from_raw(raw: libc::c_int) -> Option<Self> {
        match raw {
            SIGTERM => Some(Self::Term),
            SIGINT => Some(Self::Int),
            _ => None,
        }
    }
}

/// Why `run_internal` stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunEndCause {
    /// `run_internal` returned on its own (success or a non-signal error).
    Completed,
    /// `WARP_SANDBOX_DEADLINE` warning window fired.
    SandboxDeadline,
    /// A Unix interrupt arrived while the run was still in progress.
    Signal(InterruptSignal),
}

/// The interrupt handlers registered by [`watch_interrupt_signals`], plus the background
/// task waiting on their delivery.
///
/// Torn down through [`unregister`](Self::unregister) when the run ends on its own. On
/// the interrupt path the handlers deliberately stay registered instead, so a second
/// signal can still terminate the process if the handoff snapshot gets stuck.
#[cfg(unix)]
pub(super) struct InterruptWatch {
    sig_ids: Vec<SigId>,
    handle: signal_hook_tokio::Handle,
    task: BackgroundTask,
}

#[cfg(unix)]
impl InterruptWatch {
    pub(super) fn unregister(self) {
        self.handle.close();
        self.task.abort();
        for id in self.sig_ids {
            signal_hook::low_level::unregister(id);
        }
    }
}

/// Unregisters the handlers it holds when dropped, so a registration sequence that fails
/// partway through cannot leave handlers installed with nothing watching them.
#[cfg(unix)]
#[derive(Default)]
struct RegisteredHandlers(Vec<SigId>);

#[cfg(unix)]
impl RegisteredHandlers {
    fn push(&mut self, id: SigId) {
        self.0.push(id);
    }

    /// Hands the registrations to the caller, cancelling the unregister-on-drop.
    fn release(mut self) -> Vec<SigId> {
        std::mem::take(&mut self.0)
    }
}

#[cfg(unix)]
impl Drop for RegisteredHandlers {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

/// Registers SIGTERM/SIGINT handlers and starts watching for their delivery.
///
/// The returned receiver resolves with the first interrupt observed, or is cancelled if
/// the returned [`InterruptWatch`] is torn down first. Only the first delivery of either
/// signal is reported, so the run gets one chance to save a handoff snapshot; a second
/// delivery bypasses the watch entirely and emulates the default terminate action, so a
/// stuck snapshot cannot trap Ctrl-C or an instance teardown.
#[cfg(unix)]
pub(super) async fn watch_interrupt_signals(
    background: &Background,
) -> io::Result<(oneshot::Receiver<InterruptSignal>, InterruptWatch)> {
    let shutdown_armed = Arc::new(AtomicBool::new(false));
    let mut handlers = RegisteredHandlers::default();
    for signal in [SIGTERM, SIGINT] {
        // Registered before the flag-setting handler below so that the second delivery,
        // and only the second, sees `shutdown_armed` already set and terminates.
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
        // of them. The stream itself only ends once the watch is torn down, at which
        // point nothing is waiting on `signal_tx` any more.
        if let Some(signal) = signals.next().await.and_then(InterruptSignal::from_raw) {
            log::warn!("Received Unix signal {signal:?}");
            tracing::warn!(tags.cloud_agent = true, signal = ?signal, "received unix signal");
            let _ = signal_tx.send(signal);
        }
    });
    let handle = ready_rx
        .await
        .map_err(|_| io::Error::other("Unix signal watcher task did not start"))??;

    Ok((
        signal_rx,
        InterruptWatch {
            sig_ids: handlers.release(),
            handle,
            task,
        },
    ))
}

#[cfg(unix)]
pub(super) fn emulate_default_and_exit(signal: InterruptSignal) -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(signal.as_raw());
    signal_hook::low_level::abort();
}

#[cfg(all(test, unix))]
#[path = "termination_tests.rs"]
mod tests;
