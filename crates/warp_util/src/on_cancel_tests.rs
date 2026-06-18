use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::future::{AbortHandle, Abortable, Aborted};

use crate::on_cancel::OnCancelFutureExt;

#[test]
fn test_ready_future_doesnt_call_callback() {
    let callback_called = AtomicBool::new(false);

    let future = async {}.on_cancel(|| callback_called.store(true, Ordering::SeqCst));
    warpui_core::r#async::block_on(future);

    assert!(!callback_called.load(Ordering::Relaxed));
}

#[test]
fn test_aborted_future_calls_callback() {
    let callback_called = AtomicBool::new(false);

    let (handle, registration) = AbortHandle::new_pair();
    let future = Abortable::new(
        async {}.on_cancel(|| callback_called.store(true, Ordering::SeqCst)),
        registration,
    );

    // Abort the future before it is ever polled.
    handle.abort();
    let future_result = warpui_core::r#async::block_on(future);

    // The future should be aborted and the callback should have been called.
    assert_eq!(future_result, Err(Aborted));
    assert!(callback_called.load(Ordering::Relaxed));
}
