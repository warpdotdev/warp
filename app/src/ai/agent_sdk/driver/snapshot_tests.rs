use std::fs;
#[cfg(all(unix, not(target_os = "macos")))]
use std::os::unix::ffi::OsStringExt as _;
use std::sync::Arc;

use async_trait::async_trait;
use command::blocking::Command as BlockingCommand;
use mockito::{Matcher, Server, ServerGuard};
use tempfile::{Builder as TempDirBuilder, TempDir};
use tokio::runtime::Runtime;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::test_support::build_test_http_client;
use crate::ai::artifacts::Artifact;
use crate::server::server_api::harness_support::{
    ReportArtifactResponse, ResolvePromptRequest, ResolvedHarnessPrompt,
};

// ------------------------------------------------------------------------------------------------
// End-to-end snapshot upload tests.
//
// These drive `upload_snapshot_from_declarations_file` against real on-disk inputs (git repos
// created via git itself, files read from the filesystem) and a real HTTP client pointed at a
// `mockito::Server`. The only stub is `TestClient`, a minimal `HarnessSupportClient` impl that
// returns presigned URLs pointing at the mock server. Everything downstream — JSON serialization,
// reqwest round-trip, retry classification, manifest folding — runs as in production.
// ------------------------------------------------------------------------------------------------

/// Minimal `HarnessSupportClient` that returns upload targets pointing at a mockito server and
/// exposes a real `http_client::Client`. All other trait methods panic because they are not
/// exercised by the pipeline under test.
///
/// `fail_get_targets` and `drop_trailing_targets` provide the two failure modes we need to
/// exercise at the trait boundary without standing up a whole backend.
struct TestClient {
    server_base_url: String,
    http: http_client::Client,
    fail_get_targets: bool,
    /// Number of trailing response entries to drop, simulating a server that returns fewer
    /// targets than the request contained (contract violation). Under the positional
    /// alignment contract, the trailing files in the request end up with no target and are
    /// marked `skipped` downstream.
    drop_trailing_targets: usize,
}

impl TestClient {
    fn new(server_base_url: String) -> Arc<Self> {
        Arc::new(Self {
            server_base_url,
            http: build_test_http_client(),
            fail_get_targets: false,
            drop_trailing_targets: 0,
        })
    }

    fn new_failing_get_targets(server_base_url: String) -> Arc<Self> {
        Arc::new(Self {
            server_base_url,
            http: build_test_http_client(),
            fail_get_targets: true,
            drop_trailing_targets: 0,
        })
    }

    fn new_dropping_trailing(server_base_url: String, drop_trailing: usize) -> Arc<Self> {
        Arc::new(Self {
            server_base_url,
            http: build_test_http_client(),
            fail_get_targets: false,
            drop_trailing_targets: drop_trailing,
        })
    }
}

#[async_trait]
impl HarnessSupportClient for TestClient {
    async fn create_external_conversation(&self, _format: &str) -> Result<AIConversationId> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn get_transcript_upload_target(
        &self,
        _conversation_id: &AIConversationId,
    ) -> Result<UploadTarget> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn fetch_transcript(&self) -> Result<bytes::Bytes> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn get_block_snapshot_upload_target(
        &self,
        _conversation_id: &AIConversationId,
    ) -> Result<UploadTarget> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn resolve_prompt(
        &self,
        _request: ResolvePromptRequest,
    ) -> Result<ResolvedHarnessPrompt> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn report_artifact(&self, _artifact: &Artifact) -> Result<ReportArtifactResponse> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn notify_user(&self, _message: &str) -> Result<()> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn finish_task(&self, _success: bool, _summary: &str) -> Result<()> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn report_clean_shutdown(&self) -> Result<()> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn report_error_shutdown(
        &self,
        _error_category: String,
        _error_message: String,
    ) -> Result<()> {
        unimplemented!("not used by upload_snapshot_from_declarations_file")
    }

    async fn get_snapshot_upload_targets(
        &self,
        request: &SnapshotUploadRequest,
    ) -> Result<Vec<UploadTarget>> {
        if self.fail_get_targets {
            anyhow::bail!("simulated get_snapshot_upload_targets failure");
        }
        // The production server returns targets aligned by index with `request.files`, with
        // no filename echoed per target. Build per-file targets positionally, then optionally
        // truncate the tail to simulate a contract-violating short response. The client zips
        // request↔response by index; trailing files that lose their slot land in `upload_entry`
        // with no target and are marked `skipped`.
        let mut targets: Vec<UploadTarget> = request
            .files
            .iter()
            .map(|f| UploadTarget {
                url: format!("{}/upload/{}", self.server_base_url, f.filename),
                method: "PUT".to_string(),
                headers: HashMap::new(),
                fields: Vec::new(),
            })
            .collect();
        let keep = targets.len().saturating_sub(self.drop_trailing_targets);
        targets.truncate(keep);
        Ok(targets)
    }

    fn http_client(&self) -> &http_client::Client {
        &self.http
    }
}

