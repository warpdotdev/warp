//! Non-secret, serializable data model for the right utility panel.
//!
//! Everything in this module is safe to persist in Warp's settings/snapshot
//! layer (it is local-only user data, never synced) and contains **no** secret
//! values. Password secrets live exclusively in secure storage via
//! [`crate::PasswordVault`]; here we only keep the metadata that points at a
//! secure-storage key.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which content the shared right panel is currently showing.
///
/// The existing Code Review panel and the new utility panel are mutually
/// exclusive right-panel modes. Older persisted snapshots that predate this
/// enum default to [`RightPanelMode::CodeReview`] so existing windows restore
/// unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightPanelMode {
    /// The existing Code Review right panel.
    #[default]
    CodeReview,
    /// The new fixed utility panel (Passwords / Bookmarks).
    Utility,
}

/// The two first-class, top-level modules of the utility panel.
///
/// Custom Lists is intentionally **not** a top-level module; it is a nested
/// sub-view of [`RightUtilityModule::Bookmarks`] (see [`BookmarksSubview`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightUtilityModule {
    /// Local password / credential records.
    #[default]
    Passwords,
    /// Bookmarks and their nested Custom Lists.
    Bookmarks,
}

/// The nested sub-views within the Bookmarks module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarksSubview {
    /// The saved bookmarks list.
    #[default]
    Bookmarks,
    /// Named custom lists of checkable text items.
    CustomLists,
}

/// Restorable navigation state for the utility panel.
///
/// This is the small, serializable slice that belongs alongside the existing
/// right-panel snapshot: which top-level module is selected, which Bookmarks
/// sub-view is active, and which custom list (if any) is open.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RightUtilityPanelState {
    /// The selected top-level module.
    #[serde(default)]
    pub selected_module: RightUtilityModule,
    /// The active sub-view within the Bookmarks module.
    #[serde(default)]
    pub bookmarks_subview: BookmarksSubview,
    /// The custom list currently open within the Custom Lists sub-view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_custom_list_id: Option<Uuid>,
}

/// Metadata for a single password / credential record.
///
/// This struct is persisted with the rest of the (non-secret) panel data. It
/// deliberately has **no** field for the secret value; the value is stored in
/// secure storage under [`PasswordEntryMetadata::secret_storage_key`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordEntryMetadata {
    /// Stable identifier for the record.
    pub id: Uuid,
    /// User-facing title.
    pub title: String,
    /// Optional associated username / login.
    #[serde(default)]
    pub username: String,
    /// Optional associated URL.
    #[serde(default)]
    pub url: String,
    /// Optional free-form notes (non-secret).
    #[serde(default)]
    pub notes: String,
    /// Optional tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// The secure-storage key under which the secret value is stored.
    pub secret_storage_key: String,
}

/// The concrete target a bookmark points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BookmarkTarget {
    /// A shell command to insert into the active terminal input.
    Command {
        /// The command text.
        command: String,
        /// Optional working directory the command was captured in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// A directory path.
    Directory {
        /// The directory path.
        path: String,
    },
    /// A file path.
    File {
        /// The file path.
        path: String,
    },
    /// A URL.
    Url {
        /// The URL.
        url: String,
    },
}

/// A lightweight, payload-free discriminant for a [`BookmarkTarget`].
///
/// Used for telemetry and filtering so we never emit the target payload (which
/// may contain a path, command, or URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkTargetKind {
    /// [`BookmarkTarget::Command`].
    Command,
    /// [`BookmarkTarget::Directory`].
    Directory,
    /// [`BookmarkTarget::File`].
    File,
    /// [`BookmarkTarget::Url`].
    Url,
}

/// An error returned when a [`BookmarkTarget`] fails validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetValidationError {
    /// A command bookmark had an empty command.
    #[error("command must not be empty")]
    EmptyCommand,
    /// A directory or file bookmark had an empty path.
    #[error("path must not be empty")]
    EmptyPath,
    /// A URL bookmark had an empty URL.
    #[error("url must not be empty")]
    EmptyUrl,
    /// A URL bookmark had a malformed URL.
    #[error("url is not valid")]
    InvalidUrl,
}

