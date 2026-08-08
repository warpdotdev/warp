use std::fs;
use std::path::{Path, PathBuf};

use cloud_object_models::CodeForge;
use command::blocking::Command;
use regex::Regex;
use serial_test::serial;
use tempfile::TempDir;
use warp_core::command::ExitCode;

use super::{
    MAX_SETUP_COMMAND_OUTPUT_BYTES, PrepareEnvironmentError, build_parallel_clone_command,
    checkout_command_for, checkout_result, merge_repos_deduped,
    redact_and_truncate_setup_command_output, single_repo_name, truncate_setup_command_output,
};
use crate::ai::cloud_environments::SourceRepo;
use crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes;
use crate::terminal::shell::ShellType;

#[test]
fn single_repo_name_returns_repo_when_exactly_one_repo() {
    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp-internal".to_string(),
    )];
    let selected_repo = single_repo_name(&repos);
    assert_eq!(selected_repo, Some("warp-internal".to_string()));
}

fn repo(forge: CodeForge, owner: &str, name: &str) -> SourceRepo {
    SourceRepo::new(forge, owner.to_string(), name.to_string())
}

#[test]
fn merge_repos_dedupes_case_insensitively_and_preserves_environment_order() {
    let environment = vec![repo(CodeForge::GitHub, "WarpDotDev", "Warp")];
    let additional = vec![
        repo(CodeForge::GitHub, "warpdotdev", "warp"),
        repo(CodeForge::GitHub, "warpdotdev", "warp-server"),
    ];

    assert_eq!(
        merge_repos_deduped(environment, additional).unwrap(),
        vec![
            repo(CodeForge::GitHub, "WarpDotDev", "Warp"),
            repo(CodeForge::GitHub, "warpdotdev", "warp-server"),
        ]
    );
}

#[test]
fn merge_repos_keeps_distinct_repositories() {
    let merged = merge_repos_deduped(
        vec![repo(CodeForge::GitHub, "a", "widget")],
        vec![
            repo(CodeForge::GitHub, "b", "widget-api"),
            repo(CodeForge::GitLab, "a", "widget-web"),
        ],
    )
    .unwrap();

    assert_eq!(merged.len(), 3);
}
#[test]
fn merge_repos_rejects_clone_directory_collisions() {
    let error = merge_repos_deduped(
        vec![repo(CodeForge::GitHub, "a", "widget")],
        vec![repo(CodeForge::GitLab, "b", "widget")],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        PrepareEnvironmentError::CloneDirectoryCollision {
            repo_name,
            first_owner,
            second_owner,
        } if repo_name == "widget" && first_owner == "a" && second_owner == "b"
    ));
}

#[test]
fn merge_repos_supports_additional_only_and_empty_inputs() {
    let additional = vec![repo(CodeForge::GitHub, "warpdotdev", "warp")];
    assert_eq!(
        merge_repos_deduped(Vec::new(), additional.clone()).unwrap(),
        additional
    );
    assert!(
        merge_repos_deduped(Vec::new(), Vec::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn single_repo_name_returns_none_for_zero_or_many_repos() {
    let no_repos = Vec::<SourceRepo>::new();
    assert_eq!(single_repo_name(&no_repos), None);

    let two_repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-internal".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        ),
    ];
    assert_eq!(single_repo_name(&two_repos), None);
}

#[test]
fn parallel_clone_command_runs_repos_in_background_and_waits() {
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitLab,
            "platform/backend".to_string(),
            "api".to_string(),
        ),
    ];

    let command = build_parallel_clone_command(&repos, ShellType::Bash);

    assert!(command.starts_with("sh -c '"));
    assert!(command.contains("warpdotdev/warp"));
    assert!(command.contains("https://github.com/warpdotdev/warp.git"));
    assert!(command.contains("platform/backend/api"));
    assert!(command.contains("https://gitlab.com/platform/backend/api.git"));
    assert_eq!(command.matches("clone_repo").count(), 3);
    assert_eq!(command.matches("2>&1 &").count(), 2);
    assert!(command.contains("mktemp -d"));
    assert!(command.contains("warp-clone-logs"));
    assert!(command.contains("trap cleanup_clone_logs EXIT"));
    assert!(command.contains("repo-0.log"));
    assert!(command.contains("repo-1.log"));
    assert!(command.contains(">\"$log_file_0\" 2>&1 &"));
    assert!(command.contains(">\"$log_file_1\" 2>&1 &"));
    assert!(command.contains("pids=\"$pids $!\""));
    assert!(command.contains("wait \"$pid\""));
    assert!(command.contains("===== warpdotdev/warp ====="));
    assert!(command.contains("cat \"$log_file_0\""));
    assert!(command.contains("===== platform/backend/api ====="));
    assert!(command.contains("cat \"$log_file_1\""));
    assert!(command.contains("exit \"$failed\""));
}

