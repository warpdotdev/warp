use serde::{Deserialize, Serialize};

use super::{github_pr_number_from_url, ChipValue};

/// Lifecycle state of a GitHub pull request from `gh pr view`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubPrState {
    Open,
    Closed,
    Merged,
    #[serde(other)]
    Unknown,
}

/// Whether GitHub considers the PR mergeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubPrMergeable {
    Mergeable,
    Conflicting,
    #[serde(other)]
    Unknown,
}

/// Merge readiness from `mergeStateStatus` in `gh pr view`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubPrMergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unstable,
    #[serde(other)]
    Unknown,
}

/// Structured GitHub PR data for the prompt context chip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubPrInfo {
    pub url: String,
    pub state: GithubPrState,
    pub is_draft: bool,
    pub mergeable: GithubPrMergeable,
    pub merge_state_status: GithubPrMergeStateStatus,
}

#[derive(Deserialize)]
struct GhPrViewJson {
    url: String,
    state: GithubPrState,
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
    mergeable: Option<GithubPrMergeable>,
    #[serde(rename = "mergeStateStatus")]
    merge_state_status: Option<GithubPrMergeStateStatus>,
}

impl GithubPrInfo {
    pub fn from_url_with_defaults(url: String) -> Self {
        Self {
            url,
            state: GithubPrState::Open,
            is_draft: false,
            mergeable: GithubPrMergeable::Unknown,
            merge_state_status: GithubPrMergeStateStatus::Unknown,
        }
    }
}

/// Parses one-line JSON emitted by `github_pull_request_prompt_chip` scripts.
pub fn parse_github_pr_command_output(raw: &str) -> Option<GithubPrInfo> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Backward compatibility: legacy chip output was a bare PR URL.
    if !trimmed.starts_with('{') {
        let url = trimmed.to_string();
        if github_pr_number_from_url(&url).is_some() || !url.is_empty() {
            return Some(GithubPrInfo::from_url_with_defaults(url));
        }
        return None;
    }

    let parsed: GhPrViewJson = serde_json::from_str(trimmed).ok()?;
    let url = parsed.url.trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(GithubPrInfo {
        url,
        state: parsed.state,
        is_draft: parsed.is_draft,
        mergeable: parsed
            .mergeable
            .unwrap_or(GithubPrMergeable::Unknown),
        merge_state_status: parsed
            .merge_state_status
            .unwrap_or(GithubPrMergeStateStatus::Unknown),
    })
}

/// Extracts PR info from a chip value, including legacy text URLs.
pub fn github_pr_info_from_chip_value(value: &ChipValue) -> Option<GithubPrInfo> {
    match value {
        ChipValue::GithubPullRequest(info) => Some(info.clone()),
        ChipValue::Text(url) => {
            let url = url.trim();
            if url.is_empty() {
                None
            } else {
                Some(GithubPrInfo::from_url_with_defaults(url.to_string()))
            }
        }
        _ => None,
    }
}

/// Stable string used for display-chip change detection and logging.
pub fn github_pr_info_cache_key(info: &GithubPrInfo) -> String {
    format!(
        "{}|{:?}|{}|{:?}|{:?}",
        info.url, info.state, info.is_draft, info.mergeable, info.merge_state_status
    )
}
