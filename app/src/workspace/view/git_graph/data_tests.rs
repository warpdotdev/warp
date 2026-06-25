//! Unit tests for the pure parsing functions in [`super`] (the data layer).

use super::*;

#[test]
fn status_count_is_one_per_changed_file() {
    // `git status --porcelain` prints one line per changed file (staged "A ",
    // unstaged " M", untracked "??").
    let porcelain = "A  src/a.rs\n M src/b.rs\n?? new.txt\n";
    assert_eq!(parse_status_count(porcelain), 3);
}

#[test]
fn status_count_is_zero_for_a_clean_tree() {
    assert_eq!(parse_status_count(""), 0);
    assert_eq!(parse_status_count("\n   \n"), 0);
}

/// Build a commit record following [`LOG_FORMAT`]'s field order (including the
/// trailing record separator).
fn rec(
    hash: &str,
    parents: &str,
    author_name: &str,
    author_email: &str,
    author_time: &str,
    decorate: &str,
    subject: &str,
) -> String {
    format!(
        "{hash}{US}{parents}{US}{author_name}{US}{author_email}{US}{author_time}{US}{decorate}{US}{subject}{RS}",
        US = UNIT_SEP,
        RS = RECORD_SEP,
    )
}

#[test]
fn parse_commit_log_parses_single_linear_commit() {
    let input = rec(
        "abcdef1234567890",
        "0987654321fedcba",
        "Ada Lovelace",
        "ada@example.com",
        "1700000000",
        "",
        "Initial work",
    );
    let commits = parse_commit_log(&input);

    assert_eq!(commits.len(), 1);
    let c = &commits[0];
    assert_eq!(c.hash, "abcdef1234567890");
    assert_eq!(c.short_hash, "abcdef12");
    assert_eq!(c.parents, vec!["0987654321fedcba".to_string()]);
    assert_eq!(c.author_name, "Ada Lovelace");
    assert_eq!(c.author_email, "ada@example.com");
    assert_eq!(c.author_time, 1_700_000_000);
    assert_eq!(c.subject, "Initial work");
    assert!(c.refs.is_empty());
}

#[test]
fn parse_commit_log_handles_multiple_records_joined_by_newline() {
    // git joins records with newlines; parsing must tolerate a leading newline.
    let input = format!(
        "{}\n{}",
        rec("h1", "h2", "A", "a@x", "100", "", "second"),
        rec("h2", "", "A", "a@x", "90", "", "first"),
    );
    let commits = parse_commit_log(&input);

    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].hash, "h1");
    assert_eq!(commits[1].hash, "h2");
    // A root commit has no parents.
    assert!(commits[1].parents.is_empty());
}

#[test]
fn parse_commit_log_parses_merge_parents() {
    let input = rec("m1", "p1 p2 p3", "A", "a@x", "100", "", "octopus merge");
    let commits = parse_commit_log(&input);

    assert_eq!(commits.len(), 1);
    assert_eq!(
        commits[0].parents,
        vec!["p1".to_string(), "p2".to_string(), "p3".to_string()]
    );
}

#[test]
fn parse_commit_log_keeps_subject_with_commas_and_spaces() {
    let input = rec(
        "h1",
        "h2",
        "A",
        "a@x",
        "100",
        "",
        "fix: handle a, b, and c edge cases",
    );
    let commits = parse_commit_log(&input);

    assert_eq!(commits[0].subject, "fix: handle a, b, and c edge cases");
}

#[test]
fn parse_commit_log_skips_blank_input() {
    assert!(parse_commit_log("").is_empty());
    assert!(parse_commit_log("\n\n").is_empty());
}

#[test]
fn parse_decorate_empty_yields_no_refs() {
    assert!(parse_decorate("").is_empty());
    assert!(parse_decorate("   ").is_empty());
}

#[test]
fn parse_decorate_head_pointing_to_branch() {
    let refs = parse_decorate("HEAD -> refs/heads/main");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, RefKind::Head);
    assert_eq!(refs[0].name, "main");
}

#[test]
fn parse_decorate_detached_head() {
    let refs = parse_decorate("HEAD, refs/remotes/origin/main");
    assert_eq!(refs[0].kind, RefKind::Head);
    assert_eq!(refs[0].name, "HEAD");
    assert_eq!(refs[1].kind, RefKind::RemoteBranch);
    assert_eq!(refs[1].name, "origin/main");
}

