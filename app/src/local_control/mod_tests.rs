use std::collections::HashMap;

use ::local_control::auth::{CredentialGrant, CredentialRequest};
use ::local_control::protocol::{
    Action, ActionKind, PaneSelector, PaneTarget, TabSelector, TabTarget, TargetSelector,
    WindowSelector, WindowTarget,
};
use ::local_control::{ErrorCode, InstanceId, InvocationContext, RequestEnvelope};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, HOST, ORIGIN};
use axum::http::{HeaderMap, HeaderValue};
use chrono::Duration;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::SingletonEntity as _;

#[cfg(unix)]
use super::ensure_peer_uid;
use super::{
    capabilities, ensure_feature_enabled, ensure_protocol_version, ensure_settings_allow_action,
    handle_control_request, insert_credential, issue_credential, lookup_credential,
    outside_warp_control_enabled_for_settings, require_active_window_id, resolve_index_from_ids,
    resolve_title_from_matches, validate_action_params, validate_loopback_headers,
    validate_request_authority, validate_tab_create_target, ControlServerState, LocalControlBridge,
    LocalControlServer, MAX_ACTIVE_CREDENTIALS,
};
use crate::settings::{LocalControlMode, LocalControlModeSetting, LocalControlSettings};

fn settings_with_mode(mode: LocalControlMode) -> LocalControlSettings {
    LocalControlSettings {
        local_control_mode: LocalControlModeSetting::new(Some(mode)),
    }
}
#[cfg(unix)]
#[tokio::test]
async fn credential_broker_rejects_peer_from_different_user() {
    let (stream, _peer) = tokio::net::UnixStream::pair().expect("socket pair");
    let actual_uid = stream.peer_cred().expect("peer credentials").uid();
    let different_uid = if actual_uid == u32::MAX {
        actual_uid - 1
    } else {
        actual_uid + 1
    };

    let err = ensure_peer_uid(&stream, different_uid).expect_err("different user is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}
#[cfg(unix)]
#[tokio::test]
async fn credential_broker_accepts_peer_from_same_user() {
    let (stream, _peer) = tokio::net::UnixStream::pair().expect("socket pair");
    let actual_uid = stream.peer_cred().expect("peer credentials").uid();

    ensure_peer_uid(&stream, actual_uid).expect("same user is accepted");
}

#[test]
fn protocol_version_helper_rejects_unsupported_versions() {
    ensure_protocol_version(::local_control::PROTOCOL_VERSION)
        .expect("current version is accepted");

    let err = ensure_protocol_version(::local_control::PROTOCOL_VERSION + 1)
        .expect_err("future protocol version is rejected");
    assert_eq!(err.code, ErrorCode::ProtocolVersionUnsupported);
}

#[test]
fn tab_create_accepts_default_active_and_window_targets() {
    validate_tab_create_target(&TargetSelector::default()).expect("default target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Active),
        tab: Some(TabTarget::Active),
        pane: Some(PaneTarget::Active),
    })
    .expect("active target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        tab: None,
        pane: None,
    })
    .expect("window id target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Index { index: 0 }),
        tab: None,
        pane: None,
    })
    .expect("window index target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Title {
            title: "window".to_owned(),
        }),
        tab: None,
        pane: None,
    })
    .expect("window title target is accepted");
}

#[test]
fn tab_create_rejects_concrete_targets() {
    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        pane: None,
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: None,
        pane: Some(PaneTarget::Id {
            id: PaneSelector("pane".to_owned()),
        }),
    })
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn tab_create_rejects_unsupported_selector_forms() {
    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Index { index: 0 }),
        pane: None,
    })
    .expect_err("indexed tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn capabilities_advertises_only_first_slice_core_actions() {
    assert_eq!(
        capabilities(),
        vec![
            ActionKind::InstanceList,
            ActionKind::AppPing,
            ActionKind::AppVersion,
            ActionKind::TabCreate,
        ]
    );
}

