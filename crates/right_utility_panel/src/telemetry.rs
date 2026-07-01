//! Telemetry payloads for the right utility panel.
//!
//! These events carry **only** non-sensitive, aggregate data: enum
//! discriminants and counts. They never include password titles, usernames,
//! URLs, notes, secret values, bookmark paths/commands/URLs, list names, or
//! item text. The model deliberately makes it impossible to attach such data:
//! every event variant is payload-free or carries only another enum.

use crate::model::{BookmarkTargetKind, BookmarksSubview, RightUtilityModule};

/// A non-sensitive analytics event emitted by the utility panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightUtilityPanelEvent {
    /// The panel was opened.
    PanelOpened,
    /// The panel was closed.
    PanelClosed,
    /// A top-level module was selected.
    ModuleSelected(RightUtilityModule),
    /// A Bookmarks sub-view was selected.
    BookmarksSubviewSelected(BookmarksSubview),
    /// A bookmark was created (only the target kind is recorded).
    BookmarkCreated(BookmarkTargetKind),
    /// A bookmark was activated (only the target kind is recorded).
    BookmarkActivated(BookmarkTargetKind),
    /// A custom list was created.
    CustomListCreated,
    /// Password metadata was created.
    PasswordMetadataCreated,
    /// Password metadata was deleted.
    PasswordMetadataDeleted,
}

impl RightUtilityPanelEvent {
    /// Returns the stable event name used by the analytics backend.
    pub fn event_name(&self) -> &'static str {
        match self {
            RightUtilityPanelEvent::PanelOpened => "right_utility_panel_opened",
            RightUtilityPanelEvent::PanelClosed => "right_utility_panel_closed",
            RightUtilityPanelEvent::ModuleSelected(_) => "right_utility_panel_module_selected",
            RightUtilityPanelEvent::BookmarksSubviewSelected(_) => {
                "right_utility_panel_bookmarks_subview_selected"
            }
            RightUtilityPanelEvent::BookmarkCreated(_) => "right_utility_panel_bookmark_created",
            RightUtilityPanelEvent::BookmarkActivated(_) => {
                "right_utility_panel_bookmark_activated"
            }
            RightUtilityPanelEvent::CustomListCreated => "right_utility_panel_custom_list_created",
            RightUtilityPanelEvent::PasswordMetadataCreated => {
                "right_utility_panel_password_metadata_created"
            }
            RightUtilityPanelEvent::PasswordMetadataDeleted => {
                "right_utility_panel_password_metadata_deleted"
            }
        }
    }

    /// Returns the single non-sensitive property for this event, if any.
    ///
    /// The value is always a fixed enum discriminant string, never user data.
    pub fn property(&self) -> Option<(&'static str, &'static str)> {
        match self {
            RightUtilityPanelEvent::ModuleSelected(module) => {
                Some(("module", module_name(*module)))
            }
            RightUtilityPanelEvent::BookmarksSubviewSelected(subview) => {
                Some(("subview", subview_name(*subview)))
            }
            RightUtilityPanelEvent::BookmarkCreated(kind)
            | RightUtilityPanelEvent::BookmarkActivated(kind) => {
                Some(("target_kind", target_kind_name(*kind)))
            }
            RightUtilityPanelEvent::PanelOpened
            | RightUtilityPanelEvent::PanelClosed
            | RightUtilityPanelEvent::CustomListCreated
            | RightUtilityPanelEvent::PasswordMetadataCreated
            | RightUtilityPanelEvent::PasswordMetadataDeleted => None,
        }
    }
}

fn module_name(module: RightUtilityModule) -> &'static str {
    match module {
        RightUtilityModule::Passwords => "passwords",
        RightUtilityModule::Bookmarks => "bookmarks",
    }
}

fn subview_name(subview: BookmarksSubview) -> &'static str {
    match subview {
        BookmarksSubview::Bookmarks => "bookmarks",
        BookmarksSubview::CustomLists => "custom_lists",
    }
}

fn target_kind_name(kind: BookmarkTargetKind) -> &'static str {
    match kind {
        BookmarkTargetKind::Command => "command",
        BookmarkTargetKind::Directory => "directory",
        BookmarkTargetKind::File => "file",
        BookmarkTargetKind::Url => "url",
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
