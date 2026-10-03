use super::allocator_stats;

#[test]
fn allocator_stats_are_only_reported_when_jemalloc_is_the_allocator() {
    let stats = allocator_stats();

    if cfg!(feature = "jemalloc") {
        let stats = stats.expect("jemalloc builds should report allocator statistics");

        // A running process always holds some live allocations, and the
        // allocator cannot have handed out more than it has mapped.
        assert!(stats.allocated > 0);
        assert!(stats.active >= stats.allocated);
        assert!(stats.resident >= stats.active);
        assert!(stats.mapped >= stats.resident);
    } else {
        assert!(stats.is_none());
    }
}

#[cfg(feature = "jemalloc")]
#[test]
fn allocated_tracks_a_large_live_allocation() {
    const ALLOCATION_SIZE: u64 = 64 * 1024 * 1024;
    // These statistics are process-wide, so tests running in parallel on other
    // threads perturb them.  Require only that most of the allocation is
    // visible rather than asserting an exact delta.
    const TOLERANCE: u64 = ALLOCATION_SIZE / 2;

    let before = allocator_stats().expect("jemalloc should report statistics");

    // Write to every page so the allocation cannot be optimized away or left
    // as untouched virtual address space.
    let mut buffer = vec![0u8; ALLOCATION_SIZE as usize];
    buffer.iter_mut().for_each(|byte| *byte = 1);

    let during = allocator_stats().expect("jemalloc should report statistics");
    assert!(during.allocated.saturating_sub(before.allocated) >= TOLERANCE);

    drop(buffer);

    // Freeing returns the bytes to `allocated` immediately, even though the
    // underlying pages may stay resident until the allocator purges them --
    // which is exactly the distinction these statistics exist to expose.
    let after = allocator_stats().expect("jemalloc should report statistics");
    assert!(during.allocated.saturating_sub(after.allocated) >= TOLERANCE);
}