#[test]
fn parse_decorate_mixed_kinds() {
    let refs =
        parse_decorate("HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1.0.0");
    assert_eq!(refs.len(), 3);
    assert_eq!(
        (refs[0].kind, refs[0].name.as_str()),
        (RefKind::Head, "main")
    );
    assert_eq!(
        (refs[1].kind, refs[1].name.as_str()),
        (RefKind::RemoteBranch, "origin/main")
    );
    assert_eq!(
        (refs[2].kind, refs[2].name.as_str()),
        (RefKind::Tag, "v1.0.0")
    );
}

#[test]
fn parse_decorate_tag_carries_tag_prefix() {
    // git --decorate=full prefixes tag refs with "tag: " (both lightweight and
    // annotated tags). Without stripping that prefix the tag is dropped, so it
    // never renders in the graph.
    let refs = parse_decorate("tag: refs/tags/v2.0");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, RefKind::Tag);
    assert_eq!(refs[0].name, "v2.0");
}

#[test]
fn parse_decorate_local_branch_with_slash() {
    let refs = parse_decorate("refs/heads/feature/login");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, RefKind::LocalBranch);
    assert_eq!(refs[0].name, "feature/login");
}

#[test]
fn parse_decorate_shows_remote_symbolic_head() {
    // A remote's symbolic HEAD (origin/HEAD) is surfaced like any other remote
    // branch so the remote's default branch is visible on the graph.
    let refs = parse_decorate("refs/remotes/origin/HEAD, refs/remotes/origin/main");
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].kind, RefKind::RemoteBranch);
    assert_eq!(refs[0].name, "origin/HEAD");
    assert_eq!(refs[1].kind, RefKind::RemoteBranch);
    assert_eq!(refs[1].name, "origin/main");
}

#[test]
fn parse_decorate_ignores_unknown_tokens() {
    let refs = parse_decorate("grafted, HEAD -> refs/heads/main");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].name, "main");
}

#[test]
fn parse_commit_detail_extracts_header_and_files() {
    let input = format!(
        "Bob Committer{US}bob@example.com{US}1700000050{US}Subject line\n\nBody paragraph.{RS}\n\
         3\t1\tsrc/main.rs\n0\t0\tREADME.md\n-\t-\tlogo.png\n",
        US = UNIT_SEP,
        RS = RECORD_SEP,
    );
    let detail = parse_commit_detail(&input);

    assert_eq!(detail.committer_name, "Bob Committer");
    assert_eq!(detail.committer_email, "bob@example.com");
    assert_eq!(detail.committer_time, 1_700_000_050);
    // The full message retains both subject and body.
    assert_eq!(detail.message, "Subject line\n\nBody paragraph.");
    assert_eq!(detail.files.len(), 3);
    assert_eq!(
        detail.files[0],
        ChangedFile {
            path: "src/main.rs".to_string(),
            additions: 3,
            deletions: 1,
        }
    );
    // Binary files ("-") are treated as 0 insertions/deletions.
    assert_eq!(
        detail.files[2],
        ChangedFile {
            path: "logo.png".to_string(),
            additions: 0,
            deletions: 0,
        }
    );
}

#[test]
fn parse_numstat_normalizes_renamed_paths() {
    // Git's --numstat compresses renames/moves into a single path column.
    // We keep the post-rename real path so the tree splits cleanly and opening
    // a diff hits an existing file.
    let input = "\
        1\t1\tsrc/pane_group/{child_agent.rs => child_agent/mod.rs}\n\
        2\t0\tapp/src/{auth => crates/warp_server_auth/src}/lib.rs\n\
        3\t0\tapp/src/{ => v2}/file.rs\n\
        4\t0\told/path.rs => new/path.rs\n\
        5\t0\tsrc/plain.rs\n";
    let files = parse_numstat(input);

    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "src/pane_group/child_agent/mod.rs",
            "app/src/crates/warp_server_auth/src/lib.rs",
            "app/src/v2/file.rs",
            "new/path.rs",
            "src/plain.rs",
        ]
    );
}

#[test]
fn parse_commit_detail_handles_empty_numstat() {
    let input = format!(
        "Ann{US}ann@example.com{US}100{US}msg{RS}",
        US = UNIT_SEP,
        RS = RECORD_SEP
    );
    let detail = parse_commit_detail(&input);

    assert_eq!(detail.message, "msg");
    assert!(detail.files.is_empty());
}

// ===== scan_subdir_repos: subdirectory repository discovery (pure filesystem logic, built with temp directories, no real git needed) =====