#[test]
fn loopback_headers_reject_origin_and_host_mismatch() {
    let expected_host = "127.0.0.1:1234";
    let mut headers = HeaderMap::new();
    headers.insert(HOST, HeaderValue::from_static(expected_host));

    validate_loopback_headers(&headers, expected_host).expect("matching host should be accepted");

    headers.insert(ORIGIN, HeaderValue::from_static("https://example.com"));
    let err =
        validate_loopback_headers(&headers, expected_host).expect_err("origin should be rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);

    headers.remove(ORIGIN);
    headers.insert(HOST, HeaderValue::from_static("localhost:1234"));
    let err = validate_loopback_headers(&headers, expected_host)
        .expect_err("host mismatch should be rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);

    let headers = HeaderMap::new();
    let err = validate_loopback_headers(&headers, expected_host)
        .expect_err("missing host should be rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn outside_warp_discovery_requires_everywhere_mode() {
    assert!(!outside_warp_control_enabled_for_settings(
        &settings_with_mode(LocalControlMode::Disabled)
    ));
    assert!(!outside_warp_control_enabled_for_settings(
        &settings_with_mode(LocalControlMode::EnabledWithinWarp)
    ));
    assert!(outside_warp_control_enabled_for_settings(
        &settings_with_mode(LocalControlMode::EnabledEverywhere)
    ));
}

#[test]
fn tab_create_requires_active_window() {
    let active = warpui::WindowId::from_usize(1);

    assert_eq!(
        require_active_window_id(Some(active)).expect("active"),
        active
    );
    let err = require_active_window_id(None).expect_err("missing active window");
    assert_eq!(err.code, ErrorCode::MissingTarget);
}

#[test]
fn window_title_resolution_distinguishes_missing_and_ambiguous_targets() {
    let missing = resolve_title_from_matches(&[], ActionKind::TabCreate)
        .expect_err("zero-match title is missing");
    assert_eq!(missing.code, ErrorCode::MissingTarget);

    let matches = [
        warpui::WindowId::from_usize(1),
        warpui::WindowId::from_usize(2),
    ];
    let ambiguous = resolve_title_from_matches(&matches, ActionKind::TabCreate)
        .expect_err("multi-match title is ambiguous");
    assert_eq!(ambiguous.code, ErrorCode::AmbiguousTarget);
}

#[test]
fn missing_window_index_returns_missing_target() {
    let err = resolve_index_from_ids(std::iter::empty(), 0, ActionKind::TabCreate)
        .expect_err("zero-match index is missing");
    assert_eq!(err.code, ErrorCode::MissingTarget);
}

#[test]
fn feature_flag_disabled_denies_local_control() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(false);
    let err = ensure_feature_enabled().expect_err("feature flag disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}
#[test]
fn duplicate_server_start_is_rejected() {
    warpui::App::test((), |mut app| async move {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let server = app.add_model(|_| LocalControlServer {
            _runtime: Some(runtime),
            control_endpoint: None,
            registered_instance: None,
        });

        let err = server
            .update(&mut app, |server, ctx| server.start(ctx))
            .expect_err("duplicate start should fail");
        assert_eq!(err.code, ErrorCode::Internal);

        server
            .update(&mut app, |server, _| server._runtime.take())
            .expect("existing runtime should remain active")
            .shutdown_background();
    });
}

#[test]
fn outside_warp_requires_everywhere_mode() {
    let settings = settings_with_mode(LocalControlMode::EnabledWithinWarp);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("outside-Warp local control is disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn inside_warp_context_is_not_implemented() {
    let settings = settings_with_mode(LocalControlMode::EnabledWithinWarp);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("inside-Warp grants are not implemented");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn disabled_mode_denies_inside_warp_context() {
    let settings = settings_with_mode(LocalControlMode::Disabled);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("inside-Warp local control is disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn enabled_everywhere_allows_outside_warp_context() {
    ensure_settings_allow_action(
        &settings_with_mode(LocalControlMode::EnabledEverywhere),
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect("outside-Warp local control is enabled");
}

#[test]
fn tab_create_rejects_malformed_params() {
    let err = validate_action_params(&Action {
        kind: ActionKind::TabCreate,
        params: serde_json::json!({ "unexpected": true }),
    })
    .expect_err("tab.create params must be empty");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    validate_action_params(&Action {
        kind: ActionKind::TabCreate,
        params: serde_json::json!({}),
    })
    .expect("empty tab.create params are accepted");
}

#[test]
fn metadata_actions_reject_malformed_params() {
    let err = validate_action_params(&Action {
        kind: ActionKind::AppPing,
        params: serde_json::json!({ "unexpected": true }),
    })
    .expect_err("app.ping params must be empty");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn bridge_checks_grant_before_action_params() {
    let instance_id = InstanceId("inst_test".to_owned());
    let grant = CredentialGrant::new(
        instance_id.clone(),
        ActionKind::AppPing,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );
    let err = validate_request_authority(
        &instance_id,
        &Action {
            kind: ActionKind::AppVersion,
            params: serde_json::json!({ "unexpected": true }),
        },
        &grant,
    )
    .expect_err("wrong-action grant is rejected before params");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn credential_insertion_prunes_expired_and_caps_active_grants() {
    let mut credentials = HashMap::new();
    let instance_id = InstanceId("inst_test".to_owned());
    insert_credential(
        &mut credentials,
        "expired".to_owned(),
        CredentialGrant::new(
            instance_id.clone(),
            ActionKind::TabCreate,
            InvocationContext::OutsideWarp,
            Duration::minutes(-1),
        ),
    );
    insert_credential(
        &mut credentials,
        "active".to_owned(),
        CredentialGrant::new(
            instance_id.clone(),
            ActionKind::TabCreate,
            InvocationContext::OutsideWarp,
            Duration::minutes(5),
        ),
    );
    assert!(!credentials.contains_key("expired"));

    for index in 0..MAX_ACTIVE_CREDENTIALS {
        insert_credential(
            &mut credentials,
            format!("active-{index}"),
            CredentialGrant::new(
                instance_id.clone(),
                ActionKind::TabCreate,
                InvocationContext::OutsideWarp,
                Duration::minutes(5),
            ),
        );
    }
    assert_eq!(credentials.len(), MAX_ACTIVE_CREDENTIALS);
    assert!(credentials.contains_key(&format!("active-{}", MAX_ACTIVE_CREDENTIALS - 1)));
}

#[test]
fn expired_credential_is_rejected_and_pruned_before_request_decode() {
    let mut credentials = HashMap::new();
    let token = ::local_control::AuthToken::from_secret("expired");
    credentials.insert(
        token.secret().to_owned(),
        CredentialGrant::new(
            InstanceId("inst_test".to_owned()),
            ActionKind::TabCreate,
            InvocationContext::OutsideWarp,
            Duration::minutes(-1),
        ),
    );

    let err = lookup_credential(
        &mut credentials,
        &token,
        &InstanceId("inst_test".to_owned()),
    )
    .expect_err("expired grant is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
    assert!(!credentials.contains_key(token.secret()));
}

#[test]
fn mode_narrowing_invalidates_existing_outside_warp_grant_and_prevents_new_grants() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::EnabledEverywhere, ctx)
            })
        })
        .expect("outside-Warp control should enable");

        let instance_id = InstanceId("inst_test".to_owned());
        let expected_host = "127.0.0.1:1234".to_owned();
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let state = bridge.update(&mut app, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            ControlServerState {
                bridge_spawner: ctx.spawner(),
                instance_id: instance_id.clone(),
                expected_host: expected_host.clone(),
                credentials: Default::default(),
            }
        });
        let credential = issue_credential(
            &state,
            CredentialRequest::new(ActionKind::AppPing, InvocationContext::OutsideWarp),
        )
        .await
        .expect("outside-Warp credential should be issued");

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::EnabledWithinWarp, ctx)
            })
        })
        .expect("mode should narrow");

        let mut headers = HeaderMap::new();
        headers.insert(
            HOST,
            HeaderValue::from_str(&expected_host).expect("valid host"),
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&credential.authorization_value()).expect("valid credential"),
        );
        let request = RequestEnvelope::new(Action::new(ActionKind::AppPing));
        let response = handle_control_request(
            State(state.clone()),
            headers,
            Bytes::from(serde_json::to_vec(&request).expect("request serializes")),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);

        let err = issue_credential(
            &state,
            CredentialRequest::new(ActionKind::AppPing, InvocationContext::OutsideWarp),
        )
        .await
        .expect_err("narrowed mode should prevent new outside-Warp grants");
        assert_eq!(err.code, ErrorCode::LocalControlDisabled);
    });
}
