use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;

use warp_completer::completer::EngineDirEntry;

#[test]
fn test_parse_directory_names() {
    let names = super::parse_directory_names("./realdir\0.\0./link_to_dir\0");
    assert_eq!(
        names,
        HashSet::from_iter(["realdir".to_owned(), "link_to_dir".to_owned()])
    );
    assert!(super::parse_directory_names("").is_empty());
}

#[test]
fn test_upgrade_directory_symlinks() {
    let mut entries = vec![
        EngineDirEntry::test_file("link_to_dir"),
        EngineDirEntry::test_file("link_to_file"),
        EngineDirEntry::test_file("not_a_symlink"),
        EngineDirEntry::test_dir("real_dir"),
    ];
    let unresolved_symlinks =
        HashSet::from_iter(["link_to_dir".to_owned(), "link_to_file".to_owned()]);
    let guest_dirs = HashSet::from_iter([
        "link_to_dir".to_owned(),
        "real_dir".to_owned(),
        "not_a_symlink".to_owned(),
    ]);

    super::upgrade_directory_symlinks(&mut entries, &unresolved_symlinks, &guest_dirs);

    let by_name: HashMap<&str, &EngineDirEntry> = entries
        .iter()
        .map(|entry| (entry.file_name(), entry))
        .collect();
    assert!(by_name["link_to_dir"].is_dir());
    assert!(!by_name["link_to_file"].is_dir());
    assert!(!by_name["not_a_symlink"].is_dir());
    assert!(by_name["real_dir"].is_dir());
}
