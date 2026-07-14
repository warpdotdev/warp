use std::fs;
use std::path::Path;

use cloud_object_models::CodeForge;
use command::blocking::Command as BlockingCommand;
use tempfile::TempDir;
use warp_cli::agent::{RepositoryBaseline, RepositoryBaselineForge};

use super::{
    build_parallel_clone_command, build_parallel_prepare_command,
    build_remove_pinned_remotes_command, single_repo_name, validate_repository_baselines,
};
use crate::ai::cloud_environments::{AmbientAgentEnvironment, SourceRepo};
use crate::terminal::shell::ShellType;

fn baseline(
    code_forge: RepositoryBaselineForge,
    owner: &str,
    repo: &str,
    sha: &str,
) -> RepositoryBaseline {
    RepositoryBaseline {
        code_forge,
        repo_owner: owner.to_string(),
        repo_name: repo.to_string(),
        commit_sha: sha.to_string(),
        branch: Some("main".to_string()),
    }
}

fn environment_with_repos(repos: Vec<SourceRepo>) -> AmbientAgentEnvironment {
    let mut environment =
        AmbientAgentEnvironment::new(String::new(), None, vec![], String::new(), vec![]);
    environment.source_repos = Some(repos);
    environment
}
fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = BlockingCommand::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
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
fn repository_baseline_validation_rejects_duplicates_and_mismatches() {
    let environment = environment_with_repos(vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )]);
    let github = baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );

    let duplicate_error =
        validate_repository_baselines(Some(&environment), &[github.clone(), github.clone()])
            .expect_err("duplicate repository identity must fail");
    assert!(duplicate_error.to_string().contains("duplicate"));

    let forge_mismatch = baseline(
        RepositoryBaselineForge::GitLab,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let mismatch_error = validate_repository_baselines(Some(&environment), &[forge_mismatch])
        .expect_err("forge mismatch must fail");
    assert!(mismatch_error.to_string().contains("not declared"));
}

