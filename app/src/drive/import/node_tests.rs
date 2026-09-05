use std::collections::HashMap;
use std::path::PathBuf;

use futures_lite::io::Cursor;

use super::{
    FileId, FileNode, FileType, FileUploadState, FolderId, FolderNode, ImportedNode,
    MAX_IMPORT_FILE_SIZE_BYTES, parse_file, read_bounded_by_import_cap,
};
use crate::drive::import::nodes::{FileContent, UploadResult, UploadStatus};

/// Builds workflow YAML that parses to a single `Workflow::Command` and is exactly `total_len`
/// bytes, by padding a trailing comment out to the target size.
fn workflow_yaml_of_size(total_len: usize) -> Vec<u8> {
    let mut content = b"name: My Workflow\ncommand: echo hello\n# ".to_vec();
    let filler_len = total_len
        .checked_sub(content.len() + 1)
        .expect("total_len should be large enough for the workflow YAML preamble");
    content.extend(std::iter::repeat_n(b'a', filler_len));
    content.push(b'\n');
    content
}

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

#[tokio::test]
async fn test_parse_file_notebook_under_cap_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Heading\n\nSome notes.").unwrap();

    let content = parse_file(path, FileType::Notebook)
        .await
        .expect("under-cap file should parse");
    assert!(matches!(content, FileContent::Notebook(text) if text.contains("Heading")));
}

#[tokio::test]
async fn test_parse_file_notebook_at_cap_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("at_cap.md");
    std::fs::write(&path, vec![b'a'; MAX_IMPORT_FILE_SIZE_BYTES as usize]).unwrap();

    let content = parse_file(path, FileType::Notebook)
        .await
        .expect("exactly-at-cap file should parse");
    assert!(
        matches!(content, FileContent::Notebook(text) if text.len() == MAX_IMPORT_FILE_SIZE_BYTES as usize)
    );
}

#[tokio::test]
async fn test_parse_file_notebook_over_cap_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.md");
    std::fs::write(&path, vec![b'a'; (MAX_IMPORT_FILE_SIZE_BYTES + 1) as usize]).unwrap();

    let Err(err) = parse_file(path, FileType::Notebook).await else {
        panic!("over-cap file should be rejected");
    };
    assert!(err.to_string().contains("import size limit"));
}

#[tokio::test]
async fn test_parse_file_workflow_under_cap_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workflow.yaml");
    std::fs::write(
        &path,
        "name: My Workflow\ncommand: echo hello\narguments: []\n",
    )
    .unwrap();

    let content = parse_file(path, FileType::Workflow)
        .await
        .expect("under-cap file should parse");
    assert!(matches!(content, FileContent::Workflow { workflows, .. } if workflows.len() == 1));
}

#[tokio::test]
async fn test_parse_file_workflow_at_cap_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("at_cap.yaml");
    std::fs::write(
        &path,
        workflow_yaml_of_size(MAX_IMPORT_FILE_SIZE_BYTES as usize),
    )
    .unwrap();

    let content = parse_file(path, FileType::Workflow)
        .await
        .expect("exactly-at-cap file should parse");
    assert!(matches!(content, FileContent::Workflow { workflows, .. } if workflows.len() == 1));
}

#[tokio::test]
async fn test_parse_file_workflow_over_cap_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.yaml");
    std::fs::write(&path, vec![b'a'; (MAX_IMPORT_FILE_SIZE_BYTES + 1) as usize]).unwrap();

    let Err(err) = parse_file(path, FileType::Workflow).await else {
        panic!("over-cap file should be rejected");
    };
    assert!(err.to_string().contains("import size limit"));
}

// `parse_file`'s over-cap tests above write caps+1 bytes to disk, which always trips the
// `metadata`-based fast-reject in `read_capped_for_import` before the bounded read underneath it
// ever runs. The tests below drive `read_bounded_by_import_cap` directly over an in-memory
// `AsyncRead` so that guard is exercised the same way it would be for a source whose reported
// size understates what it actually yields (e.g. a file that grows after being stat'd).
#[tokio::test]
async fn test_read_bounded_by_import_cap_at_cap_succeeds() {
    let data = vec![b'a'; MAX_IMPORT_FILE_SIZE_BYTES as usize];

    let bytes = read_bounded_by_import_cap(Cursor::new(data.clone()))
        .await
        .expect("exactly-at-cap reader should be accepted");
    assert_eq!(bytes, data);
}

#[tokio::test]
async fn test_read_bounded_by_import_cap_over_cap_errors() {
    let data = vec![b'a'; (MAX_IMPORT_FILE_SIZE_BYTES + 1) as usize];

    let Err(err) = read_bounded_by_import_cap(Cursor::new(data)).await else {
        panic!("over-cap reader should be rejected");
    };
    assert!(err.to_string().contains("import size limit"));
}