impl BookmarkTarget {
    /// Returns the payload-free [`BookmarkTargetKind`] for this target.
    pub fn kind(&self) -> BookmarkTargetKind {
        match self {
            BookmarkTarget::Command { .. } => BookmarkTargetKind::Command,
            BookmarkTarget::Directory { .. } => BookmarkTargetKind::Directory,
            BookmarkTarget::File { .. } => BookmarkTargetKind::File,
            BookmarkTarget::Url { .. } => BookmarkTargetKind::Url,
        }
    }

    /// Validates the target's payload.
    ///
    /// Commands and paths must be non-empty; URLs must be non-empty and have a
    /// `scheme://host` shape. This is deliberately a light structural check, not
    /// a full URL parse, to avoid pulling a URL-parsing dependency into the
    /// model.
    pub fn validate(&self) -> Result<(), TargetValidationError> {
        match self {
            BookmarkTarget::Command { command, .. } => {
                if command.trim().is_empty() {
                    Err(TargetValidationError::EmptyCommand)
                } else {
                    Ok(())
                }
            }
            BookmarkTarget::Directory { path } | BookmarkTarget::File { path } => {
                if path.trim().is_empty() {
                    Err(TargetValidationError::EmptyPath)
                } else {
                    Ok(())
                }
            }
            BookmarkTarget::Url { url } => validate_url(url),
        }
    }
}

/// Light structural validation for a bookmark URL.
fn validate_url(url: &str) -> Result<(), TargetValidationError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(TargetValidationError::EmptyUrl);
    }
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err(TargetValidationError::InvalidUrl);
    };
    let scheme_ok = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
    if scheme_ok && !rest.is_empty() {
        Ok(())
    } else {
        Err(TargetValidationError::InvalidUrl)
    }
}

/// A single bookmark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    /// Stable identifier.
    pub id: Uuid,
    /// User-facing title.
    pub title: String,
    /// What the bookmark points at.
    pub target: BookmarkTarget,
    /// Optional tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Manual sort order (ascending).
    #[serde(default)]
    pub sort_order: i64,
}

/// A single checkable item within a [`CustomList`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomListItem {
    /// Stable identifier.
    pub id: Uuid,
    /// The item text. This is treated as **non-secret** data.
    pub text: String,
    /// Whether the item is checked.
    #[serde(default)]
    pub checked: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Manual sort order (ascending).
    #[serde(default)]
    pub sort_order: i64,
}

/// A named list of checkable text items, nested under Bookmarks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomList {
    /// Stable identifier.
    pub id: Uuid,
    /// User-facing title.
    pub title: String,
    /// The list's items.
    #[serde(default)]
    pub items: Vec<CustomListItem>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Manual sort order (ascending).
    #[serde(default)]
    pub sort_order: i64,
}

impl CustomList {
    /// Creates a new, empty list with the given title.
    pub fn new(title: impl Into<String>, sort_order: i64) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            items: Vec::new(),
            created_at: now,
            updated_at: now,
            sort_order,
        }
    }

    /// Adds a new item with the given text and returns its id.
    pub fn add_item(&mut self, text: impl Into<String>) -> Uuid {
        let now = Utc::now();
        let item = CustomListItem {
            id: Uuid::new_v4(),
            text: text.into(),
            checked: false,
            created_at: now,
            updated_at: now,
            sort_order: self.items.len() as i64,
        };
        let id = item.id;
        self.items.push(item);
        self.updated_at = now;
        id
    }

    /// Toggles the checked state of the item with `id`, returning the new
    /// state, or `None` if no such item exists.
    pub fn toggle_item(&mut self, id: Uuid) -> Option<bool> {
        let item = self.items.iter_mut().find(|item| item.id == id)?;
        item.checked = !item.checked;
        item.updated_at = Utc::now();
        self.updated_at = item.updated_at;
        Some(item.checked)
    }

    /// Removes the item with `id`, returning it if present.
    pub fn remove_item(&mut self, id: Uuid) -> Option<CustomListItem> {
        let index = self.items.iter().position(|item| item.id == id)?;
        self.updated_at = Utc::now();
        Some(self.items.remove(index))
    }
}

