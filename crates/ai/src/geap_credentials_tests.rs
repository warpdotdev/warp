use std::time::{Duration, SystemTime};

use super::*;

fn binding() -> GeapMintBinding {
    GeapMintBinding {
        user_uid: "user-123".to_string(),
        audience:
            "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/providers/pr"
                .to_string(),
        federation: GeapFederation::DirectWif,
    }
}

#[test]
fn admin_config_status_flags_only_non_429_4xx() {
    assert!(is_admin_config_status(Some(400)));
    assert!(is_admin_config_status(Some(401)));
    assert!(is_admin_config_status(Some(403)));
    assert!(is_admin_config_status(Some(404)));
    assert!(!is_admin_config_status(Some(429)));
    assert!(!is_admin_config_status(Some(500)));
    assert!(!is_admin_config_status(Some(503)));
    assert!(!is_admin_config_status(None));
}

#[test]
fn mint_identity_token_failure_is_user_retryable() {
    let err = LoadGeapCredentialsError::MintIdentityToken {
        detail: "warp session expired".to_string(),
    };
    assert_eq!(err.recovery_action(), GeapRecoveryAction::Retry);
}

#[test]
fn exchange_token_4xx_requires_admin() {
    for status in [400, 401, 403, 404] {
        let err = LoadGeapCredentialsError::ExchangeToken {
            status: Some(status),
            detail: "denied".to_string(),
        };
        assert_eq!(
            err.recovery_action(),
            GeapRecoveryAction::ContactAdmin,
            "status {status} should require admin"
        );
    }
}

#[test]
fn exchange_token_transient_is_user_retryable() {
    for status in [Some(429), Some(500), Some(503), None] {
        let err = LoadGeapCredentialsError::ExchangeToken {
            status,
            detail: "unavailable".to_string(),
        };
        assert_eq!(
            err.recovery_action(),
            GeapRecoveryAction::Retry,
            "status {status:?} should be user-retryable"
        );
    }
}

#[test]
fn impersonation_4xx_requires_admin() {
    for status in [401, 403, 404] {
        let err = LoadGeapCredentialsError::ImpersonateServiceAccount {
            status: Some(status),
            detail: "no permission".to_string(),
        };
        assert_eq!(
            err.recovery_action(),
            GeapRecoveryAction::ContactAdmin,
            "status {status} should require admin"
        );
    }
}

#[test]
fn impersonation_transient_is_user_retryable() {
    for status in [Some(429), Some(500), None] {
        let err = LoadGeapCredentialsError::ImpersonateServiceAccount {
            status,
            detail: "unavailable".to_string(),
        };
        assert_eq!(
            err.recovery_action(),
            GeapRecoveryAction::Retry,
            "status {status:?} should be user-retryable"
        );
    }
}

#[test]
fn user_facing_never_leaks_raw_provider_detail() {
    let secret = "SECRET_PROVIDER_DETAIL_9f3a";
    let errors = [
        LoadGeapCredentialsError::MintIdentityToken {
            detail: secret.to_string(),
        },
        LoadGeapCredentialsError::ExchangeToken {
            status: Some(403),
            detail: secret.to_string(),
        },
        LoadGeapCredentialsError::ExchangeToken {
            status: Some(500),
            detail: secret.to_string(),
        },
        LoadGeapCredentialsError::ImpersonateServiceAccount {
            status: Some(403),
            detail: secret.to_string(),
        },
        LoadGeapCredentialsError::ImpersonateServiceAccount {
            status: None,
            detail: secret.to_string(),
        },
    ];
    for err in errors {
        let (title, description, _) = err.user_facing();
        assert!(!title.contains(secret), "title leaked detail: {title}");
        assert!(
            !description.contains(secret),
            "description leaked detail: {description}"
        );
        assert!(!title.is_empty());
        assert!(!description.is_empty());
    }
}

