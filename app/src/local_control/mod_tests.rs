use ::local_control::protocol::{
    PaneSelector, PaneTarget, TabSelector, TabTarget, TargetSelector, WindowSelector, WindowTarget,
};

use super::{capabilities, preferred_window_id, validate_tab_create_target};
use ::local_control::protocol::ActionKind;
use ::local_control::ErrorCode;

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
fn tab_create_prefers_active_window() {
    let active = warpui::WindowId::from_usize(1);
    let frontmost = warpui::WindowId::from_usize(2);

    assert_eq!(
        preferred_window_id(Some(active), Some(frontmost)),
        Some(active)
    );
}

#[test]
fn tab_create_falls_back_to_frontmost_window() {
    let frontmost = warpui::WindowId::from_usize(2);

    assert_eq!(preferred_window_id(None, Some(frontmost)), Some(frontmost));
}