/// The full, non-secret data model for the utility panel.
///
/// This is the unit persisted locally (never synced to the cloud). It holds
/// password *metadata* (not values), bookmarks, and custom lists.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RightUtilityPanelData {
    /// Password record metadata.
    #[serde(default)]
    pub passwords: Vec<PasswordEntryMetadata>,
    /// Bookmarks.
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    /// Custom lists (nested under the Bookmarks module).
    #[serde(default)]
    pub custom_lists: Vec<CustomList>,
}

impl RightUtilityPanelData {
    /// Creates an empty data set.
    pub fn new() -> Self {
        Self::default()
    }

    // --- Passwords (metadata only; values go through PasswordVault) ---------

    /// Looks up password metadata by id.
    pub fn password(&self, id: Uuid) -> Option<&PasswordEntryMetadata> {
        self.passwords.iter().find(|entry| entry.id == id)
    }

    /// Removes password metadata by id, returning it if present.
    ///
    /// This only mutates metadata; deleting the secret value is the
    /// responsibility of [`crate::PasswordVault`].
    pub fn remove_password(&mut self, id: Uuid) -> Option<PasswordEntryMetadata> {
        let index = self.passwords.iter().position(|entry| entry.id == id)?;
        Some(self.passwords.remove(index))
    }

    // --- Bookmarks ---------------------------------------------------------

    /// Adds a bookmark after validating its target, returning its id.
    pub fn add_bookmark(
        &mut self,
        title: impl Into<String>,
        target: BookmarkTarget,
        tags: Vec<String>,
    ) -> Result<Uuid, TargetValidationError> {
        target.validate()?;
        let now = Utc::now();
        let bookmark = Bookmark {
            id: Uuid::new_v4(),
            title: title.into(),
            target,
            tags,
            created_at: now,
            updated_at: now,
            sort_order: self.bookmarks.len() as i64,
        };
        let id = bookmark.id;
        self.bookmarks.push(bookmark);
        Ok(id)
    }

    /// Removes a bookmark by id, returning it if present.
    pub fn remove_bookmark(&mut self, id: Uuid) -> Option<Bookmark> {
        let index = self.bookmarks.iter().position(|b| b.id == id)?;
        Some(self.bookmarks.remove(index))
    }

    /// Returns bookmarks whose title or tags contain `query` (case-insensitive).
    pub fn search_bookmarks(&self, query: &str) -> Vec<&Bookmark> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return self.bookmarks.iter().collect();
        }
        self.bookmarks
            .iter()
            .filter(|b| {
                b.title.to_lowercase().contains(&needle)
                    || b.tags.iter().any(|t| t.to_lowercase().contains(&needle))
            })
            .collect()
    }

    // --- Custom lists ------------------------------------------------------

    /// Creates a new custom list and returns its id.
    pub fn add_custom_list(&mut self, title: impl Into<String>) -> Uuid {
        let list = CustomList::new(title, self.custom_lists.len() as i64);
        let id = list.id;
        self.custom_lists.push(list);
        id
    }

    /// Returns a mutable reference to the custom list with `id`.
    pub fn custom_list_mut(&mut self, id: Uuid) -> Option<&mut CustomList> {
        self.custom_lists.iter_mut().find(|list| list.id == id)
    }

    /// Renames the custom list with `id`, returning whether it existed.
    pub fn rename_custom_list(&mut self, id: Uuid, title: impl Into<String>) -> bool {
        match self.custom_list_mut(id) {
            Some(list) => {
                list.title = title.into();
                list.updated_at = Utc::now();
                true
            }
            None => false,
        }
    }

    /// Removes the custom list with `id`, returning it if present.
    pub fn remove_custom_list(&mut self, id: Uuid) -> Option<CustomList> {
        let index = self.custom_lists.iter().position(|list| list.id == id)?;
        Some(self.custom_lists.remove(index))
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
