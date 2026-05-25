use std::collections::HashMap;
use std::path::PathBuf;

use super::{FileId, FileNode, FileType, FileUploadState, FolderId, FolderNode, ImportedNode};
use crate::drive::import::nodes::{UploadResult, UploadStatus};

fn mock_tree() -> FileUploadState {
    let mut folder_id_to_node = HashMap::new();
    let mut file_id_to_node = HashMap::new();

    let mut root_folder = FolderNode::new(String::new(), FolderId(0));

    let top_level_file = FileNode::new(
        "top_level".to_string(),
        FileType::Notebook,
        PathBuf::new(),
        FolderId(0),
    );

    let mut top_level_folder = FolderNode::new("top_folder".to_string(), FolderId::root_id());

    let second_level_file = FileNode::new(
        "second_level".to_string(),
        FileType::Workflow,
        PathBuf::new(),
        FolderId(1),
    );

    top_level_folder
        .children
        .push(ImportedNode::File(FileId(1)));
    root_folder.children.push(ImportedNode::File(FileId(0)));
    root_folder.children.push(ImportedNode::Folder(FolderId(1)));

    file_id_to_node.insert(FileId(1), second_level_file);
    file_id_to_node.insert(FileId(0), top_level_file);
    folder_id_to_node.insert(FolderId(1), top_level_folder);
    folder_id_to_node.insert(FolderId(0), root_folder);

    let state = FileUploadState {
        folder_id_to_node,
        file_id_to_node,
    };

    assert_eq!(state.debug_print(), "(top_folder(second_level), top_level)");
    state
}

#[test]
fn test_state_update_in_tree() {
    let mut state = mock_tree();

    state.mark_folder_synced(
        UploadResult::Success("mock-folder".to_string()),
        FolderId(1),
    );

    // Only the second level file is loaded. Top-level folder should be loaded but
    // root folder should still be loading.
    state.update_tree_with_file_upload_result(
        UploadResult::Success("mock-markdown".to_string()),
        FileId(1),
    );
    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(1))
            .expect("Should exist")
            .status,
        UploadStatus::Loaded("mock-folder".to_string())
    );

    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(0))
            .expect("Should exist")
            .status,
        UploadStatus::Loading
    );

    // Top level file is also loaded. Root level folder should be marked as loaded.
    state.update_tree_with_file_upload_result(
        UploadResult::Success("mock-root".to_string()),
        FileId(0),
    );
    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(0))
            .expect("Should exist")
            .status,
        UploadStatus::Loaded(String::new())
    );

    // Set second level file to be loading. All of the folders should be loading.
    state.set_file_and_parent_to_loading(FileId(1));
    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(1))
            .expect("Should exist")
            .status,
        UploadStatus::Loading
    );

    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(0))
            .expect("Should exist")
            .status,
        UploadStatus::Loading
    );

    // Second level file finished loading. All of the folders should be loaded.
    state.update_tree_with_file_upload_result(
        UploadResult::Success("mock-folder".to_string()),
        FileId(1),
    );
    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(1))
            .expect("Should exist")
            .status,
        UploadStatus::Loaded("mock-folder".to_string())
    );

    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(0))
            .expect("Should exist")
            .status,
        UploadStatus::Loaded(String::new())
    );
}

#[test]
fn test_empty_folders_update() {
    let mut folder_id_to_node = HashMap::new();
    let file_id_to_node = HashMap::new();

    let mut root_folder = FolderNode::new(String::new(), FolderId(0));
    let empty_folder_1 = FolderNode::new("empty".to_string(), FolderId(0));
    let empty_folder_2 = FolderNode::new("empty1".to_string(), FolderId(0));
    root_folder.children.push(ImportedNode::Folder(FolderId(1)));
    root_folder.children.push(ImportedNode::Folder(FolderId(2)));

    folder_id_to_node.insert(FolderId(0), root_folder);
    folder_id_to_node.insert(FolderId(1), empty_folder_1);
    folder_id_to_node.insert(FolderId(2), empty_folder_2);

    let mut state = FileUploadState {
        folder_id_to_node,
        file_id_to_node,
    };

    assert_eq!(state.debug_print(), "(empty, empty1)");

    state.mark_folder_synced(
        UploadResult::Success("mock-folder".to_string()),
        FolderId(1),
    );
    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(1))
            .expect("Should exist")
            .status,
        UploadStatus::Loaded("mock-folder".to_string())
    );

    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(0))
            .expect("Should exist")
            .status,
        UploadStatus::Loading
    );

    // Errored uploads should also be considered as completed uploads.
    state.mark_folder_synced(UploadResult::Error("Failure".to_string()), FolderId(2));
    assert_eq!(
        state
            .folder_id_to_node
            .get(&FolderId(0))
            .expect("Should exist")
            .status,
        UploadStatus::Loaded(String::new())
    );
}