/// Initialize a fresh git repo in `dir` with one committed file. Leaves the working tree dirty
/// (uncommitted edit) when `dirty` is true so `git diff --binary HEAD` has something to emit.
fn init_git_repo(dir: &Path, dirty: bool) {
    let run = |args: &[&str]| {
        let output = BlockingCommand::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(dir.join("README.md"), "initial\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
    if dirty {
        fs::write(dir.join("README.md"), "modified\n").unwrap();
    }
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = BlockingCommand::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn repo_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .unwrap()
        .to_string()
}

fn snaptest_tempdir() -> TempDir {
    TempDirBuilder::new().prefix("snaptest").tempdir().unwrap()
}

/// Path pattern matcher for `/upload/<any filename>`.
fn upload_path(filename_pattern: &str) -> Matcher {
    Matcher::Regex(format!("^/upload/{filename_pattern}$"))
}

/// Build a declarations file under `dir` listing the given repos and files. Returns the path.
fn write_declarations(dir: &Path, repos: &[&Path], files: &[&Path]) -> std::path::PathBuf {
    let mut contents = String::new();
    for repo in repos {
        contents.push_str(
            &serde_json::json!({
                "version": DECLARATION_VERSION,
                "kind": "repo",
                "path": repo.to_string_lossy().to_string(),
            })
            .to_string(),
        );
        contents.push('\n');
    }
    for file in files {
        contents.push_str(
            &serde_json::json!({
                "version": DECLARATION_VERSION,
                "kind": "file",
                "path": file.to_string_lossy().to_string(),
            })
            .to_string(),
        );
        contents.push('\n');
    }
    let path = dir.join("snapshot-declarations.jsonl");
    fs::write(&path, contents).unwrap();
    path
}

/// Run the pipeline against a declarations file synthesized from `repos` and `files`, and return
/// the outcome + summary.
fn run(
    server: &ServerGuard,
    decl_dir: &Path,
    repos: &[&Path],
    files: &[&Path],
) -> (SnapshotOutcome, SnapshotSummary) {
    let declarations_path = write_declarations(decl_dir, repos, files);
    let client = TestClient::new(server.url());
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(
            &declarations_path,
            client,
        ))
        .expect("pipeline returned None");
    let summary = SnapshotSummary::from_entries(&outcome.entries, outcome.manifest_uploaded);
    (outcome, summary)
}

fn find_entry(outcome: &SnapshotOutcome, status: EntryStatus) -> &EntryResult {
    outcome
        .entries
        .iter()
        .find(|e| e.status == status)
        .unwrap_or_else(|| panic!("no entry with status {:?} found", status.as_str()))
}

// ------------------------------------------------------------------------------------------------
// Declarations file parsing / resolution.
// ------------------------------------------------------------------------------------------------

#[test]
fn parse_declarations_ignores_blank_lines() {
    let contents = concat!(
        "\n",
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"/abs/repo\"}\n",
        "\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"/abs/file.txt\"}\n",
        "\n",
    );
    let entries = parse_declarations(contents);
    assert_eq!(
        entries,
        vec![
            DeclarationEntry {
                kind: EntryKind::Repo,
                path: "/abs/repo".to_string(),
            },
            DeclarationEntry {
                kind: EntryKind::File,
                path: "/abs/file.txt".to_string(),
            },
        ]
    );
}

#[test]
fn parse_declarations_skips_malformed_lines_without_aborting() {
    let contents = concat!(
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"/abs/good\"}\n",
        "{\"version\":1,\"kind\":\"unknown\",\"path\":\"/abs/huh\"}\n",
        "not-json\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"relative/path.txt\"}\n",
        "{\"version\":1,\"kind\":\"file\"}\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"/abs/also-good\",\"extra\":true}\n",
    );
    let entries = parse_declarations(contents);
    assert_eq!(
        entries,
        vec![
            DeclarationEntry {
                kind: EntryKind::Repo,
                path: "/abs/good".to_string(),
            },
            DeclarationEntry {
                kind: EntryKind::File,
                path: "/abs/also-good".to_string(),
            },
        ]
    );
}

#[test]
fn parse_declarations_skips_missing_or_unsupported_versions() {
    let contents = concat!(
        "{\"kind\":\"repo\",\"path\":\"/abs/missing-version\"}\n",
        "{\"version\":2,\"kind\":\"repo\",\"path\":\"/abs/unsupported-version\"}\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"/abs/good\"}\n",
    );
    let entries = parse_declarations(contents);
    assert_eq!(
        entries,
        vec![DeclarationEntry {
            kind: EntryKind::File,
            path: "/abs/good".to_string(),
        }]
    );
}

#[test]
fn parse_declarations_tolerates_crlf_line_endings() {
    // Generator scripts on Windows or files edited with a CRLF editor should still parse.
    let contents = concat!(
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"/abs/good\"}\r\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"/abs/also-good\"}\r\n",
    );
    let entries = parse_declarations(contents);
    assert_eq!(
        entries,
        vec![
            DeclarationEntry {
                kind: EntryKind::Repo,
                path: "/abs/good".to_string(),
            },
            DeclarationEntry {
                kind: EntryKind::File,
                path: "/abs/also-good".to_string(),
            },
        ]
    );
}

#[test]
fn parse_declarations_skips_lines_with_empty_path() {
    let contents = concat!(
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"\"}\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"   \"}\n",
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"/abs/still-good\"}\n",
    );
    let entries = parse_declarations(contents);
    assert_eq!(
        entries,
        vec![DeclarationEntry {
            kind: EntryKind::Repo,
            path: "/abs/still-good".to_string(),
        }]
    );
}

#[test]
fn parse_declarations_deduplicates_kind_path_pairs() {
    let contents = concat!(
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"/abs/repo\"}\n",
        "{\"version\":1,\"kind\":\"repo\",\"path\":\"/abs/repo\"}\n",
        "{\"version\":1,\"kind\":\"file\",\"path\":\"/abs/repo\"}\n",
    );
    let entries = parse_declarations(contents);
    assert_eq!(
        entries,
        vec![
            DeclarationEntry {
                kind: EntryKind::Repo,
                path: "/abs/repo".to_string(),
            },
            DeclarationEntry {
                kind: EntryKind::File,
                path: "/abs/repo".to_string(),
            },
        ]
    );
}

#[test]
fn upload_skipped_when_declarations_file_missing() {
    let tempdir = snaptest_tempdir();
    let missing = tempdir.path().join("does-not-exist.txt");
    let server = Server::new();
    let client = TestClient::new(server.url());
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(&missing, client));
    assert!(
        outcome.is_none(),
        "missing declarations file should skip the upload"
    );
}

