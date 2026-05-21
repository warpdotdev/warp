use std::slice;

use super::{
    changed_lines_intersect_terminal_range, has_malformed_terminal_correction_signal,
    proposed_terminal_line_range,
};
use ai::diff_validation::{DiffDelta, DiffType};

#[test]
fn terminal_range_accounts_for_prior_insertions() {
    let diff = DiffType::update(
        vec![
            DiffDelta {
                replacement_line_range: 2..3,
                insertion: "alpha\nbeta\ngamma\n".to_string(),
            },
            DiffDelta {
                replacement_line_range: 5..6,
                insertion: "let value = \"unterminated".to_string(),
            },
        ],
        None,
    );
    let terminal_range =
        proposed_terminal_line_range(&diff).expect("terminal hunk should produce range");
    assert_eq!(terminal_range, 6..7);
    assert!(changed_lines_intersect_terminal_range(
        slice::from_ref(&(6..7)),
        &terminal_range
    ));
}

#[test]
fn terminal_range_accounts_for_prior_deletions() {
    let diff = DiffType::update(
        vec![
            DiffDelta {
                replacement_line_range: 2..5,
                insertion: "one-line\n".to_string(),
            },
            DiffDelta {
                replacement_line_range: 8..9,
                insertion: "let value = \"unterminated".to_string(),
            },
        ],
        None,
    );
    let terminal_range =
        proposed_terminal_line_range(&diff).expect("terminal hunk should produce range");
    assert_eq!(terminal_range, 5..6);
    assert!(changed_lines_intersect_terminal_range(
        slice::from_ref(&(5..6)),
        &terminal_range
    ));
}

#[test]
fn integration_metric_signal_true_for_shifted_multi_hunk_diff() {
    let diff = DiffType::update(
        vec![
            DiffDelta {
                replacement_line_range: 2..3,
                insertion: "alpha\nbeta\ngamma\n".to_string(),
            },
            DiffDelta {
                replacement_line_range: 5..6,
                insertion: "let value = \"unterminated".to_string(),
            },
        ],
        None,
    );
    assert!(has_malformed_terminal_correction_signal(
        &diff,
        slice::from_ref(&(6..7)),
    ));
}

#[test]
fn integration_metric_signal_false_when_terminal_hunk_not_changed() {
    let diff = DiffType::update(
        vec![
            DiffDelta {
                replacement_line_range: 2..3,
                insertion: "alpha\nbeta\ngamma\n".to_string(),
            },
            DiffDelta {
                replacement_line_range: 5..6,
                insertion: "let value = \"unterminated".to_string(),
            },
        ],
        None,
    );
    assert!(!has_malformed_terminal_correction_signal(
        &diff,
        slice::from_ref(&(2..3)),
    ));
}
