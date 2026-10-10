use std::path::Path;

use command::Stdio;
use command::r#async::Command;
use tempfile::TempDir;

use super::{
    RepositoryInfo, StackDiscoveryResult, detect_current_branch, detect_current_branch_display,
    get_pr_for_branch, is_gh_auth_error, is_gh_missing_error,
};

/// Helper: run a git command inside the given repo directory.
async fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run git");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_info_from_gh_output_parses_name_and_owner() {
    // No url in the output => host is absent.
    assert_eq!(
        super::repository_info_from_gh_output(
            r#"{"name":"warp-internal","owner":{"login":"warpdotdev"}}"#
        )
        .unwrap(),
        RepositoryInfo {
            name: "warp-internal".to_owned(),
            owner: Some("warpdotdev".to_owned()),
            host: None,
        }
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_info_from_gh_output_parses_host_from_url() {
    assert_eq!(
        super::repository_info_from_gh_output(
            r#"{"name":"warp-internal","owner":{"login":"warpdotdev"},"url":"https://github.com/warpdotdev/warp-internal"}"#
        )
        .unwrap(),
        RepositoryInfo {
            name: "warp-internal".to_owned(),
            owner: Some("warpdotdev".to_owned()),
            host: Some("github.com".to_owned()),
        }
    );
}

#[cfg(all(feature = "local_fs", unix))]
#[tokio::test]
async fn get_repository_info_returns_none_when_gh_cannot_resolve_github_repo() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let (_dir, repo) = init_repo().await;

    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let gh_path = fake_bin.path().join("gh");
    fs::write(
        &gh_path,
        "#!/bin/sh\nprintf 'none of the git remotes configured for this repository point to a known GitHub host\\n' >&2\nexit 1\n",
    )
    .expect("failed to write fake gh");
    let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh_path, permissions).unwrap();

    let path_env = format!(
        "{}:{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    assert_eq!(
        super::get_repository_info(&repo, Some(&path_env))
            .await
            .unwrap(),
        None
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_info_from_gh_output_rejects_missing_name() {
    assert!(super::repository_info_from_gh_output(r#"{"owner":{"login":"warpdotdev"}}"#).is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_info_from_gh_output_rejects_missing_owner_login() {
    assert!(
        super::repository_info_from_gh_output(r#"{"name":"warp-internal","owner":{}}"#).is_err()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_info_from_gh_output_rejects_empty_fields() {
    assert!(
        super::repository_info_from_gh_output(r#"{"name":"","owner":{"login":"warpdotdev"}}"#)
            .is_err()
    );
    assert!(
        super::repository_info_from_gh_output(r#"{"name":"warp-internal","owner":{"login":""}}"#)
            .is_err()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_info_from_gh_output_rejects_malformed_json() {
    assert!(super::repository_info_from_gh_output("not json").is_err());
}

/// Creates a temp git repo with one commit and returns `(dir_handle, repo_path)`.
async fn init_repo() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().to_path_buf();

    git(&path, &["init", "-b", "main"]).await;
    git(&path, &["config", "user.email", "test@test.com"]).await;
    git(&path, &["config", "user.name", "Test"]).await;
    git(&path, &["commit", "--allow-empty", "-m", "initial"]).await;

    (dir, path)
}

#[cfg(all(feature = "local_fs", unix))]
#[tokio::test]
async fn get_repository_info_reads_gh_repo_view() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let (_dir, repo) = init_repo().await;

    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let gh_path = fake_bin.path().join("gh");
    fs::write(
        &gh_path,
        "#!/bin/sh\nprintf '{\"name\":\"warp-internal\",\"owner\":{\"login\":\"warpdotdev\"}}\\n'\n",
    )
    .expect("failed to write fake gh");
    let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh_path, permissions).unwrap();

    let path_env = format!(
        "{}:{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    assert_eq!(
        super::get_repository_info(&repo, Some(&path_env))
            .await
            .unwrap(),
        Some(RepositoryInfo {
            name: "warp-internal".to_owned(),
            owner: Some("warpdotdev".to_owned()),
            host: None,
        })
    );
}

#[test]
fn detects_missing_gh_errors() {
    assert!(is_gh_missing_error(
        "Failed to execute gh command: No such file or directory (os error 2)"
    ));
    assert!(is_gh_missing_error(
        "Failed to execute gh command: program not found"
    ));

    assert!(!is_gh_missing_error(
        "gh command failed: GraphQL: authentication required; run gh auth login"
    ));
    assert!(!is_gh_missing_error(
        "Post \"https://api.github.com/graphql\": dial tcp: lookup api.github.com: no such host"
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn detects_no_pr_for_branch_errors() {
    assert!(super::is_no_pr_for_branch_error(
        "gh command failed: no pull requests found for branch \"feature-a\""
    ));
    assert!(super::is_no_pr_for_branch_error(
        "gh command failed: no open pull requests found for branch \"feature-a\""
    ));
    assert!(super::is_no_pr_for_branch_error(
        "GraphQL: NO OPEN PULL REQUESTS FOUND FOR BRANCH feature-a"
    ));
    assert!(!super::is_no_pr_for_branch_error("authentication required"));
    assert!(!super::is_no_pr_for_branch_error("repository not found"));
}

#[cfg(feature = "local_fs")]
#[test]
fn detects_repository_lookup_not_applicable_errors() {
    assert!(super::is_repository_lookup_not_applicable_error(
        "gh command failed: none of the git remotes configured for this repository point to a known GitHub host"
    ));
    assert!(super::is_repository_lookup_not_applicable_error(
        "gh command failed: no GitHub remotes"
    ));
    assert!(super::is_repository_lookup_not_applicable_error(
        "gh command failed: not a GitHub repository"
    ));
    assert!(!super::is_repository_lookup_not_applicable_error(
        "authentication required"
    ));
    assert!(!super::is_repository_lookup_not_applicable_error(
        "repository not found"
    ));
}

#[tokio::test]
async fn on_normal_branch_returns_branch_name() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "-b", "feature-xyz"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "feature-xyz");
    assert_eq!(
        detect_current_branch_display(&repo).await.unwrap(),
        "feature-xyz"
    );
}

#[tokio::test]
async fn detached_head_raw_returns_head() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "HEAD");
}

#[tokio::test]
async fn detached_head_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(
        result, "HEAD",
        "display variant should not return literal HEAD"
    );
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

#[tokio::test]
async fn get_pr_for_branch_returns_none_for_detached_head() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;
    assert_eq!(get_pr_for_branch(&repo, None).await.unwrap(), None);
}

#[cfg(feature = "local_fs")]
#[tokio::test]
async fn committed_branch_files_excludes_uncommitted_and_untracked() {
    let (_dir, repo) = init_repo().await;
    // Branch off main; the merge base is main's initial commit.
    git(&repo, &["checkout", "-b", "feature"]).await;

    // Commit a new file on the feature branch — this SHOULD appear in the
    // committed branch diff.
    std::fs::write(repo.join("committed.txt"), "line1\nline2\n").expect("write committed.txt");
    git(&repo, &["add", "committed.txt"]).await;
    git(&repo, &["commit", "-m", "add committed.txt"]).await;

    // Further-modify the committed file in the working tree (uncommitted) and
    // add an untracked file. Neither is part of the PR's committed history, so
    // neither should appear, and the committed file's counts must reflect only
    // the committed change (2 added lines, not 3).
    std::fs::write(repo.join("committed.txt"), "line1\nline2\nline3\n")
        .expect("modify committed.txt");
    std::fs::write(repo.join("untracked.txt"), "new\n").expect("write untracked.txt");

    let entries = super::get_committed_branch_file_entries(&repo)
        .await
        .expect("committed branch files");

    assert_eq!(
        entries.len(),
        1,
        "expected only the committed file: {entries:?}"
    );
    assert_eq!(entries[0].path, "committed.txt");
    assert_eq!(entries[0].additions, 2);
    assert_eq!(entries[0].deletions, 0);
    assert!(
        !entries.iter().any(|e| e.path == "untracked.txt"),
        "untracked files must be excluded: {entries:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn get_pr_for_branch_does_not_require_origin_remote() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::PrInfo;

    let (_dir, repo) = init_repo().await;

    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let gh_path = fake_bin.path().join("gh");
    fs::write(
        &gh_path,
        "#!/bin/sh\nprintf '{\"number\":123,\"url\":\"https://github.com/warp/warp/pull/123\",\"state\":\"OPEN\",\"isDraft\":true,\"baseRefName\":\"main\"}\\n'\n",
    )
    .expect("failed to write fake gh");
    let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh_path, permissions).unwrap();

    let path_env = format!(
        "{}:{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    assert_eq!(
        get_pr_for_branch(&repo, Some(&path_env)).await.unwrap(),
        Some(PrInfo {
            number: 123,
            url: "https://github.com/warp/warp/pull/123".to_string(),
            state: "OPEN".to_string(),
            draft: true,
            base_branch: "main".to_string(),
        })
    );
}

#[cfg(unix)]
#[tokio::test]
async fn get_pr_for_branch_returns_none_when_gh_finds_no_pr() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let (_dir, repo) = init_repo().await;

    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let gh_path = fake_bin.path().join("gh");
    fs::write(
        &gh_path,
        "#!/bin/sh\nprintf 'no pull requests found for branch \"main\"\\n' >&2\nexit 1\n",
    )
    .expect("failed to write fake gh");
    let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh_path, permissions).unwrap();

    let path_env = format!(
        "{}:{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    assert_eq!(
        get_pr_for_branch(&repo, Some(&path_env)).await.unwrap(),
        None
    );
}

#[cfg(unix)]
#[tokio::test]
async fn get_pr_for_branch_returns_none_when_gh_cannot_resolve_github_repo() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let (_dir, repo) = init_repo().await;

    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let gh_path = fake_bin.path().join("gh");
    fs::write(
        &gh_path,
        "#!/bin/sh\nprintf 'none of the git remotes configured for this repository point to a known GitHub host\\n' >&2\nexit 1\n",
    )
    .expect("failed to write fake gh");
    let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh_path, permissions).unwrap();

    let path_env = format!(
        "{}:{}",
        fake_bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    assert_eq!(
        get_pr_for_branch(&repo, Some(&path_env)).await.unwrap(),
        None
    );
}
#[test]
fn detects_gh_auth_errors() {
    assert!(is_gh_auth_error(
        "You are not logged in to any GitHub hosts"
    ));
    assert!(is_gh_auth_error(
        "GraphQL: authentication required; run gh auth login"
    ));
    assert!(is_gh_auth_error(
        "To get started with GitHub CLI, run: gh auth login"
    ));

    assert!(!is_gh_auth_error(
        "Post \"https://api.github.com/graphql\": dial tcp: lookup api.github.com: no such host"
    ));
    assert!(!is_gh_auth_error("no pull requests found for branch"));
}

#[tokio::test]
async fn detached_tag_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["tag", "v1.0"]).await;
    git(&repo, &["checkout", "v1.0"]).await;

    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(result, "HEAD");
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

