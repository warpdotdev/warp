use std::path::Path;

use cloud_object_models::CodeForge;
use warp_cli::agent::{RepositoryForge, RepositoryHeadOverride, RepositoryHeadRef};

use super::{
    build_parallel_clone_command, build_remove_repository_origins_command, checkout_command_for,
    repository_clone_requests, single_repo_name, validate_repository_head_overrides,
    RepositoryCloneRequest,
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

    let command = checkout_command_for(
        &clone_request(repo, Some(RepositoryHeadRef::Branch("feature".to_string()))),
        Path::new("/workspace"),
        ShellType::Bash,
    )
    .unwrap();

    assert!(command.contains("git -C '/workspace/warp' fetch --filter=tree:0 origin 'feature'"));
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
    assert!(checkout_command_for(
        &clone_request(repo, None),
        Path::new("/workspace"),
        ShellType::Bash
    )
    .is_none());
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

    let duplicate_error =
        validate_repository_head_overrides(Some(&environment), &[github.clone(), github.clone()])
            .expect_err("duplicate repository identity must fail");
    assert!(duplicate_error.to_string().contains("duplicate"));

    let forge_mismatch = commit_head_override(
        RepositoryForge::GitLab,
        "warpdotdev",
        "warp",
        "0123456789abcdef0123456789abcdef01234567",
    );
    let mismatch_error = validate_repository_head_overrides(Some(&environment), &[forge_mismatch])
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

    validate_repository_head_overrides(Some(&environment), &partial_overrides)
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

    let command =
        build_remove_repository_origins_command(&repos, Path::new("/workspace"), ShellType::Bash);

    assert!(command.contains("/workspace/warp"));
    assert!(command.contains("/workspace/warp-server"));
    assert!(command.contains("remote get-url origin"));
    assert!(command.contains("remote remove origin"));
}
