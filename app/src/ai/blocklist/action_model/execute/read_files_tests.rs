use super::super::file_revisions::FileRevision;
use super::*;

#[test]
fn failed_display_read_does_not_commit_staged_revision() {
    let tracker = FileRevisionTracker::default();
    let conversation_id = crate::ai::agent::conversation::AIConversationId::new();
    let path = "/tmp/transactional-read.txt".to_string();
    let original = FileRevision::present("original", None);
    let staged = FileRevision::present("staged-but-not-returned", None);
    tracker.record_revisions(conversation_id, [(path.clone(), original)]);

    record_observed_revisions(
        &tracker,
        conversation_id,
        vec![(path.clone(), staged)],
        &HashSet::new(),
    );

    let expected = tracker.expected_revisions(conversation_id, [path.clone()]);
    assert_eq!(expected.get(&path), Some(&original));
}