#[test]
fn parallel_clone_command_threads_checkout_ref_and_pins_after_clone() {
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        )
        .with_checkout_ref(Some("abc123".to_string())),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        ),
    ];

    let command = build_parallel_clone_command(&repos, ShellType::Bash);

    // The shell function fetches then checks out FETCH_HEAD only when a ref is set.
    assert!(command.contains("checkout_ref=\"$4\""));
    assert!(command.contains("if [ -n \"$checkout_ref\" ]; then"));
    assert!(command.contains("git -C \"$target\" fetch --filter=tree:0 origin \"$checkout_ref\""));
    assert!(command.contains("git -C \"$target\" checkout --detach FETCH_HEAD"));
    // Pinning must not be nested under the "directory missing" branch — reused
    // target directories still need fetch/checkout when a ref is set.
    assert!(command.contains("already exists, skipping clone..."));
    assert!(!command.contains("already exists, skipping clone...\n    return 0"));
    // A clone failure must short-circuit before any checkout attempt.
    assert!(command.contains("git clone --filter=tree:0 \"$repo_url\" \"$target\" || return 1"));
    // The pinned repo's ref is threaded into the command (as the 4th positional
    // arg to clone_repo). The whole script is single-quote-escaped for `sh -c`,
    // so assert on the ref content rather than the surrounding quoting.
    assert!(command.contains("abc123"));
}

#[test]
fn checkout_command_checks_out_fetch_head_not_ref_name() {
    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )
    .with_checkout_ref(Some("abc123".to_string()));

    let command =
        checkout_command_for(&repo, Path::new("/tmp/work"), ShellType::Bash).expect("ref set");

    assert!(command.contains("fetch --filter=tree:0 origin 'abc123'"));
    assert!(command.contains("checkout --detach FETCH_HEAD"));
    // Must not check out the original ref name after fetch (stale local branch
    // risk / FETCH_HEAD-only objects).
    assert!(!command.contains("checkout 'abc123'"));
    assert!(!command.contains("checkout \"abc123\""));
}

#[test]
fn checkout_command_absent_when_no_ref() {
    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    );
    assert!(checkout_command_for(&repo, Path::new("/tmp/work"), ShellType::Bash).is_none());
}

#[test]
fn setup_command_error_includes_command_output() {
    // Regression: the failed setup command's output must be surfaced in the
    // error message so the failure is debuggable. Before the fix the
    // `SetupCommand` error carried only the command text.
    let error = PrepareEnvironmentError::SetupCommand {
        command: "npm ci".to_string(),
        output: "npm ERR! missing script: build".to_string(),
    };
    let message = error.to_string();
    assert!(
        message.contains("npm ci"),
        "message should name the command: {message}"
    );
    assert!(
        message.contains("npm ERR! missing script: build"),
        "message should include the failed command's output: {message}"
    );
}

#[test]
fn setup_command_error_without_output_keeps_command_only_message() {
    // When no output could be captured, the message is unchanged from the
    // original command-only form.
    let error = PrepareEnvironmentError::SetupCommand {
        command: "make".to_string(),
        output: String::new(),
    };
    assert_eq!(error.to_string(), "Failed to run setup command: make");
}

#[test]
fn truncate_setup_command_output_keeps_short_output_and_trims_trailing_whitespace() {
    assert_eq!(
        truncate_setup_command_output("line 1\nline 2\n\n"),
        "line 1\nline 2"
    );
}

