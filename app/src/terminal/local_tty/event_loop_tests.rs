use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{io, thread};

use instant::Instant;
use mio::Interest;
use parking_lot::FairMutex;

use super::event_loop::{EventLoop, PTY_TOKEN, SIGNALS_TOKEN};
use super::*;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::writeable_pty::Message;
use crate::terminal::{SizeInfo, TerminalModel};

struct TestPty {
    reader: io::Empty,
    writer: io::Sink,
    killed: Arc<AtomicBool>,
}

impl EventedReadWrite for TestPty {
    type Reader = io::Empty;
    type Writer = io::Sink;

    fn register(&mut self, _: &mio::Poll, _: Interest) -> io::Result<()> {
        Ok(())
    }

    fn reregister(&mut self, _: &mio::Poll, _: Interest) -> io::Result<()> {
        Ok(())
    }

    fn deregister(&mut self, _: &mio::Poll) -> io::Result<()> {
        Ok(())
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.writer
    }

    fn read_token(&self) -> mio::Token {
        PTY_TOKEN
    }

    fn write_token(&self) -> mio::Token {
        PTY_TOKEN
    }
}

impl EventedPty for TestPty {
    fn child_event_token(&self) -> mio::Token {
        SIGNALS_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }

    fn on_resize(&mut self, _: &SizeInfo) {}

    fn kill(self) -> anyhow::Result<()> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn stop_loop(
    terminal: Arc<FairMutex<TerminalModel>>,
    sender: &mio_channel::Sender<Message>,
    receiver: mio_channel::Receiver<Message>,
    message: Message,
) -> (mio_channel::Receiver<Message>, bool) {
    let killed = Arc::new(AtomicBool::new(false));
    let pty = TestPty {
        reader: io::empty(),
        writer: io::sink(),
        killed: killed.clone(),
    };
    let handle = EventLoop::new(
        terminal,
        ChannelEventListener::new_for_test(),
        pty,
        receiver,
    )
    .spawn();
    sender.send(message).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "event loop did not stop");
        thread::sleep(Duration::from_millis(1));
    }
    (handle.join().unwrap(), killed.load(Ordering::SeqCst))
}

#[test]
fn planned_replacement_preserves_terminal_and_reuses_channel() {
    let terminal = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let (sender, receiver) = mio_channel::channel();
    let (receiver, killed) = stop_loop(
        terminal.clone(),
        &sender,
        receiver,
        Message::ShutdownForReplacement,
    );
    assert!(killed);
    assert!(!terminal.lock().is_read_only());
    let (_, killed) = stop_loop(
        terminal.clone(),
        &sender,
        receiver,
        Message::ShutdownForReplacement,
    );
    assert!(killed);
    assert!(!terminal.lock().is_read_only());
}

#[test]
fn normal_shutdown_still_exits_terminal() {
    let terminal = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let (sender, receiver) = mio_channel::channel();
    let (_, killed) = stop_loop(terminal.clone(), &sender, receiver, Message::Shutdown);
    assert!(killed);
    assert!(terminal.lock().is_read_only());
}

#[cfg(unix)]
struct ReplayPty {
    stream: std::os::unix::net::UnixStream,
    replay: Option<Vec<u8>>,
    exit_after_drain: Option<Vec<u8>>,
    exited: bool,
    child_events: Option<std::os::unix::net::UnixStream>,
}

#[cfg(unix)]
impl EventedReadWrite for ReplayPty {
    type Reader = std::os::unix::net::UnixStream;
    type Writer = std::os::unix::net::UnixStream;

    fn register(&mut self, poll: &mio::Poll, interest: Interest) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        if let Some(child_events) = &self.child_events {
            poll.registry().register(
                &mut mio::unix::SourceFd(&child_events.as_raw_fd()),
                SIGNALS_TOKEN,
                Interest::READABLE,
            )?;
        }
        poll.registry().register(
            &mut mio::unix::SourceFd(&self.stream.as_raw_fd()),
            PTY_TOKEN,
            interest,
        )
    }

    fn reregister(&mut self, poll: &mio::Poll, interest: Interest) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        poll.registry().reregister(
            &mut mio::unix::SourceFd(&self.stream.as_raw_fd()),
            PTY_TOKEN,
            interest,
        )
    }

    fn deregister(&mut self, poll: &mio::Poll) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        poll.registry()
            .deregister(&mut mio::unix::SourceFd(&self.stream.as_raw_fd()))
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.stream
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.stream
    }

    fn read_token(&self) -> mio::Token {
        PTY_TOKEN
    }

    fn write_token(&self) -> mio::Token {
        PTY_TOKEN
    }
}

#[cfg(unix)]
impl EventedPty for ReplayPty {
    fn child_event_token(&self) -> mio::Token {
        SIGNALS_TOKEN
    }

    fn next_child_event(&mut self) -> Option<ChildEvent> {
        (self.exit_after_drain.is_some() && !self.exited).then(|| {
            self.exited = true;
            ChildEvent::Exited(warp_terminal::event::ObservedExitStatus::Code(0))
        })
    }

    fn on_resize(&mut self, _: &SizeInfo) {}

    fn kill(self) -> anyhow::Result<()> {
        Ok(())
    }

    fn take_replay(&mut self) -> Option<Vec<u8>> {
        self.replay.take()
    }

    fn drain_after_exit(&mut self) -> Option<Vec<u8>> {
        self.exit_after_drain.take()
    }
}

#[cfg(unix)]
#[test]
fn replay_is_processed_before_live_bytes_without_answering_queries() {
    use std::io::{Read as _, Write as _};

    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("watch", "");
    let terminal = Arc::new(FairMutex::new(model));
    let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let (sender, receiver) = mio_channel::channel();
    let handle = EventLoop::new(
        terminal.clone(),
        ChannelEventListener::new_for_test(),
        ReplayPty {
            stream,
            replay: Some(b"\x1b[cbefore\r\n".to_vec()),
            exit_after_drain: None,
            exited: false,
            child_events: None,
        },
        receiver,
    )
    .spawn();

    peer.write_all(b"after\r\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = terminal
            .lock()
            .block_list()
            .active_block()
            .output_to_string();
        if output.contains("after") {
            let before = output.find("before").expect("replay should be processed");
            assert!(before < output.find("after").unwrap());
            break;
        }
        assert!(Instant::now() < deadline, "live bytes were not processed");
        thread::sleep(Duration::from_millis(5));
    }

    sender.send(Message::Shutdown).unwrap();
    handle.join().unwrap();
    let mut written_to_pty = Vec::new();
    peer.read_to_end(&mut written_to_pty).unwrap();
    assert!(written_to_pty.is_empty());
}

#[cfg(unix)]
#[test]
fn output_drained_after_exit_is_processed_before_exiting() {
    use std::io::Write as _;

    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("watch", "");
    let terminal = Arc::new(FairMutex::new(model));
    let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let (child_events, mut child_exit) = std::os::unix::net::UnixStream::pair().unwrap();
    let (_sender, receiver) = mio_channel::channel();
    let handle = EventLoop::new(
        terminal.clone(),
        ChannelEventListener::new_for_test(),
        ReplayPty {
            stream,
            replay: None,
            exit_after_drain: Some(b"final words\r\n".to_vec()),
            exited: false,
            child_events: Some(child_events),
        },
        receiver,
    )
    .spawn();

    child_exit.write_all(b"x").unwrap();
    handle.join().unwrap();
    let output = terminal
        .lock()
        .block_list()
        .active_block()
        .output_to_string();
    assert!(output.contains("final words"), "{output:?}");
}
