use super::Lazy;

#[test]
fn provided_returns_value_without_computing() {
    let lazy: Lazy<u32, ()> = Lazy::provided(42);
    assert_eq!(*lazy.get(&()), 42);
}

#[test]
fn deferred_computes_once_and_caches() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn compute(source: &u32) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        source * 2
    }

    let lazy: Lazy<u32, u32> = Lazy::deferred(compute);
    assert_eq!(*lazy.get(&21), 42);
    assert_eq!(*lazy.get(&21), 42);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn deferred_closure_can_capture_per_instance_data() {
    // Each instance closes over its own offset, demonstrating why a boxed closure (rather
    // than a plain non-capturing fn pointer) is necessary.
    let offset = 10;
    let lazy: Lazy<u32, u32> = Lazy::deferred(move |source| *source + offset);
    assert_eq!(*lazy.get(&5), 15);
}

#[test]
fn get_if_cached_reflects_computed_state() {
    let lazy: Lazy<u32, u32> = Lazy::deferred(|source| *source);
    assert_eq!(lazy.get_if_cached(), None);
    assert_eq!(*lazy.get(&5), 5);
    assert_eq!(lazy.get_if_cached(), Some(&5));
}

#[test]
fn clone_shares_cache() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn compute(source: &u32) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        *source
    }

    let lazy: Lazy<u32, u32> = Lazy::deferred(compute);
    let cloned = lazy.clone();
    assert_eq!(*lazy.get(&7), 7);
    assert_eq!(*cloned.get(&7), 7);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn get_with_locks_source_to_compute_then_caches() {
    // Mirrors the real-world usage pattern: `S` (here a `u32`) lives behind a lock, and
    // `with_source` is responsible for acquiring it just long enough to call `compute`.
    let source_mutex = parking_lot::Mutex::new(21u32);
    let lazy: Lazy<u32, u32> = Lazy::deferred(|source| *source * 2);

    let value = lazy.get_with(|compute| {
        let guard = source_mutex.lock();
        compute(&guard)
    });
    assert_eq!(*value, 42);

    // Later reads must not need `with_source` (or the lock) again: hold the lock here to prove
    // `get_with` doesn't try to re-acquire it.
    let _guard = source_mutex.lock();
    assert_eq!(
        *lazy.get_with(|_compute| unreachable!("already cached")),
        42
    );
}

#[test]
fn get_with_skips_with_source_when_already_cached() {
    let lazy: Lazy<u32, u32> = Lazy::provided(5);
    assert_eq!(
        *lazy.get_with(|_compute| unreachable!("already provided")),
        5
    );
}