// #[serial] because the secret regexes configured below are global state.
#[test]
#[serial]
fn setup_command_output_is_secret_redacted() {
    // block_output_plaintext does NOT mask secrets in the default (non-Asterisks)
    // session mode used by cloud/agent setup, so the captured output must be
    // redacted before it reaches logs / Sentry / the task status. Configure a
    // secret pattern (a GitHub classic PAT), mirroring how these are populated
    // from privacy settings in production.
    set_user_and_enterprise_secret_regexes(
        [&Regex::new(r"\bghp_[A-Za-z0-9_]{36}\b").expect("pattern should compile")],
        std::iter::empty(),
    );

    let token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij";
    let output = format!("Cloning into 'repo'...\nremote: using token {token}\nfatal: auth failed");

    let redacted = redact_and_truncate_setup_command_output(&output);

    assert!(
        !redacted.contains(token),
        "captured setup-command output must not contain the raw secret: {redacted}"
    );
    assert!(
        redacted.contains(&"*".repeat(token.len())),
        "the secret must be replaced with the redaction character: {redacted}"
    );
    // Non-secret context is preserved so the failure stays debuggable.
    assert!(
        redacted.contains("fatal: auth failed"),
        "non-secret output should be preserved: {redacted}"
    );
}

#[test]
fn truncate_setup_command_output_keeps_tail_and_marks_truncation() {
    let long = "a".repeat(MAX_SETUP_COMMAND_OUTPUT_BYTES + 500);
    let truncated = truncate_setup_command_output(&long);

    let tail = truncated
        .strip_prefix("…(truncated)\n")
        .expect("over-long output is marked as truncated");
    assert_eq!(
        tail.len(),
        MAX_SETUP_COMMAND_OUTPUT_BYTES,
        "only the bounded tail is retained"
    );
    assert!(
        long.ends_with(tail),
        "the retained slice is the tail of the original output"
    );
}

#[test]
fn checkout_result_maps_nonzero_exit_to_checkout_failed() {
    assert!(checkout_result("warpdotdev/warp", "abc123", ExitCode::from(0)).is_ok());
    let err = checkout_result("warpdotdev/warp", "abc123", ExitCode::from(1)).unwrap_err();
    assert!(matches!(
        err,
        PrepareEnvironmentError::CheckoutFailed { repo_name, checkout_ref }
            if repo_name == "warpdotdev/warp" && checkout_ref == "abc123"
    ));
}

// --- Real-git fixture tests -------------------------------------------------
//
// These exercise the actual fetch-then-checkout command produced by
// `checkout_command_for` against a local git "forge", so we verify real git
// behavior (fetch of a commit not brought down by the partial clone, then
// checkout) rather than just the command string.

fn git(args: &[&str], cwd: &Path) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "saga-test")
        .env("GIT_AUTHOR_EMAIL", "saga-test@example.com")
        .env("GIT_COMMITTER_NAME", "saga-test")
        .env("GIT_COMMITTER_EMAIL", "saga-test@example.com")
        .output()
        .expect("git should be runnable");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(args: &[&str], cwd: &Path) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should be runnable");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

struct Fixture {
    _tmp: TempDir,
    origin_url: String,
    working_dir: PathBuf,
    /// Commit that is present on the origin but NOT reachable from the default
    /// branch, so a plain clone will not bring it down.
    pinned_sha: String,
    /// Default-branch tip, used to confirm the no-ref path stays put.
    base_sha: String,
    repo_name: String,
}

/// Build a local bare "origin" repo whose default branch is at `base_sha` and
/// which additionally holds `pinned_sha` under a hidden ref (`refs/pinned/base`)
/// so `git clone` (which only fetches `refs/heads/*`) does not pull it in.
fn build_fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let origin = root.join("origin.git");
    fs::create_dir_all(&origin).unwrap();
    git(&["init", "-b", "main", "--bare", "."], &origin);
    // Allow partial clone + fetch-by-SHA over the smart protocol, matching what
    // a real forge (e.g. GitHub) permits.
    git(&["config", "uploadpack.allowFilter", "true"], &origin);
    git(
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
        &origin,
    );

    let seed = root.join("seed");
    fs::create_dir_all(&seed).unwrap();
    git(&["init", "-b", "main", "."], &seed);
    fs::write(seed.join("README.md"), "base\n").unwrap();
    git(&["add", "."], &seed);
    git(&["commit", "-m", "base"], &seed);
    let base_sha = git_stdout(&["rev-parse", "HEAD"], &seed);
    git(
        &["remote", "add", "origin", origin.to_str().unwrap()],
        &seed,
    );
    git(&["push", "origin", "main"], &seed);

    // A second commit that is intentionally left off `main`.
    fs::write(seed.join("PINNED.md"), "pinned\n").unwrap();
    git(&["add", "."], &seed);
    git(&["commit", "-m", "pinned"], &seed);
    let pinned_sha = git_stdout(&["rev-parse", "HEAD"], &seed);
    git(&["push", "origin", "HEAD:refs/pinned/base"], &seed);

    let working_dir = root.join("work");
    fs::create_dir_all(&working_dir).unwrap();

    // Force the smart protocol (not the local-hardlink optimization) so the
    // partial-clone filter and hidden-ref exclusion behave like a real remote.
    let origin_url = format!("file://{}", origin.display());

    Fixture {
        _tmp: tmp,
        origin_url,
        working_dir,
        pinned_sha,
        base_sha,
        repo_name: "fixture".to_string(),
    }
}

