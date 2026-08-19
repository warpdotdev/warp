use std::fs;
use std::path::{Path, PathBuf};

use cloud_object_models::CodeForge;
use command::blocking::Command;
use tempfile::TempDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadOverride, RepositoryHeadRef};
use warp_core::command::ExitCode;

use super::{
    PrepareEnvironmentError, RepositoryCloneRequest, build_parallel_clone_command,
    build_remove_repository_origins_command, checkout_command_for, checkout_result,
    merge_repos_deduped, repository_clone_requests, single_repo_name,
    validate_repository_head_overrides,
};
use crate::ai::cloud_environments::{AmbientAgentEnvironment, SourceRepo};
use crate::terminal::shell::ShellType;

fn commit_head_override(
    code_forge: RepositoryForge,
    owner: &str,
    repo: &str,
    sha: &str,
) -> RepositoryHeadOverride {
    RepositoryHeadOverride {
        code_forge,
        repo_owner: owner.to_string(),
        repo_name: repo.to_string(),
        head: RepositoryHeadRef::CommitSha(sha.to_string()),
    }
}
fn branch_head_override(
    code_forge: RepositoryForge,
    owner: &str,
    repo: &str,
    branch: &str,
) -> RepositoryHeadOverride {
    RepositoryHeadOverride {
        code_forge,
        repo_owner: owner.to_string(),
        repo_name: repo.to_string(),
        head: RepositoryHeadRef::Branch(branch.to_string()),
    }
}

fn clone_request(repo: SourceRepo, checkout: Option<RepositoryHeadRef>) -> RepositoryCloneRequest {
    RepositoryCloneRequest { repo, checkout }
}

fn named_checkout_request(repo: SourceRepo) -> RepositoryCloneRequest {
    let checkout = repo.checkout_ref.clone().map(RepositoryHeadRef::Branch);
    clone_request(repo, checkout)
}

fn environment_with_repos(repos: Vec<SourceRepo>) -> AmbientAgentEnvironment {
    let mut environment =
        AmbientAgentEnvironment::new(String::new(), None, vec![], String::new(), vec![]);
    environment.source_repos = Some(repos);
    environment
}

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

    let command = build_parallel_clone_command(
        &repos
            .into_iter()
            .map(|repo| clone_request(repo, None))
            .collect::<Vec<_>>(),
        ShellType::Bash,
    );

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
            CodeForge::GitLab,
            "platform/backend".to_string(),
            "api".to_string(),
        )
        .with_checkout_ref(Some("feature".to_string())),
    ];

    let command = build_parallel_clone_command(
        &repos
            .into_iter()
            .map(|repo| {
                let checkout = repo.checkout_ref.clone().map(RepositoryHeadRef::Branch);
                clone_request(repo, checkout)
            })
            .collect::<Vec<_>>(),
        ShellType::Bash,
    );

    assert!(command.contains("checkout_ref=\"$4\""));
    assert!(command.contains("'abc123'"));
    assert!(command.contains("'feature'"));
    assert!(command.contains("if [ -n \"$checkout_ref\" ]; then"));
    assert!(command.contains(
        "git -C \"$target\" fetch --filter=tree:0 origin \"$checkout_ref\" && git -C \"$target\" checkout --detach FETCH_HEAD"
    ));
    assert!(command.contains("git clone --filter=tree:0 \"$repo_url\" \"$target\""));
}

#[test]
fn parallel_clone_command_fetches_commit_shas_without_cloning_later_history() {
    let command = build_parallel_clone_command(
        &[
            clone_request(
                SourceRepo::new(
                    CodeForge::GitHub,
                    "warpdotdev".to_string(),
                    "warp".to_string(),
                ),
                Some(RepositoryHeadRef::CommitSha(
                    "0123456789abcdef0123456789abcdef01234567".to_string(),
                )),
            ),
            clone_request(
                SourceRepo::new(
                    CodeForge::GitHub,
                    "warpdotdev".to_string(),
                    "warp-server".to_string(),
                ),
                Some(RepositoryHeadRef::Branch("develop".to_string())),
            ),
        ],
        ShellType::Bash,
    );

    assert!(command.contains("is_commit_sha=\"$5\""));
    assert!(command.contains("'1'"));
    assert!(command.contains("'0'"));
    assert!(command.contains("git init --quiet \"$target\""));
    assert!(command.contains("git -C \"$target\" remote add origin \"$repo_url\""));
    assert_eq!(command.matches("git clone --filter=tree:0").count(), 1);
    assert!(!command.contains("--depth=1"));
}

#[test]
fn checkout_command_checks_out_fetch_head_not_ref_name() {
    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )
    .with_checkout_ref(Some("feature".to_string()));

    let workspace = Path::new("/workspace");
    let command = checkout_command_for(
        &clone_request(repo, Some(RepositoryHeadRef::Branch("feature".to_string()))),
        workspace,
        ShellType::Bash,
    )
    .unwrap();

    let warp_dir = workspace.join("warp");
    assert!(command.contains(&format!(
        "git -C '{}' fetch --filter=tree:0 origin 'feature'",
        warp_dir.to_string_lossy()
    )));
    assert!(command.contains("checkout --detach FETCH_HEAD"));
    assert!(!command.contains("checkout --detach 'feature'"));
}

#[test]
fn checkout_command_absent_when_no_ref() {
    let repo = SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    );
    assert!(
        checkout_command_for(
            &clone_request(repo, None),
            Path::new("/workspace"),
            ShellType::Bash
        )
        .is_none()
    );
}