#[test]
fn upload_skipped_when_declarations_file_empty() {
    let tempdir = snaptest_tempdir();
    let decl = tempdir.path().join("empty.txt");
    fs::write(&decl, "").unwrap();
    let server = Server::new();
    let client = TestClient::new(server.url());
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(&decl, client));
    assert!(outcome.is_none(), "empty declarations file should skip");
}

#[test]
fn upload_skipped_when_declarations_file_has_no_valid_jsonl_entries() {
    let tempdir = snaptest_tempdir();
    let decl = tempdir.path().join("invalid.jsonl");
    fs::write(
        &decl,
        "not-json\n{\"kind\":\"repo\",\"path\":\"relative\"}\n",
    )
    .unwrap();
    let server = Server::new();
    let client = TestClient::new(server.url());
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(&decl, client));
    assert!(
        outcome.is_none(),
        "declarations file with no valid entries should skip"
    );
}

#[test]
fn resolve_declarations_path_respects_override() {
    let override_path = std::ffi::OsString::from("/tmp/test-oz-declarations-override.txt");
    // Even with a task_id, the override wins.
    let resolved =
        resolve_declarations_path_with_override(Some(&fake_task_id()), Some(override_path.clone()));
    assert_eq!(resolved, Path::new(&override_path));
}

#[test]
fn resolve_declarations_path_defaults_without_override_or_task_id() {
    let resolved = resolve_declarations_path_with_override(None, None);
    assert_eq!(
        resolved,
        Path::new(DEFAULT_DECLARATIONS_DIR).join(DEFAULT_DECLARATIONS_FILENAME)
    );
}

#[test]
fn resolve_declarations_path_uses_task_id_when_provided() {
    let task_id = fake_task_id();
    let resolved = resolve_declarations_path_with_override(Some(&task_id), None);
    assert_eq!(
        resolved,
        Path::new(DEFAULT_DECLARATIONS_DIR)
            .join(task_id.to_string())
            .join(DEFAULT_DECLARATIONS_FILENAME)
    );
}

/// Builder-side helper used by `resolve_declarations_path_*` tests. Any valid task ID works;
/// the string form is what ends up as the per-run directory name.
fn fake_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

// ------------------------------------------------------------------------------------------------
// End-to-end upload pipeline.
// ------------------------------------------------------------------------------------------------

#[test]
fn e2e_dirty_repo_uploads_patch_and_manifest_reports_success() {
    let tempdir = snaptest_tempdir();
    init_git_repo(tempdir.path(), true);
    let branch = git_stdout(
        tempdir.path(),
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    );
    let head_sha = git_stdout(tempdir.path(), &["rev-parse", "HEAD"]);
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let patch_mock = server
        .mock("PUT", upload_path(r".+\.patch"))
        .with_status(200)
        .expect(1)
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "version": 1,
            "repos": [{
                "path": tempdir.path().to_string_lossy(),
                "repo_name": repo_name(tempdir.path()),
                "branch": branch,
                "head_sha": head_sha,
                "status": "uploaded",
                "uploaded": true,
            }],
            "files": [],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[tempdir.path()], &[]);

    assert!(summary.all_uploaded(), "expected all entries uploaded");
    assert_eq!(summary.uploaded, 2); // patch + manifest
    assert_eq!(summary.total, 2);
    assert!(outcome.manifest_uploaded);
    patch_mock.assert();
    manifest_mock.assert();
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn build_repo_patch_preserves_non_utf8_untracked_paths() {
    let tempdir = snaptest_tempdir();
    init_git_repo(tempdir.path(), false);
    let filename = std::ffi::OsString::from_vec(b"non-utf8-\xFF.txt".to_vec());
    let file_path = tempdir.path().join(filename);
    fs::write(&file_path, b"content from non-utf8 path\n").unwrap();

    let patch = Runtime::new()
        .unwrap()
        .block_on(build_repo_patch(tempdir.path()))
        .unwrap();
    let patch = String::from_utf8_lossy(&patch);

    assert!(
        patch.contains("content from non-utf8 path"),
        "patch should include non-UTF-8 untracked file contents: {patch}"
    );
}

