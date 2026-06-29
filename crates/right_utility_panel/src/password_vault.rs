//! The [`PasswordVault`]: the only path through which password *values* are
//! created, read, updated, and removed.
//!
//! The vault couples a [`SecretStore`] (platform secure storage) with the
//! non-secret [`PasswordEntryMetadata`] held in [`RightUtilityPanelData`].
//! Invariants it enforces:
//!
//! - A secret value is written to secure storage **before** its metadata is
//!   added, so metadata never points at a missing secret. If the write fails
//!   (e.g. secure storage is unavailable), no metadata is created.
//! - On delete, the secret is removed first; if removal fails the metadata is
//!   left in place and the error is surfaced, so we never pretend a secret was
//!   removed.
//! - Plaintext only ever exists as a transient [`SecretValue`]; it is never
//!   stored in the serializable model.

use chrono::Utc;
use uuid::Uuid;

use crate::model::{PasswordEntryMetadata, RightUtilityPanelData};
use crate::secret::{SecretResult, SecretStore, SecretValue};

/// The secure-storage service namespace under which password secrets are stored.
///
/// Mirrors the reverse-DNS convention used elsewhere in Warp
/// (`dev.warp.Warp.*`).
pub const PASSWORD_SECRET_SERVICE: &str = "dev.warp.Warp.RightUtilityPanel.Passwords";

/// Input describing a new password record to create.
///
/// The [`NewPassword::secret`] is the only secret field; everything else is
/// non-secret metadata.
#[derive(Debug)]
pub struct NewPassword {
    /// User-facing title.
    pub title: String,
    /// Optional username / login.
    pub username: String,
    /// Optional URL.
    pub url: String,
    /// Optional non-secret notes.
    pub notes: String,
    /// Optional tags.
    pub tags: Vec<String>,
    /// The secret value to store in secure storage.
    pub secret: SecretValue,
}

impl NewPassword {
    /// Creates a new-password input with just a title and secret; other
    /// metadata fields default to empty.
    pub fn new(title: impl Into<String>, secret: SecretValue) -> Self {
        Self {
            title: title.into(),
            username: String::new(),
            url: String::new(),
            notes: String::new(),
            tags: Vec::new(),
            secret,
        }
    }
}

/// Couples password metadata with secure storage for secret values.
pub struct PasswordVault<S: SecretStore> {
    store: S,
}

impl<S: SecretStore> PasswordVault<S> {
    /// Creates a vault backed by `store`.
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Returns the backing secret store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Derives the secure-storage key for a record id.
    ///
    /// The key is derived from a stable UUID, never from the user-facing title.
    fn secret_key(id: Uuid) -> String {
        format!("password/{id}")
    }

    /// Creates a password: writes the secret to secure storage, then records
    /// its metadata in `data`.
    ///
    /// If the secret write fails, **no** metadata is added and the error is
    /// returned, so metadata never points at a missing secret.
    pub fn create(
        &self,
        data: &mut RightUtilityPanelData,
        input: NewPassword,
    ) -> SecretResult<Uuid> {
        let id = Uuid::new_v4();
        let key = Self::secret_key(id);

        // Write the secret first; only persist metadata if it succeeds.
        self.store
            .write_value_with_owner_only_fallback(&key, input.secret.expose_secret())?;

        let now = Utc::now();
        data.passwords.push(PasswordEntryMetadata {
            id,
            title: input.title,
            username: input.username,
            url: input.url,
            notes: input.notes,
            tags: input.tags,
            created_at: now,
            updated_at: now,
            secret_storage_key: key,
        });
        Ok(id)
    }

    /// Reveals (reads) the secret value for a record.
    pub fn reveal(&self, metadata: &PasswordEntryMetadata) -> SecretResult<SecretValue> {
        let value = self.store.read_value(&metadata.secret_storage_key)?;
        Ok(SecretValue::new(value))
    }

    /// Updates the secret value for an existing record, bumping its
    /// `updated_at` timestamp.
    pub fn update_secret(
        &self,
        data: &mut RightUtilityPanelData,
        id: Uuid,
        new_secret: SecretValue,
    ) -> SecretResult<()> {
        let metadata = data
            .passwords
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(crate::secret::SecretStoreError::NotFound)?;
        self.store.write_value_with_owner_only_fallback(
            &metadata.secret_storage_key,
            new_secret.expose_secret(),
        )?;
        metadata.updated_at = Utc::now();
        Ok(())
    }

    /// Deletes a password: removes the secret from secure storage first, then
    /// removes its metadata.
    ///
    /// If the secret removal fails, the metadata is left intact and the error
    /// is returned, so we never pretend a secret was removed.
    pub fn delete(
        &self,
        data: &mut RightUtilityPanelData,
        id: Uuid,
    ) -> SecretResult<Option<PasswordEntryMetadata>> {
        let Some(metadata) = data.password(id) else {
            return Ok(None);
        };
        self.store.remove_value(&metadata.secret_storage_key)?;
        Ok(data.remove_password(id))
    }
}

#[cfg(test)]
#[path = "password_vault_tests.rs"]
mod tests;