#[test]
fn head_overrides_replace_checkout_ref_only_for_matching_repos() {
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
        )
        .with_checkout_ref(Some("old-pin".to_string())),
    ];
    let overrides = vec![
        commit_head_override(
            RepositoryForge::GitHub,
            "warpdotdev",
            "warp",
            "0123456789abcdef0123456789abcdef01234567",
        ),
        branch_head_override(RepositoryForge::GitHub, "warpdotdev", "unused", "develop"),
    ];

    let prepared = repository_clone_requests(&repos, &overrides);

    assert_eq!(
        prepared[0].checkout,
        Some(RepositoryHeadRef::CommitSha(
            "0123456789abcdef0123456789abcdef01234567".to_string(),
        ))
    );
    assert_eq!(
        prepared[1].checkout,
        Some(RepositoryHeadRef::Branch("old-pin".to_string()))
    );
}

#[test]
fn repository_head_override_validation_rejects_duplicates_and_mismatches() {
    let environment = environment_with_repos(vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )]);
    let github = commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );

    let duplicate_error = validate_repository_head_overrides(
        &environment.effective_repos(),
        &[github.clone(), github.clone()],
    )
    .expect_err("duplicate repository identity must fail");
    assert!(duplicate_error.to_string().contains("duplicate"));

    let forge_mismatch = commit_head_override(
        RepositoryForge::GitLab,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let mismatch_error =
        validate_repository_head_overrides(&environment.effective_repos(), &[forge_mismatch])
            .expect_err("forge mismatch must fail");
    assert!(mismatch_error.to_string().contains("not declared"));
}

#[test]
fn repository_head_override_validation_accepts_partial_multi_repo_sets() {
    let environment = environment_with_repos(vec![
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
    ]);
    let partial_overrides = vec![commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    validate_repository_head_overrides(&environment.effective_repos(), &partial_overrides)
        .expect("repositories without overrides should use their default branches");
}

#[test]
fn applied_head_overrides_are_threaded_through_the_existing_clone_command() {
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
        )
        .with_checkout_ref(Some("old-pin".to_string())),
    ];
    let overrides = vec![branch_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "develop",
    )];
    let command = build_parallel_clone_command(
        &repository_clone_requests(&repos, &overrides),
        ShellType::Bash,
    );

    assert!(command.contains("'develop'"));
    assert!(!command.contains("'abc123'"));
    assert!(command.contains("'old-pin'"));
    assert!(command.contains(
        "git -C \"$target\" fetch --filter=tree:0 origin \"$checkout_ref\" && git -C \"$target\" checkout --detach FETCH_HEAD"
    ));
    assert!(!command.contains("checkout_commit"));
    assert!(!command.contains("checkout_branch"));
    assert_eq!(command.matches("'1'").count(), 0);
}

#[test]
fn applied_commit_override_uses_sha_only_fetch() {
    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];
    let overrides = vec![commit_head_override(
        RepositoryForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];
    let command = build_parallel_clone_command(
        &repository_clone_requests(&repos, &overrides),
        ShellType::Bash,
    );

    assert!(command.contains("'0123456789abcdef0123456789abcdef01234567'"));
    assert!(command.contains("'1'"));
    assert!(command.contains("git init --quiet \"$target\""));
    assert!(!command.contains("--depth=1"));
}

#[test]
fn repository_origin_removal_targets_all_environment_repositories() {
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

    let workspace = Path::new("/workspace");
    let command = build_remove_repository_origins_command(&repos, workspace, ShellType::Bash);

    let warp_dir = workspace.join("warp").to_string_lossy().into_owned();
    let warp_server_dir = workspace.join("warp-server").to_string_lossy().into_owned();
    assert!(command.contains(&warp_dir));
    assert!(command.contains(&warp_server_dir));
    assert!(command.contains("remote get-url origin"));
    assert!(command.contains("remote remove origin"));
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
        named_checkout_request(
            SourceRepo::new(
                CodeForge::GitHub,
                "warpdotdev".to_string(),
                fixture.repo_name.clone(),
            )
            .with_checkout_ref(checkout_ref.map(str::to_string)),
        ),
        clone_request(
            SourceRepo::new(
                CodeForge::GitHub,
                "warpdotdev".to_string(),
                "other".to_string(),
            ),
            None,
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
        "set -e\n{helper}\nclone_repo 'warpdotdev/{repo}' '{origin}' '{target}' '{checkout_ref}' '{is_commit_sha}'\n",
        repo = fixture.repo_name,
        origin = fixture.origin_url,
        target = target.display(),
        checkout_ref = checkout_ref.unwrap_or(""),
        is_commit_sha = if checkout_ref.is_some() { "1" } else { "0" },
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
    let command = checkout_command_for(
        &named_checkout_request(repo),
        &fixture.working_dir,
        ShellType::Bash,
    )
    .expect("a ref was set");

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
    let command = checkout_command_for(
        &named_checkout_request(repo),
        &fixture.working_dir,
        ShellType::Bash,
    )
    .expect("a ref was set");

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
    let command = checkout_command_for(
        &named_checkout_request(repo),
        &fixture.working_dir,
        ShellType::Bash,
    )
    .expect("a ref was set");
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
    assert!(
        checkout_command_for(
            &clone_request(unpinned, None),
            &fixture.working_dir,
            ShellType::Bash
        )
        .is_none()
    );
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
    let command = checkout_command_for(
        &named_checkout_request(repo),
        &fixture.working_dir,
        ShellType::Bash,
    )
    .expect("a ref was set");

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
    assert!(
        checkout_command_for(
            &clone_request(repo, None),
            &fixture.working_dir,
            ShellType::Bash
        )
        .is_none()
    );
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
