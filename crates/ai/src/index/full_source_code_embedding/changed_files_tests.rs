use super::*;
use std::path::PathBuf;

// Helper function to create a PathBuf from a string
fn pb(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[test]
fn test_basic_merge_non_conflicting() {
    // Initial: delete set {a}, upsert set {b}
    let mut changes1 = ChangedFiles::default();
    changes1.deletions.insert(pb("a"));
    changes1.upsertions.insert(pb("b"));

    // Later changes: delete set {c}, upsert set {d}
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("c"));
    changes2.upsertions.insert(pb("d"));

    // Merge changes
    changes1.merge_subsequent(changes2);

    // Expected: deletions {a, c}, upsertions {b, d}
    assert_eq!(changes1.deletions.len(), 2);
    assert!(changes1.deletions.contains(&pb("a")));
    assert!(changes1.deletions.contains(&pb("c")));

    assert_eq!(changes1.upsertions.len(), 2);
    assert!(changes1.upsertions.contains(&pb("b")));
    assert!(changes1.upsertions.contains(&pb("d")));
}

#[test]
fn test_delete_then_upsert() {
    // Initial: delete set {file1}
    let mut changes1 = ChangedFiles::default();
    changes1.deletions.insert(pb("file1"));

    // Later changes: upsert set {file1}
    let mut changes2 = ChangedFiles::default();
    changes2.upsertions.insert(pb("file1"));

    // Merge changes
    changes1.merge_subsequent(changes2);

    // Expected: deletions {}, upsertions {file1}
    assert_eq!(changes1.deletions.len(), 0);
    assert_eq!(changes1.upsertions.len(), 1);
    assert!(changes1.upsertions.contains(&pb("file1")));
}

#[test]
fn test_upsert_then_delete() {
    // Initial: upsert set {file1}
    let mut changes1 = ChangedFiles::default();
    changes1.upsertions.insert(pb("file1"));

    // Later changes: delete set {file1}
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("file1"));

    // Merge changes
    changes1.merge_subsequent(changes2);

    // Expected: upsertions {}, deletions {file1}
    assert_eq!(changes1.upsertions.len(), 0);
    assert_eq!(changes1.deletions.len(), 1);
    assert!(changes1.deletions.contains(&pb("file1")));
}

#[test]
fn test_delete_then_delete() {
    // Initial: delete set {file1}
    let mut changes1 = ChangedFiles::default();
    changes1.deletions.insert(pb("file1"));

    // Later changes: delete set {file1} again
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("file1"));

    // Merge changes
    changes1.merge_subsequent(changes2);

    // Expected: deletions {file1}, upsertions {}
    assert_eq!(changes1.deletions.len(), 1);
    assert!(changes1.deletions.contains(&pb("file1")));
    assert_eq!(changes1.upsertions.len(), 0);
}

#[test]
fn test_upsert_then_upsert() {
    // Initial: upsert set {file1}
    let mut changes1 = ChangedFiles::default();
    changes1.upsertions.insert(pb("file1"));

    // Later changes: upsert set {file1} again
    let mut changes2 = ChangedFiles::default();
    changes2.upsertions.insert(pb("file1"));

    // Merge changes
    changes1.merge_subsequent(changes2);

    // Expected: upsertions {file1}, deletions {}
    assert_eq!(changes1.upsertions.len(), 1);
    assert!(changes1.upsertions.contains(&pb("file1")));
    assert_eq!(changes1.deletions.len(), 0);
}

#[test]
fn test_empty_sets() {
    // Test 1: Empty merged into populated
    let mut changes1 = ChangedFiles::default();
    changes1.upsertions.insert(pb("file1"));
    changes1.deletions.insert(pb("file2"));

    let changes2 = ChangedFiles::default();

    changes1.merge_subsequent(changes2);

    // Should remain unchanged
    assert_eq!(changes1.upsertions.len(), 1);
    assert!(changes1.upsertions.contains(&pb("file1")));
    assert_eq!(changes1.deletions.len(), 1);
    assert!(changes1.deletions.contains(&pb("file2")));

    // Test 2: Populated merged into empty
    let mut changes3 = ChangedFiles::default();

    let mut changes4 = ChangedFiles::default();
    changes4.upsertions.insert(pb("file3"));
    changes4.deletions.insert(pb("file4"));

    changes3.merge_subsequent(changes4);

    // Should take all changes
    assert_eq!(changes3.upsertions.len(), 1);
    assert!(changes3.upsertions.contains(&pb("file3")));
    assert_eq!(changes3.deletions.len(), 1);
    assert!(changes3.deletions.contains(&pb("file4")));
}

