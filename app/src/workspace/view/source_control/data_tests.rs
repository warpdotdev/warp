use super::*;

#[test]
fn parses_history_parents_and_decorations() {
    let output = concat!(
        "\u{1f}abcdef0123456789\u{1f}parent1 parent2\u{1f}Ada\u{1f}1700000000\u{1f}Merge topic\u{1f}",
        "HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1.0\u{1f}\u{1e}"
    );

    let commits = parse_history(output);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].parents, vec!["parent1", "parent2"]);
    assert_eq!(commits[0].short_hash(), "abcdef0");
    assert_eq!(
        commits[0].refs,
        vec![
            GitRefLabel {
                name: "HEAD".to_string(),
                kind: GitRefKind::Head,
            },
            GitRefLabel {
                name: "main".to_string(),
                kind: GitRefKind::LocalBranch,
            },
            GitRefLabel {
                name: "origin/main".to_string(),
                kind: GitRefKind::RemoteBranch,
            },
            GitRefLabel {
                name: "v1.0".to_string(),
                kind: GitRefKind::Tag,
            },
        ]
    );
}

#[test]
fn parses_multiline_and_empty_commit_bodies() {
    let output = concat!(
        "\u{1f}abc\u{1f}\u{1f}Ada\u{1f}1\u{1f}Subject\u{1f}\u{1f}First line\nSecond line\n\n\u{1e}",
        "\u{1f}def\u{1f}abc\u{1f}Grace\u{1f}2\u{1f}No body\u{1f}\u{1f}\u{1e}"
    );

    let commits = parse_history(output);
    assert_eq!(commits[0].body, "First line\nSecond line");
    assert_eq!(commits[1].body, "");
}

#[test]
fn parses_shortstat_variants_and_omits_commits_without_stats() {
    let output = concat!(
        "\u{1e}full\n\n 3 files changed, 12 insertions(+), 4 deletions(-)\n",
        "\u{1e}deletions\n\n 2 files changed, 7 deletions(-)\n",
        "\u{1e}merge\n",
        "\u{1e}singular\n\n 1 file changed, 1 insertion(+)\n"
    );

    let stats = parse_shortstat_log(output);
    assert_eq!(
        stats.get("full"),
        Some(&CommitStats {
            files_changed: 3,
            insertions: 12,
            deletions: 4,
        })
    );
    assert_eq!(
        stats.get("deletions"),
        Some(&CommitStats {
            files_changed: 2,
            insertions: 0,
            deletions: 7,
        })
    );
    assert_eq!(stats.get("merge"), None);
    assert_eq!(
        stats.get("singular"),
        Some(&CommitStats {
            files_changed: 1,
            insertions: 1,
            deletions: 0,
        })
    );
}

#[test]
fn skips_malformed_history_records() {
    let valid = "\u{1f}abc\u{1f}\u{1f}Ada\u{1f}1\u{1f}Root\u{1f}\u{1f}\u{1e}";
    let invalid = "\u{1f}missing-fields\u{1e}";

    assert_eq!(parse_history(&format!("{invalid}{valid}")).len(), 1);
}
