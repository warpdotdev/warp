use super::super::proto;
use crate::code_review::diff_state::DiffStats;
use crate::code_review::git_repo_model::GitStatusMetadata;
use crate::context_chips::display_chip::GitBranchTrackingStatus;
use crate::context_chips::git_operation_state::GitOperationKind;

fn sample_metadata(git_operation_state: Option<GitOperationKind>) -> GitStatusMetadata {
    GitStatusMetadata {
        current_branch_name: "feature".to_string(),
        main_branch_name: "main".to_string(),
        stats_against_head: DiffStats {
            files_changed: 1,
            total_additions: 2,
            total_deletions: 3,
        },
        branch_tracking_status: GitBranchTrackingStatus::new(
            "feature".to_string(),
            Some("origin/feature".to_string()),
            1,
            0,
        ),
        git_operation_state,
    }
}

#[test]
fn git_status_metadata_round_trips_with_no_operation_in_progress() {
    let metadata = sample_metadata(None);

    let proto_metadata = proto::GitStatusMetadata::from(&metadata);
    assert_eq!(proto_metadata.git_operation_state, None);

    let decoded = GitStatusMetadata::try_from(&proto_metadata).unwrap();
    assert_eq!(decoded.git_operation_state, None);
}

#[test]
fn git_status_metadata_round_trips_every_operation_kind() {
    for kind in [
        GitOperationKind::RebaseInteractive,
        GitOperationKind::RebaseApply,
        GitOperationKind::Am,
        GitOperationKind::Merge,
        GitOperationKind::CherryPick,
        GitOperationKind::Revert,
        GitOperationKind::Bisect,
    ] {
        let metadata = sample_metadata(Some(kind));

        let proto_metadata = proto::GitStatusMetadata::from(&metadata);
        assert_eq!(
            proto_metadata.git_operation_state.as_deref(),
            Some(kind.token())
        );

        let decoded = GitStatusMetadata::try_from(&proto_metadata).unwrap();
        assert_eq!(decoded.git_operation_state, Some(kind));
    }
}

#[test]
fn git_status_metadata_decode_treats_unrecognized_token_as_no_operation() {
    let mut proto_metadata = proto::GitStatusMetadata::from(&sample_metadata(None));
    proto_metadata.git_operation_state = Some("not-a-real-state".to_string());

    let decoded = GitStatusMetadata::try_from(&proto_metadata).unwrap();
    assert_eq!(decoded.git_operation_state, None);
}