#[test]
fn repository_baseline_validation_rejects_partial_multi_repo_pin_sets() {
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
    let partial_baselines = vec![baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    let error = validate_repository_baselines(Some(&environment), &partial_baselines)
        .expect_err("partial baseline set must fail");

    assert!(error.to_string().contains("one baseline for every"));
    assert!(error.to_string().contains("platform/backend/api"));
}

#[test]
fn pinned_prepare_command_uses_exact_shallow_detached_checkouts_in_parallel() {
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
    let baselines = vec![
        baseline(
            RepositoryBaselineForge::GitHub,
            "warpdotdev",
            "warp",
            "0123456789abcdef0123456789abcdef01234567",
        ),
        baseline(
            RepositoryBaselineForge::GitLab,
            "platform/backend",
            "api",
            "89abcdef0123456789abcdef0123456789abcdef",
        ),
    ];

    let command = build_parallel_prepare_command(&repos, &baselines, ShellType::Bash);

    assert!(command.contains("https://github.com/warpdotdev/warp.git"));
    assert!(command.contains("https://gitlab.com/platform/backend/api.git"));
    assert_eq!(command.matches("checkout_pinned_repo '").count(), 2);
    assert_eq!(command.matches("2>&1 &").count(), 2);
    assert!(command.contains("fetch --depth=1 origin \"$commit_sha\""));
    assert!(command.contains("checkout --detach \"$commit_sha\""));
    assert!(command.contains("rev-parse --is-inside-work-tree"));
    assert!(command.contains("[ \"$actual_sha\" != \"$commit_sha\" ]"));
    assert!(command.contains("symbolic-ref --quiet HEAD"));
    assert!(command.contains("rev-parse --is-shallow-repository"));
    assert!(command.contains("rev-list --count HEAD --all"));
    assert!(command.contains("[ \"$reachable_commits\" != \"1\" ]"));
    assert!(command.contains("return 1"));
    assert!(!command.contains("fetch origin HEAD"));
    assert!(!command.contains("checkout main"));
}

#[test]
fn pinned_prepare_command_verifies_existing_directories_instead_of_skipping() {
    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];
    let baselines = vec![baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    let command = build_parallel_prepare_command(&repos, &baselines, ShellType::Bash);

    assert!(command.contains("if [ -e \"$target\" ]"));
    assert!(command.contains("Verifying existing pinned repository"));
    assert!(command.contains("Pinned repository path $target is not a Git repository"));
    assert!(command.contains("expected $commit_sha"));
    assert!(command.contains("must use a detached HEAD"));
    assert!(command.contains("is not shallow"));
    assert!(command.contains("expected exactly 1"));
}

#[test]
fn pinned_prepare_command_fails_for_existing_repository_at_wrong_head() {
    let workspace = TempDir::new().unwrap();
    let repo_dir = workspace.path().join("warp");
    fs::create_dir(&repo_dir).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(repo_dir.join("README.md"), "existing checkout\n").unwrap();
    for args in [
        vec!["add", "README.md"],
        vec!["commit", "-q", "-m", "existing"],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];
    let baselines = vec![baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        "0000000000000000000000000000000000000000",
    )];
    let command = build_parallel_prepare_command(&repos, &baselines, ShellType::Bash);

    let output = BlockingCommand::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("expected"),
        "unexpected command output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn pinned_prepare_command_rejects_full_repo_at_matching_detached_head() {
    let workspace = TempDir::new().unwrap();
    let repo_dir = workspace.path().join("warp");
    fs::create_dir(&repo_dir).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(repo_dir.join("README.md"), "full checkout\n").unwrap();
    for args in [vec!["add", "README.md"], vec!["commit", "-q", "-m", "full"]] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let pinned_sha = git_stdout(&repo_dir, &["rev-parse", "HEAD"]);
    let checkout = BlockingCommand::new("git")
        .current_dir(&repo_dir)
        .args(["checkout", "--detach", &pinned_sha])
        .output()
        .unwrap();
    assert!(checkout.status.success());

    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];
    let baselines = vec![baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        &pinned_sha,
    )];
    let command = build_parallel_prepare_command(&repos, &baselines, ShellType::Bash);

    let output = BlockingCommand::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("is not shallow"),
        "unexpected command output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn pinned_prepare_command_rejects_shallow_repo_with_history_ref() {
    let workspace = TempDir::new().unwrap();
    let repo_dir = workspace.path().join("warp");
    fs::create_dir(&repo_dir).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(repo_dir.join("README.md"), "first\n").unwrap();
    for args in [
        vec!["add", "README.md"],
        vec!["commit", "-q", "-m", "first"],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let first_sha = git_stdout(&repo_dir, &["rev-parse", "HEAD"]);
    fs::write(repo_dir.join("README.md"), "second\n").unwrap();
    for args in [
        vec!["add", "README.md"],
        vec!["commit", "-q", "-m", "second"],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    let pinned_sha = git_stdout(&repo_dir, &["rev-parse", "HEAD"]);
    for args in [
        vec!["checkout", "--detach", &pinned_sha],
        vec!["branch", "exposed-history", &first_sha],
    ] {
        let output = BlockingCommand::new("git")
            .current_dir(&repo_dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(repo_dir.join(".git/shallow"), format!("{pinned_sha}\n")).unwrap();

    let repos = vec![SourceRepo::new(
        CodeForge::GitHub,
        "warpdotdev".to_string(),
        "warp".to_string(),
    )];
    let baselines = vec![baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        &pinned_sha,
    )];
    let command = build_parallel_prepare_command(&repos, &baselines, ShellType::Bash);

    let output = BlockingCommand::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("exposes 2 commits"),
        "unexpected command output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn pinned_remote_removal_targets_only_supplied_repositories() {
    let baselines = vec![baseline(
        RepositoryBaselineForge::GitHub,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    )];

    let command =
        build_remove_pinned_remotes_command(&baselines, Path::new("/workspace"), ShellType::Bash);

    assert!(command.contains("/workspace/warp"));
    assert!(command.contains("remote get-url origin"));
    assert!(command.contains("remote remove origin"));
}
