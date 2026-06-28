//! This crate defines the low-level, model-agnostic cloud object substrate shared by Warp crates.
//!
//! It owns server-facing identifiers, user identifiers, object metadata, object type and format
//! definitions, and sharing or drive primitives that do not depend on concrete Warp object models.
//!
//! It should remain independent of model-specific payloads, SQLite persistence, app runtime state,
//! and UI rendering concerns.

pub mod auth;
pub mod cloud_object;
pub mod drive;
pub mod ids;

pub use auth::UserUid;
