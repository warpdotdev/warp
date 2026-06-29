//! Secret value handling and the [`SecretStore`] abstraction.
//!
//! Password *values* never live in the serializable metadata model. They are
//! read from and written to a [`SecretStore`], which mirrors the shape of
//! `warpui_extras::secure_storage::SecureStorage` so the platform keychain can
//! be adapted directly in the app layer while this crate stays
//! dependency-light and deterministically testable.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

/// Errors that can occur when interacting with secure storage.
///
/// This mirrors the meaningful cases of
/// `warpui_extras::secure_storage::Error` without depending on the platform
/// keychain crates.
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    /// No secret was found for the given key.
    #[error("secret not found")]
    NotFound,

    /// Secure storage is not available on this platform / environment, so
    /// password values cannot be created, revealed, or copied.
    #[error("secure storage is unavailable")]
    Unavailable,

    /// The underlying secure-storage backend returned an error.
    #[error("secure storage backend error: {0}")]
    Backend(String),
}

/// Convenience result alias for secret-store operations.
pub type SecretResult<T> = Result<T, SecretStoreError>;

/// A plaintext secret value held only transiently in memory.
///
/// [`SecretValue`] intentionally does **not** implement [`serde::Serialize`],
/// [`serde::Deserialize`], or [`std::fmt::Display`], and its [`fmt::Debug`]
/// implementation is redacted, so a secret can never accidentally end up in
/// serialized settings/snapshots, logs, telemetry, or test failure messages.
/// Use [`SecretValue::expose_secret`] only at the point where the plaintext is
/// actually needed (writing to storage, copying to clipboard, revealing in the
/// UI).
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wraps a plaintext value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the plaintext value. Call sites should keep the returned
    /// reference as short-lived as possible.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Returns whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(***redacted***)")
    }
}

/// Best-effort scrub of the plaintext when the value is dropped.
impl Drop for SecretValue {
    fn drop(&mut self) {
        // Overwrite the backing bytes so the plaintext does not linger in the
        // freed allocation. This is best-effort (the compiler/allocator may
        // still move the bytes), but it removes the most obvious lingering copy.
        let bytes = unsafe { self.0.as_bytes_mut() };
        bytes.iter_mut().for_each(|b| *b = 0);
    }
}

/// Abstraction over a platform secure key/value store.
///
/// This mirrors `warpui_extras::secure_storage::SecureStorage`
/// (`write_value` / `read_value` / `remove_value` /
/// `write_value_with_owner_only_fallback`) so the real platform backend can be
/// adapted with a trivial forwarding impl in the app layer.
pub trait SecretStore {
    /// Writes `value` at `key`.
    fn write_value(&self, key: &str, value: &str) -> SecretResult<()>;

    /// Writes `value` at `key`, requiring any file fallback to be owner-only.
    ///
    /// Backends without a file fallback use their normal write path. Password
    /// writes should prefer this method so that, on platforms like Linux that
    /// may fall back to a file, the fallback is created with owner-only
    /// permissions.
    fn write_value_with_owner_only_fallback(&self, key: &str, value: &str) -> SecretResult<()> {
        self.write_value(key, value)
    }

    /// Reads the value stored at `key`, returning [`SecretStoreError::NotFound`]
    /// if there is none.
    fn read_value(&self, key: &str) -> SecretResult<String>;

    /// Removes the value stored at `key`. Removing a missing key succeeds.
    fn remove_value(&self, key: &str) -> SecretResult<()>;
}

/// An in-memory [`SecretStore`] for tests and headless contexts.
#[derive(Default)]
pub struct InMemorySecretStore {
    inner: Mutex<HashMap<String, String>>,
}

impl InMemorySecretStore {
    /// Creates an empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of stored secrets (useful for assertions in tests).
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("secret store mutex poisoned")
            .len()
    }

    /// Returns whether the store holds no secrets.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns whether a value is stored at `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.inner
            .lock()
            .expect("secret store mutex poisoned")
            .contains_key(key)
    }
}

impl SecretStore for InMemorySecretStore {
    fn write_value(&self, key: &str, value: &str) -> SecretResult<()> {
        self.inner
            .lock()
            .expect("secret store mutex poisoned")
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn read_value(&self, key: &str) -> SecretResult<String> {
        self.inner
            .lock()
            .expect("secret store mutex poisoned")
            .get(key)
            .cloned()
            .ok_or(SecretStoreError::NotFound)
    }

    fn remove_value(&self, key: &str) -> SecretResult<()> {
        self.inner
            .lock()
            .expect("secret store mutex poisoned")
            .remove(key);
        Ok(())
    }
}

/// A [`SecretStore`] that reports secure storage as unavailable.
///
/// Reads report [`SecretStoreError::NotFound`] and writes report
/// [`SecretStoreError::Unavailable`], modelling a platform/environment where
/// password values cannot be stored or retrieved. The panel uses this to show
/// an explicit disabled/error state instead of silently dropping secrets.
pub struct UnavailableSecretStore;

impl SecretStore for UnavailableSecretStore {
    fn write_value(&self, _key: &str, _value: &str) -> SecretResult<()> {
        Err(SecretStoreError::Unavailable)
    }

    fn read_value(&self, _key: &str) -> SecretResult<String> {
        Err(SecretStoreError::NotFound)
    }

    fn remove_value(&self, _key: &str) -> SecretResult<()> {
        // Removing from an unavailable store is a no-op success; there is
        // nothing persisted to remove.
        Ok(())
    }
}

#[cfg(test)]
#[path = "secret_tests.rs"]
mod tests;
