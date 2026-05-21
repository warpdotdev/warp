use super::Availability;

/// Helper: constructs a session context for an agent view in a local session with a repo,
/// no active LRC, and an active conversation.
fn local_agent_view_with_repo() -> Availability {
    Availability::AGENT_VIEW
        | Availability::LOCAL
        | Availability::REPOSITORY
        | Availability::NO_LRC_CONTROL
        | Availability::ACTIVE_CONVERSATION
}

/// Helper: constructs a session context for a terminal view in a local session with a repo,
/// no active LRC, and an active conversation.
fn local_terminal_view_with_repo() -> Availability {
    Availability::TERMINAL_VIEW
        | Availability::LOCAL
        | Availability::REPOSITORY
        | Availability::NO_LRC_CONTROL
        | Availability::ACTIVE_CONVERSATION
}

// --- ALWAYS ---

#[test]
fn always_available_in_any_context() {
    let command = Availability::ALWAYS;
    assert!(local_agent_view_with_repo().contains(command));
    assert!(local_terminal_view_with_repo().contains(command));
    // Even a minimal remote session satisfies ALWAYS.
    assert!(Availability::AGENT_VIEW.contains(command));
}

// --- View flag tests ---

#[test]
fn no_view_requirement_available_in_any_view() {
    let command = Availability::ALWAYS;
    assert!(local_agent_view_with_repo().contains(command));
    assert!(local_terminal_view_with_repo().contains(command));
}

#[test]
fn agent_view_requirement_only_in_agent_view() {
    let command = Availability::AGENT_VIEW;
    assert!(local_agent_view_with_repo().contains(command));
    assert!(!local_terminal_view_with_repo().contains(command));
}

#[test]
fn terminal_view_requirement_only_in_terminal_view() {
    let command = Availability::TERMINAL_VIEW;
    assert!(!local_agent_view_with_repo().contains(command));
    assert!(local_terminal_view_with_repo().contains(command));
}

#[test]
fn both_view_bits_satisfy_either_view_requirement() {
    // When AgentView feature flag is disabled, both view bits are set.
    let session = Availability::AGENT_VIEW | Availability::TERMINAL_VIEW | Availability::LOCAL;
    assert!(session.contains(Availability::AGENT_VIEW));
    assert!(session.contains(Availability::TERMINAL_VIEW));
    assert!(session.contains(Availability::ALWAYS));
}

// --- Repository flag tests ---

#[test]
fn repository_requirement_satisfied_when_in_repo() {
    let command = Availability::REPOSITORY;
    let session = Availability::AGENT_VIEW | Availability::LOCAL | Availability::REPOSITORY;
    assert!(session.contains(command));
}

#[test]
fn repository_requirement_not_satisfied_when_not_in_repo() {
    let command = Availability::REPOSITORY;
    let session = Availability::AGENT_VIEW | Availability::LOCAL;
    assert!(!session.contains(command));
}

#[test]
fn no_repository_requirement_available_regardless() {
    let command = Availability::ALWAYS;
    let session_with_repo =
        Availability::AGENT_VIEW | Availability::LOCAL | Availability::REPOSITORY;
    let session_without_repo = Availability::AGENT_VIEW | Availability::LOCAL;
    assert!(session_with_repo.contains(command));
    assert!(session_without_repo.contains(command));
}

// --- LOCAL flag tests ---

#[test]
fn local_requirement_satisfied_in_local_session() {
    let command = Availability::LOCAL;
    let session = Availability::AGENT_VIEW | Availability::LOCAL;
    assert!(session.contains(command));
}

#[test]
fn local_requirement_not_satisfied_in_remote_session() {
    let command = Availability::LOCAL;
    let session = Availability::AGENT_VIEW; // remote: no LOCAL flag
    assert!(!session.contains(command));
}

#[test]
fn no_local_requirement_available_in_any_session_type() {
    let command = Availability::ALWAYS;
    let local_session = Availability::AGENT_VIEW | Availability::LOCAL;
    let remote_session = Availability::AGENT_VIEW;
    assert!(local_session.contains(command));
    assert!(remote_session.contains(command));
}

