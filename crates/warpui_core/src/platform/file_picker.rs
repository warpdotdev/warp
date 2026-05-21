use std::{fmt, path::PathBuf, sync::Arc};

#[derive(Debug, Clone, thiserror::Error)]
pub enum FilePickerError {
    #[error("Failed to spawn file picker thread: {0}")]
    ThreadSpawnFailed(Arc<std::io::Error>),

    #[error("File dialog failed: {0}")]
    DialogFailed(String),
}

// Define complex type here for the file picker callback.
pub type FilePickerCallback =
    Box<dyn FnOnce(Result<Vec<String>, FilePickerError>, &mut crate::AppContext) + Send + Sync>;

// Define callback type for save file picker - returns single path or None if cancelled
pub type SaveFilePickerCallback =
    Box<dyn FnOnce(Option<String>, &mut crate::AppContext) + Send + Sync>;

pub enum FileType {
    Image,
    Yaml,
    Markdown,
}

impl FileType {
    /// List of supported file extensions for this file type.
    pub fn extensions(&self) -> &[&str] {
        match self {
            FileType::Image => &["png", "jpg", "jpeg"],
            FileType::Yaml => &["yaml", "yml"],
            FileType::Markdown => &["md", "markdown"],
        }
    }

    /// Human-readable name for this general category of files.
    pub fn display_name(&self) -> &str {
        match self {
            FileType::Image => "Image",
            FileType::Yaml => "Yaml",
            FileType::Markdown => "Markdown",
        }
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Configuration for the file picker.
///
/// Not all configurations are supported on all platforms:
/// * Linux can only show a single-file picker, a multi-file picker, or a single-directory picker.
///   If choosing a folder is allowed ([`Self::allow_folder`] or [`Self::folders_only`]), a
///   single-directory picker is shown, regardless of the other settings.
/// * macOS supports any combination of allowing files, allowing folders, and allowing
///   multi-select.
pub struct FilePickerConfiguration {
    allows_files: bool,
    allows_folder: bool,
    file_types: Vec<FileType>,
    can_multi_select: bool,
}

impl FilePickerConfiguration {
    pub fn new() -> Self {
        Self {
            allows_files: true,
            allows_folder: false,
            file_types: Default::default(),
            can_multi_select: false,
        }
    }

    /// Configure the file picker to allow choosing folders, in addition to files.
    pub fn allow_folder(mut self) -> Self {
        self.allows_folder = true;
        self
    }

    /// Configure the file picker to *only* allow choosing folders.
    pub fn folders_only(mut self) -> Self {
        self.allows_folder = true;
        self.allows_files = false;
        self
    }

    /// Configure the file picker to allow selecting multiple files.
    pub fn allow_multi_select(mut self) -> Self {
        self.can_multi_select = true;
        self
    }

    pub fn set_allowed_file_types(mut self, file_types: Vec<FileType>) -> Self {
        self.file_types = file_types;
        self
    }

    pub fn allows_files(&self) -> bool {
        self.allows_files
    }

    // TODO(CORE-2324): open file picker on Windows

    pub fn allows_folder(&self) -> bool {
        self.allows_folder
    }

    pub fn allows_multi_select(&self) -> bool {
        self.can_multi_select
    }

    pub fn file_types(&self) -> &Vec<FileType> {
        &self.file_types
    }
}

impl Default for FilePickerConfiguration {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
pub struct SaveFilePickerConfiguration {
    /// Pre-fill the editable filename editor with this.
    pub default_filename: Option<String>,
    /// Open the picker into this directory location to start.
    pub default_directory: Option<PathBuf>,
}

impl SaveFilePickerConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default_filename(mut self, filename: String) -> Self {
        self.default_filename = Some(filename);
        self
    }

    pub fn with_default_directory(mut self, directory: PathBuf) -> Self {
        self.default_directory = Some(directory);
        self
    }
}

#[cfg(test)]
#[path = "file_picker_tests.rs"]
mod tests;
