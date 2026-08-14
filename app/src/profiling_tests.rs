use super::*;

#[test]
fn memory_diagnostics_includes_footprint_and_jemalloc_stats() {
    let diagnostics = memory_diagnostics_for_sentry(123_456_789);
    let serde_json::Value::Object(map) = diagnostics else {
        panic!("expected memory_diagnostics_for_sentry to return a JSON object");
    };

    assert_eq!(
        map.get("footprint_at_threshold_trip_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(123_456_789)
    );
    assert!(
        map.get("footprint_at_dump_bytes")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "expected a freshly-read footprint to be attached alongside the triggering footprint"
    );

    // jemalloc is the global allocator whenever `heap_usage_tracking` is
    // enabled, so its allocator stats should always be readable here -- this
    // is what lets us tell apart memory freed before the dump, memory
    // retained by the allocator, and non-heap memory.
    assert!(
        map.get("jemalloc_allocated_bytes")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|value| value > 0)
    );
    assert!(map.contains_key("jemalloc_resident_bytes"));
    assert!(map.contains_key("jemalloc_retained_bytes"));
    assert!(map.contains_key("jemalloc_mapped_bytes"));
}
