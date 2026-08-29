use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::ops::DerefMut;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use parking_lot::{FairMutex, FairMutexGuard, Mutex};
use warp_core::SessionId;
use warp_terminal::shell::ShellType;

use crate::terminal::TerminalModel;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::event_loop::CHANNEL_TOKEN;
use crate::terminal::local_tty::mio_channel::{self, Receiver};
use crate::terminal::local_tty::{self, EventedPty};
use crate::terminal::model::ansi;
use crate::terminal::tmux::pane_bytes::{feed_control_bytes, notify_exit, sink_writer};
use crate::terminal::tmux::parser::PaneId;
use crate::terminal::tmux::protocol::{
    kill_server_command, refresh_client_command, send_keys_commands, silent_bootstrap_bytes,
    zsh_init_bytes,
};
use crate::terminal::writeable_pty::Message;
use crate::terminal::writeable_pty::pty_controller::{EventLoopSendError, EventLoopSender};

const READ_BUFFER_SIZE: usize = 0x4_0000;
const MAX_LOCKED_READ: usize = 0x1_0000;

/// Shared between the UI-thread sender and the control-client reader thread.
pub struct SharedControlState {
    pane_id: Mutex<Option<PaneId>>,
    pending_pane_writes: Mutex<Vec<Cow<'static, [u8]>>>,
    pending_control: Mutex<Vec<Cow<'static, [u8]>>>,
}

impl SharedControlState {
    pub fn new() -> Self {
        Self {
            pane_id: Mutex::new(None),
            pending_pane_writes: Mutex::new(Vec::new()),
            pending_control: Mutex::new(Vec::new()),
        }
    }

    #[cfg(test)]
    fn bind_pane(&self, pane_id: PaneId) {
        *self.pane_id.lock() = Some(pane_id);
    }
}

impl Default for SharedControlState {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts pane-bound PTY writes into control-mode commands on the tmux client stream.
#[derive(Clone)]
pub struct TmuxControlSender {
    inner: mio_channel::Sender<Message>,
    shared: Arc<SharedControlState>,
}

impl TmuxControlSender {
    pub fn new(inner: mio_channel::Sender<Message>, shared: Arc<SharedControlState>) -> Self {
        Self { inner, shared }
    }

    fn send_inner(&self, message: Message) -> Result<(), EventLoopSendError> {
        self.inner
            .send(message)
            .map_err(|_| EventLoopSendError::Disconnected)
    }

    fn send_control(&self, bytes: Cow<'static, [u8]>) -> Result<(), EventLoopSendError> {
        let in_control = self.shared.pane_id.lock().is_some();
        if in_control {
            self.send_inner(Message::TmuxControlCommand(bytes))
        } else {
            self.shared.pending_control.lock().push(bytes);
            Ok(())
        }
    }

    fn send_pane_bytes(
        &self,
        pane_id: Option<PaneId>,
        bytes: Cow<'static, [u8]>,
    ) -> Result<(), EventLoopSendError> {
        let encoded = {
            let stored = self.shared.pane_id.lock();
            let target = pane_id.as_ref().or(stored.as_ref());
            if let Some(target) = target {
                send_keys_commands(target, &bytes)
                    .into_iter()
                    .map(|command| Cow::Owned(command.into_bytes()))
                    .collect::<Vec<_>>()
            } else {
                self.shared.pending_pane_writes.lock().push(bytes);
                Vec::new()
            }
        };
        for command in encoded {
            self.send_inner(Message::TmuxControlCommand(command))?;
        }
        Ok(())
    }
}

impl EventLoopSender for TmuxControlSender {
    fn send(&self, message: Message) -> Result<(), EventLoopSendError> {
        match message {
            Message::Input(_) => Ok(()),
            Message::TmuxPaneInput { pane_id, bytes } => self.send_pane_bytes(Some(pane_id), bytes),
            Message::TmuxControlCommand(bytes) => self.send_control(bytes),
            Message::Resize(size) => {
                let command = refresh_client_command(size.columns(), size.rows());
                self.send_control(Cow::Owned(command.into_bytes()))
            }
            Message::Shutdown => {
                let _ = self.send_inner(Message::TmuxControlCommand(Cow::Borrowed(
                    kill_server_command().as_bytes(),
                )));
                self.send_inner(Message::Shutdown)
            }
            other => self.send_inner(other),
        }
    }
}

struct Writing {
    source: Cow<'static, [u8]>,
    written: usize,
}

impl Writing {
    fn new(source: Cow<'static, [u8]>) -> Self {
        Self { source, written: 0 }
    }

