//! Plain data types describing a resolved file diff.
//!
//! These live outside the GUI `code_diff_view` module so that the shared,
//! surface-agnostic executor and persistence models can name them without
//! depending on any GUI view.
use ai::diff_validation::DiffType;
use warp_core::HostId;

/// The base content and file path for a diff.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DiffBase {
    /// The original file content before the diff is applied.
    /// Empty for new file creation.
    pub content: String,
    /// The absolute file path.
    pub file_path: String,
}

/// User-visible file diff with the original contents of the file
/// and the changes to those contents.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct FileDiff {
    pub base: DiffBase,
    pub diff_type: DiffType,
}

impl FileDiff {
    /// Creates a `FileDiff` from base content, an absolute path, and the diff to apply.
    pub fn new(content: String, file_path: String, diff_type: DiffType) -> FileDiff {
        FileDiff {
            base: DiffBase { content, file_path },
            diff_type,
        }
    }

    /// Returns the absolute path this diff targets.
    pub fn file_path(&self) -> String {
        self.base.file_path.clone()
    }
}

/// Whether a code diff targets the local filesystem or a remote host.
#[derive(Clone, Debug)]
pub enum DiffSessionType {
    Local,
    Remote(HostId),
}
