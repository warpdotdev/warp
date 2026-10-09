use std::io;
pub use std::sync::mpsc::SendError;
use std::sync::{Arc, Mutex, mpsc};

use mio::{Token, Waker, event};

/// Create a [`Sender`] and [`Receiver`] pair, for sending messages into a
/// [`mio`]-managed event loop.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = mpsc::channel();

    let state = Arc::new(Mutex::new(State {
        waker: None,
        needs_wake_on_register: false,
    }));

    (
        Sender {
            state: state.clone(),
            tx,
        },
        Receiver { state, rx },
    )
}

/// An [`mpsc::Receiver`] wrapper.
///
/// It implements [`event::Source`] so that it can be registered with a
/// [`mio::poll::Poll`]. It ignores the [`mio::Interest`], producing readable
/// events even if read interest is not registered. Deregistering and registering again with a
/// different poll is supported, so one channel can outlive the event loop draining it.
pub struct Receiver<T> {
    state: Arc<Mutex<State>>,
    rx: mpsc::Receiver<T>,
}

impl<T> Receiver<T> {
    /// Try to receive a value. It works just like [`mpsc::Receiver::try_recv`].
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        self.rx.try_recv()
    }
}

impl<T> event::Source for Receiver<T> {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        _: mio::Interest,
    ) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();

        let waker = Waker::new(registry, token)?;
        if state.needs_wake_on_register {
            waker.wake()?;
            state.needs_wake_on_register = false;
        }
        state.waker = Some(waker);

        Ok(())
    }

    fn reregister(
        &mut self,
        _registry: &mio::Registry,
        _token: Token,
        _: mio::Interest,
    ) -> io::Result<()> {
        // Not actually supported, so we do nothing.
        Ok(())
    }

    fn deregister(&mut self, _: &mio::Registry) -> io::Result<()> {
        // Messages sent from here on are delivered by the wake performed on the next registration.
        self.state.lock().unwrap().waker = None;
        Ok(())
    }
}

/// An [`mpsc::Sender`] wrapper.
pub struct Sender<T> {
    state: Arc<Mutex<State>>,
    tx: mpsc::Sender<T>,
}

impl<T> Sender<T> {
    /// Try to send a value.
    ///
    /// This works the same way as [`mpsc::Sender::send`]. After sending the
    /// value, it wakes upthe [`mio::poll::Poll`].
    ///
    /// Note that I/O errors from waking up the [`mio::poll::Poll`] are
    /// swallowed.
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        self.tx.send(t)?;

        let mut state = self.state.lock().unwrap();
        if let Some(waker) = &mut state.waker {
            let _ = waker.wake();
        } else {
            state.needs_wake_on_register = true;
        }

        Ok(())
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            tx: self.tx.clone(),
        }
    }
}

struct State {
    /// The underlying waker for the channel.  This is None while the channel
    /// is not registered.
    waker: Option<Waker>,
    /// Whether or not we need to wake the waker immediately upon registration.
    /// This can happen if a message was sent while the receiver was not registered
    /// with a [`mio::Registry`].
    needs_wake_on_register: bool,
}