// ── PR stack discovery ───────────────────────────────────────────────────────
// Fixtures below are the exact response shapes captured against a live
// GitHub-native stack (warpdotdev/warp-cli-survey, stack #7, PRs #4/#5/#6).

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_topology_response_parses_two_layer_response() {
    let json = r#"[
      {
        "id": 245657,
        "number": 7,
        "base": { "ref": "master" },
        "open": true,
        "pull_requests": [
          { "number": 4, "state": "open", "draft": false, "merged_at": null,
            "head": { "ref": "demo/stack-1-greeting-pkg", "sha": "5b3d024" } },
          { "number": 5, "state": "open", "draft": false, "merged_at": null,
            "head": { "ref": "demo/stack-2-cli-integration", "sha": "668a0b5" } }
        ]
      }
    ]"#;

    let topology = super::parse_stack_topology_response(json).unwrap();
    assert_eq!(topology.stack_number, 7);
    assert_eq!(topology.trunk_ref, "master");
    assert_eq!(topology.layer_numbers, vec![4, 5]);
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_topology_response_parses_multi_layer_response() {
    let json = r#"[
      {
        "number": 7,
        "base": { "ref": "master" },
        "pull_requests": [
          { "number": 4, "state": "open", "draft": false, "merged_at": null, "head": { "ref": "a", "sha": "1" } },
          { "number": 5, "state": "open", "draft": false, "merged_at": null, "head": { "ref": "b", "sha": "2" } },
          { "number": 6, "state": "open", "draft": false, "merged_at": null, "head": { "ref": "c", "sha": "3" } }
        ]
      }
    ]"#;

    let topology = super::parse_stack_topology_response(json).unwrap();
    assert_eq!(topology.layer_numbers, vec![4, 5, 6]);
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_topology_response_handles_empty_array() {
    let topology = super::parse_stack_topology_response("[]").unwrap();
    assert!(topology.layer_numbers.is_empty());
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_topology_response_rejects_malformed_json() {
    assert!(super::parse_stack_topology_response("not json").is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_topology_response_rejects_missing_base_ref() {
    let json = r#"[{"number":7,"pull_requests":[{"number":4}]}]"#;
    assert!(super::parse_stack_topology_response(json).is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn build_stack_enrichment_query_aliases_every_pull_request_in_one_request() {
    let query = super::build_stack_enrichment_query("warpdotdev", "warp-cli-survey", &[4, 5, 6]);
    assert!(query.contains("pr0: pullRequest(number: 4)"));
    assert!(query.contains("pr1: pullRequest(number: 5)"));
    assert!(query.contains("pr2: pullRequest(number: 6)"));
    assert!(query.contains("warpdotdev"));
    assert!(query.contains("warp-cli-survey"));
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_enrichment_response_parses_multiple_pull_requests_in_one_round_trip() {
    let json = r#"{
      "data": {
        "repository": {
          "pr0": {
            "title": "Add greeting package for the survey CLI",
            "url": "https://github.com/warpdotdev/warp-cli-survey/pull/4",
            "state": "OPEN",
            "isDraft": false,
            "mergedAt": null,
            "baseRefName": "master",
            "baseRefOid": "f371810d1b3cf50ff186c536ccec5b3a6322ed2b",
            "headRefName": "demo/stack-1-greeting-pkg",
            "headRefOid": "5b3d0244e39d89194ebae14d414ce2366d66a8b3"
          },
          "pr1": {
            "title": "Print greeting message when the survey starts",
            "url": "https://github.com/warpdotdev/warp-cli-survey/pull/5",
            "state": "OPEN",
            "isDraft": false,
            "mergedAt": null,
            "baseRefName": "demo/stack-1-greeting-pkg",
            "baseRefOid": "5b3d0244e39d89194ebae14d414ce2366d66a8b3",
            "headRefName": "demo/stack-2-cli-integration",
            "headRefOid": "668a0b582314f832c6407aaf7f90be171acf89f7"
          }
        }
      }
    }"#;

    let layers = super::parse_stack_enrichment_response(json, &[4, 5]).unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[&4].head_ref, "demo/stack-1-greeting-pkg".to_string());
    assert_eq!(layers[&5].base_ref, "demo/stack-1-greeting-pkg".to_string());
    assert_eq!(
        layers[&5].base_oid,
        "5b3d0244e39d89194ebae14d414ce2366d66a8b3".to_string()
    );
    assert_eq!(layers[&4].pr.state, "OPEN".to_string());
    assert_eq!(layers[&4].merged_at, None);
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_enrichment_response_rejects_missing_pull_request() {
    let json = r#"{"data":{"repository":{"pr0":{
        "title": "only one", "url": "u", "state": "OPEN", "isDraft": false, "mergedAt": null,
        "baseRefName": "master", "baseRefOid": "aaa", "headRefName": "h", "headRefOid": "bbb"
    }}}}"#;

    // Expects enrichment for both #4 and #5, but only #4 ("pr0") is present.
    assert!(super::parse_stack_enrichment_response(json, &[4, 5]).is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn parse_stack_enrichment_response_rejects_missing_object_id() {
    let json = r#"{"data":{"repository":{"pr0":{
        "title": "missing oid", "url": "u", "state": "OPEN", "isDraft": false, "mergedAt": null,
        "baseRefName": "master", "headRefName": "h", "headRefOid": "bbb"
    }}}}"#;

    assert!(super::parse_stack_enrichment_response(json, &[4]).is_err());
}

