use std::os::fd::RawFd;
use std::time::Duration;

use nix::Result;
use nix::sys::termios::{self, Termios};
use warpui::{Entity, ModelContext};

/// The default amount of time we wait before polling the terminal attributes again.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// A model that polls the terminal attributes of a given tty via the termios interface
/// and produces an event whenever a query successfully completes.
/// The poller can be start and stopped.
pub struct TerminalAttributesPoller {
    /// The file descriptor identifying the terminal to query.
    fd: RawFd,
    /// Simple counter to ensure there is exactly one poll running at a time, eventually.
    poll_epoch: usize,
}

pub enum Event {
    TermiosQueryFinished { termios: Termios },
}

impl Entity for TerminalAttributesPoller {
    type Event = Event;
}

impl TerminalAttributesPoller {
    pub fn new(fd: RawFd) -> Self {
        Self { fd, poll_epoch: 0 }
    }

    /// If there is already a poll running, calling `start_polling`
    /// will effectively restart it, ultimately resulting in one single poll.
    pub fn start_polling(&mut self, ctx: &mut ModelContext<Self>) {
        self.poll_epoch += 1;
        self.poll_terminal_attributes(self.poll_epoch, ctx);
    }

    pub fn stop_polling(&mut self) {
        self.poll_epoch += 1;
    }

    fn poll_terminal_attributes(&mut self, poll_epoch: usize, ctx: &mut ModelContext<Self>) {
        if poll_epoch != self.poll_epoch {
            return;
        }

        // TODO: consider using Timer::interval() here to implement the interval-based polling.
        let fd = self.fd;
        ctx.spawn(
            async move {
                warpui::r#async::Timer::after(POLL_INTERVAL).await;
                fetch_termial_attributes(fd)
            },
            move |me, termios, ctx| {
                if let Ok(termios) = termios {
                    ctx.emit(Event::TermiosQueryFinished { termios });
                }
                me.poll_terminal_attributes(poll_epoch, ctx);
            },
        );
    }
}

/// Queries the terminal attributes via the `termios` API.
fn fetch_termial_attributes(fd: RawFd) -> Result<Termios> {
    termios::tcgetattr(fd)
}
