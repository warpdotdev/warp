use super::*;

#[test]
fn local_revision_detects_same_mtime_content_change() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("same-mtime.txt");
    std::fs::write(&path, "original\n").unwrap();
    let original_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let original = read_local_revision(path.to_str().unwrap()).unwrap();

    std::fs::write(&path, "modified\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_mtime)
        .unwrap();
    let modified = read_local_revision(path.to_str().unwrap()).unwrap();

    assert_ne!(original, modified);
}

#[test]
fn oversized_local_revision_fails_closed_without_hashing_the_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("oversized.txt");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(u64::from(MAX_REVISION_READ_BYTES) + 1)
        .unwrap();

    let revision = read_local_revision(path.to_str().unwrap()).unwrap();

    assert_eq!(revision, FileRevision::Uneditable);
}

#[test]
fn remote_revision_detects_same_epoch_millis_content_change() {
    let context = |content: &str| remote_server::proto::FileContextProto {
        file_name: "/remote/file.txt".to_string(),
        content: Some(
            remote_server::proto::file_context_proto::Content::TextContent(content.to_string()),
        ),
        last_modified_epoch_millis: Some(1_000),
        ..Default::default()
    };

    let original = revision_from_remote_context(context("original\n")).unwrap();
    let modified = revision_from_remote_context(context("modified\n")).unwrap();

    assert_ne!(original, modified);
}

#[test]
fn missing_reread_allows_recreate_from_current_state() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("deleted.txt");
    let path = path.to_string_lossy().to_string();
    std::fs::write(&path, "original\n").unwrap();
    let tracker = FileRevisionTracker::default();
    let conversation_id = AIConversationId::new();

    tracker.record_revisions(conversation_id, read_local_revisions([path.clone()]));
    std::fs::remove_file(&path).unwrap();
    tracker.record_revisions(conversation_id, read_local_revisions([path.clone()]));

    let expected = tracker.expected_revisions(conversation_id, [path.clone()]);
    assert_eq!(expected.get(&path), Some(&FileRevision::Missing));

    std::fs::write(&path, "recreated\n").unwrap();
    assert_ne!(
        read_local_revision(&path).unwrap(),
        *expected.get(&path).unwrap()
    );
}