/// Create directory `rel` under `root` and give it a `.git` marker directory
/// (simulating a repository root).
#[cfg(not(target_family = "wasm"))]
fn make_repo(root: &std::path::Path, rel: &str) {
    let dir = root.join(rel);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
}

/// Create a plain directory `rel` under `root` (without `.git`).
#[cfg(not(target_family = "wasm"))]
fn make_plain_dir(root: &std::path::Path, rel: &str) {
    std::fs::create_dir_all(root.join(rel)).unwrap();
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn scan_subdir_repos_depth_one_finds_direct_children_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_repo(root, "alpha"); // level 1 repository
    make_repo(root, "beta"); // level 1 repository
    make_plain_dir(root, "plain"); // level 1 plain directory
    make_repo(root, "plain/nested"); // level 2 repository (should not be found at depth=1)

    let found = scan_subdir_repos(root, 1);

    assert_eq!(found, vec![root.join("alpha"), root.join("beta")]);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn scan_subdir_repos_depth_zero_scans_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_repo(root, "alpha");

    assert!(scan_subdir_repos(root, 0).is_empty());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn scan_subdir_repos_depth_two_finds_nested_under_plain_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_plain_dir(root, "group");
    make_repo(root, "group/inner"); // level 2 repository

    let found = scan_subdir_repos(root, 2);

    assert_eq!(found, vec![root.join("group/inner")]);
}

#[test]
fn parse_branch_refs_splits_local_and_remote_sorted() {
    // Deliberately out of order, with a remote symbolic HEAD mixed in (should be
    // filtered out).
    let input = "refs/heads/main\n\
                 refs/remotes/origin/dev\n\
                 refs/heads/feature/login\n\
                 refs/remotes/origin/HEAD\n\
                 refs/remotes/origin/main\n";
    let branches = parse_branch_refs(input);

    // Locals first (sorted by name), remotes after (sorted by name); origin/HEAD
    // is filtered out.
    let got: Vec<(&str, RefKind)> = branches
        .iter()
        .map(|b| (b.display_name.as_str(), b.kind))
        .collect();
    assert_eq!(
        got,
        vec![
            ("feature/login", RefKind::LocalBranch),
            ("main", RefKind::LocalBranch),
            ("origin/dev", RefKind::RemoteBranch),
            ("origin/main", RefKind::RemoteBranch),
        ]
    );
    // ref_name keeps the full ref, for use with git log.
    assert_eq!(branches[0].ref_name, "refs/heads/feature/login");
    assert_eq!(branches[2].ref_name, "refs/remotes/origin/dev");
}

#[test]
fn parse_branch_refs_handles_empty() {
    assert!(parse_branch_refs("").is_empty());
    assert!(parse_branch_refs("\n  \n").is_empty());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn scan_subdir_repos_does_not_descend_into_found_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    make_repo(root, "outer"); // level 1 repository
    make_repo(root, "outer/sub"); // a nested repository inside it (submodule-like); should not be collected as a sibling

    let found = scan_subdir_repos(root, 3);

    assert_eq!(found, vec![root.join("outer")]);
}