fn partial_clone(fixture: &Fixture) -> PathBuf {
    let repo_dir = fixture.working_dir.join(&fixture.repo_name);
    git(
        &[
            "clone",
            "--filter=tree:0",
            &fixture.origin_url,
            repo_dir.to_str().unwrap(),
        ],
        &fixture.working_dir,
    );
    repo_dir
}

fn run_command(command: &str) -> std::process::ExitStatus {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .expect("sh should be runnable")
}

/// Unwrap the outer `sh -c '...'` quoting produced by `build_parallel_clone_command`
/// so tests can execute the embedded shell script directly.
fn unwrap_sh_c_script(command: &str) -> String {
    let prefix = "sh -c '";
    let suffix = "'";
    let inner = command
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .unwrap_or_else(|| panic!("expected sh -c '...' wrapper, got: {command}"));
    // Reverse the single-quote escaping used by shell_escape_single_quotes:
    // interior `'` becomes `'"'"'`.
    inner.replace("'\"'\"'", "'")
}

/// Extract the `clone_repo` function body from the parallel clone script and
/// invoke it once against a local fixture origin/target.
fn run_parallel_clone_repo_helper(
    fixture: &Fixture,
    target: &Path,
    checkout_ref: Option<&str>,
) -> std::process::ExitStatus {
    // Two dummy repos so we hit the parallel path (and its shell helper).
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            fixture.repo_name.clone(),
        )
        .with_checkout_ref(checkout_ref.map(str::to_string)),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "other".to_string(),
        ),
    ];
    let script = unwrap_sh_c_script(&build_parallel_clone_command(&repos, ShellType::Bash));

    // Keep only the clone_repo function definition; drop background clones /
    // waits / logs. Locate by name so we don't grab cleanup_clone_logs instead.
    let helper_start = script
        .find("clone_repo() {")
        .expect("clone_repo function definition");
    let helper_end = script[helper_start..]
        .find("\n}")
        .expect("clone_repo function terminator")
        + helper_start
        + 2;
    let helper = &script[helper_start..helper_end];
    let invoke = format!(
        "set -e\n{helper}\nclone_repo 'warpdotdev/{repo}' '{origin}' '{target}' '{checkout_ref}'\n",
        repo = fixture.repo_name,
        origin = fixture.origin_url,
        target = target.display(),
        checkout_ref = checkout_ref.unwrap_or(""),
    );
    run_command(&invoke)
}

#[test]
fn checkout_command_pins_head_to_commit_absent_from_default_branch() {
    let fixture = build_fixture();
    let repo_dir = partial_clone(&fixture);

    // The partial clone (`--filter=tree:0`) only fetched `main`, so the pinned
    // commit — which lives off the default branch — requires the fetch step
    // baked into the command before it can be checked out.
    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        fixture.repo_name.clone(),
    )
    .with_checkout_ref(Some(fixture.pinned_sha.clone()));
    let command =
        checkout_command_for(&repo, &fixture.working_dir, ShellType::Bash).expect("a ref was set");

    assert!(run_command(&command).success());
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.pinned_sha
    );
    // Detached HEAD is intentional for trial pins.
    assert_eq!(
        git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"], &repo_dir),
        "HEAD"
    );
}

