use cloud_object_models::CodeForge;

use super::{
    PrepareEnvironmentError, build_parallel_clone_command, merge_repos_deduped, single_repo_name,
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
    assert!(command.contains("pids=\"$pids $!\""));
    assert!(command.contains("wait \"$pid\""));
    assert!(command.contains("===== warpdotdev/warp ====="));
    assert!(command.contains("cat \"$log_file_0\""));
    assert!(command.contains("===== platform/backend/api ====="));
    assert!(command.contains("cat \"$log_file_1\""));
    assert!(command.contains("exit \"$failed\""));
}
