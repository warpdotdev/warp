use itertools::Itertools;
use warpui::keymap::BindingId;

use super::*;

#[test]
fn test_enqueue_new_item() {
    let mut selected_items = SelectedItems::new();

    let summary_1 = ItemSummary::Action {
        binding_id: BindingId::new(),
    };
    let summary_2 = ItemSummary::Action {
        binding_id: BindingId::new(),
    };

    // Enqueue two items.
    selected_items.enqueue(summary_1.clone());
    selected_items.enqueue(summary_2.clone());

    // Items should be returned in reverse order of they were enqueued.
    assert_eq!(
        selected_items.iter().collect_vec(),
        vec![&summary_2, &summary_1]
    );
}

#[test]
fn test_enqueue_existing_item() {
    let mut selected_items = SelectedItems::new();

    let summary_1 = ItemSummary::Action {
        binding_id: BindingId::new(),
    };
    let summary_2 = ItemSummary::Action {
        binding_id: BindingId::new(),
    };

    // Enqueue `summary_1` twice.
    selected_items.enqueue(summary_1.clone());
    selected_items.enqueue(summary_2.clone());
    selected_items.enqueue(summary_1.clone());

    // Ensure `summary_1` is returned first since it was enqueued more recently and that it isn't
    // included in the selected items list twice.
    assert_eq!(
        selected_items.iter().collect_vec(),
        vec![&summary_1, &summary_2]
    );
}
