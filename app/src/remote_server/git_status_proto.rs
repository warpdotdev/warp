//! Conversion between the git-status / GitHub-info proto types and the app
//! domain types consumed by `RemoteGitRepoStatusModel` and
//! `RemoteGitHubRepoModel`.
//!
//! Rust types are canonical, proto types are the wire format. Git status
//! (branch + HEAD diff stats), GitHub PR info, and GitHub repository info are
//! kept separate so they can be pushed on independent cadences. The
//! `DiffStats` / `PrInfo` conversions are reused from `diff_state_proto`.

use super::proto;
use crate::code_review::diff_state::DiffStats;
use crate::code_review::git_repo_model::GitStatusMetadata;
use crate::context_chips::display_chip::GitBranchTrackingStatus;
use crate::util::git::RepositoryInfo;

impl From<&proto::RepositoryInfo> for RepositoryInfo {
    fn from(info: &proto::RepositoryInfo) -> Self {
        RepositoryInfo {
            name: info.name.clone(),
            owner: info.owner.clone(),
        }
    }
}

impl From<&RepositoryInfo> for proto::RepositoryInfo {
    fn from(info: &RepositoryInfo) -> Self {
        proto::RepositoryInfo {
            name: info.name.clone(),
            owner: info.owner.clone(),
        }
    }
}

impl From<&GitStatusMetadata> for proto::GitStatusMetadata {
    fn from(metadata: &GitStatusMetadata) -> Self {
        proto::GitStatusMetadata {
            current_branch_name: metadata.current_branch_name.clone(),
            main_branch_name: metadata.main_branch_name.clone(),
            stats_against_head: Some((&metadata.stats_against_head).into()),
            tracking_upstream: metadata.branch_tracking_status.upstream.clone(),
            tracking_ahead: metadata.branch_tracking_status.ahead,
            tracking_behind: metadata.branch_tracking_status.behind,
            tracking_counts_available: metadata.branch_tracking_status.counts_available,
        }
    }
}

impl TryFrom<&proto::GitStatusMetadata> for GitStatusMetadata {
    type Error = String;

    fn try_from(metadata: &proto::GitStatusMetadata) -> Result<Self, Self::Error> {
        let stats = metadata
            .stats_against_head
            .as_ref()
            .ok_or_else(|| "missing stats_against_head in GitStatusMetadata".to_string())?;
        Ok(GitStatusMetadata {
            current_branch_name: metadata.current_branch_name.clone(),
            main_branch_name: metadata.main_branch_name.clone(),
            stats_against_head: DiffStats::from(stats),
            branch_tracking_status: if metadata.tracking_counts_available {
                GitBranchTrackingStatus::new(
                    metadata.current_branch_name.clone(),
                    metadata.tracking_upstream.clone(),
                    metadata.tracking_ahead,
                    metadata.tracking_behind,
                )
            } else {
                GitBranchTrackingStatus::without_counts(
                    metadata.current_branch_name.clone(),
                    metadata.tracking_upstream.clone(),
                )
            },
        })
    }
}
