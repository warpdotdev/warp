//! Unit tests for the pure helpers in `host_picker.rs`. View-driven
//! behaviors (custom-mode commit, blur, etc.) are covered by manual smoke
//! testing.

use super::{
    DropdownAction, InternalAction, MenuItem, ORCHESTRATION_WARP_WORKER_HOST, build_menu_items,
    menu_label_for, normalize_slug,
};

/// Extracts the visible label text out of a `MenuItem::Item`, panicking
/// on the unreachable `Header` / `Separator` cases that our builder
/// doesn't emit.
fn item_label(item: &MenuItem<DropdownAction>) -> &str {
    match item {
        MenuItem::Item(fields) => fields.label(),
        other => panic!("expected MenuItem::Item, got {other:?}"),
    }
}

/// Extracts the on-select action from a `MenuItem::Item`.
fn item_action(item: &MenuItem<DropdownAction>) -> &DropdownAction {
    match item {
        MenuItem::Item(fields) => fields
            .on_select_action()
            .expect("test items always have a select action"),
        other => panic!("expected MenuItem::Item, got {other:?}"),
    }
}

#[test]
fn build_menu_items_with_no_defaults_shows_warp_and_custom() {
    let items = build_menu_items(None, None, &[]);
    assert_eq!(items.len(), 2, "expected warp + custom-host entries");
    assert_eq!(item_label(&items[0]), ORCHESTRATION_WARP_WORKER_HOST);
    assert_eq!(item_label(&items[1]), "Custom host\u{2026}");
}

#[test]
fn build_menu_items_promotes_default_to_top() {
    // Workspace default sits above warp and gets the "Default" badge,
    // matching the Oz webapp's HostSelector layout.
    let items = build_menu_items(Some("my-corp"), None, &[]);
    assert_eq!(items.len(), 3);
    assert_eq!(item_label(&items[0]), "my-corp");
    assert_eq!(item_label(&items[1]), ORCHESTRATION_WARP_WORKER_HOST);
    assert_eq!(item_label(&items[2]), "Custom host\u{2026}");
}

#[test]
fn build_menu_items_adds_recent_after_warp() {
    let items = build_menu_items(None, Some("other-host"), &[]);
    assert_eq!(items.len(), 3);
    assert_eq!(item_label(&items[0]), ORCHESTRATION_WARP_WORKER_HOST);
    // Recent hosts render as plain slugs (no "(Recent)" suffix).
    assert_eq!(item_label(&items[1]), "other-host");
    assert_eq!(item_label(&items[2]), "Custom host\u{2026}");
}

#[test]
fn build_menu_items_dedups_recent_when_it_matches_default_or_warp() {
    // Same as the workspace default → no duplicate "Recent" row.
    let items = build_menu_items(Some("my-corp"), Some("my-corp"), &[]);
    assert_eq!(items.len(), 3);
    assert_eq!(item_label(&items[0]), "my-corp");
    assert_eq!(item_label(&items[1]), ORCHESTRATION_WARP_WORKER_HOST);
    assert_eq!(item_label(&items[2]), "Custom host\u{2026}");

    // Recent == "warp" is also skipped (warp is already a row).
    let items = build_menu_items(Some("my-corp"), Some("warp"), &[]);
    assert_eq!(items.len(), 3, "warp recent should not double-add");
}
#[test]
fn build_menu_items_adds_connected_hosts_before_recent_and_dedups_known_hosts() {
    let connected_hosts = vec![
        "alpha".to_string(),
        "warp".to_string(),
        "my-corp".to_string(),
        "alpha".to_string(),
        "beta".to_string(),
    ];
    let items = build_menu_items(Some("my-corp"), Some("beta"), &connected_hosts);
    assert_eq!(items.len(), 5);
    assert_eq!(item_label(&items[0]), "my-corp");
    assert_eq!(item_label(&items[1]), ORCHESTRATION_WARP_WORKER_HOST);
    assert_eq!(item_label(&items[2]), "alpha");
    assert_eq!(item_label(&items[3]), "beta");
    assert_eq!(item_label(&items[4]), "Custom host\u{2026}");
}

#[test]
fn build_menu_items_warp_entry_dispatches_select_known_warp() {
    let items = build_menu_items(None, None, &[]);
    match item_action(&items[0]) {
        DropdownAction::SelectActionAndClose(action) => {
            let action = action
                .as_any()
                .downcast_ref::<InternalAction>()
                .expect("expected InternalAction");
            match action {
                InternalAction::SelectKnown(slug) => {
                    assert_eq!(slug, ORCHESTRATION_WARP_WORKER_HOST);
                }
                other => panic!("expected SelectKnown, got {other:?}"),
            }
        }
        other => panic!("expected SelectActionAndClose(SelectKnown), got {other:?}"),
    }
}

#[test]
fn build_menu_items_custom_entry_dispatches_enter_custom_mode() {
    let items = build_menu_items(None, None, &[]);
    let custom = items.last().expect("custom entry is always last");
    match item_action(custom) {
        DropdownAction::SelectActionAndClose(action) => {
            assert_eq!(
                action.as_any().downcast_ref::<InternalAction>(),
                Some(&InternalAction::EnterCustomMode)
            );
        }
        other => panic!("expected EnterCustomMode, got {other:?}"),
    }
}

#[test]
fn menu_label_for_picks_default_badge_when_slug_matches_default() {
    let label = menu_label_for("my-corp", Some("my-corp"));
    assert_eq!(label, "my-corp  (Default)");
}

#[test]
fn menu_label_for_returns_plain_slug_for_warp() {
    let label = menu_label_for(ORCHESTRATION_WARP_WORKER_HOST, Some("my-corp"));
    assert_eq!(label, ORCHESTRATION_WARP_WORKER_HOST);
}

#[test]
fn menu_label_for_returns_plain_slug_for_unknown_value() {
    // A slug typed via custom mode that we haven't promoted to "recent" yet.
    let label = menu_label_for("typed-once", Some("my-corp"));
    assert_eq!(label, "typed-once");
}

#[test]
fn normalize_slug_trims_whitespace_and_falls_back_to_warp_when_empty() {
    assert_eq!(normalize_slug("  my-corp  "), "my-corp");
    assert_eq!(normalize_slug(""), ORCHESTRATION_WARP_WORKER_HOST);
    assert_eq!(normalize_slug("   "), ORCHESTRATION_WARP_WORKER_HOST);
}
