//! Storage for user preferences.

pub mod file_backed;
pub mod in_memory;
#[cfg(target_family = "wasm")]
pub mod local_storage;
#[cfg(target_os = "windows")]
pub mod registry_backed;
#[cfg(feature = "user_preferences-toml")]
pub mod toml_backed;
#[cfg(target_os = "macos")]
pub mod user_defaults;

/// A type alias for a boxed user preferences backend.
pub type Model = Box<dyn UserPreferences>;

/// A trait representing storage for user preferences.
pub trait UserPreferences {
    /// Writes a value at the given key.
    fn write_value(&self, key: &str, value: String) -> Result<(), Error>;

    /// Reads the value stored at the given key.
    ///
    /// Returns Ok(None) if no value was found.
    fn read_value(&self, key: &str) -> Result<Option<String>, Error>;

    /// Removes the value stored at the given key, if any.
    fn remove_value(&self, key: &str) -> Result<(), Error>;

    /// Writes a value at the given key, with optional hierarchy context.
    ///
    /// Hierarchy-aware backends (like TOML) use the hierarchy to place the
    /// value in the correct section. The default implementation ignores
    /// the hierarchy and delegates to [`write_value`](Self::write_value).
    ///
    /// `max_table_depth` controls how deeply nested objects are rendered as
    /// section tables before switching to inline tables:
    /// - `None` — unlimited depth (all section tables)
    /// - `Some(0)` — fully inline (`key = { ... }`)
    /// - `Some(n)` — `n` levels of section tables, then inline
    fn write_value_with_hierarchy(
        &self,
        key: &str,
        value: String,
        hierarchy: Option<&str>,
        max_table_depth: Option<u32>,
    ) -> Result<(), Error> {
        let _ = (hierarchy, max_table_depth);
        self.write_value(key, value)
    }

    /// Reads the value stored at the given key, with optional hierarchy context.
    ///
    /// The default implementation ignores the hierarchy and delegates to
    /// [`read_value`](Self::read_value).
    fn read_value_with_hierarchy(
        &self,
        key: &str,
        hierarchy: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let _ = hierarchy;
        self.read_value(key)
    }

    /// Removes the value stored at the given key, with optional hierarchy context.
    ///
    /// The default implementation ignores the hierarchy and delegates to
    /// [`remove_value`](Self::remove_value).
    fn remove_value_with_hierarchy(&self, key: &str, hierarchy: Option<&str>) -> Result<(), Error> {
        let _ = hierarchy;
        self.remove_value(key)
    }

    /// Returns whether this backend is the user-visible settings file.
    ///
    /// When true, settings that define custom file serialization (via
    /// `file_serialize` / `file_deserialize`) will use their custom format
    /// instead of the standard serde representation. This produces a more
    /// human-readable settings file.
    ///
    /// Other backends (NSUserDefaults, in-memory, etc.) return `false` and
    /// always use the standard serde format.
    fn is_settings_file(&self) -> bool {
        false
    }

    /// Reloads the backing store from disk.
    ///
    /// File-backed backends re-read their file and replace the in-memory
    /// contents. Non-file backends do nothing. On parse failure the
    /// implementation should keep the previous state and return an error.
    fn reload_from_disk(&self) -> Result<(), Error> {
        Ok(())
    }

    /// Marks a key as write-inhibited so that subsequent writes and removes
    /// for this key are silently skipped.
    ///
    /// This is used to protect individual setting values that exist in the
    /// backing store but could not be deserialized into the expected type.
    /// The user's broken-but-fixable value is preserved until they correct
    /// it in the file.
    ///
    /// The default implementation is a no-op (non-file backends don't need
    /// per-key inhibition).
    fn inhibit_writes_for_key(&self, key: &str, hierarchy: Option<&str>) {
        let _ = (key, hierarchy);
    }

    /// Clears all per-key write inhibitions.
    ///
    /// Called after a successful reload from disk so that inhibitions can
    /// be re-derived from the freshly loaded values.
    fn clear_all_write_inhibitions(&self) {}
}

/// Enumerates the various errors that can occur when interacting with user
/// preferences.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to decode the stored bytes into a UTF-8 string.
    #[error("failed to decode UTF-8 string from bytes")]
    DecodeError(#[from] std::str::Utf8Error),

    /// Generic I/O error.
    #[error("i/o error")]
    IoError(#[from] std::io::Error),

    /// Catch-all for unclassifiable errors.
    #[error("unknown error")]
    Unknown(#[from] anyhow::Error),
}
