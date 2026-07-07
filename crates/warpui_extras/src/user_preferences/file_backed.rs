//! Implementation of the [`UserPreferences`] trait using a file for
//! persistence.

use std::path::{Path, PathBuf};

use super::in_memory::InMemoryPreferences;
use super::Error;

/// An implementation of the [`UserPreferences`] trait using a file for
/// persistence.
///
/// Note that this is currently not robust to external modifications to
/// the backing file (either by another instance of the application or
/// by an end user).  This reads the file once when initialized and keeps
/// an in-memory copy of the preferences, flushing to disk after each
/// update.
pub struct FileBackedUserPreferences {
    /// The path to the file that backs this preferences store.
    file_path: PathBuf,

    /// A backing in-memory preferences store that we can flush to disk upon
    /// modification.
    inner: InMemoryPreferences,
}

impl FileBackedUserPreferences {
    /// Constructs a new file-backed user preferences store.
    ///
    /// If no file exists at the given path, an empty in-memory backing store
    /// will be used, and any modifications will trigger creation of the file
    /// (including any missing parent directories).
    ///
    /// Returns an error if something went wrong while attempting to read the
    /// existing persisted preferences at the given path.
    pub fn new(file_path: PathBuf) -> Result<Self, Error> {
        let inner = Self::initialize_in_memory_preferences(file_path.as_path())?;
        Ok(Self { file_path, inner })
    }

    /// Loads the contents of the file at the given path into an in-memory
    /// preferences store.
    ///
    /// If the file is not found, an empty store is returned.  The file is
    /// not created.
    fn initialize_in_memory_preferences(file_path: &Path) -> Result<InMemoryPreferences, Error> {
        let file_contents = match std::fs::read_to_string(file_path) {
            Ok(file_contents) => file_contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(InMemoryPreferences::default());
            }
            Err(err) => return Err(err.into()),
        };

        // If the file is empty/only whitespace, proceed with the default preferences.
        if file_contents.trim().is_empty() {
            return Ok(InMemoryPreferences::default());
        }

        let prefs = serde_json::from_str(&file_contents).map_err(anyhow::Error::new);
        if let Err(err) = &prefs {
            log::warn!("Failed to deserialize file preferences: {err:#}");
        }

        Ok(prefs?)
    }

    /// Flushes the internal in-memory preferences store to disk.
    fn flush(&self) -> Result<(), Error> {
        let parent_dir = self
            .file_path
            .parent()
            .expect("absolute path to file should have parent");
        std::fs::create_dir_all(parent_dir)?;

        let data = serde_json::to_string_pretty(&self.inner).map_err(anyhow::Error::new)?;
        std::fs::write(&self.file_path, data)?;
        Ok(())
    }
}

impl super::UserPreferences for FileBackedUserPreferences {
    fn write_value(&self, key: &str, value: String) -> Result<(), super::Error> {
        self.inner.write_value(key, value)?;
        self.flush()
    }

    fn read_value(&self, key: &str) -> Result<Option<String>, super::Error> {
        self.inner.read_value(key)
    }

    fn remove_value(&self, key: &str) -> Result<(), super::Error> {
        self.inner.remove_value(key)?;
        self.flush()
    }
}
