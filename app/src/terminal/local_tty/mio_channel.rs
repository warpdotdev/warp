pub use std::sync::mpsc::SendError;
use std::{
    io,
    sync::{mpsc, Arc, Mutex},
};

use mio::{event, Token, Waker};

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
/// events even if read interest is not registered.
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

        if state.waker.is_none() {
            let waker = Waker::new(registry, token)?;
            if state.needs_wake_on_register {
                waker.wake()?;
                state.needs_wake_on_register = false;
            }
            state.waker = Some(waker);
        }

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
        // Not actually supported, so we do nothing.
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
    /// This works the same way as [`mpsc::Sender::send`]. After sending the value, it wakes up the
    /// [`mio::poll::Poll`].
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        self.tx.send(t)?;

        let mut state = self.state.lock().unwrap();
        if let Some(waker) = &mut state.waker {
            if let Err(e) = waker.wake() {
                log::error!("PTY mio Waker::wake() failed: {e:#}");
            }
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
    /// The underlying waker for the channel.  This is None until the channel
    /// is registered.
    waker: Option<Waker>,
    /// Whether or not we need to wake the waker immediately upon registration.
    /// This can happen if a message was sent before the receiver was registered
    /// with a [`mio::Registry`].
    needs_wake_on_register: bool,
}
