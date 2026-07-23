use super::*;

fn comment(
    id: &str,
    author: &str,
    body: &str,
    parent_id: Option<&str>,
    timestamp: &str,
) -> InsertReviewComment {
    InsertReviewComment {
        comment_id: id.to_owned(),
        author: author.to_owned(),
        last_modified_timestamp: timestamp.to_owned(),
        comment_body: body.to_owned(),
        parent_comment_id: parent_id.map(str::to_owned),
        comment_location: None,
        html_url: None,
    }
}

#[test]
fn groups_roots_and_depth_first_replies_deterministically() {
    let comments = vec![
        comment("4", "dana", "Early", Some("1"), "2024-01-01T00:30:00Z"),
        comment("2", "bob", "Later", Some("1"), "2024-01-01T01:00:00Z"),
        comment("3", "charlie", "Nested", Some("2"), "2024-01-01T02:00:00Z"),
        comment("5", "eve", "Second root", None, "2024-01-01T00:00:00Z"),
        comment("1", "alice", "Root", None, "2024-01-01T00:00:00Z"),
    ];

    let threads = group_review_comment_threads(&comments);

    assert_eq!(threads.len(), 2);
    assert_eq!(
        threads[0]
            .comments()
            .iter()
            .map(|comment| comment.comment_id())
            .collect::<Vec<_>>(),
        vec!["1", "4", "2", "3"]
    );
    assert_eq!(threads[1].root().comment_id(), "5");
    assert_eq!(
        format_review_comment_thread(&threads[0]),
        "**@alice**:\nRoot\n---\n**@dana**:\nEarly\n---\n**@bob**:\nLater\n---\n**@charlie**:\nNested"
    );
}

#[test]
fn retains_orphaned_replies_as_roots() {
    let comments = vec![comment(
        "2",
        "bob",
        "Orphan",
        Some("missing"),
        "2024-01-01T00:00:00Z",
    )];

    let threads = group_review_comment_threads(&comments);

    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].root().comment_id(), "2");
    assert_eq!(threads[0].missing_parent_id(), Some("missing"));
}
