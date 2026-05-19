use std::collections::HashSet;

use crate::ai::artifacts::Artifact;

use super::{prompt_snapshot::PromptSnapshot, ChipValue, ContextChipKind};

pub(crate) fn apply_cloud_artifact_pr_to_prompt_snapshot(
    snapshot: &mut PromptSnapshot,
    artifacts: &[Artifact],
) -> bool {
    if !artifacts
        .iter()
        .any(|artifact| matches!(artifact, Artifact::PullRequest { .. }))
    {
        return false;
    }

    let current_working_directory = snapshot
        .chip_value(&ContextChipKind::WorkingDirectory)
        .and_then(|value| value.as_text().map(str::to_string));
    let current_git_branch = snapshot
        .chip_value(&ContextChipKind::ShellGitBranch)
        .and_then(|value| value.as_text().map(str::to_string));

    let matching_pr_urls = matching_pull_request_urls(
        artifacts,
        current_working_directory.as_deref(),
        current_git_branch.as_deref(),
    );
    let value = (matching_pr_urls.len() == 1).then(|| ChipValue::Text(matching_pr_urls[0].clone()));

    snapshot.set_chip_value(&ContextChipKind::GithubPullRequest, value)
}

fn matching_pull_request_urls(
    artifacts: &[Artifact],
    current_working_directory: Option<&str>,
    current_git_branch: Option<&str>,
) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    for artifact in artifacts {
        let Artifact::PullRequest {
            url, branch, repo, ..
        } = artifact
        else {
            continue;
        };

        if url.trim().is_empty() {
            continue;
        }

        if !repo_matches_current_working_directory(repo.as_deref(), current_working_directory) {
            continue;
        }

        if !branch_matches_current_git_branch(branch, current_git_branch) {
            continue;
        }

        if seen.insert(url.as_str()) {
            urls.push(url.clone());
        }
    }

    urls
}

fn repo_matches_current_working_directory(
    repo: Option<&str>,
    current_working_directory: Option<&str>,
) -> bool {
    let Some(repo) = repo.map(str::trim).filter(|repo| !repo.is_empty()) else {
        return false;
    };
    let Some(current_working_directory) = current_working_directory
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
    else {
        return false;
    };

    current_working_directory
        .split(['/', '\\'])
        .any(|segment| segment == repo)
}

fn branch_matches_current_git_branch(branch: &str, current_git_branch: Option<&str>) -> bool {
    let branch = branch.trim();
    if branch.is_empty() {
        return false;
    }

    let Some(current_git_branch) = current_git_branch
        .map(str::trim)
        .filter(|current_git_branch| !current_git_branch.is_empty())
    else {
        return true;
    };

    branch == current_git_branch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_chips::ChipResult;
    use crate::settings::WarpPromptSeparator;

    fn pr_artifact(url: &str, branch: &str) -> Artifact {
        let (repo, number) = crate::ai::artifacts::parse_github_pr_url(url).unzip();
        Artifact::PullRequest {
            url: url.to_string(),
            branch: branch.to_string(),
            repo,
            number,
        }
    }

    fn snapshot(cwd: Option<&str>, branch: Option<&str>, pr: Option<&str>) -> PromptSnapshot {
        let chips = vec![
            ChipResult {
                kind: ContextChipKind::WorkingDirectory,
                value: cwd.map(|cwd| ChipValue::Text(cwd.to_string())),
                on_click_values: vec![],
            },
            ChipResult {
                kind: ContextChipKind::ShellGitBranch,
                value: branch.map(|branch| ChipValue::Text(branch.to_string())),
                on_click_values: vec![],
            },
            ChipResult {
                kind: ContextChipKind::GithubPullRequest,
                value: pr.map(|pr| ChipValue::Text(pr.to_string())),
                on_click_values: vec![],
            },
        ];
        PromptSnapshot::from_chips(chips, false, WarpPromptSeparator::None)
    }

    #[test]
    fn sets_pr_chip_when_repo_and_branch_match() {
        let mut snapshot = snapshot(Some("/workspace/warp/app"), Some("feature"), None);
        let changed = apply_cloud_artifact_pr_to_prompt_snapshot(
            &mut snapshot,
            &[pr_artifact(
                "https://github.com/warpdotdev/warp/pull/123",
                "feature",
            )],
        );

        assert!(changed);
        assert_eq!(
            snapshot
                .chip_value(&ContextChipKind::GithubPullRequest)
                .and_then(|value| value.as_text().map(str::to_string)),
            Some("https://github.com/warpdotdev/warp/pull/123".to_string())
        );
    }

    #[test]
    fn does_not_set_pr_chip_when_repo_does_not_match_cwd() {
        let mut snapshot = snapshot(Some("/workspace/docs"), Some("feature"), None);
        let changed = apply_cloud_artifact_pr_to_prompt_snapshot(
            &mut snapshot,
            &[pr_artifact(
                "https://github.com/warpdotdev/warp/pull/123",
                "feature",
            )],
        );

        assert!(!changed);
        assert_eq!(
            snapshot.chip_value(&ContextChipKind::GithubPullRequest),
            None
        );
    }

    #[test]
    fn does_not_set_pr_chip_when_branch_does_not_match() {
        let mut snapshot = snapshot(Some("/workspace/warp"), Some("other"), None);
        let changed = apply_cloud_artifact_pr_to_prompt_snapshot(
            &mut snapshot,
            &[pr_artifact(
                "https://github.com/warpdotdev/warp/pull/123",
                "feature",
            )],
        );

        assert!(!changed);
        assert_eq!(
            snapshot.chip_value(&ContextChipKind::GithubPullRequest),
            None
        );
    }

    #[test]
    fn clears_pr_chip_when_multiple_artifact_prs_match() {
        let mut snapshot = snapshot(
            Some("/workspace/warp"),
            Some("feature"),
            Some("https://github.com/warpdotdev/warp/pull/123"),
        );
        let changed = apply_cloud_artifact_pr_to_prompt_snapshot(
            &mut snapshot,
            &[
                pr_artifact("https://github.com/warpdotdev/warp/pull/123", "feature"),
                pr_artifact("https://github.com/warpdotdev/warp/pull/456", "feature"),
            ],
        );

        assert!(changed);
        assert_eq!(
            snapshot.chip_value(&ContextChipKind::GithubPullRequest),
            None
        );
    }

    #[test]
    fn leaves_snapshot_unchanged_without_pr_artifacts() {
        let mut snapshot = snapshot(
            Some("/workspace/warp"),
            Some("feature"),
            Some("https://github.com/warpdotdev/warp/pull/123"),
        );
        let original = snapshot.chip_value(&ContextChipKind::GithubPullRequest);
        let changed = apply_cloud_artifact_pr_to_prompt_snapshot(&mut snapshot, &[]);

        assert!(!changed);
        assert_eq!(
            snapshot.chip_value(&ContextChipKind::GithubPullRequest),
            original
        );
    }
}
