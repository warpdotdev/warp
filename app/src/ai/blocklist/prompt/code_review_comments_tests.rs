use warp_core::features::FeatureFlag;

use super::resolved_and_total;
use crate::ai::agent::comment::{CodeReview, ReviewComment, ReviewDiff};
use crate::ai::blocklist::agent_view::agent_input_footer::toolbar_item::AgentToolbarItemKind;
use crate::code_review::comments::CommentId;
use crate::context_chips::{agent_footer_available_chips, ContextChipKind};

fn comment(content: &str) -> ReviewComment {
    ReviewComment {
        id: CommentId::default(),
        content: content.to_string(),
        diff: ReviewDiff {
            file_path: None,
            line_number: None,
        },
        head_title: None,
    }
}

#[test]
fn resolved_and_total_counts_addressed_then_total() {
    let code_review = CodeReview {
        pending_comments: vec![comment("a"), comment("b")],
        addressed_comments: vec![comment("c")],
    };
    // resolved == addressed, total == addressed + pending.
    assert_eq!(resolved_and_total(&code_review), (1, 3));

    let empty = CodeReview::default();
    assert_eq!(resolved_and_total(&empty), (0, 0));
}

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
