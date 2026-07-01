use warp_core::features::FeatureFlag;

use crate::ai::blocklist::agent_view::agent_input_footer::toolbar_item::AgentToolbarItemKind;
use crate::context_chips::{agent_footer_available_chips, ContextChipKind};

#[test]
fn chip_is_available_but_not_default_when_flag_enabled() {
    let _guard = FeatureFlag::CodeReviewCommentsChip.override_enabled(true);

    let is_crc = |item: &AgentToolbarItemKind| {
        item.context_chip_kind() == Some(&ContextChipKind::CodeReviewComments)
    };

    // Offered in the footer configurator's available list...
    assert!(agent_footer_available_chips().contains(&ContextChipKind::CodeReviewComments));
    assert!(AgentToolbarItemKind::all_available().iter().any(is_crc));

    // ...but never in the default left/right footer selections.
    assert!(!AgentToolbarItemKind::default_left().iter().any(is_crc));
    assert!(!AgentToolbarItemKind::default_right().iter().any(is_crc));
}

#[test]
fn chip_is_not_available_when_flag_disabled() {
    let _guard = FeatureFlag::CodeReviewCommentsChip.override_enabled(false);
    assert!(!agent_footer_available_chips().contains(&ContextChipKind::CodeReviewComments));
}
