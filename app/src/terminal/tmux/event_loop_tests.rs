use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use mio::{Interest, Token};
use parking_lot::FairMutex;
use warp_core::SessionId;
use warp_terminal::shell::ShellType;

use super::{ControlClientEventLoop, SharedControlState, TmuxControlSender};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::{ChildEvent, EventedPty, EventedReadWrite, mio_channel};
use crate::terminal::tmux::parser::{CONTROL_MODE_DCS, PaneId};
use crate::terminal::tmux::protocol::{kill_server_command, send_keys_commands, zsh_init_bytes};
use crate::terminal::writeable_pty::Message;
use crate::terminal::writeable_pty::pty_controller::EventLoopSender as _;
use crate::terminal::{SizeInfo, TerminalModel};

const IO_TOKEN: Token = Token(1);
const CHILD_TOKEN: Token = Token(2);

struct FakeReader {
    error: Option<io::ErrorKind>,
    pending: Mutex<Vec<u8>>,
}

impl Read for FakeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(kind) = self.error {
            return Err(io::Error::new(kind, "fake read error"));
        }
        let mut pending = self.pending.lock().expect("reader lock");
        if pending.is_empty() {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "no fake bytes"));
        }
        let n = pending.len().min(buf.len());
        buf[..n].copy_from_slice(&pending[..n]);
        pending.drain(..n);
        Ok(n)
    }
}

struct FakeWriter {
    written: Arc<Mutex<Vec<u8>>>,
    error: Option<io::ErrorKind>,
}

impl Write for FakeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(kind) = self.error {
            return Err(io::Error::new(kind, "fake write error"));
        }
        self.written
            .lock()
            .expect("writer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FakePty {
    stream: mio::net::UnixStream,
    reader: FakeReader,
    writer: FakeWriter,
}

fn fake_pty(
    reader_error: Option<io::ErrorKind>,
    writer_error: Option<io::ErrorKind>,
    pending_read: Vec<u8>,
) -> (FakePty, mio::net::UnixStream, Arc<Mutex<Vec<u8>>>) {
    let (stream, peer) = mio::net::UnixStream::pair().expect("unix stream pair");
    let written = Arc::new(Mutex::new(Vec::new()));
    (
        FakePty {
            stream,
            reader: FakeReader {
                error: reader_error,
                pending: Mutex::new(pending_read),
            },
            writer: FakeWriter {
                written: written.clone(),
                error: writer_error,
            },
        },
        peer,
        written,
    )
}

impl EventedReadWrite for FakePty {
    type Reader = FakeReader;
    type Writer = FakeWriter;

    fn register(&mut self, poll: &mio::Poll, interest: Interest) -> io::Result<()> {
        poll.registry()
            .register(&mut self.stream, IO_TOKEN, interest)
    }

    fn reregister(&mut self, poll: &mio::Poll, interest: Interest) -> io::Result<()> {
        poll.registry()
            .reregister(&mut self.stream, IO_TOKEN, interest)
    }

    fn deregister(&mut self, poll: &mio::Poll) -> io::Result<()> {
        poll.registry().deregister(&mut self.stream)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn read_token(&self) -> Token {
        IO_TOKEN
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.writer
    }

    fn write_token(&self) -> Token {
        IO_TOKEN
    }
}

impl EventedPty for FakePty {
    fn child_event_token(&self) -> Token {
        CHILD_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }

    fn on_resize(&mut self, _size: &crate::terminal::SizeInfo) {}

