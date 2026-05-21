use std::ops::Range;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A line-based file fragment location.
///
/// Represents a specific portion of a file by its path and line range.
/// Used for passing precise code context fragments between components.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileFragmentLocation {
    /// The absolute path to the file
    pub path: PathBuf,
    /// The line range (inclusive start, inclusive end)
    pub line_ranges: Vec<Range<usize>>,
}

/// Combined representation of a file context, which can be either
/// a whole file or a specific fragment of a file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CodeContextLocation {
    /// Represent an entire file (used in outline-based context)
    WholeFile(PathBuf),
    /// Represent a specific fragment of a file (used with FullSourceCodeEmbedding)
    Fragment(FileFragmentLocation),
}

impl CodeContextLocation {
    /// Get the file path regardless of whether this is a whole file or fragment
    pub fn path(&self) -> &PathBuf {
        match self {
            CodeContextLocation::WholeFile(path) => path,
            CodeContextLocation::Fragment(fragment) => &fragment.path,
        }
    }
}
