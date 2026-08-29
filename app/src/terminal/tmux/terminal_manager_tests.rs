use super::should_kill_dedicated_server;
use crate::pane_group::pane::DetachType;

#[test]
fn hidden_for_close_preserves_dedicated_server_for_restore() {
    assert!(
        !should_kill_dedicated_server(DetachType::HiddenForClose),
        "HiddenForClose is the undo-close grace window"
    );
    assert!(!should_kill_dedicated_server(DetachType::Moved));
    assert!(should_kill_dedicated_server(DetachType::Closed));
}
