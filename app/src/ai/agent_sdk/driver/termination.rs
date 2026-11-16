//! Termination plumbing for [`AgentDriver`](super::AgentDriver): naming why a run stopped and
//! mediating platform-specific interrupts so a handoff snapshot can be saved before exit.

use std::{fmt, io};

use futures::future;
use warpui::r#async::executor::Background;

#[cfg(unix)]
mod unix;

/// An interrupt that aborts an in-progress agent run so a handoff snapshot can be saved before
/// the process exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Interrupt {
    #[cfg_attr(not(unix), expect(dead_code))]
    Terminate,
    #[cfg_attr(not(unix), expect(dead_code))]
    Interrupt,
}

impl fmt::Display for Interrupt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Terminate => "SIGTERM",
            Self::Interrupt => "SIGINT",
        })
    }
}

/// Why `run_internal` stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunEndCause {
    /// `run_internal` returned on its own (success or a non-interrupt error).
    Completed,
    /// `WARP_SANDBOX_DEADLINE` warning window fired.
    SandboxDeadline,
    /// A platform interrupt arrived while the run was still in progress.
    Signal(Interrupt),
}

/// Owns active platform interrupt registrations or remains inert when interrupt handling is
/// unsupported or could not be initialized.
pub(super) struct InterruptWatch {
    inner: InterruptWatchInner,
}

enum InterruptWatchInner {
    #[cfg(unix)]
    Active(unix::InterruptWatch),
    Noop,
}

impl InterruptWatch {
    /// Registers the platform interrupt handlers and starts watching for their delivery.
    pub(super) async fn register(background: &Background) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                inner: InterruptWatchInner::Active(
                    unix::InterruptWatch::register(background).await?,
                ),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = background;
            Ok(Self::noop())
        }
    }

    /// Creates a watch that never observes an interrupt.
    pub(super) fn noop() -> Self {
        Self {
            inner: InterruptWatchInner::Noop,
        }
    }

    /// Resolves with the first observed interrupt and remains pending on unsupported platforms.
    pub(super) async fn wait(&mut self) -> Interrupt {
        match &mut self.inner {
            #[cfg(unix)]
            InterruptWatchInner::Active(inner) => inner.wait().await,
            InterruptWatchInner::Noop => future::pending().await,
        }
    }

    /// Removes the platform interrupt handlers and stops watching for their delivery.
    pub(super) fn disarm(self) {
        match self.inner {
            #[cfg(unix)]
            InterruptWatchInner::Active(inner) => inner.disarm(),
            InterruptWatchInner::Noop => {}
        }
    }

    /// Performs the platform's default termination behavior for the observed interrupt.
    pub(super) fn terminate(self, interrupt: Interrupt) -> ! {
        match self.inner {
            #[cfg(unix)]
            InterruptWatchInner::Active(inner) => inner.terminate(interrupt),
            InterruptWatchInner::Noop => {
                let _ = interrupt;
                unreachable!("a no-op interrupt watch cannot deliver interrupts");
            }
        }
    }
}