#[test]
fn e2e_clean_repo_uploads_only_manifest() {
    let tempdir = snaptest_tempdir();
    init_git_repo(tempdir.path(), false);
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    // No patch mock — if the pipeline tries to upload a patch for a clean repo, the test panics
    // on an unexpected request.
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "repos": [{
                "path": tempdir.path().to_string_lossy(),
                "status": "clean",
                "uploaded": null,
            }],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (_outcome, summary) = run(&server, decl_dir.path(), &[tempdir.path()], &[]);

    assert!(summary.all_uploaded());
    assert_eq!(summary.total, 1);
    assert_eq!(summary.uploaded, 1);
    manifest_mock.assert();
}

#[test]
fn e2e_gather_failed_entry_captured_in_manifest() {
    // A tempdir that is NOT a git repo triggers build_repo_patch failure → gather_failed.
    let tempdir = snaptest_tempdir();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "repos": [{
                "path": tempdir.path().to_string_lossy(),
                "status": "gather_failed",
                "uploaded": null,
            }],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[tempdir.path()], &[]);

    assert!(!summary.all_uploaded(), "expected failures to surface");
    assert_eq!(summary.gather_failed, 1);
    assert_eq!(summary.uploaded, 1); // only the manifest
    assert!(outcome.manifest_uploaded);
    let entry = find_entry(&outcome, EntryStatus::GatherFailed);
    assert!(
        entry.error.is_some(),
        "gather_failed entry should carry an error string"
    );
    manifest_mock.assert();
}

#[test]
fn e2e_read_failed_for_missing_file_continues_pipeline() {
    // Point a `file` entry at a path that doesn't exist → read_failed, with a clean repo also
    // included so we verify the pipeline didn't abort after the read failure. Keep the missing
    // file outside the repo so the repo-overlap filter does not strip it before gather.
    let tempdir = snaptest_tempdir();
    init_git_repo(tempdir.path(), false);
    let missing_dir = snaptest_tempdir();
    let missing_file = missing_dir.path().join("does-not-exist.txt");
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "files": [{
                "path": missing_file.to_string_lossy(),
                "status": "read_failed",
                "uploaded": null,
            }],
            "repos": [{
                "status": "clean",
            }],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(
        &server,
        decl_dir.path(),
        &[tempdir.path()],
        &[&missing_file],
    );

    assert!(!summary.all_uploaded());
    assert_eq!(summary.read_failed, 1);
    assert_eq!(summary.gather_failed, 0);
    assert!(outcome.manifest_uploaded);
    let entry = find_entry(&outcome, EntryStatus::ReadFailed);
    assert!(
        entry
            .error
            .as_deref()
            .is_some_and(|e| e.contains("does-not-exist.txt")),
        "unexpected error: {:?}",
        entry.error
    );
    manifest_mock.assert();
}

