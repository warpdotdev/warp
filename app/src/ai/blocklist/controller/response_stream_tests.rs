use std::time::{Duration, SystemTime};

use ::ai::api_keys::CodexTokens;
use uuid::Uuid;
use warp_multi_agent_api::request::settings::api_keys::CodexOauthCredentials;
use warp_multi_agent_api::request::settings::ApiKeys;

use super::{
    complete_codex_refresh, recovery_action, should_refresh_codex_request, CodexRefreshAction,
    RecoveryAction,
};

// Argument order: has_received_client_actions, is_recoverable, has_retry_budget,
// can_attempt_resume_on_error, is_online.

#[test]
fn pre_action_failures_retry() {
    assert_eq!(
        recovery_action(false, true, true, true, true),
        RecoveryAction::RetryNow
    );
    // Resume eligibility is irrelevant pre-actions.
    assert_eq!(
        recovery_action(false, true, true, false, true),
        RecoveryAction::RetryNow
    );
}

#[test]
fn pre_action_failures_wait_for_connectivity_when_offline() {
    assert_eq!(
        recovery_action(false, true, true, true, false),
        RecoveryAction::RetryWhenOnline
    );
}

#[test]
fn pre_action_budget_exhaustion_is_terminal() {
    // The request has already been retried MAX_RETRIES times; stop.
    assert_eq!(
        recovery_action(false, true, false, true, true),
        RecoveryAction::Fail
    );
    assert_eq!(
        recovery_action(false, true, false, true, false),
        RecoveryAction::Fail
    );
}

#[test]
fn non_recoverable_pre_action_failure_is_terminal() {
    assert_eq!(
        recovery_action(false, false, true, true, true),
        RecoveryAction::Fail
    );
}

#[test]
fn post_action_recoverable_failures_resume() {
    assert_eq!(
        recovery_action(true, true, true, true, true),
        RecoveryAction::Resume
    );
    // Offline doesn't change the decision; the resume spawn waits for connectivity.
    assert_eq!(
        recovery_action(true, true, true, true, false),
        RecoveryAction::Resume
    );
    // The in-request retry budget is irrelevant once actions have executed.
    assert_eq!(
        recovery_action(true, true, false, true, true),
        RecoveryAction::Resume
    );
}

#[test]
fn post_action_failures_without_resume_eligibility_are_terminal() {
    // Resume requests themselves run with can_attempt_resume_on_error=false,
    // bounding recovery to a single resume.
    assert_eq!(
        recovery_action(true, true, true, false, true),
        RecoveryAction::Fail
    );
}

#[test]
fn non_recoverable_post_action_failure_is_terminal() {
    // A non-recoverable error (e.g. a client error) ends the conversation even
    // after actions have executed.
    assert_eq!(
        recovery_action(true, false, true, true, true),
        RecoveryAction::Fail
    );
}

#[test]
fn codex_refresh_requires_selected_subscription_credentials() {
    assert!(should_refresh_codex_request(true, true, true));
    assert!(!should_refresh_codex_request(false, true, true));
    assert!(!should_refresh_codex_request(true, false, true));
    assert!(!should_refresh_codex_request(true, true, false));
}

