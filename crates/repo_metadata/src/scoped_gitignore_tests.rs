use std::fs;

use super::{gitignore_file_metadata, GitignoreRuleCache, GitignoreRuleKey};

#[test]
fn is_ignored_requires_explicit_refresh() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let ignored = root.join("ignored.txt");
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(&ignored, "ignored").unwrap();

    let mut cache = GitignoreRuleCache::empty_for_root(&root);

    assert!(!cache.is_ignored(&ignored, false, false));
    assert!(cache.is_ignored_with_refresh(&ignored, false, false));
}

#[test]
fn active_keys_are_ordered_root_to_leaf_then_codebase_index_files() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let a = root.join("a");
    let b = a.join("b");
    let file = b.join("file.rs");
    fs::create_dir_all(&b).unwrap();
    fs::write(root.join(".gitignore"), "root\n").unwrap();
    fs::write(a.join(".gitignore"), "a\n").unwrap();
    fs::write(b.join(".gitignore"), "b\n").unwrap();
    fs::write(root.join(".warpindexingignore"), "index\n").unwrap();
    fs::write(&file, "fn main() {}\n").unwrap();

    let mut cache = GitignoreRuleCache::empty_for_root(&root);
    cache.root_scoped_ignore_files = vec![root.join(".warpindexingignore")];
    cache.refresh_for_path(&file);
    let traversal = cache.scoped_traversal_for_path(&file);

    assert_eq!(
        traversal.active_keys,
        vec![
            GitignoreRuleKey::File(root.join(".gitignore")),
            GitignoreRuleKey::File(a.join(".gitignore")),
            GitignoreRuleKey::File(b.join(".gitignore")),
            GitignoreRuleKey::File(root.join(".warpindexingignore")),
        ]
    );
}

#[test]
fn later_whitelist_rule_reincludes_path() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    fs::write(root.join(".gitignore"), "*.txt\n").unwrap();
    fs::write(child.join(".gitignore"), "!allowed.txt\n").unwrap();
    fs::write(child.join("allowed.txt"), "allowed").unwrap();
    fs::write(child.join("blocked.txt"), "blocked").unwrap();

    let mut cache = GitignoreRuleCache::empty_for_root(&root);

    assert!(!cache.is_ignored_with_refresh(&child.join("allowed.txt"), false, false));
    assert!(cache.is_ignored_with_refresh(&child.join("blocked.txt"), false, false));
}

#[test]
fn codebase_index_ignore_files_have_final_precedence() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    fs::write(root.join(".gitignore"), "*.txt\n").unwrap();
    fs::write(child.join(".gitignore"), "!allowed.txt\n").unwrap();
    fs::write(root.join(".warpindexingignore"), "child/allowed.txt\n").unwrap();
    fs::write(child.join("allowed.txt"), "allowed").unwrap();

    let mut cache = GitignoreRuleCache::for_codebase_index(&root);

    assert!(cache.is_ignored_with_refresh(&child.join("allowed.txt"), false, false));
}

#[test]
fn gitignore_metadata_hash_changes_for_same_length_edits() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let gitignore = root.join(".gitignore");

    fs::write(&gitignore, "one.txt\n").unwrap();
    let first_metadata = gitignore_file_metadata(&gitignore).unwrap();

    fs::write(&gitignore, "two.txt\n").unwrap();
    let second_metadata = gitignore_file_metadata(&gitignore).unwrap();

    assert_eq!(first_metadata.len, second_metadata.len);
    assert_ne!(first_metadata.content_hash, second_metadata.content_hash);
}
