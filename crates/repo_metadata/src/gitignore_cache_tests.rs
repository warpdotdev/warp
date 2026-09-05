use std::sync::Arc;

use super::get_or_parse;

/// Reading the same unchanged `.gitignore` twice must return the exact same `Arc<Gitignore>`
/// instance (not merely an equal one), since a distinct instance means a distinct compiled
/// regex and pool were allocated.
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

/// A same-length edit that lands within the filesystem's mtime resolution must still be
/// detected: content hashing must not mistake it for an unchanged file.
#[test]
fn rebuilds_when_content_changes_at_the_same_length() {
    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();
    let original_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

    let before = get_or_parse(&path);
    assert!(before.matched("target", true).is_ignore());

    let replacement = "assets/\n";
    assert_eq!(
        replacement.len(),
        "target/\n".len(),
        "the replacement content must keep the file's byte length unchanged \
         to exercise the same-size case"
    );
    std::fs::write(&path, replacement).unwrap();
    // Force the mtime back to its original value so this test deterministically exercises the
    // same-mtime case regardless of the filesystem's actual clock resolution. `set_modified`
    // requires a handle opened for write on Windows (a read-only handle lacks
    // FILE_WRITE_ATTRIBUTES), even though the same call succeeds on a read-only handle on Unix.
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_mtime)
        .unwrap();
    let after = get_or_parse(&path);

    assert!(
        !Arc::ptr_eq(&before, &after),
        "a same-length content change must not reuse the stale cached instance"
    );
    assert!(after.matched("assets", true).is_ignore());
    assert!(!after.matched("target", true).is_ignore());
}

/// A `.gitignore` first touched while transiently unreadable (e.g. a permissions race during
/// checkout) must not cache the resulting empty matcher: a failed read must never be cached,
/// regardless of the file's later content.
#[cfg(unix)]
#[test]
fn recovers_after_a_transient_read_failure() {
    use std::os::unix::fs::PermissionsExt;

    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        // Running as a user (e.g. root) that ignores permission bits: the failure this test
        // exercises can't be reproduced here.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }

    // The very first access happens while the file is unreadable: it must fail open (an empty
    // matcher) without poisoning the cache with that result.
    let during_failure = get_or_parse(&path);
    assert!(!during_failure.matched("target", true).is_ignore());

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let after_recovery = get_or_parse(&path);
    assert!(
        after_recovery.matched("target", true).is_ignore(),
        "a transient read failure on first access must not permanently cache \
         an empty result"
    );
}

/// A `.gitignore` with a malformed line (`[z-a]` is an invalid character range) makes
/// `Gitignore::new` report a partial error. That result must never be cached: two calls
/// against the same still-broken content must each produce an independent parse, and fixing
/// the line must not be shadowed by a previously cached partial result.
#[test]
fn does_not_cache_a_failed_parse() {
    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n[z-a]\n").unwrap();

    let first = get_or_parse(&path);
    let second = get_or_parse(&path);
    assert!(
        !Arc::ptr_eq(&first, &second),
        "a result from a file with a parse error must never be cached"
    );

    std::fs::write(&path, "target/\n").unwrap();
    let fixed = get_or_parse(&path);
    let fixed_again = get_or_parse(&path);
    assert!(
        Arc::ptr_eq(&fixed, &fixed_again),
        "once the error is fixed, the valid result should be cached normally"
    );
}

/// Exceeding the cache's byte budget evicts the least-recently-used entry first.
#[test]
fn evicts_least_recently_used_entry_over_capacity() {
    super::clear_for_test();
    let temp_dir = tempfile::tempdir().unwrap();

    // Each file is 8 bytes ("target/\n"), so under the test budget of 24 source bytes, three
    // files fit (24) but a fourth does not (32) and forces an eviction.
    let paths: Vec<_> = (0..3)
        .map(|i| {
            let path = temp_dir.path().join(format!("gitignore_{i}"));
            std::fs::write(&path, "target/\n").unwrap();
            path
        })
        .collect();
    let first_instances: Vec<_> = paths.iter().map(|path| get_or_parse(path)).collect();

    let fourth_path = temp_dir.path().join("gitignore_3");
    std::fs::write(&fourth_path, "target/\n").unwrap();
    get_or_parse(&fourth_path);

    let refetched_first = get_or_parse(&paths[0]);
    assert!(
        !Arc::ptr_eq(&first_instances[0], &refetched_first),
        "the least-recently-used entry should have been evicted and re-parsed"
    );
}