#[test]
fn test_multiple_sequential_merges() {
    // Initial: upsert set {a}, delete set {b}
    let mut changes1 = ChangedFiles::default();
    changes1.upsertions.insert(pb("a"));
    changes1.deletions.insert(pb("b"));

    // First merge: delete a, delete c, upsert b
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("a"));
    changes2.deletions.insert(pb("c"));
    changes2.upsertions.insert(pb("b"));

    // Second merge: upsert c, delete d
    let mut changes3 = ChangedFiles::default();
    changes3.upsertions.insert(pb("c"));
    changes3.deletions.insert(pb("d"));

    // Apply first merge
    changes1.merge_subsequent(changes2);

    // After first merge:
    // Expected: upsertions {b}, deletions {a, c}
    assert_eq!(changes1.upsertions.len(), 1);
    assert!(changes1.upsertions.contains(&pb("b")));
    assert_eq!(changes1.deletions.len(), 2);
    assert!(changes1.deletions.contains(&pb("a")));
    assert!(changes1.deletions.contains(&pb("c")));

    // Apply second merge
    changes1.merge_subsequent(changes3);

    // After second merge:
    // Expected: upsertions {b, c}, deletions {a, d}
    assert_eq!(changes1.upsertions.len(), 2);
    assert!(changes1.upsertions.contains(&pb("b")));
    assert!(changes1.upsertions.contains(&pb("c")));
    assert_eq!(changes1.deletions.len(), 2);
    assert!(changes1.deletions.contains(&pb("a")));
    assert!(changes1.deletions.contains(&pb("d")));
}

#[test]
fn test_rename_then_delete() {
    // Initial: rename {a -> b}
    let mut changes1 = ChangedFiles::default();
    changes1.deletions.insert(pb("a"));
    changes1.upsertions.insert(pb("b"));

    // Later changes: delete {b}
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("b"));

    changes1.merge_subsequent(changes2);

    // Expected: deletions {a, b}, upsertions {}
    // Note that we don't know whether b had any prior content,
    // so we can't assume it was a rename.
    assert_eq!(changes1.deletions.len(), 2);
    assert!(changes1.deletions.contains(&pb("a")));
    assert!(changes1.deletions.contains(&pb("b")));
    assert_eq!(changes1.upsertions.len(), 0);
}

#[test]
fn test_upsert_then_rename() {
    // Initial: upsert {a}
    let mut changes1 = ChangedFiles::default();
    changes1.upsertions.insert(pb("a"));

    // Later changes: rename {a -> b}
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("a"));
    changes2.upsertions.insert(pb("b"));

    changes1.merge_subsequent(changes2);

    // Expected: deletions {a}, upsertions {b}
    // Note that we don't know whether a had any prior content,
    // so we can't assume it was a rename.
    assert_eq!(changes1.deletions.len(), 1);
    assert!(changes1.deletions.contains(&pb("a")));
    assert_eq!(changes1.upsertions.len(), 1);
    assert!(changes1.upsertions.contains(&pb("b")));
}

#[test]
fn test_rename_then_rename() {
    // Initial: rename {a -> b}
    let mut changes1 = ChangedFiles::default();
    changes1.deletions.insert(pb("a"));
    changes1.upsertions.insert(pb("b"));

    // Later changes: rename {b -> c}
    let mut changes2 = ChangedFiles::default();
    changes2.deletions.insert(pb("b"));
    changes2.upsertions.insert(pb("c"));

    changes1.merge_subsequent(changes2);

    // Expected: deletions {a, b}, upsertions {c}
    // Note that we don't know whether b had any prior content,
    // so we can't assume it was a rename.
    assert_eq!(changes1.deletions.len(), 2);
    assert!(changes1.deletions.contains(&pb("a")));
    assert!(changes1.deletions.contains(&pb("b")));
    assert_eq!(changes1.upsertions.len(), 1);
    assert!(changes1.upsertions.contains(&pb("c")));
}
