//! Data model and secure-storage-backed vault for Warp's fixed right utility
//! panel.
//!
//! This crate is the dependency-light foundation for the feature-flagged right
//! utility panel described in QUALITY-906. It owns:
//!
//! - The panel's mode/navigation state ([`RightPanelMode`],
//!   [`RightUtilityModule`], [`BookmarksSubview`]).
//! - The non-secret, serializable metadata model ([`RightUtilityPanelData`],
//!   [`PasswordEntryMetadata`], [`Bookmark`], [`CustomList`], ...).
//! - A [`SecretStore`] abstraction plus a [`PasswordVault`] that keeps password
//!   *values* exclusively inside platform secure storage, never in the
//!   serializable metadata, logs, or telemetry.
//! - Telemetry payloads ([`telemetry`]) that carry only non-sensitive,
//!   aggregate data.
//!
//! The crate is deliberately free of UI and platform dependencies so it can be
//! unit-tested deterministically. The Warp client wires it into the workspace
//! right-panel system and adapts [`SecretStore`] to
//! `warpui_extras::secure_storage` in the UI layer.

mod model;
mod password_vault;
mod secret;
pub mod telemetry;

pub use model::{
    Bookmark, BookmarkTarget, BookmarkTargetKind, BookmarksSubview, CustomList, CustomListItem,
    PasswordEntryMetadata, RightPanelMode, RightUtilityModule, RightUtilityPanelData,
    RightUtilityPanelState, TargetValidationError,
};
pub use password_vault::{NewPassword, PasswordVault, PASSWORD_SECRET_SERVICE};
pub use secret::{
    InMemorySecretStore, SecretResult, SecretStore, SecretStoreError, SecretValue,
    UnavailableSecretStore,
};
