use std::ffi::c_void;

use mio::event::Source;
use mio::{Interest, Registry, Token};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    RegisterWaitForSingleObject, UnregisterWait, INFINITE, WT_EXECUTEINWAITTHREAD,
    WT_EXECUTEONLYONCE,
};

use crate::terminal::local_tty::mio_channel;
use crate::terminal::local_tty::windows::ShareableHandle;
use crate::terminal::writeable_pty::Message;

struct ChildExitSender {
    sender: mio_channel::Sender<Message>,
}

/// WinAPI callback to run when child process exits.
extern "system" fn child_exit_callback(ctx: *mut c_void, timed_out: bool) {
    // Convert context back into a Box<ChildExitSender>.  We do this immediately
    // to ensure it doesn't get leaked if we hit the timeout.
    let event_tx = unsafe { Box::from_raw(ctx as *mut ChildExitSender) };

    // This will not be hit by our current invocation strategy, as we
    // call RegisterWaitForSingleObject with both a timeout of INFINITE
    // and with the flag WT_EXECUTEONLYONCE. But it's still here in case
    // this ever gets refactored to break those guarantees.
    if timed_out {
        return;
    }

    event_tx.sender.send(Message::ChildExited).ok();
}

pub(super) struct ChildExitWatcher {
    wait_handle: ShareableHandle,
}

// Mark `ChildExitWatcher` as being safe to share between threads,
// even though `HANDLE` holds a `*mut c_void`, which isn't inherently
// safe to share.
unsafe impl Send for ChildExitWatcher {}
unsafe impl Sync for ChildExitWatcher {}

impl ChildExitWatcher {
    pub fn new(
        child_handle: HANDLE,
        event_loop_tx: mio_channel::Sender<Message>,
    ) -> windows::core::Result<ChildExitWatcher> {
        let mut wait_handle = HANDLE::default();
        let sender_ref = Box::new(ChildExitSender {
            sender: event_loop_tx,
        });

        unsafe {
            RegisterWaitForSingleObject(
                &mut wait_handle,
                child_handle,
                Some(child_exit_callback),
                Some(Box::into_raw(sender_ref).cast()),
                INFINITE,
                WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE,
            )?
        };

        Ok(ChildExitWatcher {
            wait_handle: ShareableHandle(wait_handle),
        })
    }
}

impl Source for ChildExitWatcher {
    fn register(
        &mut self,
        _registry: &Registry,
        _token: Token,
        _interest: Interest,
    ) -> std::io::Result<()> {
        // Nothing to do.
        Ok(())
    }

    fn reregister(
        &mut self,
        _registry: &Registry,
        _token: Token,
        _interest: Interest,
    ) -> std::io::Result<()> {
        // Nothing to do.
        Ok(())
    }

    fn deregister(&mut self, _registry: &Registry) -> std::io::Result<()> {
        // Nothing to do.
        Ok(())
    }
}

impl Drop for ChildExitWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterWait(self.wait_handle.0);
        }
    }
}

#[cfg(test)]
#[path = "child_tests.rs"]
mod tests;
