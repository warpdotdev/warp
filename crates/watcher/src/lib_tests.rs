// `DebouncedEvent::time` requires `std::time::Instant` (the crate is not built with the
// `web-time` feature), so this cannot use the workspace's usual `instant::Instant`.
#[allow(clippy::disallowed_types)]
use std::time::Instant;

use notify_debouncer_full::notify::Event;

use super::*;

/// APP-5243 / WARP-CLIENT-DEV-XT3: an empty path must never reach the platform watcher, because
/// macOS turns it into a null-`CFError` release that traps and kills the process.
#[test]
fn empty_paths_never_reach_the_platform_watcher() {
    assert!(ensure_watchable_path(Path::new("")).is_err());

    let directory = std::env::temp_dir();
    assert_eq!(ensure_watchable_path(&directory).ok(), Some(directory));
}

// GH15698: on a case-insensitive filesystem (e.g. default macOS APFS), `Path::exists` resolves
// case-insensitively, so a case-only rename (`Foo` -> `foo`) makes *both* the old and new path
// look like they still exist. `path_exists_with_exact_case` fixes this by checking the parent
// directory's actual listing for an exact-case match instead.

#[test]
fn path_exists_with_exact_case_matches_the_exact_case_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Company")).unwrap();

    assert!(path_exists_with_exact_case(&dir.path().join("Company")));
    assert!(!path_exists_with_exact_case(&dir.path().join("company")));
    assert!(!path_exists_with_exact_case(&dir.path().join("COMPANY")));
}

#[test]
fn path_exists_with_exact_case_is_false_for_a_missing_path() {
    let dir = tempfile::tempdir().unwrap();

    assert!(!path_exists_with_exact_case(&dir.path().join("missing")));
}

#[test]
fn case_only_rename_is_treated_as_delete_of_old_case_and_create_of_new_case() {
    let dir = tempfile::tempdir().unwrap();
    let old_path = dir.path().join("Company");
    let new_path = dir.path().join("company");

    // Only the post-rename, new-case entry exists on disk.
    std::fs::create_dir(&new_path).unwrap();

    #[allow(clippy::disallowed_types)]
    let now = Instant::now();
    let raw_events = vec![
        DebouncedEvent::new(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Any)))
                .add_path(old_path.clone()),
            now,
        ),
        DebouncedEvent::new(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Any)))
                .add_path(new_path.clone()),
            now,
        ),
    ];

    let update = deduplicate_and_merge_raw_notifier_events(&raw_events).unwrap();

    assert!(update.deleted.contains(&old_path));
    assert!(update.added.contains(&new_path));
    assert!(!update.added.contains(&old_path));
    assert!(!update.deleted.contains(&new_path));
}
