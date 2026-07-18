use super::*;

#[test]
fn parses_nul_delimited_status_and_preserves_unusual_whitespace() {
    let output = "A\0new file.rs\0M\0src/line\nbreak.rs\0D\0old.rs\0";

    assert_eq!(
        parse_name_status_z(output),
        vec![
            FileChange {
                path: "new file.rs".to_string(),
                kind: GitChangeKind::Added,
            },
            FileChange {
                path: "src/line\nbreak.rs".to_string(),
                kind: GitChangeKind::Modified,
            },
            FileChange {
                path: "old.rs".to_string(),
                kind: GitChangeKind::Deleted,
            },
        ]
    );
}

#[test]
fn parses_renames_copies_and_conflicts() {
    let output = "R100\0old.rs\0new.rs\0C75\0source.rs\0copy.rs\0U\0conflict.rs\0AA\0both.rs\0";

    assert_eq!(
        parse_name_status_z(output),
        vec![
            FileChange {
                path: "new.rs".to_string(),
                kind: GitChangeKind::Renamed {
                    old_path: "old.rs".to_string(),
                },
            },
            FileChange {
                path: "copy.rs".to_string(),
                kind: GitChangeKind::Copied {
                    old_path: "source.rs".to_string(),
                },
            },
            FileChange {
                path: "conflict.rs".to_string(),
                kind: GitChangeKind::Conflicted,
            },
            FileChange {
                path: "both.rs".to_string(),
                kind: GitChangeKind::Conflicted,
            },
        ]
    );
}

#[test]
fn malformed_or_lossy_status_records_do_not_corrupt_following_entries() {
    let output = "Z\0ignored.rs\0M\0bad\u{fffd}.rs\0A\0kept.rs\0R100\0missing-new.rs\0";

    assert_eq!(
        parse_name_status_z(output),
        vec![FileChange {
            path: "kept.rs".to_string(),
            kind: GitChangeKind::Added,
        }]
    );
}

#[test]
fn parses_untracked_paths() {
    assert_eq!(
        parse_untracked_z("z.rs\0directory/a file.rs\0\0"),
        vec![
            FileChange {
                path: "z.rs".to_string(),
                kind: GitChangeKind::Untracked,
            },
            FileChange {
                path: "directory/a file.rs".to_string(),
                kind: GitChangeKind::Untracked,
            },
        ]
    );
}

#[test]
fn groups_conflicts_once_and_keeps_staged_and_unstaged_copies() {
    let staged = parse_name_status_z("M\0both.rs\0U\0conflict.rs\0");
    let changes = parse_name_status_z("M\0both.rs\0U\0conflict.rs\0");
    let snapshot = group_snapshot(staged, changes, Vec::new(), Vec::new(), false, true);

    assert_eq!(snapshot.merge_changes.len(), 1);
    assert_eq!(snapshot.staged_changes[0].path, "both.rs");
    assert_eq!(snapshot.changes[0].path, "both.rs");
}

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

#[test]
fn uses_reset_to_unstage_when_head_exists() {
    assert_eq!(
        mutation_args(
            &GitMutation::UnstagePaths(vec!["old.rs".to_string(), "new.rs".to_string()]),
            true
        ),
        vec!["reset", "-q", "HEAD", "--", "old.rs", "new.rs"]
    );
    assert_eq!(
        mutation_args(&GitMutation::UnstageAll, true),
        vec!["reset", "-q", "HEAD", "--"]
    );
}

#[test]
fn stages_both_sides_of_a_rename() {
    let kind = GitChangeKind::Renamed {
        old_path: "old.rs".to_string(),
    };
    assert_eq!(kind.paths_for_action("new.rs"), vec!["old.rs", "new.rs"]);
    assert_eq!(
        mutation_args(
            &GitMutation::StagePaths(kind.paths_for_action("new.rs")),
            true
        ),
        vec!["add", "--", "old.rs", "new.rs"]
    );
}

#[test]
fn uses_cached_rm_to_unstage_an_unborn_branch() {
    assert_eq!(
        mutation_args(
            &GitMutation::UnstagePaths(vec!["a file.rs".to_string()]),
            false
        ),
        vec![
            "rm",
            "--cached",
            "-q",
            "--ignore-unmatch",
            "--",
            "a file.rs"
        ]
    );
    assert_eq!(
        mutation_args(&GitMutation::UnstageAll, false),
        vec!["rm", "--cached", "-q", "-r", "--", "."]
    );
}
