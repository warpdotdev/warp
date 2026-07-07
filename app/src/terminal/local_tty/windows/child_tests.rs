use std::time::Duration;

use instant::Instant;
use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};

use super::*;
use crate::terminal::local_tty::event_loop::CHANNEL_TOKEN;

#[test]
pub fn test_event_is_emitted_when_child_exits() {
    const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

    let mut poll = mio::Poll::new().unwrap();

    let (tx, mut rx) = mio_channel::channel();

    let event_handle = unsafe { CreateEventW(None, true, false, PCWSTR::null()).unwrap() };
    let mut child_exit_watcher = ChildExitWatcher::new(event_handle, tx).unwrap();
    // We need to register the receiver with the poller so that it can be woken up when the child exits.
    poll.registry()
        .register(&mut rx, CHANNEL_TOKEN, Interest::READABLE)
        .unwrap();
    // This doesn't actually do anything, but we're calling it anyway for "completeness".
    child_exit_watcher
        .register(poll.registry(), CHANNEL_TOKEN, Interest::READABLE)
        .unwrap();

    unsafe { SetEvent(event_handle).unwrap() };

    // Poll until the event arrives or the overall timeout elapses.
    let mut events = mio::Events::with_capacity(10);
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        events.clear();
        poll.poll(
            &mut events,
            Some(deadline.saturating_duration_since(Instant::now())),
        )
        .unwrap();

        if events.iter().any(|event| event.token() == CHANNEL_TOKEN) {
            break;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for child-exit event; receiver state: {:?}",
            rx.try_recv()
        );
    }
    // Verify that at least one `ChildEvent::Exited` was received.
    assert!(matches!(rx.try_recv(), Ok(Message::ChildExited)));

    drop(child_exit_watcher);
    unsafe {
        CloseHandle(event_handle).unwrap();
    }
}
