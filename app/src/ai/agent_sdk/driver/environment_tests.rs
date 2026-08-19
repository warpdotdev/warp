use std::fs;
use std::path::{Path, PathBuf};

use cloud_object_models::CodeForge;
use command::blocking::Command;
use tempfile::TempDir;
use warp_core::command::ExitCode;

use super::{
    CloneTargetState, PrepareEnvironmentError, build_parallel_clone_command, checkout_command_for,
    checkout_result, clone_repo_error, merge_repos_deduped, parse_failed_repo_names,
    single_repo_name,
};
use crate::ai::cloud_environments::SourceRepo;
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
    assert!(command.contains("pid_0=$!"));
    assert!(command.contains("pid_1=$!"));
    // Waits are generated per index (not a generic `for pid in $pids`) so a
    // failure can be attributed back to the specific repo that failed.
    assert!(command.contains("wait \"$pid_0\""));
    assert!(command.contains("wait \"$pid_1\""));
    assert!(command.contains("===== warpdotdev/warp ====="));
    assert!(command.contains("cat \"$log_file_0\""));
    assert!(command.contains("===== platform/backend/api ====="));
    assert!(command.contains("cat \"$log_file_1\""));
    assert!(command.contains("exit \"$failed\""));
}

#[test]
fn clone_target_state_classifies_directory_file_and_absent() {
    assert_eq!(
        CloneTargetState::classify(true, true),
        CloneTargetState::ExistingDirectory
    );
    assert_eq!(
        CloneTargetState::classify(true, false),
        CloneTargetState::ExistingDirectory
    );
    assert_eq!(
        CloneTargetState::classify(false, true),
        CloneTargetState::Blocked
    );
    assert_eq!(
        CloneTargetState::classify(false, false),
        CloneTargetState::Absent
    );
}

#[test]
fn parallel_clone_command_retries_clone_and_checkout_with_backoff() {
    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];

    let command = build_parallel_clone_command(&repos, ShellType::Bash);

    // Both the clone and the pinned-ref fetch+checkout are retried, bounded
    // by the same small attempt count, with a partial clone cleaned up
    // before the next attempt.
    assert!(command.contains("CLONE_MAX_ATTEMPTS=3"));
    assert!(command.contains("CLONE_RETRY_BASE_DELAY_SECS=2"));
    assert_eq!(
        command
            .matches("if [ \"$attempt\" -ge \"$CLONE_MAX_ATTEMPTS\" ]; then")
            .count(),
        2
    );
    assert_eq!(
        command
            .matches("sleep $((attempt * CLONE_RETRY_BASE_DELAY_SECS))")
            .count(),
        2
    );
    assert!(command.contains("git clone --filter=tree:0 \"$repo_url\" \"$target\" && break"));
    assert!(command.contains("rm -rf \"$target\""));
    assert!(command.contains(
        "git -C \"$target\" fetch --filter=tree:0 origin \"$checkout_ref\" && git -C \"$target\" checkout --detach FETCH_HEAD && break"
    ));
}

#[test]
fn parallel_clone_command_refuses_non_directory_targets_without_cleanup() {
    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];

    let command = build_parallel_clone_command(&repos, ShellType::Bash);

    // A pre-existing non-directory target (a file, or a dangling symlink via
    // `-L`) must be refused outright -- not cleaned up and retried like a
    // partial clone.
    assert!(command.contains("elif [ -e \"$target\" ] || [ -L \"$target\" ]; then"));
    assert!(command.contains("refusing to clone $repo_name over it"));
    // The refusal branch must return before ever reaching the `rm -rf`
    // that's only safe once we know the retry loop's own attempt created it.
    let refuse_pos = command
        .find("refusing to clone $repo_name over it")
        .unwrap();
    let rm_pos = command.find("rm -rf \"$target\"").unwrap();
    assert!(refuse_pos < rm_pos);
}

