use super::super::file_revisions::FileRevision;
use super::*;

#[test]
fn failed_display_read_does_not_commit_staged_revision() {
    let tracker = FileRevisionTracker::default();
    let conversation_id = crate::ai::agent::conversation::AIConversationId::new();
    let path = "/tmp/transactional-read.txt".to_string();
    let original = FileRevision::present("original", None);
    tracker.record_revisions(conversation_id, [(path.clone(), original)]);
    let failed_result = ReadFileContextResult {
        file_contexts: Vec::new(),
        file_revisions: Vec::new(),
        failed_files: vec![ReadFilesFailedFile {
            path: path.clone(),
            message: "Permission denied".to_string(),
        }],
    };
    tracker.record_revisions(conversation_id, revisions_from_local_result(&failed_result));

    let expected = tracker.expected_revisions(conversation_id, [path.clone()]);
    assert_eq!(expected.get(&path), Some(&original));
}

#[test]
fn revision_is_captured_by_the_same_read_as_returned_content() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("combined-read.txt");
    std::fs::write(&path, "observed\r\n").unwrap();
    let path = path.to_string_lossy().to_string();
    let result = futures::executor::block_on(read_local_file_context(
        &[crate::ai::agent::FileLocations {
            name: path.clone(),
            lines: Vec::new(),
        }],
        None,
        None,
        None,
        None,
    ))
    .unwrap();
    std::fs::write(&path, "external mutation\n").unwrap();

    let revisions = revisions_from_local_result(&result);

    assert_eq!(
        result.file_contexts[0].content,
        crate::ai::agent::AnyFileContent::StringContent("observed\n".to_string())
    );
    assert_eq!(
        revisions,
        vec![(path, FileRevision::present("observed\r\n", None))]
    );
}
