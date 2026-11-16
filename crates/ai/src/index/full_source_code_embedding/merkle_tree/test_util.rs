use repo_metadata::{DirectoryEntry, Entry};
use virtual_fs::{Stub, VirtualFS};

use super::MerkleTree;
use crate::index::full_source_code_embedding::fragment_metadata::LeafToFragmentMetadata;

/// Construct a test Merkle tree with the following structure:
/// ```
/// root.txt
/// top_dir/
/// ├── file1.txt
/// ├── subdir_a/
/// │   ├── file2.txt
/// │   └── file3.txt
/// └── subdir_b/
///     └── file4.txt
/// ```
#[cfg(test)]
pub async fn construct_test_merkle_tree(
    dirs: &virtual_fs::Dirs,
    sandbox: &mut VirtualFS,
) -> (MerkleTree, LeafToFragmentMetadata) {
    sandbox.mkdir("top_dir");
    sandbox.mkdir("top_dir/subdir_a");
    sandbox.mkdir("top_dir/subdir_b");
    sandbox.with_files(vec![
        Stub::FileWithContent("root.txt", "root content"),
        Stub::FileWithContent("top_dir/file1.txt", "file1 content"),
        Stub::FileWithContent("top_dir/subdir_a/file2.txt", "file2 content"),
        Stub::FileWithContent("top_dir/subdir_a/file3.txt", "file3 content"),
        Stub::FileWithContent("top_dir/subdir_b/file4.txt", "file4 content"),
    ]);

    let mut root_dir_entry = DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests()).unwrap(),
        children: vec![],
        ignored: false,
        loaded: true,
    };
    root_dir_entry
        .find_or_insert_child(&dirs.tests().join("root.txt"))
        .expect("Should be able to insert root file");

    let mut top_dir_entry = DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &dirs.tests().join("top_dir"),
        )
        .unwrap(),
        children: vec![],
        ignored: false,
        loaded: true,
    };
    top_dir_entry
        .find_or_insert_child(&dirs.tests().join("top_dir/file1.txt"))
        .expect("Should be able to insert file1");

    let mut subdir_a_entry = DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &dirs.tests().join("top_dir/subdir_a"),
        )
        .unwrap(),
        children: vec![],
        ignored: false,
        loaded: true,
    };
    subdir_a_entry
        .find_or_insert_child(&dirs.tests().join("top_dir/subdir_a/file2.txt"))
        .expect("Should be able to insert file2");
    subdir_a_entry
        .find_or_insert_child(&dirs.tests().join("top_dir/subdir_a/file3.txt"))
        .expect("Should be able to insert file3");

    let mut subdir_b_entry = DirectoryEntry {
        path: warp_util::standardized_path::StandardizedPath::try_from_local(
            &dirs.tests().join("top_dir/subdir_b"),
        )
        .unwrap(),
        children: vec![],
        ignored: false,
        loaded: true,
    };
    subdir_b_entry
        .find_or_insert_child(&dirs.tests().join("top_dir/subdir_b/file4.txt"))
        .expect("Should be able to insert file4");

    top_dir_entry
        .children
        .push(Entry::Directory(subdir_a_entry));
    top_dir_entry
        .children
        .push(Entry::Directory(subdir_b_entry));

    root_dir_entry
        .children
        .push(Entry::Directory(top_dir_entry));

    MerkleTree::try_new(Entry::Directory(root_dir_entry))
        .await
        .expect("Should be able to construct tree")
}
