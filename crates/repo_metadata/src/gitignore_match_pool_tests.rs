use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::run;

/// Returns the calling thread's regex-automata pool "thread ID" the same way
/// `regex_automata::util::pool::Pool` does internally: a process-wide counter,
/// assigned once per OS thread on its first call and stable for that thread's
/// lifetime. `regex_automata` doesn't expose this counter, so this test drives one of
/// its own that is assigned by exactly the same rule, to observe how many distinct OS
/// threads a batch of calls actually lands on.
fn thread_identity() -> usize {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static ID: Cell<Option<usize>> = const { Cell::new(None) };
    }
    ID.with(|id| {
        if let Some(id) = id.get() {
            return id;
        }
        let assigned = COUNTER.fetch_add(1, Ordering::Relaxed);
        id.set(Some(assigned));
        assigned
    })
}

/// Verifies the property the module docs rely on: no matter how many callers ask for
/// work concurrently, only a fixed, small set of distinct OS threads ever executes it.
///
/// Removing the dispatch in `run` (i.e. calling `f()` directly on the caller's own
/// thread) makes this test fail: many distinct caller threads below would each execute
/// `thread_identity()` on themselves, observing far more than `THREAD_COUNT` distinct
/// identities.
#[test]
fn run_executes_on_a_fixed_small_set_of_threads() {
    const CALLERS: usize = 64;

    let observed_identities = Arc::new(Mutex::new(HashSet::new()));
    let handles: Vec<_> = (0..CALLERS)
        .map(|_| {
            let observed_identities = observed_identities.clone();
            std::thread::spawn(move || {
                let identity = run(thread_identity);
                observed_identities.lock().unwrap().insert(identity);
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let observed_identities = observed_identities.lock().unwrap();
    assert!(
        observed_identities.len() <= super::THREAD_COUNT,
        "expected at most THREAD_COUNT ({}) distinct executing threads across {CALLERS} \
         concurrent callers, but observed {}: {observed_identities:?}",
        super::THREAD_COUNT,
        observed_identities.len(),
    );
    assert!(
        !observed_identities.is_empty(),
        "the dedicated pool should have executed at least one job"
    );
}
