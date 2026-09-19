use super::*;

#[test]
fn remote_revision_detects_same_epoch_millis_content_change() {
    let context = |content: &str| remote_server::proto::FileContextProto {
        file_name: "/remote/file.txt".to_string(),
        content: Some(
            remote_server::proto::file_context_proto::Content::TextContent(content.to_string()),
        ),
        last_modified_epoch_millis: Some(1_000),
        content_sha256: Sha256::digest(content).to_vec(),
        ..Default::default()
    };

    let original = revision_from_remote_context(context("original\n"));
    let modified = revision_from_remote_context(context("modified\n"));

    assert_ne!(original, modified);
}

#[test]
fn remote_revision_ignores_millisecond_timestamp_metadata() {
    let context = |last_modified_epoch_millis| remote_server::proto::FileContextProto {
        file_name: "/remote/file.txt".to_string(),
        content: Some(
            remote_server::proto::file_context_proto::Content::TextContent("content\n".to_string()),
        ),
        last_modified_epoch_millis,
        content_sha256: Sha256::digest("content\n").to_vec(),
        ..Default::default()
    };

    assert_eq!(
        revision_from_remote_context(context(Some(1_000))),
        revision_from_remote_context(context(Some(1_001)))
    );
}

#[test]
fn missing_reread_allows_recreate_from_current_state() {
    let path = "/tmp/deleted.txt".to_string();
    let tracker = FileRevisionTracker::default();
    let conversation_id = AIConversationId::new();
    tracker.record_revisions(
        conversation_id,
        [(path.clone(), FileRevision::present("original\n", None))],
    );
    tracker.record_revisions(conversation_id, [(path.clone(), FileRevision::Missing)]);

    let expected = tracker.expected_revisions(conversation_id, [path.clone()]);
    assert_eq!(expected.get(&path), Some(&FileRevision::Missing));

    assert_ne!(
        FileRevision::present("recreated\n", None),
        *expected.get(&path).unwrap()
    );
}
