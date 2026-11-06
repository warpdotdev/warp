use super::MissCache;

#[test]
fn test_contains_reflects_recorded_misses() {
    let cache = MissCache::new(3);

    assert!(!cache.contains("a"));
    cache.insert("a".to_string());
    assert!(cache.contains("a"));
}

#[test]
fn test_inserting_an_existing_entry_is_a_no_op() {
    let cache = MissCache::new(3);

    cache.insert("a".to_string());
    cache.insert("a".to_string());
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_stops_growing_at_capacity() {
    let capacity = 4;
    let cache = MissCache::new(capacity);

    for i in 0..capacity * 3 {
        cache.insert(format!("miss-{i}"));
        assert!(cache.len() <= capacity);
    }
    assert_eq!(cache.len(), capacity);
}

#[test]
fn test_evicts_in_fifo_order_regardless_of_lookups() {
    let cache = MissCache::new(3);
    cache.insert("a".to_string());
    cache.insert("b".to_string());
    cache.insert("c".to_string());

    for _ in 0..5 {
        assert!(cache.contains("a"));
    }

    cache.insert("d".to_string());
    assert!(
        !cache.contains("a"),
        "the oldest-inserted entry should have been evicted regardless of being looked up again"
    );
    assert!(
        cache.contains("b"),
        "a newer entry should not have been evicted"
    );
    assert!(cache.contains("c"));
    assert!(cache.contains("d"));
}
