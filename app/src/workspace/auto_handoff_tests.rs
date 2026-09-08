use super::{AutoCloudHandoffEligibility, AutoCloudHandoffSkipReason};

fn eligibility() -> AutoCloudHandoffEligibility {
    AutoCloudHandoffEligibility {
        is_empty: false,
        is_in_progress: true,
        has_server_conversation_token: true,
        is_viewing_shared_session: false,
        can_handoff_to_cloud: true,
        already_attempted: false,
        has_local_orchestrated_children: false,
        active_model_not_cloud_runnable: false,
    }
}

#[test]
fn eligible_running_synced_conversation_is_not_skipped() {
    assert_eq!(eligibility().skip_reason(), None);
}

#[test]
fn auto_handoff_skips_orchestrator_with_local_children() {
    let eligibility = AutoCloudHandoffEligibility {
        has_local_orchestrated_children: true,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::OrchestratorWithLocalChildren)
    );
}

#[test]
fn auto_handoff_skips_empty_conversations() {
    let eligibility = AutoCloudHandoffEligibility {
        is_empty: true,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::EmptyConversation)
    );
}

#[test]
fn auto_handoff_skips_idle_conversations() {
    let eligibility = AutoCloudHandoffEligibility {
        is_in_progress: false,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::NotInProgress)
    );
}

#[test]
fn auto_handoff_skips_unsynced_conversations() {
    let eligibility = AutoCloudHandoffEligibility {
        has_server_conversation_token: false,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::MissingServerConversationToken)
    );
}

#[test]
fn auto_handoff_skips_shared_session_viewers() {
    let eligibility = AutoCloudHandoffEligibility {
        is_viewing_shared_session: true,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::SharedSessionViewer)
    );
}

#[test]
fn auto_handoff_skips_already_attempted_conversations() {
    let eligibility = AutoCloudHandoffEligibility {
        already_attempted: true,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::AlreadyAttempted)
    );
}

#[test]
fn auto_handoff_skips_conversations_that_cannot_handoff_to_cloud() {
    let eligibility = AutoCloudHandoffEligibility {
        can_handoff_to_cloud: false,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::CloudHandoffUnavailable)
    );
}

#[test]
fn auto_handoff_skips_non_cloud_runnable_models() {
    let eligibility = AutoCloudHandoffEligibility {
        active_model_not_cloud_runnable: true,
        ..eligibility()
    };

    assert_eq!(
        eligibility.skip_reason(),
        Some(AutoCloudHandoffSkipReason::ModelNotCloudRunnable)
    );
}
