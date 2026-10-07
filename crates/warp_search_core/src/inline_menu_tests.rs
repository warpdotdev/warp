use super::{InlineMenuResultsUpdate, InlineMenuSelection, InputDrivenInlineMenuLifecycle};

#[test]
fn manual_dismissal_remains_disabled_until_input_is_cleared() {
    let mut lifecycle = InputDrivenInlineMenuLifecycle::default();

    assert!(lifecycle.input_changed(false, true));
    lifecycle.disable_until_empty_buffer(false);
    assert!(!lifecycle.input_changed(false, true));
    assert!(!lifecycle.input_changed(false, true));
    assert!(lifecycle.input_changed(true, false));
    assert!(lifecycle.input_changed(false, true));
}

#[test]
fn adding_a_new_trigger_reenables_after_manual_dismissal() {
    let mut lifecycle = InputDrivenInlineMenuLifecycle::default();

    assert!(lifecycle.input_changed(false, true));
    lifecycle.disable_until_empty_buffer(false);
    assert!(!lifecycle.input_changed(false, false));
    assert!(lifecycle.input_changed(false, true));
}

#[test]
fn reset_to_best_clears_selection_when_no_item_is_enabled() {
    let mut selection = InlineMenuSelection::default();
    selection.select(0, 1, |_| true);

    assert_eq!(selection.reset_to_best(3, |_| false), None);
    assert_eq!(selection.selected_index(), None);
}
#[test]
fn reset_to_best_selects_the_last_enabled_mixer_result() {
    let enabled = [true, false, true, false];
    let mut selection = InlineMenuSelection::default();

    assert_eq!(
        selection.reset_to_best(enabled.len(), |index| enabled[index]),
        Some(2)
    );
}

#[test]
fn reconcile_results_shares_loading_empty_and_best_result_policy() {
    let mut selection = InlineMenuSelection::default();
    selection.select(0, 2, |_| true);

    assert_eq!(
        selection.reconcile_results(true, 2, |_| true),
        InlineMenuResultsUpdate::Loading
    );
    assert_eq!(selection.selected_index(), Some(0));

    assert_eq!(
        selection.reconcile_results(false, 0, |_| true),
        InlineMenuResultsUpdate::Empty
    );
    assert_eq!(selection.selected_index(), None);

    assert_eq!(
        selection.reconcile_results(false, 4, |index| index != 3),
        InlineMenuResultsUpdate::Ready {
            selected_index: Some(2)
        }
    );
}

#[test]
fn next_and_previous_wrap_and_skip_disabled_items() {
    let enabled = [true, false, true, false];
    let mut selection = InlineMenuSelection::default();
    selection.select(0, enabled.len(), |index| enabled[index]);

    assert_eq!(
        selection.select_next(enabled.len(), |index| enabled[index]),
        Some(2)
    );
    assert_eq!(
        selection.select_next(enabled.len(), |index| enabled[index]),
        Some(0)
    );
    assert_eq!(
        selection.select_previous(enabled.len(), |index| enabled[index]),
        Some(2)
    );
}

#[test]
fn navigation_from_no_selection_uses_directional_edge() {
    let enabled = [true, false, true];
    let mut selection = InlineMenuSelection::default();

    assert_eq!(
        selection.select_next(enabled.len(), |index| enabled[index]),
        Some(0)
    );
    selection.clear();
    assert_eq!(
        selection.select_previous(enabled.len(), |index| enabled[index]),
        Some(2)
    );
}

#[test]
fn direct_selection_rejects_invalid_or_disabled_indices_without_moving() {
    let enabled = [true, false];
    let mut selection = InlineMenuSelection::default();
    selection.select(0, enabled.len(), |index| enabled[index]);

    assert_eq!(
        selection.select(1, enabled.len(), |index| enabled[index]),
        None
    );
    assert_eq!(selection.selected_index(), Some(0));
    assert_eq!(
        selection.select(2, enabled.len(), |index| enabled[index]),
        None
    );
    assert_eq!(selection.selected_index(), Some(0));
}

#[test]
fn empty_navigation_clears_stale_selection() {
    let mut selection = InlineMenuSelection::default();
    selection.select(0, 1, |_| true);

    assert_eq!(selection.select_next(0, |_| true), None);
    assert_eq!(selection.selected_index(), None);
}