#[test]
fn e2e_file_is_uploaded_with_correct_body() {
    // A real file on disk gets read, uploaded, and its bytes arrive at the mock server.
    let tempdir = snaptest_tempdir();
    let file_path = tempdir.path().join("note.txt");
    fs::write(&file_path, b"hello from the snapshot pipeline").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let file_mock = server
        .mock("PUT", upload_path("note\\.txt"))
        .match_body(Matcher::Exact(
            "hello from the snapshot pipeline".to_string(),
        ))
        .with_status(200)
        .expect(1)
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "files": [{
                "path": file_path.to_string_lossy(),
                "snapshot_file": "note.txt",
                "status": "uploaded",
                "uploaded": true,
            }],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (_outcome, summary) = run(&server, decl_dir.path(), &[], &[&file_path]);

    assert!(summary.all_uploaded());
    assert_eq!(summary.uploaded, 2); // file + manifest
    file_mock.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_transient_5xx_is_retried_then_succeeds() {
    // The mock responds with 503 on the first attempt and 200 on the second. The retry helper
    // must exercise both, emit a warning, and the file must land as "uploaded" in the manifest.
    let tempdir = snaptest_tempdir();
    let file_path = tempdir.path().join("flaky.txt");
    fs::write(&file_path, b"transient-retry").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    // First call: 503. Mockito matches mocks in order, so declare the failing mock before the
    // success mock. Each is `.expect(1)` so together they cover attempts #1 and #2.
    let flaky_fail = server
        .mock("PUT", upload_path("flaky\\.txt"))
        .with_status(503)
        .with_body("temporarily unavailable")
        .expect(1)
        .create();
    let flaky_ok = server
        .mock("PUT", upload_path("flaky\\.txt"))
        .with_status(200)
        .expect(1)
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[], &[&file_path]);

    assert!(
        summary.all_uploaded(),
        "retry should have eventually succeeded"
    );
    let file_entry = outcome
        .entries
        .iter()
        .find(|e| e.label == "flaky.txt")
        .expect("flaky.txt entry missing");
    assert_eq!(file_entry.status, EntryStatus::Uploaded);
    flaky_fail.assert();
    flaky_ok.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_permanent_4xx_fails_fast_without_retries() {
    // 403 is classified as permanent — the retry loop must NOT retry.
    let tempdir = snaptest_tempdir();
    let file_path = tempdir.path().join("denied.txt");
    fs::write(&file_path, b"will-not-upload").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let denied = server
        .mock("PUT", upload_path("denied\\.txt"))
        .with_status(403)
        .with_body("forbidden")
        .expect(1) // exactly one attempt
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "files": [{
                "path": file_path.to_string_lossy(),
                "snapshot_file": "denied.txt",
                "status": "failed",
                "uploaded": false,
            }],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[], &[&file_path]);

    assert!(!summary.all_uploaded());
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.uploaded, 1); // just the manifest
    assert!(outcome.manifest_uploaded);
    let file_entry = outcome
        .entries
        .iter()
        .find(|e| e.label == "denied.txt")
        .expect("denied.txt entry missing");
    assert_eq!(file_entry.status, EntryStatus::Failed);
    assert!(
        file_entry
            .error
            .as_deref()
            .is_some_and(|e| e.contains("403")),
        "expected 403 mention in error, got {:?}",
        file_entry.error
    );
    denied.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_manifest_reflects_mixed_outcomes() {
    // One file succeeds, one file fails permanently. The manifest uploaded to GCS must list both
    // outcomes accurately (status + uploaded + error per entry).
    let tempdir = snaptest_tempdir();
    let ok_file = tempdir.path().join("ok.txt");
    let bad_file = tempdir.path().join("bad.txt");
    fs::write(&ok_file, b"good").unwrap();
    fs::write(&bad_file, b"bad").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let ok_mock = server
        .mock("PUT", upload_path("ok\\.txt"))
        .with_status(200)
        .expect(1)
        .create();
    let bad_mock = server
        .mock("PUT", upload_path("bad\\.txt"))
        .with_status(404)
        .expect(1)
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::AllOf(vec![Matcher::PartialJson(
            serde_json::json!({
                "files": [
                    {"snapshot_file": "ok.txt", "status": "uploaded", "uploaded": true},
                    {"snapshot_file": "bad.txt", "status": "failed", "uploaded": false},
                ],
            }),
        )]))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[], &[&ok_file, &bad_file]);

    assert!(
        !summary.all_uploaded(),
        "one file failed, summary should reflect partial"
    );
    assert_eq!(summary.uploaded, 2); // ok.txt + manifest
    assert_eq!(summary.failed, 1); // bad.txt
    assert!(outcome.manifest_uploaded);
    ok_mock.assert();
    bad_mock.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_retry_exhaustion_marks_entry_failed_and_records_in_manifest() {
    // All three attempts return 5xx. The retry loop must bail out after MAX_ATTEMPTS, the entry
    // becomes `failed`, and the manifest still uploads successfully with `uploaded: false` for
    // the dead file.
    let tempdir = snaptest_tempdir();
    let file_path = tempdir.path().join("dead.txt");
    fs::write(&file_path, b"never-succeeds").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let dead_mock = server
        .mock("PUT", upload_path("dead\\.txt"))
        .with_status(503)
        .with_body("still unavailable")
        .expect(3) // exactly MAX_ATTEMPTS
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "files": [{
                "snapshot_file": "dead.txt",
                "status": "failed",
                "uploaded": false,
            }],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[], &[&file_path]);

    assert!(!summary.all_uploaded());
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.uploaded, 1); // only the manifest
    assert!(outcome.manifest_uploaded);
    let file_entry = outcome
        .entries
        .iter()
        .find(|e| e.label == "dead.txt")
        .expect("dead.txt entry missing");
    assert_eq!(file_entry.status, EntryStatus::Failed);
    assert!(
        file_entry
            .error
            .as_deref()
            .is_some_and(|e| e.contains("503")),
        "expected 503 in entry error, got {:?}",
        file_entry.error
    );
    dead_mock.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_manifest_upload_failure_produces_partial_outcome() {
    // All files upload, but the manifest's three PUT attempts all return 500. The pipeline
    // should finish with `manifest_uploaded=false`, a populated `manifest_error`, the manifest
    // entry appended as `failed`, and an overall partial summary.
    let tempdir = snaptest_tempdir();
    let file_path = tempdir.path().join("happy.txt");
    fs::write(&file_path, b"good").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let file_mock = server
        .mock("PUT", upload_path("happy\\.txt"))
        .with_status(200)
        .expect(1)
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .with_status(500)
        .with_body("manifest store is down")
        .expect(3) // three retries for persistent 5xx
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[], &[&file_path]);

    assert!(!summary.all_uploaded());
    assert!(!outcome.manifest_uploaded);
    assert_eq!(summary.uploaded, 1); // just the file
    assert_eq!(summary.failed, 1); // the manifest
    let manifest_entry = outcome.entries.last().expect("entries missing manifest");
    assert_eq!(manifest_entry.label, "snapshot_state.json");
    assert_eq!(manifest_entry.status, EntryStatus::Failed);
    assert!(
        manifest_entry
            .error
            .as_deref()
            .is_some_and(|e| e.contains("500")),
        "expected 500 in manifest entry error, got {:?}",
        manifest_entry.error
    );
    file_mock.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_get_snapshot_upload_targets_failure_returns_none() {
    // The server-side call to allocate presigned URLs errors. The pipeline logs a warning and
    // returns None — upload is abandoned but the driver's cleanup continues.
    let tempdir = snaptest_tempdir();
    init_git_repo(tempdir.path(), true);
    let decl_dir = snaptest_tempdir();
    let declarations_path = write_declarations(decl_dir.path(), &[tempdir.path()], &[]);
    let server = Server::new();
    let client = TestClient::new_failing_get_targets(server.url());
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(
            &declarations_path,
            client,
        ));
    assert!(
        outcome.is_none(),
        "get_snapshot_upload_targets failure should produce None"
    );
}