    fn advance(&mut self, n: usize) {
        self.written += n;
    }

    fn remaining_bytes(&self) -> &[u8] {
        &self.source[self.written..]
    }

    fn finished(&self) -> bool {
        self.written >= self.source.len()
    }
}

struct LoopState {
    write_list: VecDeque<Cow<'static, [u8]>>,
    writing: Option<Writing>,
    control_parser: super::parser::ControlModeParser,
    ansi_parser: ansi::Processor,
    tracked_pane: Option<PaneId>,
}

impl LoopState {
    fn new() -> Self {
        Self {
            write_list: VecDeque::new(),
            writing: None,
            control_parser: super::parser::ControlModeParser::new(),
            ansi_parser: ansi::Processor::new(),
            tracked_pane: None,
        }
    }

    fn ensure_next(&mut self) {
        if self.writing.is_none() {
            self.writing = self.write_list.pop_front().map(Writing::new);
        }
    }

    fn needs_write(&self) -> bool {
        self.writing.is_some() || !self.write_list.is_empty()
    }
}

enum ChannelResult {
    Continue,
    TerminateLoop { child_exited: bool },
}

/// Control-client PTY loop: parse tmux -CC off the client stream and feed only decoded pane
/// bytes into the TerminalModel.
pub struct ControlClientEventLoop<P: EventedPty> {
    poll: mio::Poll,
    pty: P,
    rx: Receiver<Message>,
    terminal: Arc<FairMutex<TerminalModel>>,
    event_listener: ChannelEventListener,
    shared: Arc<SharedControlState>,
    expected_session: SessionId,
    zsh_init: Option<(String, ShellType)>,
}

impl<P> ControlClientEventLoop<P>
where
    P: EventedPty + Send + 'static,
{
    pub fn new(
        terminal: Arc<FairMutex<TerminalModel>>,
        event_listener: ChannelEventListener,
        pty: P,
        rx: Receiver<Message>,
        shared: Arc<SharedControlState>,
        expected_session: SessionId,
        zsh_init: Option<(String, ShellType)>,
    ) -> Self {
        Self {
            poll: mio::Poll::new().expect("create mio Poll"),
            pty,
            rx,
            terminal,
            event_listener,
            shared,
            expected_session,
            zsh_init,
        }
    }

    pub fn spawn(self) -> JoinHandle<()> {
        thread::Builder::new()
            .name("tmux control-mode reader".into())
            .spawn(move || self.run())
            .expect("spawn tmux control-mode reader")
    }

    fn run(mut self) {
        let mut state = LoopState::new();
        let mut buf = [0u8; READ_BUFFER_SIZE];
        let mut can_read = false;
        let mut can_write = false;

        self.poll
            .registry()
            .register(&mut self.rx, CHANNEL_TOKEN, mio::Interest::READABLE)
            .unwrap();
        self.pty
            .register(
                &self.poll,
                mio::Interest::READABLE | mio::Interest::WRITABLE,
            )
            .unwrap();

        let mut events = mio::Events::with_capacity(1024);
        let mut shutdown = false;

        'event_loop: loop {
            events.clear();
            if let Err(err) = self.poll.poll(&mut events, None) {
                match err.kind() {
                    ErrorKind::Interrupted => continue,
                    _ => {
                        log::error!("tmux control-mode event loop polling error: {err}");
                        self.notify_abnormal_exit();
                        break 'event_loop;
                    }
                }
            }

            for event in events.iter() {
                match event.token() {
                    token if token == CHANNEL_TOKEN => match self.drain_recv_channel(&mut state) {
                        ChannelResult::Continue => {}
                        ChannelResult::TerminateLoop {
                            child_exited: exited,
                        } => {
                            if exited {
                                self.notify_abnormal_exit();
                            } else {
                                shutdown = true;
                            }
                            break 'event_loop;
                        }
                    },
                    token if token == self.pty.child_event_token() => {
                        if let Some(local_tty::ChildEvent::Exited) = self.pty.next_child_event() {
                            self.notify_abnormal_exit();
                            break 'event_loop;
                        }
                    }
                    token if token == self.pty.read_token() || token == self.pty.write_token() => {
                        if event.is_read_closed() || event.is_write_closed() {
                            self.notify_abnormal_exit();
                            break 'event_loop;
                        }
                        if event.is_readable() {
                            can_read = true;
                        }
                        if event.is_writable() {
                            can_write = true;
                        }
                    }
                    _ => {}
                }
            }

            while can_read || (state.needs_write() && can_write) {
                if can_read {
                    match self.pty_read(&mut state, &mut buf, &mut can_read) {
                        Ok(()) => {}
                        Err(err) => {
                            log::error!("Error reading tmux control client: {err}");
                            self.notify_abnormal_exit();
                            break 'event_loop;
                        }
                    }
                }
                if state.needs_write()
                    && can_write
                    && let Err(err) = self.pty_write(&mut state, &mut can_write)
                {
                    log::error!("Error writing tmux control client: {err}");
                    self.notify_abnormal_exit();
                    break 'event_loop;
                }
            }
        }

        if shutdown {
            self.flush_pending_writes(&mut state);
        }
        let _ = self.pty.kill();
    }

