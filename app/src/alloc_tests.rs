use super::purge_unused_pages;

/// Guards the `arena.<i>.purge` mallctl name and argument convention: jemalloc
/// rejects a malformed name or a non-null value pointer with a non-zero return,
/// which surfaces here as `false`.
#[test]
fn test_purge_unused_pages_succeeds_when_jemalloc_is_enabled() {
    let expected = cfg!(feature = "jemalloc");

    assert_eq!(purge_unused_pages(), expected);
    // Purging is driven from a periodic check, so it has to stay valid when
    // there is nothing left to reclaim.
    assert_eq!(purge_unused_pages(), expected);
}
