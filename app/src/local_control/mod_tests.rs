use ::local_control::protocol::{
    ExecutionContextProof, InvocationContext, PaneSelector, PaneTarget, TabSelector, TabTarget,
    TargetSelector, WindowSelector, WindowTarget,
};

use super::{
    capabilities, preferred_window_id, validate_tab_create_target, verify_execution_context,
};
use ::local_control::auth::CredentialRequest;
use ::local_control::protocol::ActionKind;
use ::local_control::ErrorCode;
use warp_core::features::FeatureFlag;

use super::warp_control_cli_enabled;

#[test]
fn tab_create_accepts_default_and_active_targets() {
    validate_tab_create_target(&TargetSelector::default()).expect("default target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Active),
        tab: Some(TabTarget::Active),
        pane: Some(PaneTarget::Active),
    })
    .expect("active target is accepted");
}

#[test]
fn tab_create_rejects_concrete_targets() {
    let err = validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        tab: None,
        pane: None,
    })
    .expect_err("concrete window target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        pane: None,
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: None,
        pane: Some(PaneTarget::Id {
            id: PaneSelector("pane".to_owned()),
        }),
    })
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn capabilities_only_advertises_tab_create() {
    assert_eq!(capabilities(), vec![ActionKind::TabCreate]);
}

#[test]
fn local_control_disabled_when_warp_control_cli_flag_is_disabled() {
    let _guard = FeatureFlag::WarpControlCli.override_enabled(false);

    assert!(!warp_control_cli_enabled());
}

#[test]
fn local_control_enabled_when_warp_control_cli_flag_is_enabled() {
    let _guard = FeatureFlag::WarpControlCli.override_enabled(true);

    assert!(warp_control_cli_enabled());
}

#[test]
fn tab_create_prefers_active_window() {
    let active = warpui::WindowId::from_usize(1);
    assert_eq!(preferred_window_id(Some(active)), Some(active));
}

#[test]
fn tab_create_requires_active_window() {
    assert_eq!(preferred_window_id(None), None);
}

#[test]
fn inside_warp_credentials_require_verified_execution_proof() {
    let request = CredentialRequest::new(ActionKind::TabCreate, InvocationContext::InsideWarp);
    let err = verify_execution_context(&request).expect_err("missing proof rejected");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn inside_warp_credentials_accept_verified_execution_proof() {
    let mut request = CredentialRequest::new(ActionKind::TabCreate, InvocationContext::InsideWarp);
    request.execution_context_proof = Some(ExecutionContextProof::VerifiedWarpTerminal {
        proof_id: "proof".to_owned(),
    });
    verify_execution_context(&request).expect("verified proof accepted");
}