#[test]
fn parallel_clone_command_reports_only_failed_repos_via_marker() {
    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp".to_string(),
        ),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "warp-server".to_string(),
        ),
    ];

    // Assert against the unwrapped inner script: the outer `sh -c '...'`
    // escaping rewrites embedded single quotes, so raw single-quoted
    // substrings only match before that escaping is applied.
    let command = unwrap_sh_c_script(&build_parallel_clone_command(&repos, ShellType::Bash));

    assert!(command.contains("failed_repos=\"\""));
    assert!(command.contains("repo_name_0='warpdotdev/warp'"));
    assert!(command.contains("repo_name_1='warpdotdev/warp-server'"));
    assert!(command.contains("failed_repos=\"$failed_repos $repo_name_0\""));
    assert!(command.contains("failed_repos=\"$failed_repos $repo_name_1\""));
    assert!(command.contains("if [ -n \"$failed_repos\" ]; then"));
    assert!(command.contains("printf '%s\\n' \"WARP_CLONE_FAILED:$failed_repos\""));
}

#[test]
fn parse_failed_repo_names_extracts_marker_line() {
    let output = "Cloning repository warpdotdev/warp...\n\
                   WARP_CLONE_FAILED:warpdotdev/warp-server\n\
                   ===== warpdotdev/warp =====\n";
    assert_eq!(
        parse_failed_repo_names(output),
        Some(vec!["warpdotdev/warp-server".to_string()])
    );
}

#[test]
fn parse_failed_repo_names_extracts_multiple_repos() {
    let output = "WARP_CLONE_FAILED:warpdotdev/warp warpdotdev/warp-server\n";
    assert_eq!(
        parse_failed_repo_names(output),
        Some(vec![
            "warpdotdev/warp".to_string(),
            "warpdotdev/warp-server".to_string()
        ])
    );
}

#[test]
fn parse_failed_repo_names_returns_none_without_marker_or_when_empty() {
    assert_eq!(parse_failed_repo_names("no marker here\n"), None);
    assert_eq!(parse_failed_repo_names("WARP_CLONE_FAILED:\n"), None);
}

#[test]
fn clone_repo_error_names_only_failed_repos_from_marker() {
    let repo_names = vec![
        "warpdotdev/warp".to_string(),
        "warpdotdev/warp-server".to_string(),
    ];
    let output = "===== warpdotdev/warp =====\nok\n\
                   WARP_CLONE_FAILED:warpdotdev/warp-server\n";

    let error = clone_repo_error(&repo_names, output);

    assert!(matches!(
        error,
        PrepareEnvironmentError::CloneRepo { repo_name } if repo_name == "warpdotdev/warp-server"
    ));
}

