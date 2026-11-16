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