    fn kill(self) -> Result<()> {
        Ok(())
    }
}

struct Harness {
    handle: JoinHandle<()>,
    sender: TmuxControlSender,
    shared: Arc<SharedControlState>,
    model: Arc<FairMutex<TerminalModel>>,
    wakeups_rx: async_channel::Receiver<()>,
    written: Arc<Mutex<Vec<u8>>>,
    peer: Option<mio::net::UnixStream>,
}

fn start_loop(reader_error: Option<io::ErrorKind>, writer_error: Option<io::ErrorKind>) -> Harness {
    start_loop_with(reader_error, writer_error, None, Vec::new())
}

fn start_loop_with(
    reader_error: Option<io::ErrorKind>,
    writer_error: Option<io::ErrorKind>,
    zsh_init: Option<(String, ShellType, SessionId)>,
    pending_read: Vec<u8>,
) -> Harness {
    let (pty, peer, written) = fake_pty(reader_error, writer_error, pending_read);
    let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
    let listener = ChannelEventListener::builder_for_test::<crate::terminal::event::Event>()
        .with_wakeups_tx(wakeups_tx)
        .build();
    let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let expected_session = zsh_init
        .as_ref()
        .map(|(_, _, session_id)| *session_id)
        .unwrap_or(SessionId::from(1));
    let zsh_init = zsh_init.map(|(script, shell, _)| (script, shell));
    let (tx, rx) = mio_channel::channel();
    let shared = Arc::new(SharedControlState::new());
    let sender = TmuxControlSender::new(tx, shared.clone());
    let event_loop = ControlClientEventLoop::new(
        model.clone(),
        listener,
        pty,
        rx,
        shared.clone(),
        expected_session,
        zsh_init,
    );
    Harness {
        handle: event_loop.spawn(),
        sender,
        shared,
        model,
        wakeups_rx,
        written,
        peer: Some(peer),
    }
}

fn join_loop(handle: JoinHandle<()>) {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = handle.join();
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("tmux control-mode event loop did not stop");
}

#[test]
fn shutdown_writes_kill_server_before_stopping() {
    let harness = start_loop(None, None);
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written = harness.written.lock().expect("written lock").clone();
    assert_eq!(written, kill_server_command().as_bytes());
    assert!(!harness.model.lock().is_read_only());
}

#[test]
fn read_error_exits_terminal_and_wakes_view() {
    let mut harness = start_loop(Some(io::ErrorKind::ConnectionReset), None);
    let mut peer = harness.peer.take().expect("peer");
    peer.write_all(&[1]).expect("wake readable");
    join_loop(harness.handle);
    assert!(harness.model.lock().is_read_only());
    assert!(harness.wakeups_rx.try_recv().is_ok());
}

#[test]
fn write_error_exits_terminal_and_wakes_view() {
    let harness = start_loop(None, Some(io::ErrorKind::BrokenPipe));
    harness.shared.bind_pane(PaneId::from("%0"));
    harness
        .sender
        .send(Message::Resize(SizeInfo::new_without_font_metrics(24, 80)))
        .expect("send resize");
    join_loop(harness.handle);
    assert!(harness.model.lock().is_read_only());
    assert!(harness.wakeups_rx.try_recv().is_ok());
}

#[test]
fn closed_pty_exits_terminal_and_wakes_view() {
    let mut harness = start_loop(None, None);
    drop(harness.peer.take());
    join_loop(harness.handle);
    assert!(harness.model.lock().is_read_only());
    assert!(harness.wakeups_rx.try_recv().is_ok());
}

#[test]
fn internal_send_keys_command_is_written_exactly() {
    let harness = start_loop(None, None);
    harness.shared.bind_pane(PaneId::from("%0"));
    let command = b"send-keys -t %0 -H 41\n".to_vec();
    harness
        .sender
        .send(Message::TmuxControlCommand(command.clone().into()))
        .expect("send control");
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written = harness.written.lock().expect("written lock").clone();
    assert_eq!(&written[..command.len()], command.as_slice());
    assert!(
        !written
            .windows(command.len() * 2)
            .any(|w| { w[..command.len()] == command && w[command.len()..] == command })
    );
}

#[test]
fn user_key_a_becomes_one_send_keys_command() {
    let harness = start_loop(None, None);
    harness.shared.bind_pane(PaneId::from("%0"));
    harness
        .sender
        .send(Message::TmuxPaneInput {
            pane_id: PaneId::from("%0"),
            bytes: std::borrow::Cow::Borrowed(b"A"),
        })
        .expect("send pane input");
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written = harness.written.lock().expect("written lock").clone();
    let encoded = send_keys_commands(&PaneId::from("%0"), b"A");
    assert_eq!(encoded.len(), 1);
    assert!(written.starts_with(encoded[0].as_bytes()));
    assert_eq!(
        written
            .windows(encoded[0].len())
            .filter(|w| *w == encoded[0].as_bytes())
            .count(),
        1
    );
}

#[test]
fn bootstrap_and_split_are_not_double_encoded() {
    let harness = start_loop(None, None);
    harness.shared.bind_pane(PaneId::from("%0"));
    let bootstrap = send_keys_commands(&PaneId::from("%0"), b":\n");
    harness
        .sender
        .send(Message::TmuxControlCommand(
            bootstrap[0].clone().into_bytes().into(),
        ))
        .expect("send bootstrap");
    harness
        .sender
        .send(Message::TmuxControlCommand(std::borrow::Cow::Borrowed(
            b"split-window -h -t %0\n",
        )))
        .expect("send split");
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written =
        String::from_utf8(harness.written.lock().expect("written lock").clone()).expect("utf8");
    assert!(written.contains(&bootstrap[0]));
    assert!(written.contains("split-window -h -t %0\n"));
    assert!(
        !written.contains("send-keys -t %0 -H 73 65 6e 64"),
        "bootstrap send-keys must not be hex-encoded again: {written}"
    );
    assert!(
        !written.contains("send-keys -t %0 -H 73 70 6c 69 74"),
        "split-window must not be hex-encoded: {written}"
    );
}

#[test]
fn generic_input_is_not_encoded_as_pane_send_keys() {
    let harness = start_loop(None, None);
    harness.shared.bind_pane(PaneId::from("%0"));
    let bootstrap = b"warp_bootstrapped() {\nread -r -d '' WARP_BOOTSTRAP_VAR << 'EOM'\nEOM\n\n";
    harness
        .sender
        .send(Message::Input(std::borrow::Cow::Borrowed(bootstrap)))
        .expect("send generic input");
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written =
        String::from_utf8(harness.written.lock().expect("written lock").clone()).expect("utf8");
    assert!(
        !written.contains("warp_bootstrapped"),
        "generic Input must not be typed into the pane: {written}"
    );
    assert!(
        !written.contains("send-keys -t %0"),
        "generic Input must not be send-keys encoded: {written}"
    );
}

fn wait_for_written(written: &Arc<Mutex<Vec<u8>>>, needle: &[u8]) {
    let deadline = instant::Instant::now() + Duration::from_secs(2);
    loop {
        {
            let got = written.lock().expect("written lock");
            if got.windows(needle.len()).any(|w| w == needle) {
                return;
            }
        }
        if instant::Instant::now() >= deadline {
            let got = written.lock().expect("written lock").clone();
            panic!("timed out waiting for send-keys, got {got:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn bound_pane_writes_retained_zsh_init_send_keys_exactly_once() {
    let script = "WARP_ZSH_INIT_MARKER".to_owned();
    let init_bytes = zsh_init_bytes(&script, ShellType::Zsh, SessionId::from(7));
    let encoded = send_keys_commands(&PaneId::from("%0"), &init_bytes);
    let expected: Vec<u8> = encoded.iter().flat_map(|c| c.as_bytes()).copied().collect();
    let mut control = CONTROL_MODE_DCS.to_vec();
    control.extend_from_slice(b"%window-pane-changed @0 %0\n");
    let mut harness = start_loop_with(
        None,
        None,
        Some((script, ShellType::Zsh, SessionId::from(7))),
        control,
    );
    let mut peer = harness.peer.take().expect("peer");
    peer.write_all(&[1]).expect("wake readable");
    wait_for_written(&harness.written, &expected);
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    let written = harness.written.lock().expect("written lock").clone();
    assert_eq!(
        written
            .windows(expected.len())
            .filter(|w| *w == expected)
            .count(),
        1
    );
    let written_text = String::from_utf8_lossy(&written);
    assert!(
        !written_text.contains("send-keys -t %0 -H 73 65 6e 64"),
        "zsh init send-keys must not be hex-encoded again: {written_text}"
    );
}

#[test]
fn bound_pane_records_retained_zsh_init_ownership_before_write() {
    use crate::terminal::tmux::bridge::TmuxRuntime;

    let runtime = TmuxRuntime::new();
    let script = "WARP_ZSH_INIT_MARKER".to_owned();
    let init_bytes = zsh_init_bytes(&script, ShellType::Zsh, SessionId::from(7));
    let encoded = send_keys_commands(&PaneId::from("%0"), &init_bytes);
    let expected: Vec<u8> = encoded.iter().flat_map(|c| c.as_bytes()).copied().collect();
    let mut control = CONTROL_MODE_DCS.to_vec();
    control.extend_from_slice(b"%window-pane-changed @0 %0\n");
    let mut harness = start_loop_with(
        None,
        None,
        Some((script, ShellType::Zsh, SessionId::from(7))),
        control,
    );
    harness
        .model
        .lock()
        .set_tmux_instance_id(Some(runtime.id().as_u64()));
    let mut peer = harness.peer.take().expect("peer");
    peer.write_all(&[1]).expect("wake readable");
    wait_for_written(&harness.written, &expected);
    assert!(runtime.control_pane_owns_retained_init("%0"));
    assert!(!runtime.control_pane_owns_retained_init("%1"));
    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    runtime.unregister();
}

fn octal_escape_output(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b < 0x20 || b == 0x5c || b >= 0x7f {
            out.push_str(&format!("\\{b:03o}"));
        } else {
            out.push(char::from(b));
        }
    }
    out
}

#[test]
fn init_shell_before_remaining_staged_bytes_does_not_write_bootstrap() {
    use crate::terminal::tmux::bridge::TmuxRuntime;
    use crate::terminal::tmux::protocol::{STAGE_COMPLETE_OSC_PREFIX, silent_bootstrap_bytes};

    let runtime = TmuxRuntime::new();
    runtime.note_shell_type(ShellType::Zsh);
    runtime.note_tracked_control_pane("%0");
    runtime.set_tracked_expected_session(SessionId::from(7));
    runtime
        .begin_pane_bootstrap("%0", SessionId::from(7))
        .expect("stage");
    assert_eq!(
        runtime.note_early_init_shell("%0", SessionId::from(7), ShellType::Zsh),
        Some(ShellType::Zsh)
    );

    let script = "WARP_ZSH_INIT_MARKER".to_owned();
    let mut ack = STAGE_COMPLETE_OSC_PREFIX.to_vec();
    ack.extend_from_slice(b"7\x07");
    let mut control = CONTROL_MODE_DCS.to_vec();
    control.extend_from_slice(b"%window-pane-changed @0 %0\n");
    control.extend_from_slice(
        format!("%output %0 {}\n", octal_escape_output(b"still-staging")).as_bytes(),
    );
    control.extend_from_slice(format!("%output %0 {}\n", octal_escape_output(&ack)).as_bytes());
    let mut harness = start_loop_with(
        None,
        None,
        Some((script, ShellType::Zsh, SessionId::from(7))),
        control,
    );
    harness
        .model
        .lock()
        .set_tmux_instance_id(Some(runtime.id().as_u64()));
    let mut peer = harness.peer.take().expect("peer");
    peer.write_all(&[1]).expect("wake readable");

    let bootstrap = silent_bootstrap_bytes(ShellType::Zsh);
    let encoded = send_keys_commands(&PaneId::from("%0"), &bootstrap);
    wait_for_written(&harness.written, encoded[0].as_bytes());
    assert_eq!(runtime.bootstrap_script_count("%0"), 1);
    assert!(
        runtime
            .on_stage_complete("%0", SessionId::from(7))
            .is_none()
    );
    assert_eq!(runtime.bootstrap_script_count("%0"), 1);

    harness
        .sender
        .send(Message::Shutdown)
        .expect("send shutdown");
    join_loop(harness.handle);
    runtime.unregister();
    let written = harness.written.lock().expect("written lock").clone();
    let bootstrap_prefix = encoded[0].as_bytes();
    let count = written
        .windows(bootstrap_prefix.len())
        .filter(|w| *w == bootstrap_prefix)
        .count();
    assert_eq!(count, 1, "ack must inject silent bootstrap once");
}