#[test]
fn clone_repo_error_falls_back_to_full_batch_without_marker() {
    // Defensive fallback for e.g. the shell dying before it could print the
    // marker line: blame the whole batch rather than an empty repo name.
    let repo_names = vec![
        "warpdotdev/warp".to_string(),
        "warpdotdev/warp-server".to_string(),
    ];

    let error = clone_repo_error(&repo_names, "no marker in this output\n");

    assert!(matches!(
        error,
        PrepareEnvironmentError::CloneRepo { repo_name }
            if repo_name == "warpdotdev/warp, warpdotdev/warp-server"
    ));
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
    assert!(command.contains("git clone --filter=tree:0 \"$repo_url\" \"$target\" && break"));
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

/// Extract the `CLONE_MAX_ATTEMPTS`/`CLONE_RETRY_BASE_DELAY_SECS` assignments
/// and the `clone_repo` function definition from a generated parallel clone
/// script, dropping the background clones / waits / logs around them.
/// Located by name so this doesn't accidentally grab `cleanup_clone_logs`
/// instead, and includes the retry constants since `clone_repo`'s retry
/// loops reference them.
fn extract_clone_repo_helper(script: &str) -> &str {
    let helper_start = script
        .find("CLONE_MAX_ATTEMPTS=")
        .expect("CLONE_MAX_ATTEMPTS assignment");
    let helper_end = script[helper_start..]
        .find("\n}")
        .expect("clone_repo function terminator")
        + helper_start
        + 2;
    &script[helper_start..helper_end]
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
    let helper = extract_clone_repo_helper(&script);
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

/// Resolve the real `git` binary's absolute path, so a PATH-shadowing fake
/// `git` (used to simulate a failing first attempt) can still delegate to it.
fn real_git_path() -> String {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .expect("sh should be runnable");
    assert!(output.status.success(), "git must be on PATH for tests");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// Install a fake `git` ahead of the real one on `PATH` that fails the first
/// `clone` invocation (after leaving a partial target directory behind, like
/// a real interrupted clone would) and delegates to the real `git` for every
/// other invocation. Returns the directory to prepend to `PATH`.
fn install_failing_once_git(bin_dir: &Path) {
    fs::create_dir_all(bin_dir).unwrap();
    let marker = bin_dir.join("clone-attempted");
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = clone ] && [ ! -f '{marker}' ]; then\n\
         \ttouch '{marker}'\n\
         \ttarget=\"$4\"\n\
         \tmkdir -p \"$target\"\n\
         \ttouch \"$target/partial-clone-debris\"\n\
         \texit 1\n\
         fi\n\
         exec '{real_git}' \"$@\"\n",
        marker = marker.display(),
        real_git = real_git_path(),
    );
    let git_path = bin_dir.join("git");
    fs::write(&git_path, script).unwrap();
    let mut perms = fs::metadata(&git_path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(&git_path, perms).unwrap();
}

/// A `git clone` that fails on its first attempt (leaving a partial target
/// directory behind, like a real interrupted clone does) must have that
/// directory cleaned up and the clone retried, rather than failing outright.
#[test]
fn parallel_clone_shell_retries_and_cleans_up_after_a_failed_attempt() {
    let fixture = build_fixture();
    let bin_dir = fixture.working_dir.join("fake-bin");
    install_failing_once_git(&bin_dir);
    let repo_dir = fixture.working_dir.join(&fixture.repo_name);

    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            fixture.repo_name.clone(),
        ),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "other".to_string(),
        ),
    ];
    let script = unwrap_sh_c_script(&build_parallel_clone_command(&repos, ShellType::Bash));
    let helper = extract_clone_repo_helper(&script);
    let invoke = format!(
        "PATH='{bin_dir}':\"$PATH\"\n\
         set -e\n\
         {helper}\n\
         clone_repo 'warpdotdev/{repo}' '{origin}' '{target}' ''\n",
        bin_dir = bin_dir.display(),
        repo = fixture.repo_name,
        origin = fixture.origin_url,
        target = repo_dir.display(),
    );

    let status = run_command(&invoke);

    assert!(
        status.success(),
        "the second attempt (real git) must succeed after the first attempt's \
         partial directory is cleaned up"
    );
    assert!(
        !repo_dir.join("partial-clone-debris").exists(),
        "the partial directory from the failed attempt must be removed before retrying"
    );
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.base_sha,
        "the retried clone must produce a real checkout of the fixture repo"
    );
}

#[test]
fn parallel_clone_shell_refuses_to_clone_over_existing_file_target() {
    let fixture = build_fixture();
    let repo_dir = fixture.working_dir.join(&fixture.repo_name);
    fs::write(&repo_dir, b"unrelated pre-existing file\n").unwrap();

    let status = run_parallel_clone_repo_helper(&fixture, &repo_dir, None);

    assert!(
        !status.success(),
        "must refuse to clone over an existing non-directory target"
    );
    assert!(
        repo_dir.is_file(),
        "the pre-existing file must not be deleted"
    );
    assert_eq!(
        fs::read_to_string(&repo_dir).unwrap(),
        "unrelated pre-existing file\n",
        "file contents must be untouched"
    );
}