#[cfg(feature = "local_fs")]
#[test]
fn classify_stack_discovery_error_maps_known_failures_to_unavailable() {
    let cases = [
        "gh command failed: HTTP 404: Not Found (https://api.github.com/repos/o/r/stacks)",
        "Failed to execute gh command: No such file or directory (os error 2)",
        "gh command failed: GraphQL: authentication required; run gh auth login",
        "gh command failed: request timed out",
    ];
    for msg in cases {
        match super::classify_stack_discovery_error(msg) {
            StackDiscoveryResult::Unavailable { reason } => assert!(!reason.is_empty()),
            other => panic!("expected Unavailable for {msg:?}, got {other:?}"),
        }
    }
}

#[cfg(all(feature = "local_fs", unix))]
fn write_fake_gh(dir: &TempDir, script_body: &str) -> String {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let gh_path = dir.path().join("gh");
    fs::write(&gh_path, format!("#!/bin/sh\n{script_body}\n")).expect("failed to write fake gh");
    let mut permissions = fs::metadata(&gh_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&gh_path, permissions).unwrap();

    format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[cfg(all(feature = "local_fs", unix))]
#[tokio::test]
async fn get_pr_stack_normalizes_empty_result_to_not_stacked() {
    let (_dir, repo) = init_repo().await;
    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let path_env = write_fake_gh(&fake_bin, "printf '[]\\n'");

    let result =
        super::get_pr_stack(&repo, Some(&path_env), 4, "warpdotdev", "warp-cli-survey").await;
    assert_eq!(result, StackDiscoveryResult::NotStacked);
}

#[cfg(all(feature = "local_fs", unix))]
#[tokio::test]
async fn get_pr_stack_normalizes_single_layer_result_to_not_stacked() {
    let (_dir, repo) = init_repo().await;
    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let path_env = write_fake_gh(
        &fake_bin,
        r#"printf '[{"number":7,"base":{"ref":"master"},"pull_requests":[{"number":4,"state":"open","draft":false,"merged_at":null,"head":{"ref":"demo","sha":"aaa"}}]}]\n'"#,
    );

    let result =
        super::get_pr_stack(&repo, Some(&path_env), 4, "warpdotdev", "warp-cli-survey").await;
    assert_eq!(result, StackDiscoveryResult::NotStacked);
}

#[cfg(all(feature = "local_fs", unix))]
#[tokio::test]
async fn get_pr_stack_returns_unavailable_on_404() {
    let (_dir, repo) = init_repo().await;
    let fake_bin = tempfile::tempdir().expect("failed to create fake bin dir");
    let path_env = write_fake_gh(&fake_bin, "printf 'HTTP 404: Not Found\\n' >&2\nexit 1");

    let result =
        super::get_pr_stack(&repo, Some(&path_env), 4, "warpdotdev", "warp-cli-survey").await;
    assert!(matches!(result, StackDiscoveryResult::Unavailable { .. }));
}