#[test]
fn e2e_short_response_leaves_trailing_file_without_target() {
    // Simulate a contract-violating server that returns one fewer target than requested.
    // With positional alignment, the trailing entry in the request — the manifest, since
    // `upload_gathered_snapshot` always appends `snapshot_state.json` last — loses its slot
    // in `target_map` and the manifest upload is recorded as failed with a
    // "no upload target" error. The preceding declared files still upload cleanly because
    // their positional targets are intact.
    let tempdir = snaptest_tempdir();
    let first_file = tempdir.path().join("first.txt");
    let second_file = tempdir.path().join("second.txt");
    fs::write(&first_file, b"one").unwrap();
    fs::write(&second_file, b"two").unwrap();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let first_mock = server
        .mock("PUT", upload_path("first\\.txt"))
        .with_status(200)
        .expect(1)
        .create();
    let second_mock = server
        .mock("PUT", upload_path("second\\.txt"))
        .with_status(200)
        .expect(1)
        .create();
    // No manifest mock — the pipeline must never attempt to upload the manifest because the
    // truncated response gave us no target for it.

    let declarations_path = write_declarations(decl_dir.path(), &[], &[&first_file, &second_file]);
    let client = TestClient::new_dropping_trailing(server.url(), 1);
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(
            &declarations_path,
            client,
        ))
        .expect("pipeline returned None");
    let summary = SnapshotSummary::from_entries(&outcome.entries, outcome.manifest_uploaded);

    assert!(!summary.all_uploaded());
    assert!(!outcome.manifest_uploaded);
    assert_eq!(summary.uploaded, 2); // first.txt + second.txt
    assert_eq!(summary.failed, 1); // manifest
    let manifest_entry = outcome.entries.last().expect("entries missing manifest");
    assert_eq!(manifest_entry.label, "snapshot_state.json");
    assert_eq!(manifest_entry.status, EntryStatus::Failed);
    assert!(
        manifest_entry
            .error
            .as_deref()
            .is_some_and(|e| e.contains("no upload target")),
        "expected 'no upload target' in manifest error, got {:?}",
        manifest_entry.error
    );
    first_mock.assert();
    second_mock.assert();
}

#[test]
fn e2e_multi_repo_mixed_statuses_roundtrip_to_manifest() {
    // One dirty repo, one clean repo, one non-git dir. Exercise:
    // - the multi-repo enumeration path
    // - patch filename uniqueness (only dirty produces a patch)
    // - the manifest containing all three entries with correct statuses.
    let dirty = snaptest_tempdir();
    init_git_repo(dirty.path(), true);
    let clean = snaptest_tempdir();
    init_git_repo(clean.path(), false);
    let broken = snaptest_tempdir();
    let decl_dir = snaptest_tempdir();

    let mut server = Server::new();
    let patch_mock = server
        .mock("PUT", upload_path(r".+\.patch"))
        .with_status(200)
        .expect(1) // only the dirty repo produces a patch
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "repos": [
                {
                    "path": dirty.path().to_string_lossy(),
                    "status": "uploaded",
                    "uploaded": true,
                },
                {
                    "path": clean.path().to_string_lossy(),
                    "status": "clean",
                    "uploaded": null,
                },
                {
                    "path": broken.path().to_string_lossy(),
                    "status": "gather_failed",
                    "uploaded": null,
                },
            ],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let (outcome, summary) = run(
        &server,
        decl_dir.path(),
        &[dirty.path(), clean.path(), broken.path()],
        &[],
    );

    assert!(
        !summary.all_uploaded(),
        "one repo was gather_failed, summary should reflect partial"
    );
    assert_eq!(summary.uploaded, 2); // dirty's patch + manifest
    assert_eq!(summary.gather_failed, 1);
    assert_eq!(summary.failed, 0);
    assert!(outcome.manifest_uploaded);
    patch_mock.assert();
    manifest_mock.assert();
}

#[test]
fn e2e_per_run_cap_drops_excess_blobs_as_skipped() {
    // Declaring more file entries than MAX_SNAPSHOT_FILES_PER_RUN should cause the excess to
    // be dropped from upload and marked `skipped` in the manifest with a cap-reason error,
    // while the kept blobs upload cleanly across chunked `upload-snapshot` calls.
    let tempdir = snaptest_tempdir();
    let decl_dir = snaptest_tempdir();

    // The cap reserves one slot for snapshot_state.json, so blobs share
    // (MAX_SNAPSHOT_FILES_PER_RUN - 1) slots. Declaring extra entries on top forces the cap
    // to trim them.
    let extra_declarations: usize = 2;
    let declared_count = MAX_SNAPSHOT_FILES_PER_RUN - 1 + extra_declarations;
    let expected_uploaded_blobs = MAX_SNAPSHOT_FILES_PER_RUN - 1;
    let expected_uploaded_total = expected_uploaded_blobs + 1; // + manifest

    let file_paths: Vec<std::path::PathBuf> = (0..declared_count)
        .map(|i| {
            let path = tempdir.path().join(format!("file_{i:03}.txt"));
            fs::write(&path, format!("content-{i}").as_bytes()).unwrap();
            path
        })
        .collect();
    let file_refs: Vec<&Path> = file_paths.iter().map(|p| p.as_path()).collect();

    let mut server = Server::new();
    // One catch-all mock for any blob PUT; the pipeline should perform exactly
    // expected_uploaded_total PUTs (kept blobs + manifest) and never try to upload the
    // dropped entries.
    let upload_mock = server
        .mock("PUT", upload_path(r".+"))
        .with_status(200)
        .expect(expected_uploaded_total)
        .create();

    let (outcome, summary) = run(&server, decl_dir.path(), &[], &file_refs);

    assert!(
        !summary.all_uploaded(),
        "capped entries should surface as partial success"
    );
    assert_eq!(
        summary.uploaded, expected_uploaded_total,
        "uploaded should include all kept blobs plus the manifest"
    );
    assert_eq!(
        summary.skipped, extra_declarations,
        "cap should mark the excess declarations as skipped"
    );
    assert_eq!(
        summary.total,
        declared_count + 1,
        "total entries = declared + manifest"
    );
    assert!(outcome.manifest_uploaded);
    let skipped_entries: Vec<&EntryResult> = outcome
        .entries
        .iter()
        .filter(|e| e.status == EntryStatus::Skipped)
        .collect();
    assert_eq!(skipped_entries.len(), extra_declarations);
    for entry in &skipped_entries {
        assert!(
            entry
                .error
                .as_deref()
                .is_some_and(|m| m.contains("per-run snapshot cap")),
            "skipped entry should reference cap error, got {:?}",
            entry.error
        );
    }
    upload_mock.assert();
}