#[test]
#[cfg(unix)]
fn parallel_clone_shell_refuses_to_clone_over_dangling_symlink_target() {
    let fixture = build_fixture();
    let repo_dir = fixture.working_dir.join(&fixture.repo_name);
    std::os::unix::fs::symlink(
        fixture.working_dir.join("nonexistent-symlink-target"),
        &repo_dir,
    )
    .unwrap();

    let status = run_parallel_clone_repo_helper(&fixture, &repo_dir, None);

    assert!(
        !status.success(),
        "must refuse to clone over a dangling symlink target"
    );
    assert!(
        fs::symlink_metadata(&repo_dir)
            .expect("the symlink must still exist")
            .file_type()
            .is_symlink(),
        "the pre-existing dangling symlink must not be deleted"
    );
}

/// Install a fake `git` ahead of the real one on `PATH` that fails the first
/// invocation whose argument list contains `failing_subcommand` (e.g.
/// `"fetch"` or `"checkout"`) and delegates to the real `git` for every
/// other invocation, including subsequent invocations of the same
/// subcommand. Unlike [`install_failing_once_git`], this doesn't leave any
/// debris behind: a failed `fetch`/`checkout` doesn't corrupt the working
/// tree the way an interrupted `clone` can, so the retry loop for those
/// steps has no cleanup to verify, only that the retry itself happens.
fn install_failing_once_git_step(bin_dir: &Path, failing_subcommand: &str) {
    fs::create_dir_all(bin_dir).unwrap();
    let marker = bin_dir.join(format!("{failing_subcommand}-attempted"));
    let script = format!(
        "#!/bin/sh\n\
         if [ ! -f '{marker}' ]; then\n\
         \tfor arg in \"$@\"; do\n\
         \t\tif [ \"$arg\" = '{failing_subcommand}' ]; then\n\
         \t\t\ttouch '{marker}'\n\
         \t\t\texit 1\n\
         \t\tfi\n\
         \tdone\n\
         fi\n\
         exec '{real_git}' \"$@\"\n",
        marker = marker.display(),
        failing_subcommand = failing_subcommand,
        real_git = real_git_path(),
    );
    let git_path = bin_dir.join("git");
    fs::write(&git_path, script).unwrap();
    let mut perms = fs::metadata(&git_path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(&git_path, perms).unwrap();
}

/// A pinned-ref `fetch` that fails on its first attempt must be retried
/// (rather than immediately surfacing `CheckoutFailed`) and still land on
/// the pinned commit once a later attempt succeeds. This exercises the real
/// `git -C ... fetch ... && git -C ... checkout ...` retry loop end to end,
/// rather than only asserting it was generated as text.
#[test]
fn parallel_clone_shell_retries_fetch_before_pinning() {
    let fixture = build_fixture();
    let bin_dir = fixture.working_dir.join("fake-bin-fetch");
    install_failing_once_git_step(&bin_dir, "fetch");
    let repo_dir = fixture.working_dir.join(&fixture.repo_name);

    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            fixture.repo_name.clone(),
        )
        .with_checkout_ref(Some(fixture.pinned_sha.clone())),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "other".to_string(),
        ),
    ];
    let script = unwrap_sh_c_script(&build_parallel_clone_command(&repos, ShellType::Bash));
    let helper = extract_clone_repo_helper(&script);
    let invoke = format!(
        "PATH='{bin_dir}':\"$PATH\"\n\
         set -e\n\
         {helper}\n\
         clone_repo 'warpdotdev/{repo}' '{origin}' '{target}' '{checkout_ref}'\n",
        bin_dir = bin_dir.display(),
        repo = fixture.repo_name,
        origin = fixture.origin_url,
        target = repo_dir.display(),
        checkout_ref = fixture.pinned_sha,
    );

    let status = run_command(&invoke);

    assert!(
        status.success(),
        "the pin must succeed once the retried fetch goes through"
    );
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.pinned_sha,
        "HEAD must land on the pinned commit despite the first fetch failing"
    );
}