// --- NO_LRC_CONTROL flag tests ---

#[test]
fn no_lrc_control_requirement_satisfied_when_not_in_control() {
    let command = Availability::NO_LRC_CONTROL;
    let session = Availability::AGENT_VIEW | Availability::NO_LRC_CONTROL;
    assert!(session.contains(command));
}

#[test]
fn no_lrc_control_requirement_not_satisfied_when_in_control() {
    let command = Availability::NO_LRC_CONTROL;
    let session = Availability::AGENT_VIEW; // agent is in control: no NO_LRC_CONTROL flag
    assert!(!session.contains(command));
}

// --- ACTIVE_CONVERSATION flag tests ---

#[test]
fn active_conversation_requirement_satisfied_when_conversation_active() {
    let command = Availability::ACTIVE_CONVERSATION;
    let session = Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION;
    assert!(session.contains(command));
}

#[test]
fn active_conversation_requirement_not_satisfied_when_no_conversation() {
    let command = Availability::ACTIVE_CONVERSATION;
    let session = Availability::AGENT_VIEW;
    assert!(!session.contains(command));
}

// --- CODEBASE_CONTEXT flag tests ---

#[test]
fn codebase_context_requirement_satisfied_when_enabled() {
    let command = Availability::CODEBASE_CONTEXT;
    let session = Availability::AGENT_VIEW | Availability::CODEBASE_CONTEXT;
    assert!(session.contains(command));
}

#[test]
fn codebase_context_requirement_not_satisfied_when_disabled() {
    let command = Availability::CODEBASE_CONTEXT;
    let session = Availability::AGENT_VIEW;
    assert!(!session.contains(command));
}

// --- CLOUD_AGENT_V2 flag tests ---
#[test]
fn cloud_agent_v2_required_command_satisfied_in_v2_session() {
    let command =
        Availability::AGENT_VIEW | Availability::AI_ENABLED | Availability::CLOUD_AGENT_V2;

    // V2 cloud-mode composing input has AGENT_VIEW + AI_ENABLED + CLOUD_AGENT_V2 set.
    let session =
        Availability::AGENT_VIEW | Availability::AI_ENABLED | Availability::CLOUD_AGENT_V2;
    assert!(session.contains(command));
}

#[test]
fn cloud_agent_v2_required_command_not_satisfied_outside_v2() {
    let command =
        Availability::AGENT_VIEW | Availability::AI_ENABLED | Availability::CLOUD_AGENT_V2;

    // A regular agent view session without the V2 bit must not match.
    let session = Availability::AGENT_VIEW | Availability::AI_ENABLED;
    assert!(!session.contains(command));

    // Local-mode agent view (NOT_CLOUD_AGENT instead of CLOUD_AGENT_V2) must not match either.
    let session =
        Availability::AGENT_VIEW | Availability::AI_ENABLED | Availability::NOT_CLOUD_AGENT;
    assert!(!session.contains(command));
}

// --- AI_ENABLED flag tests ---

#[test]
fn ai_enabled_requirement_satisfied_when_ai_on() {
    let command = Availability::AI_ENABLED;
    let session = Availability::AGENT_VIEW | Availability::AI_ENABLED;
    assert!(session.contains(command));
}

#[test]
fn ai_enabled_requirement_not_satisfied_when_ai_off() {
    let command = Availability::AI_ENABLED;
    let session = Availability::AGENT_VIEW;
    assert!(!session.contains(command));
}

#[test]
fn commands_without_ai_enabled_remain_available_when_ai_off() {
    // Commands like `/open-file`, `/rename-tab`, `/changelog` only set session-context bits.
    // With AI off, `session_context` has no `AI_ENABLED` bit, but these should still match.
    let command_local = Availability::LOCAL;
    let command_always = Availability::ALWAYS;
    let session_ai_off = Availability::TERMINAL_VIEW | Availability::LOCAL;
    assert!(session_ai_off.contains(command_local));
    assert!(session_ai_off.contains(command_always));
}