// `diff_is_binary` only exists on native targets (it's used by the diff loaders,
// which are themselves native-only), so gate the tests the same way.
#[cfg(not(target_family = "wasm"))]
#[test]
fn diff_is_binary_detects_gits_binary_marker() {
    // For binary files git emits a top-level `Binary files ... differ` line
    // (no `@@` hunks) instead of a textual diff.
    let diff = "diff --git a/logo.png b/logo.png\n\
                index e69de29..a1b2c3d 100644\n\
                Binary files a/logo.png and b/logo.png differ\n";
    assert!(diff_is_binary(diff));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn diff_is_binary_is_false_for_a_textual_diff() {
    // A normal text diff has `@@` hunks and no top-level "Binary files" line;
    // the words appearing inside a `+`/`-` content line must not trip detection
    // (those lines never start with "Binary files ").
    let diff = "diff --git a/notes.txt b/notes.txt\n\
                @@ -1 +1 @@\n\
                -Binary files are great\n\
                +Binary files differ in spirit\n";
    assert!(!diff_is_binary(diff));
}

// `diff_is_symlink` / `symlink_target` are native-only for the same reason as
// `diff_is_binary` (they back the native diff loaders).
#[cfg(not(target_family = "wasm"))]
#[test]
fn diff_is_symlink_detects_the_120000_file_mode() {
    // A newly added symlink: git records the symlink file mode 120000.
    let added = "diff --git a/AGENTS.md b/AGENTS.md\n\
                 new file mode 120000\n\
                 index 0000000..681311e\n\
                 --- /dev/null\n\
                 +++ b/AGENTS.md\n\
                 @@ -0,0 +1 @@\n\
                 +CLAUDE.md\n\
                 \\ No newline at end of file\n";
    assert!(diff_is_symlink(added));

    // A retargeted symlink: the mode trails the `index` line instead.
    let retargeted = "diff --git a/AGENTS.md b/AGENTS.md\n\
                      index 681311e..b6fc4c6 120000\n\
                      --- a/AGENTS.md\n\
                      +++ b/AGENTS.md\n\
                      @@ -1 +1 @@\n\
                      -CLAUDE.md\n\
                      \\ No newline at end of file\n\
                      +hello\n\
                      \\ No newline at end of file\n";
    assert!(diff_is_symlink(retargeted));

    // A deleted symlink.
    let deleted = "diff --git a/AGENTS.md b/AGENTS.md\n\
                   deleted file mode 120000\n\
                   index 681311e..0000000\n";
    assert!(diff_is_symlink(deleted));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn diff_is_symlink_is_false_for_a_regular_file() {
    // A regular file's mode is 100644; an `index` line whose blob hash merely
    // ends in the digits "120000" must not be mistaken for the symlink mode.
    let diff = "diff --git a/notes.txt b/notes.txt\n\
                index 0000000..ab120000 100644\n\
                @@ -0,0 +1 @@\n\
                +120000\n";
    assert!(!diff_is_symlink(diff));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn symlink_target_reads_the_pointed_to_path() {
    // Added / retargeted: the new target is the added (`+`) content line.
    let added = "new file mode 120000\n\
                 +++ b/AGENTS.md\n\
                 @@ -0,0 +1 @@\n\
                 +CLAUDE.md\n\
                 \\ No newline at end of file\n";
    assert_eq!(symlink_target(added), "CLAUDE.md");

    // Deleted: no added line, so fall back to the removed (`-`) line.
    let deleted = "deleted file mode 120000\n\
                   --- a/AGENTS.md\n\
                   @@ -1 +0,0 @@\n\
                   -CLAUDE.md\n\
                   \\ No newline at end of file\n";
    assert_eq!(symlink_target(deleted), "CLAUDE.md");
}

#[test]
fn parse_stash_list_keeps_only_base_parent() {
    let us = UNIT_SEP;
    let rs = RECORD_SEP;
    // selector / hash / "base index untracked" / author / email / time / subject
    let record = format!(
        "stash@{{0}}{us}aaaa1111{us}base222 idx333 unt444{us}Ada{us}a@x{us}1000{us}WIP on main{rs}"
    );
    let nodes = parse_stash_list(&record);
    assert_eq!(nodes.len(), 1);
    let n = &nodes[0];
    assert_eq!(n.hash, "aaaa1111");
    assert_eq!(n.short_hash, "aaaa1111");
    // Only the base parent survives; the index/untracked parents are dropped so
    // they don't draw stray nodes.
    assert_eq!(n.parents, vec!["base222".to_string()]);
    assert_eq!(n.author_time, 1000);
    assert_eq!(n.subject, "WIP on main");
    assert_eq!(n.refs.len(), 1);
    assert_eq!(n.refs[0].kind, RefKind::Stash);
    assert_eq!(n.refs[0].name, "stash@{0}");
    assert!(is_stash_node(n));
}

#[test]
fn merge_stashes_orders_by_time_desc() {
    let commit = |hash: &str, t: i64| CommitNode {
        hash: hash.into(),
        short_hash: hash.into(),
        parents: vec![],
        author_name: String::new(),
        author_email: String::new(),
        author_time: t,
        subject: String::new(),
        refs: vec![],
    };
    let stash = |hash: &str, t: i64| CommitNode {
        refs: vec![RefLabel {
            kind: RefKind::Stash,
            name: "stash@{0}".into(),
        }],
        ..commit(hash, t)
    };
    let commits = vec![commit("c30", 30), commit("c10", 10)];
    let stashes = vec![stash("s20", 20)];
    let order: Vec<String> = merge_stashes(commits, stashes)
        .into_iter()
        .map(|c| c.hash)
        .collect();
    // 30 (commit) > 20 (stash) > 10 (commit).
    assert_eq!(order, vec!["c30", "s20", "c10"]);
}