// ------------------------------------------------------------------------------------------------
// REMOTE-1465: repo-overlap dedup + DeclarationsWriterHandle.
// ------------------------------------------------------------------------------------------------

/// Build a `DeclarationEntry` without exposing the private type to call sites.
fn repo_entry(path: &str) -> DeclarationEntry {
    DeclarationEntry {
        kind: EntryKind::Repo,
        path: path.to_string(),
    }
}

fn file_entry(path: &str) -> DeclarationEntry {
    DeclarationEntry {
        kind: EntryKind::File,
        path: path.to_string(),
    }
}

#[test]
fn drop_files_covered_by_repos_keeps_everything_when_no_repos_declared() {
    let entries = vec![
        file_entry("/abs/outside.txt"),
        file_entry("/other/also-outside.txt"),
    ];
    let after = drop_files_covered_by_repos(entries.clone());
    assert_eq!(after, entries);
}

#[test]
fn drop_files_covered_by_repos_drops_file_inside_repo_keeps_file_outside() {
    let entries = vec![
        repo_entry("/workspace/my-repo"),
        file_entry("/workspace/my-repo/src/foo.rs"),
        file_entry("/tmp/outside.txt"),
    ];
    let after = drop_files_covered_by_repos(entries);
    assert_eq!(
        after,
        vec![
            repo_entry("/workspace/my-repo"),
            file_entry("/tmp/outside.txt"),
        ]
    );
}

#[test]
fn drop_files_covered_by_repos_handles_nested_repo_paths() {
    // A file under /a/b/sub should be filtered by either /a or /a/b/sub.
    let entries = vec![
        repo_entry("/a"),
        repo_entry("/a/b/sub"),
        file_entry("/a/b/sub/file.txt"),
        file_entry("/a/top.txt"),
        file_entry("/unrelated.txt"),
    ];
    let after = drop_files_covered_by_repos(entries);
    assert_eq!(
        after,
        vec![
            repo_entry("/a"),
            repo_entry("/a/b/sub"),
            file_entry("/unrelated.txt"),
        ]
    );
}

/// Parse the declarations file written by `DeclarationsWriterHandle` into the paths we care
/// about for assertions, ignoring any lines the helper tests weren't asked to produce.
fn parsed_file_paths(path: &Path) -> Vec<String> {
    let contents = fs::read_to_string(path).unwrap_or_default();
    let entries = parse_declarations(&contents);
    entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::File)
        .map(|e| e.path)
        .collect()
}

#[test]
fn declarations_writer_appends_unique_absolute_paths_once() {
    let tempdir = snaptest_tempdir();
    let workspace = tempdir.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let decl_path = tempdir.path().join("declarations.jsonl");
    let task_id = fake_task_id();

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let handle =
            DeclarationsWriterHandle::new_at_path(decl_path.clone(), workspace.clone(), task_id);
        let path_one = workspace.join("one.txt");
        let path_two = workspace.join("two.txt");
        handle.append(vec![
            path_one.to_string_lossy().into_owned(),
            path_two.to_string_lossy().into_owned(),
            // Duplicate: should still only produce one entry per unique path.
            path_one.to_string_lossy().into_owned(),
        ]);
        handle.flush().await;
        // A second batch that re-declares path_one should also be a no-op.
        handle.append(vec![path_one.to_string_lossy().into_owned()]);
        handle.flush().await;
    });

    let paths = parsed_file_paths(&decl_path);
    assert_eq!(
        paths,
        vec![
            workspace.join("one.txt").to_string_lossy().into_owned(),
            workspace.join("two.txt").to_string_lossy().into_owned(),
        ]
    );
}

#[test]
fn declarations_writer_resolves_relative_paths_against_working_dir() {
    let tempdir = snaptest_tempdir();
    let workspace = tempdir.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let decl_path = tempdir.path().join("declarations.jsonl");
    let task_id = fake_task_id();

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let handle =
            DeclarationsWriterHandle::new_at_path(decl_path.clone(), workspace.clone(), task_id);
        handle.append(vec!["notes/relative.txt".to_string()]);
        handle.flush().await;
    });

    let paths = parsed_file_paths(&decl_path);
    assert_eq!(
        paths,
        vec![workspace
            .join("notes/relative.txt")
            .to_string_lossy()
            .into_owned()]
    );
}