#[test]
fn checkout_command_prefers_fetched_object_over_stale_local_branch() {
    let fixture = build_fixture();
    let repo_dir = partial_clone(&fixture);

    // Create a local branch named like the remote branch we will fetch, but
    // pointing at the default-branch tip (stale). Checking out the branch name
    // would stay on base_sha; FETCH_HEAD must win.
    git(&["branch", "feature", &fixture.base_sha], &repo_dir);

    // Publish the pinned commit under refs/heads/feature on the origin.
    git(
        &[
            "push",
            &fixture.origin_url,
            &format!("{}:refs/heads/feature", fixture.pinned_sha),
        ],
        &repo_dir,
    );

    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        fixture.repo_name.clone(),
    )
    .with_checkout_ref(Some("feature".to_string()));
    let command =
        checkout_command_for(&repo, &fixture.working_dir, ShellType::Bash).expect("a ref was set");

    assert!(run_command(&command).success());
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.pinned_sha,
        "must pin to the just-fetched remote tip, not the stale local branch"
    );
}

#[test]
fn parallel_clone_shell_pins_existing_target_dir_when_checkout_ref_set() {
    let fixture = build_fixture();
    // Simulate a reused multi-repo target directory already present on the
    // default branch tip.
    let repo_dir = partial_clone(&fixture);
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.base_sha
    );

    // Drive the real shell helper extracted from build_parallel_clone_command.
    assert!(
        run_parallel_clone_repo_helper(&fixture, &repo_dir, Some(&fixture.pinned_sha)).success()
    );
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.pinned_sha,
        "reused target directory must still pin when checkout_ref is set"
    );
}

#[test]
fn parallel_clone_shell_leaves_existing_target_dir_when_no_checkout_ref() {
    let fixture = build_fixture();
    let repo_dir = partial_clone(&fixture);

    assert!(run_parallel_clone_repo_helper(&fixture, &repo_dir, None).success());
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.base_sha,
        "no checkout_ref must leave an existing directory untouched"
    );
}

/// Single-repo `clone_repo()` skips clone when the directory already exists, but
/// must still run `checkout_command_for` when `checkout_ref` is set so a reused
/// working tree is pinned rather than left on a stale default-branch tip.
#[test]
fn single_repo_checkout_pins_existing_dir_when_checkout_ref_set() {
    let fixture = build_fixture();
    // Existing single-repo target on the default branch (the reuse path).
    let repo_dir = partial_clone(&fixture);
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.base_sha
    );

    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        fixture.repo_name.clone(),
    )
    .with_checkout_ref(Some(fixture.pinned_sha.clone()));

    // This is the command single-repo clone_repo runs after the if/else when a
    // checkout_ref is present — including when the clone itself was skipped.
    let command =
        checkout_command_for(&repo, &fixture.working_dir, ShellType::Bash).expect("a ref was set");
    assert!(
        run_command(&command).success(),
        "single-repo reuse path must fetch and pin checkout_ref"
    );
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.pinned_sha,
        "reused single-repo directory must pin when checkout_ref is set"
    );
    assert_eq!(
        git_stdout(&["rev-parse", "--abbrev-ref", "HEAD"], &repo_dir),
        "HEAD",
        "pin should detach HEAD via FETCH_HEAD"
    );

    // No checkout_ref still means no pin command (reused dirs stay untouched).
    let unpinned = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        fixture.repo_name.clone(),
    );
    assert!(checkout_command_for(&unpinned, &fixture.working_dir, ShellType::Bash).is_none());
}

#[test]
fn checkout_command_fails_for_unknown_ref() {
    let fixture = build_fixture();
    let repo_dir = partial_clone(&fixture);

    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        fixture.repo_name.clone(),
    )
    .with_checkout_ref(Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string()));
    let command =
        checkout_command_for(&repo, &fixture.working_dir, ShellType::Bash).expect("a ref was set");

    let status = run_command(&command);
    assert!(!status.success(), "unknown ref should fail");

    // A non-zero exit is what the clone path maps to CheckoutFailed, rather than
    // silently leaving the clone on the default branch.
    let result = checkout_result(
        "warpdotdev/fixture",
        "deadbeef",
        ExitCode::from(status.code().unwrap_or(1)),
    );
    assert!(matches!(
        result,
        Err(PrepareEnvironmentError::CheckoutFailed { .. })
    ));

    // HEAD must remain on the default branch tip.
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.base_sha
    );
}

#[test]
fn no_checkout_ref_leaves_clone_on_default_branch() {
    let fixture = build_fixture();
    let repo_dir = partial_clone(&fixture);

    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        fixture.repo_name.clone(),
    );
    // No ref means no checkout command is produced ...
    assert!(checkout_command_for(&repo, &fixture.working_dir, ShellType::Bash).is_none());
    // ... and the clone stays exactly where a plain clone leaves it.
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.base_sha
    );
}