#[test]
fn index_command_requires_repo_and_codebase_context() {
    let command = Availability::REPOSITORY | Availability::CODEBASE_CONTEXT;

    // Both present → available
    let session = Availability::AGENT_VIEW
        | Availability::LOCAL
        | Availability::REPOSITORY
        | Availability::CODEBASE_CONTEXT;
    assert!(session.contains(command));

    // Missing CODEBASE_CONTEXT → not available
    let session = Availability::AGENT_VIEW | Availability::LOCAL | Availability::REPOSITORY;
    assert!(!session.contains(command));

    // Missing REPOSITORY → not available
    let session = Availability::AGENT_VIEW | Availability::LOCAL | Availability::CODEBASE_CONTEXT;
    assert!(!session.contains(command));
}

// --- Combined flag tests ---

#[test]
fn agent_view_and_repository_both_required() {
    let command = Availability::AGENT_VIEW | Availability::REPOSITORY;

    // Agent view + repo → available
    let session = Availability::AGENT_VIEW | Availability::LOCAL | Availability::REPOSITORY;
    assert!(session.contains(command));

    // Terminal view + repo → not available (missing AGENT_VIEW)
    let session = Availability::TERMINAL_VIEW | Availability::LOCAL | Availability::REPOSITORY;
    assert!(!session.contains(command));

    // Agent view, no repo → not available (missing REPOSITORY)
    let session = Availability::AGENT_VIEW | Availability::LOCAL;
    assert!(!session.contains(command));
}

#[test]
fn agent_view_and_local_both_required() {
    let command = Availability::AGENT_VIEW | Availability::LOCAL;

    // Agent view + local → available
    let session = Availability::AGENT_VIEW | Availability::LOCAL;
    assert!(session.contains(command));

    // Agent view + remote → not available (missing LOCAL)
    let session = Availability::AGENT_VIEW;
    assert!(!session.contains(command));

    // Terminal view + local → not available (missing AGENT_VIEW)
    let session = Availability::TERMINAL_VIEW | Availability::LOCAL;
    assert!(!session.contains(command));
}

#[test]
fn fork_like_command_requires_agent_view_active_conversation_no_lrc() {
    let command =
        Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION | Availability::NO_LRC_CONTROL;

    // All conditions met → available
    let session = Availability::AGENT_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::NO_LRC_CONTROL
        | Availability::LOCAL;
    assert!(session.contains(command));

    // Agent in control of LRC → not available
    let session = Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION;
    assert!(!session.contains(command));

    // No active conversation → not available
    let session = Availability::AGENT_VIEW | Availability::NO_LRC_CONTROL;
    assert!(!session.contains(command));

    // Wrong view → not available
    let session = Availability::TERMINAL_VIEW
        | Availability::ACTIVE_CONVERSATION
        | Availability::NO_LRC_CONTROL;
    assert!(!session.contains(command));
}

#[test]
fn all_flags_required() {
    let command = Availability::AGENT_VIEW
        | Availability::LOCAL
        | Availability::REPOSITORY
        | Availability::NO_LRC_CONTROL
        | Availability::ACTIVE_CONVERSATION;

    let full_session = local_agent_view_with_repo();
    assert!(full_session.contains(command));

    // Missing any single flag → not available
    let missing_local = Availability::AGENT_VIEW
        | Availability::REPOSITORY
        | Availability::NO_LRC_CONTROL
        | Availability::ACTIVE_CONVERSATION;
    assert!(!missing_local.contains(command));

    let missing_repo = Availability::AGENT_VIEW
        | Availability::LOCAL
        | Availability::NO_LRC_CONTROL
        | Availability::ACTIVE_CONVERSATION;
    assert!(!missing_repo.contains(command));

    let missing_view = Availability::LOCAL
        | Availability::REPOSITORY
        | Availability::NO_LRC_CONTROL
        | Availability::ACTIVE_CONVERSATION;
    assert!(!missing_view.contains(command));
}
