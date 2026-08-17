use std::sync::Arc;

use super::get_or_parse;

/// Reading the same unchanged `.gitignore` twice must return the exact same
/// `Arc<Gitignore>` instance (not merely an equal one), since a distinct
/// instance means a distinct compiled regex and pool were allocated.
#[test]
fn reuses_cached_entry_for_unchanged_file() {
    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();

    let first = get_or_parse(&path);
    let second = get_or_parse(&path);

    assert!(
        Arc::ptr_eq(&first, &second),
        "an unchanged .gitignore should reuse the cached Gitignore instance"
    );
}

/// Editing a cached `.gitignore` (changing both its content and length, so
/// the fingerprint changes even under coarse filesystem mtime resolution)
/// must invalidate the cache entry and produce a fresh `Gitignore` whose
/// rules reflect the new content.
#[test]
fn rebuilds_when_file_changes() {
    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();

    let before = get_or_parse(&path);
    assert!(before.matched("target", true).is_ignore());

    std::fs::write(&path, "target/\nnode_modules/\n").unwrap();
    let after = get_or_parse(&path);

    assert!(
        !Arc::ptr_eq(&before, &after),
        "an edited .gitignore must not reuse the stale cached instance"
    );
    assert!(after.matched("node_modules", true).is_ignore());
}

/// The cache is bounded: once more distinct paths are parsed than the
/// configured capacity, the least-recently-touched entry is evicted first.
#[test]
fn evicts_least_recently_used_entry_over_capacity() {
    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();

    // `MAX_CACHED_GITIGNORES` is 3 under `#[cfg(test)]`.
    let paths: Vec<_> = (0..3)
        .map(|i| {
            let path = temp_dir.path().join(format!("gitignore_{i}"));
            std::fs::write(&path, "target/\n").unwrap();
            path
        })
        .collect();
    let first_instances: Vec<_> = paths.iter().map(|path| get_or_parse(path)).collect();

    // A fourth distinct path pushes the cache over capacity, evicting the
    // least-recently-touched entry (`paths[0]`, touched first and never
    // re-touched).
    let fourth_path = temp_dir.path().join("gitignore_3");
    std::fs::write(&fourth_path, "target/\n").unwrap();
    get_or_parse(&fourth_path);

    let refetched_first = get_or_parse(&paths[0]);
    assert!(
        !Arc::ptr_eq(&first_instances[0], &refetched_first),
        "the least-recently-used entry should have been evicted and re-parsed"
    );
}