#[test]
fn declarations_writer_continues_after_per_path_write_failures() {
    // Pre-create a directory at the declarations file path so the first append's open call
    // fails. Once we remove the directory, a subsequent append must succeed, proving the
    // writer task absorbed the failure and kept servicing commands.
    let tempdir = snaptest_tempdir();
    let workspace = tempdir.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let decl_path = tempdir.path().join("declarations.jsonl");
    fs::create_dir(&decl_path).unwrap();
    let task_id = fake_task_id();

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let handle =
            DeclarationsWriterHandle::new_at_path(decl_path.clone(), workspace.clone(), task_id);
        handle.append(vec![workspace
            .join("first.txt")
            .to_string_lossy()
            .into_owned()]);
        handle.flush().await;
        // Replace the staged directory so the next append's open call can succeed.
        fs::remove_dir(&decl_path).unwrap();
        handle.append(vec![workspace
            .join("second.txt")
            .to_string_lossy()
            .into_owned()]);
        handle.flush().await;
    });

    let paths = parsed_file_paths(&decl_path);
    assert_eq!(
        paths,
        vec![workspace.join("second.txt").to_string_lossy().into_owned()],
        "writer task should absorb the first failure and process the second append"
    );
}

#[test]
fn declarations_writer_preempts_paths_inside_existing_repo() {
    let tempdir = snaptest_tempdir();
    // Simulate an existing repo by creating the `.git` directory the ancestor walker checks.
    let repo = tempdir.path().join("existing-repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let inside = repo.join("inside.txt");
    let outside = tempdir.path().join("outside.txt");
    let decl_path = tempdir.path().join("declarations.jsonl");
    let task_id = fake_task_id();

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let handle = DeclarationsWriterHandle::new_at_path(
            decl_path.clone(),
            tempdir.path().to_path_buf(),
            task_id,
        );
        handle.append(vec![
            inside.to_string_lossy().into_owned(),
            outside.to_string_lossy().into_owned(),
        ]);
        handle.flush().await;
    });

    let paths = parsed_file_paths(&decl_path);
    assert_eq!(paths, vec![outside.to_string_lossy().into_owned()]);
}

#[test]
fn e2e_repo_plus_inside_and_outside_files_filters_overlap() {
    // The writer-written declarations file feeds straight into the upload pipeline. Pair one
    // `repo` with two `file` entries (one inside the repo, one outside). The gather-time
    // overlap filter should drop the inside-repo file entry before upload so only the repo's
    // patch + the outside-repo file + the manifest land on the server.
    let repo_dir = snaptest_tempdir();
    init_git_repo(repo_dir.path(), true);
    let inside_file = repo_dir.path().join("new-untracked.txt");
    fs::write(&inside_file, b"tracked-or-not, handled by the patch\n").unwrap();

    let outside_dir = snaptest_tempdir();
    let outside_file = outside_dir.path().join("standalone_log.txt");
    fs::write(&outside_file, b"agent-produced log\n").unwrap();

    let decl_dir = snaptest_tempdir();
    let decl_path = decl_dir.path().join("snapshot-declarations.jsonl");
    let contents = format!(
        concat!(
            "{{\"version\":1,\"kind\":\"repo\",\"path\":{repo:?}}}\n",
            "{{\"version\":1,\"kind\":\"file\",\"path\":{inside:?}}}\n",
            "{{\"version\":1,\"kind\":\"file\",\"path\":{outside:?}}}\n",
        ),
        repo = repo_dir.path().to_string_lossy(),
        inside = inside_file.to_string_lossy(),
        outside = outside_file.to_string_lossy(),
    );
    fs::write(&decl_path, contents).unwrap();

    let mut server = Server::new();
    let patch_mock = server
        .mock("PUT", upload_path(r".+\.patch"))
        .with_status(200)
        .expect(1)
        .create();
    let file_mock = server
        .mock("PUT", upload_path("standalone_log\\.txt"))
        .with_status(200)
        .expect(1)
        .create();
    let manifest_mock = server
        .mock("PUT", upload_path("snapshot_state\\.json"))
        .match_body(Matcher::PartialJson(serde_json::json!({
            "files": [
                {
                    "path": outside_file.to_string_lossy(),
                    "status": "uploaded",
                    "uploaded": true,
                }
            ],
        })))
        .with_status(200)
        .expect(1)
        .create();

    let client = TestClient::new(server.url());
    let outcome = Runtime::new()
        .unwrap()
        .block_on(upload_snapshot_from_declarations_file(&decl_path, client))
        .expect("pipeline returned None");
    let summary = SnapshotSummary::from_entries(&outcome.entries, outcome.manifest_uploaded);

    assert!(summary.all_uploaded(), "expected all uploads to succeed");
    // repo patch + outside file + manifest = 3 uploaded entries total; the inside-repo file
    // entry was filtered before gather so it never hits the entries list.
    assert_eq!(summary.uploaded, 3);
    assert_eq!(summary.total, 3);
    assert!(outcome.manifest_uploaded);
    patch_mock.assert();
    file_mock.assert();
    manifest_mock.assert();
}
