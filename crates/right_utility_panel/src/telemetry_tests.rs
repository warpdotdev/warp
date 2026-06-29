use super::*;
use crate::model::{BookmarkTargetKind, BookmarksSubview, RightUtilityModule};

#[test]
fn event_names_are_stable() {
    assert_eq!(
        RightUtilityPanelEvent::PanelOpened.event_name(),
        "right_utility_panel_opened"
    );
    assert_eq!(
        RightUtilityPanelEvent::ModuleSelected(RightUtilityModule::Passwords).event_name(),
        "right_utility_panel_module_selected"
    );
    assert_eq!(
        RightUtilityPanelEvent::PasswordMetadataDeleted.event_name(),
        "right_utility_panel_password_metadata_deleted"
    );
}

#[test]
fn properties_are_fixed_discriminants_only() {
    let allowed_values = [
        "passwords",
        "bookmarks",
        "custom_lists",
        "command",
        "directory",
        "file",
        "url",
    ];

    let events = [
        RightUtilityPanelEvent::PanelOpened,
        RightUtilityPanelEvent::PanelClosed,
        RightUtilityPanelEvent::ModuleSelected(RightUtilityModule::Passwords),
        RightUtilityPanelEvent::ModuleSelected(RightUtilityModule::Bookmarks),
        RightUtilityPanelEvent::BookmarksSubviewSelected(BookmarksSubview::Bookmarks),
        RightUtilityPanelEvent::BookmarksSubviewSelected(BookmarksSubview::CustomLists),
        RightUtilityPanelEvent::BookmarkCreated(BookmarkTargetKind::Command),
        RightUtilityPanelEvent::BookmarkActivated(BookmarkTargetKind::Url),
        RightUtilityPanelEvent::CustomListCreated,
        RightUtilityPanelEvent::PasswordMetadataCreated,
        RightUtilityPanelEvent::PasswordMetadataDeleted,
    ];

    for event in events {
        if let Some((key, value)) = event.property() {
            assert!(
                ["module", "subview", "target_kind"].contains(&key),
                "unexpected property key {key}"
            );
            assert!(
                allowed_values.contains(&value),
                "property value {value} is not a fixed discriminant"
            );
        }
    }
}

#[test]
fn payload_free_events_have_no_property() {
    assert!(RightUtilityPanelEvent::PanelOpened.property().is_none());
    assert!(RightUtilityPanelEvent::PanelClosed.property().is_none());
    assert!(RightUtilityPanelEvent::CustomListCreated
        .property()
        .is_none());
    assert!(RightUtilityPanelEvent::PasswordMetadataCreated
        .property()
        .is_none());
}