#[test]
fn codex_refresh_replaces_only_nested_credentials() {
    let mut keys = ApiKeys {
        openai: "ordinary-openai-key".into(),
        codex_oauth_credentials: Some(CodexOauthCredentials {
            access_token: "expired-access".into(),
            chatgpt_account_id: "old-account".into(),
        }),
        ..Default::default()
    };
    let tokens = CodexTokens {
        access_token: "fresh-access".into(),
        refresh_token: Some("fresh-refresh".into()),
        id_token: None,
        chatgpt_account_id: "fresh-account".into(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        connected_at: Some(SystemTime::now()),
    };

    let request_id = Uuid::new_v4();
    assert_eq!(
        complete_codex_refresh(Some(request_id), request_id, Some(&mut keys), Some(&tokens),),
        CodexRefreshAction::Send
    );
    assert_eq!(keys.openai, "ordinary-openai-key");
    assert_eq!(
        keys.codex_oauth_credentials,
        Some(CodexOauthCredentials {
            access_token: "fresh-access".into(),
            chatgpt_account_id: "fresh-account".into(),
        })
    );
}

#[test]
fn blank_refreshed_codex_access_token_does_not_replace_credentials() {
    let original = CodexOauthCredentials {
        access_token: "expired-access".into(),
        chatgpt_account_id: "old-account".into(),
    };
    let mut keys = ApiKeys {
        openai: "ordinary-openai-key".into(),
        codex_oauth_credentials: Some(original.clone()),
        ..Default::default()
    };
    let tokens = CodexTokens {
        access_token: "  ".into(),
        chatgpt_account_id: "fresh-account".into(),
        ..Default::default()
    };

    let request_id = Uuid::new_v4();
    assert_eq!(
        complete_codex_refresh(Some(request_id), request_id, Some(&mut keys), Some(&tokens),),
        CodexRefreshAction::Fail
    );
    assert_eq!(keys.codex_oauth_credentials, Some(original));
    assert_eq!(keys.openai, "ordinary-openai-key");
}

#[test]
fn blank_refreshed_codex_account_id_does_not_replace_credentials() {
    let original = CodexOauthCredentials {
        access_token: "expired-access".into(),
        chatgpt_account_id: "old-account".into(),
    };
    let mut keys = ApiKeys {
        codex_oauth_credentials: Some(original.clone()),
        ..Default::default()
    };
    let tokens = CodexTokens {
        access_token: "fresh-access".into(),
        chatgpt_account_id: "\t".into(),
        ..Default::default()
    };

    let request_id = Uuid::new_v4();
    assert_eq!(
        complete_codex_refresh(Some(request_id), request_id, Some(&mut keys), Some(&tokens),),
        CodexRefreshAction::Fail
    );
    assert_eq!(keys.codex_oauth_credentials, Some(original));
}

#[test]
fn missing_current_codex_credentials_are_terminal() {
    let request_id = Uuid::new_v4();
    assert_eq!(
        complete_codex_refresh(Some(request_id), request_id, None, None),
        CodexRefreshAction::Fail
    );
}

#[test]
fn usable_replacement_credentials_allow_request_to_continue() {
    let request_id = Uuid::new_v4();
    let tokens = CodexTokens {
        access_token: "replacement-access".into(),
        chatgpt_account_id: "replacement-account".into(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        ..Default::default()
    };
    let mut keys = ApiKeys {
        codex_oauth_credentials: Some(CodexOauthCredentials {
            access_token: "expired-access".into(),
            chatgpt_account_id: "old-account".into(),
        }),
        ..Default::default()
    };

    assert_eq!(
        complete_codex_refresh(Some(request_id), request_id, Some(&mut keys), Some(&tokens),),
        CodexRefreshAction::Send
    );
    assert_eq!(
        keys.codex_oauth_credentials,
        Some(CodexOauthCredentials {
            access_token: "replacement-access".into(),
            chatgpt_account_id: "replacement-account".into(),
        })
    );
}

#[test]
fn expired_current_credentials_remain_terminal() {
    let request_id = Uuid::new_v4();
    let tokens = CodexTokens {
        access_token: "expired-access".into(),
        chatgpt_account_id: "old-account".into(),
        expires_at: Some(SystemTime::UNIX_EPOCH),
        ..Default::default()
    };
    let mut keys = ApiKeys {
        codex_oauth_credentials: Some(CodexOauthCredentials {
            access_token: "expired-access".into(),
            chatgpt_account_id: "old-account".into(),
        }),
        ..Default::default()
    };

    assert_eq!(
        complete_codex_refresh(Some(request_id), request_id, Some(&mut keys), Some(&tokens),),
        CodexRefreshAction::Fail
    );
}

#[test]
fn superseded_codex_refresh_is_dropped_without_replacing_credentials() {
    let original = CodexOauthCredentials {
        access_token: "expired-access".into(),
        chatgpt_account_id: "old-account".into(),
    };
    let mut keys = ApiKeys {
        codex_oauth_credentials: Some(original.clone()),
        ..Default::default()
    };
    let tokens = CodexTokens {
        access_token: "fresh-access".into(),
        chatgpt_account_id: "fresh-account".into(),
        ..Default::default()
    };

    assert_eq!(
        complete_codex_refresh(
            Some(Uuid::new_v4()),
            Uuid::new_v4(),
            Some(&mut keys),
            Some(&tokens),
        ),
        CodexRefreshAction::Drop
    );
    assert_eq!(keys.codex_oauth_credentials, Some(original));
}
