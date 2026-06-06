use futures_lite::future;

use super::Condition;

#[test]
fn test_condition_multiple_waiters() {
    future::block_on(async {
        let condition = Condition::new();

        let mut listener1 = Box::pin(condition.wait());
        let mut listener2 = Box::pin(condition.wait());

        // Neither listener should be ready.
        assert!(future::poll_once(&mut listener1).await.is_none());
        assert!(future::poll_once(&mut listener2).await.is_none());

        condition.set();

        // Now, both should complete.
        assert!(future::poll_once(listener1).await.is_some());
        assert!(future::poll_once(listener2).await.is_some());
    })
}

#[test]
fn test_condition_after_set() {
    future::block_on(async {
        let condition = Condition::new();
        condition.set();

        // After the condition is set, waiting should complete immediately.
        assert!(future::poll_once(condition.wait()).await.is_some());
    })
}

#[test]
fn test_condition_multiple_sets() {
    future::block_on(async {
        let condition = Condition::new();

        // Test that multiple interleavings of `wait` and `set` all complete as expected.
        let first = condition.wait();
        condition.set();
        let second = condition.wait();
        condition.set();
        let third = condition.wait();

        assert!(future::poll_once(first).await.is_some());
        assert!(future::poll_once(second).await.is_some());
        assert!(future::poll_once(third).await.is_some());
    })
}