#[test]
fn state_recovery_action_for_failed_and_unconfigured() {
    let loaded = GeapCredentialsState::Loaded {
        credentials: GeapCredentials::new("token".to_string(), Some(SystemTime::now())),
        loaded_at: SystemTime::now(),
        minted_for: binding(),
    };
    assert_eq!(GeapCredentialsState::Missing.recovery_action(), None);
    assert!(!GeapCredentialsState::Missing.requires_admin_action());
    assert_eq!(GeapCredentialsState::Disabled.recovery_action(), None);
    assert!(!GeapCredentialsState::Disabled.requires_admin_action());
    assert_eq!(
        GeapCredentialsState::Refreshing { previous: None }.recovery_action(),
        None
    );
    assert!(!GeapCredentialsState::Refreshing { previous: None }.requires_admin_action());
    assert_eq!(loaded.recovery_action(), None);
    assert!(!loaded.requires_admin_action());

    // An incomplete admin setup routes to admin guidance, not a client retry.
    assert_eq!(
        GeapCredentialsState::Unconfigured.recovery_action(),
        Some(GeapRecoveryAction::ContactAdmin)
    );
    assert!(GeapCredentialsState::Unconfigured.requires_admin_action());

    let failed = GeapCredentialsState::Failed {
        error: LoadGeapCredentialsError::ExchangeToken {
            status: Some(403),
            detail: "denied".to_string(),
        },
    };
    assert_eq!(
        failed.recovery_action(),
        Some(GeapRecoveryAction::ContactAdmin)
    );
    assert!(failed.requires_admin_action());

    let retryable = GeapCredentialsState::Failed {
        error: LoadGeapCredentialsError::ExchangeToken {
            status: Some(503),
            detail: "unavailable".to_string(),
        },
    };
    assert!(!retryable.requires_admin_action());
}

#[test]
fn unconfigured_state_points_user_to_admin_setup() {
    let (title, description, _) = GeapCredentialsState::Unconfigured.user_facing_components();
    assert!(
        title.to_lowercase().contains("setup") || title.to_lowercase().contains("incomplete"),
        "unexpected title: {title}"
    );
    assert!(
        description.to_lowercase().contains("admin"),
        "description should point at the admin: {description}"
    );
}

#[test]
fn state_components_use_expected_icons() {
    assert!(matches!(
        GeapCredentialsState::Missing.user_facing_components().2,
        Icon::Key
    ));
    assert!(matches!(
        GeapCredentialsState::Disabled.user_facing_components().2,
        Icon::Key
    ));
    assert!(matches!(
        GeapCredentialsState::Unconfigured
            .user_facing_components()
            .2,
        Icon::AlertTriangle
    ));
    assert!(matches!(
        GeapCredentialsState::Refreshing { previous: None }
            .user_facing_components()
            .2,
        Icon::RefreshCw04
    ));
    let loaded = GeapCredentialsState::Loaded {
        credentials: GeapCredentials::new("token".to_string(), Some(SystemTime::now())),
        loaded_at: SystemTime::now(),
        minted_for: binding(),
    };
    assert!(matches!(
        loaded.user_facing_components().2,
        Icon::CheckCircleBroken
    ));
    let failed = GeapCredentialsState::Failed {
        error: LoadGeapCredentialsError::MintIdentityToken {
            detail: "boom".to_string(),
        },
    };
    assert!(matches!(
        failed.user_facing_components().2,
        Icon::AlertTriangle
    ));
}

#[test]
fn loaded_state_shows_scheduled_refresh_instead_of_expiry() {
    let loaded_at = SystemTime::now();
    let expires_at = loaded_at + Duration::from_secs(60 * 60);
    let loaded = GeapCredentialsState::Loaded {
        credentials: GeapCredentials::new("token".to_string(), Some(expires_at)),
        loaded_at,
        minted_for: binding(),
    };

    let (_, description, _) = loaded.user_facing_components();
    assert!(description.starts_with("Loaded at "));
    assert!(description.contains(" · Refresh scheduled for "));
    assert!(!description.contains("expires"));
    assert_eq!(
        refresh_scheduled_at(expires_at),
        expires_at - GEAP_REFRESH_LEAD_TIME
    );
}

#[test]
fn failed_state_components_match_error_copy() {
    let error = LoadGeapCredentialsError::ImpersonateServiceAccount {
        status: Some(403),
        detail: "denied".to_string(),
    };
    let (err_title, err_desc, _) = error.user_facing();
    let state = GeapCredentialsState::Failed { error };
    let (title, description, _) = state.user_facing_components();
    assert_eq!(title, err_title);
    assert_eq!(description, err_desc);
}