    fn notify_abnormal_exit(&self) {
        notify_exit(&mut self.terminal.lock());
        self.event_listener.send_wakeup_event();
    }

    fn flush_pending_writes(&mut self, state: &mut LoopState) {
        if !state.needs_write() {
            return;
        }
        let mut can_write = true;
        if self.pty_write(state, &mut can_write).is_err() || !state.needs_write() {
            return;
        }
        if !can_write {
            let mut events = mio::Events::with_capacity(16);
            if self
                .poll
                .poll(&mut events, Some(std::time::Duration::from_millis(100)))
                .is_err()
            {
                return;
            }
            can_write = events.iter().any(|event| {
                (event.token() == self.pty.write_token() || event.token() == self.pty.read_token())
                    && event.is_writable()
                    && !event.is_write_closed()
            });
        }
        if can_write {
            let _ = self.pty_write(state, &mut can_write);
        }
    }

    fn drain_recv_channel(&mut self, state: &mut LoopState) -> ChannelResult {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Message::Input(input) | Message::TmuxControlCommand(input) => {
                    state.write_list.push_back(input)
                }
                Message::TmuxPaneInput { pane_id, bytes } => {
                    for command in send_keys_commands(&pane_id, &bytes) {
                        state.write_list.push_back(Cow::Owned(command.into_bytes()));
                    }
                }
                Message::Shutdown => {
                    return ChannelResult::TerminateLoop {
                        child_exited: false,
                    };
                }
                Message::Resize(size) => self.pty.on_resize(&size),
                Message::ChildExited => return ChannelResult::TerminateLoop { child_exited: true },
            }
        }
        ChannelResult::Continue
    }

    fn pty_read(
        &mut self,
        state: &mut LoopState,
        buf: &mut [u8],
        can_read: &mut bool,
    ) -> io::Result<()> {
        let mut bytes_in_buffer = 0;
        let mut bytes_processed = 0;
        let mut terminal = None;
        let mut stage_complete = Vec::new();
        let mut instance_id = None;

        loop {
            match self.pty.reader().read(&mut buf[bytes_in_buffer..]) {
                Ok(0) if bytes_in_buffer == 0 => {
                    *can_read = false;
                    break;
                }
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
                    None if bytes_in_buffer >= READ_BUFFER_SIZE => self.terminal.lock(),
                    None => continue,
                    Some(terminal) => terminal,
                }),
            };

            let mut writer = sink_writer();
            let feed = feed_control_bytes(
                &mut state.control_parser,
                &mut state.ansi_parser,
                terminal.deref_mut(),
                &mut writer,
                &mut state.tracked_pane,
                &buf[..bytes_in_buffer],
            );
            if feed.entered_control_mode {
                log::info!("tmux control mode entered");
            }
            if feed.exited {
                notify_exit(terminal.deref_mut());
            }
            instance_id = terminal.tmux_instance_id();
            Self::maybe_bind_pane(
                &self.shared,
                self.expected_session,
                &mut self.zsh_init,
                state,
                instance_id,
            );
            stage_complete.extend(feed.stage_complete);

            bytes_processed += bytes_in_buffer;
            bytes_in_buffer = 0;
            if bytes_processed >= MAX_LOCKED_READ {
                break;
            }
            FairMutexGuard::bump(terminal);
        }
        drop(terminal);
        Self::apply_stage_complete(instance_id, &stage_complete, state);

        if bytes_processed > 0 {
            self.event_listener.send_wakeup_event();
        }
        Ok(())
    }

    fn maybe_bind_pane(
        shared: &SharedControlState,
        expected_session: SessionId,
        zsh_init: &mut Option<(String, ShellType)>,
        state: &mut LoopState,
        instance_id: Option<u64>,
    ) {
        let Some(pane_id) = state.tracked_pane.clone() else {
            return;
        };
        let mut stored = shared.pane_id.lock();
        if stored.is_some() {
            return;
        }
        *stored = Some(pane_id.clone());
        drop(stored);
        if let Some(instance_id) = instance_id {
            use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};
            if let Some(runtime) = TmuxRuntime::for_id(TmuxInstanceId::from_u64(instance_id)) {
                runtime.note_tracked_control_pane(pane_id.as_str());
                runtime.set_tracked_expected_session(expected_session);
                if zsh_init.is_some() {
                    runtime.note_retained_zsh_init(pane_id.as_str(), expected_session);
                }
            }
        }

        let pending = std::mem::take(&mut *shared.pending_pane_writes.lock());
        let pending_control = std::mem::take(&mut *shared.pending_control.lock());
        let mut to_send = Vec::new();
        if let Some((script, shell_type)) = zsh_init.take() {
            to_send.push(zsh_init_bytes(&script, shell_type, expected_session));
        }
        to_send.extend(pending.into_iter().map(|bytes| bytes.into_owned()));
        for bytes in to_send {
            for command in send_keys_commands(&pane_id, &bytes) {
                state.write_list.push_back(Cow::Owned(command.into_bytes()));
            }
        }
        state.write_list.extend(pending_control);
    }

    fn apply_stage_complete(
        instance_id: Option<u64>,
        completed: &[(PaneId, SessionId)],
        state: &mut LoopState,
    ) {
        use crate::terminal::tmux::bridge::{TmuxInstanceId, TmuxRuntime};
        let Some(instance_id) = instance_id else {
            return;
        };
        let Some(runtime) = TmuxRuntime::for_id(TmuxInstanceId::from_u64(instance_id)) else {
            return;
        };
        for (pane_id, session_id) in completed {
            let _ = runtime.on_stage_complete(pane_id.as_str(), *session_id);
        }
        for (pane_id, shell_type) in runtime.take_pending_silent_bootstrap() {
            let bytes = silent_bootstrap_bytes(shell_type);
            for command in send_keys_commands(&PaneId::from(pane_id.as_str()), &bytes) {
                state.write_list.push_back(Cow::Owned(command.into_bytes()));
            }
        }
    }

    fn pty_write(&mut self, state: &mut LoopState, can_write: &mut bool) -> io::Result<()> {
        state.ensure_next();
        'write_many: while let Some(mut current) = state.writing.take() {
            loop {
                match self.pty.writer().write(current.remaining_bytes()) {
                    Ok(0) => {
                        state.writing = Some(current);
                        *can_write = false;
                        break 'write_many;
                    }
                    Ok(n) => {
                        current.advance(n);
                        if current.finished() {
                            state.writing = state.write_list.pop_front().map(Writing::new);
                            break;
                        }
                    }
                    Err(err) => {
                        state.writing = Some(current);
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
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod tests;
