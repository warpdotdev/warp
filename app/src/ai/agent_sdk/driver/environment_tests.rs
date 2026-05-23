use super::single_repo_name;
use crate::ai::cloud_environments::GithubRepo;

#[test]
fn single_repo_name_returns_repo_when_exactly_one_repo() {
    let repos = vec![GithubRepo::new(
        "blackdagger".to_string(),
        "black-internal".to_string(),
    )];
    let selected_repo = single_repo_name(&repos);
    assert_eq!(selected_repo, Some("black-internal".to_string()));
}

#[test]
fn single_repo_name_returns_none_for_zero_or_many_repos() {
    let no_repos = Vec::<GithubRepo>::new();
    assert_eq!(single_repo_name(&no_repos), None);

    let two_repos = vec![
        GithubRepo::new("blackdagger".to_string(), "black-internal".to_string()),
        GithubRepo::new("blackdagger".to_string(), "black-server".to_string()),
    ];
    assert_eq!(single_repo_name(&two_repos), None);
}
