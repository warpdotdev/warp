// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! The main event loop which performs I/O on the pseudoterminal.

use std::{
    borrow::Cow,
    collections::VecDeque,
    io::{self, ErrorKind, Read, Write},
    marker::Send,
    ops::DerefMut,
    sync::Arc,
    thread::{self, JoinHandle},
};

use log::error;
use mio::{self, Events, Interest};
use parking_lot::{FairMutex, FairMutexGuard};

use crate::terminal::{
    event_listener::ChannelEventListener, local_tty, model::ansi, TerminalModel,
};
use crate::terminal::{model::terminal_model::ExitReason, writeable_pty::Message};

use super::mio_channel::Receiver;

/// The size of the buffer to read data into from the PTY.
const READ_BUFFER_SIZE: usize = 0x4_0000;

/// Max bytes to process from the PTY while holding the lock before giving
/// someone else an opportunity to lock it.
const MAX_LOCKED_READ: usize = 0x1_0000;

pub const CHANNEL_TOKEN: mio::Token = mio::Token(0);
pub const PTY_TOKEN: mio::Token = mio::Token(1);
pub const SIGNALS_TOKEN: mio::Token = mio::Token(2);

/// The main event!.. loop.
///
/// Handles all the PTY I/O and runs the PTY parser which updates terminal
/// state.
pub struct EventLoop<T: local_tty::EventedPty> {
    poll: mio::Poll,
    pty: T,
    rx: Receiver<Message>,
    terminal: Arc<FairMutex<TerminalModel>>,

    /// The event listener is available to the PTY event loop
    /// to emit relevant events to subscribers. The ansi handler
    /// also has a handle to the event listener, so events may also
    /// be emitted at a later stage (i.e. when we have a better idea
    /// of what the bytes from the PTY actually meant).
    event_listener: ChannelEventListener,
}

/// Helper type which tracks how much of a buffer has been written.
struct Writing {
    source: Cow<'static, [u8]>,
    written: usize,
}

/// All of the mutable state needed to run the event loop.
///
/// Contains list of items to write, current write state, etc. Anything that
/// would otherwise be mutated on the `EventLoop` goes here.
pub struct State {
    write_list: VecDeque<Cow<'static, [u8]>>,
    writing: Option<Writing>,
    parser: ansi::Processor,
}

impl Default for State {
    fn default() -> State {
        State {
            write_list: VecDeque::new(),
            parser: ansi::Processor::new(),
            writing: None,
        }
    }
}

impl State {
    #[inline]
    fn ensure_next(&mut self) {
        if self.writing.is_none() {
            self.goto_next();
        }
    }

    #[inline]
    fn goto_next(&mut self) {
        self.writing = self.write_list.pop_front().map(Writing::new);
    }

    #[inline]
    fn take_current(&mut self) -> Option<Writing> {
        self.writing.take()
    }

    #[inline]
    fn needs_write(&self) -> bool {
        self.writing.is_some() || !self.write_list.is_empty()
    }

    #[inline]
    fn set_current(&mut self, new: Option<Writing>) {
        self.writing = new;
    }
}

impl Writing {
    #[inline]
    fn new(c: Cow<'static, [u8]>) -> Writing {
        Writing {
            source: c,
            written: 0,
        }
    }

    #[inline]
    fn advance(&mut self, n: usize) {
        self.written += n;
    }

    #[inline]
    fn remaining_bytes(&self) -> &[u8] {
        &self.source[self.written..]
    }

    #[inline]
    fn finished(&self) -> bool {
        self.written >= self.source.len()
    }
}

enum ChannelResult {
    Continue,
    TerminateLoop { child_exited: bool },
}

