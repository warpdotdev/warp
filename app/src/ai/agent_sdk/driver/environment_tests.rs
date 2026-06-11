use super::{build_parallel_clone_command, single_repo_name};
use crate::ai::cloud_environments::GithubRepo;
use crate::terminal::shell::ShellType;

#[test]
fn single_repo_name_returns_repo_when_exactly_one_repo() {
    let repos = vec![GithubRepo::new(
        "warpdotdev".to_string(),
        "warp-internal".to_string(),
    )];
    let selected_repo = single_repo_name(&repos);
    assert_eq!(selected_repo, Some("warp-internal".to_string()));
}

#[test]
fn single_repo_name_returns_none_for_zero_or_many_repos() {
    let no_repos = Vec::<GithubRepo>::new();
    assert_eq!(single_repo_name(&no_repos), None);

    let two_repos = vec![
        GithubRepo::new("warpdotdev".to_string(), "warp-internal".to_string()),
        GithubRepo::new("warpdotdev".to_string(), "warp-server".to_string()),
    ];
    assert_eq!(single_repo_name(&two_repos), None);
}

#[test]
fn parallel_clone_command_runs_repos_in_background_and_waits() {
    let repos = vec![
        GithubRepo::new("warpdotdev".to_string(), "warp".to_string()),
        GithubRepo::new("warpdotdev".to_string(), "warp-server".to_string()),
    ];

    let command = build_parallel_clone_command(&repos, ShellType::Bash);

    assert!(command.starts_with("sh -c '"));
    assert!(command.contains("warpdotdev/warp"));
    assert!(command.contains("https://github.com/warpdotdev/warp.git"));
    assert!(command.contains("warpdotdev/warp-server"));
    assert!(command.contains("https://github.com/warpdotdev/warp-server.git"));
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
    assert!(command.contains("===== warpdotdev/warp-server ====="));
    assert!(command.contains("cat \"$log_file_1\""));
    assert!(command.contains("exit \"$failed\""));
}