/// Same as [`parallel_clone_shell_retries_fetch_before_pinning`] but for a
/// `checkout` that fails on its first attempt after a successful `fetch`.
#[test]
fn parallel_clone_shell_retries_checkout_before_pinning() {
    let fixture = build_fixture();
    let bin_dir = fixture.working_dir.join("fake-bin-checkout");
    install_failing_once_git_step(&bin_dir, "checkout");
    let repo_dir = fixture.working_dir.join(&fixture.repo_name);

    let repos = vec![
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            fixture.repo_name.clone(),
        )
        .with_checkout_ref(Some(fixture.pinned_sha.clone())),
        SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".to_string(),
            "other".to_string(),
        ),
    ];
    let script = unwrap_sh_c_script(&build_parallel_clone_command(&repos, ShellType::Bash));
    let helper = extract_clone_repo_helper(&script);
    let invoke = format!(
        "PATH='{bin_dir}':\"$PATH\"\n\
         set -e\n\
         {helper}\n\
         clone_repo 'warpdotdev/{repo}' '{origin}' '{target}' '{checkout_ref}'\n",
        bin_dir = bin_dir.display(),
        repo = fixture.repo_name,
        origin = fixture.origin_url,
        target = repo_dir.display(),
        checkout_ref = fixture.pinned_sha,
    );

    let status = run_command(&invoke);

    assert!(
        status.success(),
        "the pin must succeed once the retried checkout goes through"
    );
    assert_eq!(
        git_stdout(&["rev-parse", "HEAD"], &repo_dir),
        fixture.pinned_sha,
        "HEAD must land on the pinned commit despite the first checkout failing"
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

#[test]
fn factory_clone_is_prepended_when_clone_values_are_present() {
    let mut setup_commands = vec!["make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "https://t:token@definitions.example.com/team/factory.git",
        "acme_factory_repo",
        &mut setup_commands,
    );
    assert_eq!(
        setup_commands,
        vec![
            "git clone \"$WARP_FACTORY_REPO_CLONE_URL\" \"$WARP_FACTORY_REPO_DIR\"".to_string(),
            "make setup".to_string(),
        ]
    );
}

#[test]
fn factory_clone_is_skipped_without_clone_values() {
    let mut setup_commands = vec!["make setup".to_string()];
    super::prepend_factory_definition_clone_for_values("", "", &mut setup_commands);
    super::prepend_factory_definition_clone_for_values("url", "  ", &mut setup_commands);
    super::prepend_factory_definition_clone_for_values("  ", "dir", &mut setup_commands);
    assert_eq!(setup_commands, vec!["make setup".to_string()]);
}

#[test]
fn factory_clone_defers_to_a_persisted_environment_copy() {
    // Environments provisioned before run-scoped cloning persist their own
    // copy of the clone command. Detection keys off the URL env var name,
    // not the command's exact shape, so this must still be recognized and
    // left alone rather than duplicated.
    let persisted =
        "git clone \"$WARP_FACTORY_REPO_CLONE_URL\" \"$WARP_FACTORY_REPO_DIR\"".to_string();
    let mut setup_commands = vec![persisted.clone(), "make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "https://t:token@definitions.example.com/team/factory.git",
        "acme_factory_repo",
        &mut setup_commands,
    );
    assert_eq!(setup_commands, vec![persisted, "make setup".to_string()]);
}

#[test]
fn factory_clone_defers_to_a_persisted_bare_clone_copy() {
    // A persisted copy in the bare (no target dir) shape must also be
    // recognized, since detection is shape-independent.
    let persisted = "git clone \"$WARP_FACTORY_REPO_CLONE_URL\"".to_string();
    let mut setup_commands = vec![persisted.clone(), "make setup".to_string()];
    super::prepend_factory_definition_clone_for_values(
        "https://t:token@definitions.example.com/team/factory.git",
        "acme_factory_repo",
        &mut setup_commands,
    );
    assert_eq!(setup_commands, vec![persisted, "make setup".to_string()]);
}