impl<T> EventLoop<T>
where
    T: local_tty::EventedPty + Send + 'static,
{
    /// Create a new event loop.
    pub fn new(
        terminal: Arc<FairMutex<TerminalModel>>,
        event_listener: ChannelEventListener,
        pty: T,
        rx: Receiver<Message>,
    ) -> EventLoop<T> {
        EventLoop {
            poll: mio::Poll::new().expect("create mio Poll"),
            pty,
            rx,
            terminal,
            event_listener,
        }
    }

    /// Drain the channel.
    ///
    /// Returns `false` when a shutdown message was received.
    fn drain_recv_channel(&mut self, state: &mut State) -> ChannelResult {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Message::Input(input) => state.write_list.push_back(input),
                Message::Shutdown => {
                    return ChannelResult::TerminateLoop {
                        child_exited: false,
                    }
                }
                Message::Resize(size) => self.pty.on_resize(&size),
                Message::ChildExited => return ChannelResult::TerminateLoop { child_exited: true },
            }
        }

        ChannelResult::Continue
    }

    /// Returns a `bool` indicating whether or not the event loop should continue running.
    #[inline]
    fn channel_event(&mut self, state: &mut State) -> ChannelResult {
        self.drain_recv_channel(state)
    }

    /// Reads from the pty into the provided buffer, using the provided state
    /// information in order to properly advance the ANSI parser.
    ///
    /// If `writer` is `Some`, a copy of all bytes read will be written to that
    /// writer.
    ///
    /// Returns the number of bytes read from the PTY.
    #[inline]
    #[allow(clippy::unwrap_in_result)]
    fn pty_read(
        &mut self,
        state: &mut State,
        buf: &mut [u8],
        can_read: &mut bool,
    ) -> io::Result<()> {
        let mut bytes_in_buffer = 0;
        let mut bytes_processed = 0;

        let mut terminal = None;

        // We read up to sizeof(buf) to limit the amount of time spent
        // reading from the PTY for a given event. Currently, the buf
        // has size [`MAX_READ`].
        loop {
            match self.pty.reader().read(&mut buf[bytes_in_buffer..]) {
                Ok(0) if bytes_in_buffer == 0 => {
                    // If we get 0 here with an empty buffer (guaranteed if
                    // bytes_in_buffer == 0), it means the object is unable to
                    // receive reads.
                    *can_read = false;
                    // There is nothing to be processed in the buffer, so return
                    // to the event loop.
                    break;
                }
                // Otherwise, track how many additional bytes we read and move
                // on to byte processing.
                Ok(got) => bytes_in_buffer += got,
                Err(err) => match err.kind() {
                    ErrorKind::Interrupted | ErrorKind::WouldBlock => {
                        if err.kind() == ErrorKind::WouldBlock {
                            *can_read = false;
                        }
                        if bytes_in_buffer == 0 {
                            break;
                        }
                    }
                    _ => return Err(err),
                },
            }

            let terminal = match &mut terminal {
                Some(terminal) => terminal,
                None => terminal.insert(match self.terminal.try_lock() {
                    // If we've filled up the buffer, block on locking the terminal.
                    None if bytes_in_buffer >= READ_BUFFER_SIZE => self.terminal.lock(),
                    // Otherwise, if we failed to acquire the lock, try to read more
                    // data into the buffer.
                    None => continue,
                    // Finally, if we acquired the lock, make use of it.
                    Some(terminal) => terminal,
                }),
            };

            // Process the bytes read into the buffer.
            state.parser.parse_bytes(
                terminal.deref_mut(),
                &buf[..bytes_in_buffer],
                &mut self.pty.writer(),
            );

            bytes_processed += bytes_in_buffer;
            bytes_in_buffer = 0;

            if bytes_processed >= MAX_LOCKED_READ {
                break;
            }

            // Give up the lock to a waiting thread, if any, before reading
            // more bytes from the PTY.
            FairMutexGuard::bump(terminal);
        }

        // Queue a terminal redraw if we processed some number
        // of non-(synchronized output) bytes.
        if bytes_processed > state.parser.sync_output_buffer_len().unwrap_or(0) {
            self.event_listener.send_wakeup_event();
        }

        Ok(())
    }

    #[inline]
    fn pty_write(&mut self, state: &mut State, can_write: &mut bool) -> io::Result<()> {
        state.ensure_next();

        'write_many: while let Some(mut current) = state.take_current() {
            'write_one: loop {
                match self.pty.writer().write(current.remaining_bytes()) {
                    Ok(0) => {
                        state.set_current(Some(current));
                        // We never attempt to write an empty buffer, so if we
                        // get 0 here, it means the object is unable to receive
                        // writes.
                        *can_write = false;
                        break 'write_many;
                    }
                    Ok(n) => {
                        current.advance(n);
                        if current.finished() {
                            state.goto_next();
                            break 'write_one;
                        }
                    }
                    Err(err) => {
                        state.set_current(Some(current));
                        match err.kind() {
                            ErrorKind::Interrupted | ErrorKind::WouldBlock => {
                                if err.kind() == ErrorKind::WouldBlock {
                                    *can_write = false;
                                }
                                break 'write_many;
                            }
                            _ => return Err(err),
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn spawn(mut self) -> JoinHandle<()> {
        #[cfg(test)]
        let feature_flag_overrides = warp_core::features::get_overrides();

        thread::Builder::new()
            .name("PTY reader".into())
            .spawn(move || {
                // Make sure any overridden feature flags are also overridden
                // in the PTY reader thread.
                #[cfg(test)]
                warp_core::features::set_overrides(feature_flag_overrides);

                let mut state = State::default();
                let mut buf = [0u8; READ_BUFFER_SIZE];

                // Keep track of whether we've "drained" read and write
                // readiness.  Once we receive a read or write readiness event,
                // we won't receive another until the operation would block.
                // These let us know whether we should keep processing reads
                // and writes, even without receiving a new readiness event.
                let mut can_read = false;
                let mut can_write = false;

                self.poll
                    .registry()
                    .register(&mut self.rx, CHANNEL_TOKEN, Interest::READABLE)
                    .unwrap();

                // Register TTY through EventedRW interface.
                self.pty
                    .register(&self.poll, Interest::READABLE | Interest::WRITABLE)
                    .unwrap();

                let mut events = Events::with_capacity(1024);

                // True if the child exiting caused the event loop to wind down
                // (e.g. CTRL D or `exit`) rather than the inverse.
                let mut child_exited = false;

                'event_loop: loop {
                    // Clear the events so that we can reliably equate the absence of events
                    // to the timeout being fired.
                    events.clear();

                    // Wait for events, but only up to the remaining timeout for the synchronous output
                    // update (if any).
                    let sync_state_timeout = state.parser.sync_output_remaining_timeout();
                    if let Err(err) = self.poll.poll(&mut events, sync_state_timeout) {
                        match err.kind() {
                            ErrorKind::Interrupted => continue,
                            _ => panic!("EventLoop polling error: {err:?}"),
                        }
                    }

                    // If there were no events but `poll` returned, that means we hit the timeout.
                    if events.is_empty() {
                        state
                            .parser
                            .finish_sync_output(&mut *self.terminal.lock(), &mut self.pty.writer());
                        continue;
                    }

                    for event in events.iter() {
                        match event.token() {
                            token if token == CHANNEL_TOKEN => {
                                match self.channel_event(&mut state) {
                                    ChannelResult::Continue => {}
                                    ChannelResult::TerminateLoop {
                                        child_exited: exited,
                                    } => {
                                        if exited {
                                            self.terminal
                                                .lock()
                                                .exit(ExitReason::ShellProcessExited);
                                            child_exited = true;
                                            self.event_listener.send_wakeup_event();
                                        }
                                        break 'event_loop;
                                    }
                                }
                            }

                            token if token == self.pty.child_event_token() => {
                                if let Some(local_tty::ChildEvent::Exited) =
                                    self.pty.next_child_event()
                                {
                                    self.terminal.lock().exit(ExitReason::ShellProcessExited);
                                    child_exited = true;
                                    self.event_listener.send_wakeup_event();
                                    break 'event_loop;
                                }
                            }

                            token
                                if token == self.pty.read_token()
                                    || token == self.pty.write_token() =>
                            {
                                #[cfg(unix)]
                                if event.is_read_closed() || event.is_write_closed() {
                                    // Don't try to do I/O on a dead PTY.
                                    continue;
                                }

                                if event.is_readable() {
                                    can_read = true;
                                }
                                if event.is_writable() {
                                    can_write = true;
                                }
                            }
                            _ => (),
                        }
                    }

                    // As long as we have work to do, do it.  Once we need to
                    // wait on some readiness (pty readability, pty writability,
                    // or new data to write), go back to the start of the event
                    // loop.
                    while can_read || (state.needs_write() && can_write) {
                        if can_read {
                            match self.pty_read(&mut state, &mut buf, &mut can_read) {
                                Ok(_) => {}
                                Err(err) => {
                                    // On Linux, a `read` on the master side of a PTY can fail
                                    // with `EIO` if the client side hangs up.  In that case,
                                    // just loop back round for the inevitable `Exited` event.
                                    // This sucks, but checking the process is either racy or
                                    // blocking.
                                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                                    if err.kind() == ErrorKind::Other {
                                        continue;
                                    }

                                    error!("Error reading from PTY in event loop: {err}");
                                    break 'event_loop;
                                }
                            }
                        }

                        if state.needs_write() && can_write {
                            if let Err(err) = self.pty_write(&mut state, &mut can_write) {
                                error!("Error writing to PTY in event loop: {err}");
                                break 'event_loop;
                            }
                        }
                    }
                }

                // The evented instances are not dropped here so deregister them explicitly.
                let _ = self.poll.registry().deregister(&mut self.rx);
                let _ = self.pty.deregister(&self.poll);

                // Terminate the PTY process, if it's not the initiator of the shutdown.
                if !child_exited {
                    let res = self.pty.kill();
                    if let Err(err) = res {
                        log::error!("Failed to kill PTY process {err:?}");
                    }
                }
                // Notify the terminal model that the PTY process has exited.
                self.terminal.lock().exit(ExitReason::PtyDisconnected);
            })
            .expect("thread spawn works")
    }
}
