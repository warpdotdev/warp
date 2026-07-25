use anyhow::anyhow;
use warp_errors::{AnyhowErrorExt, ErrorExt};

use super::{Error, InconsistentStateError};
use crate::index::BuildTreeError;

#[test]
fn codebase_index_errors_preserve_per_variant_actionability() {
    let environmental = Error::BuildTreeError(BuildTreeError::ExceededMaxFileLimit);
    assert!(!environmental.is_actionable());
    assert!(!anyhow::Error::new(environmental).is_actionable());
    assert!(!Error::Io(std::io::Error::other("permission denied")).is_actionable());
    assert!(!Error::NotAGitRepository.is_actionable());
    assert!(!Error::UnsupportedPlatform.is_actionable());
    assert!(!Error::FileSizeExceeded.is_actionable());
    assert!(!Error::FileSystemStateChanged.is_actionable());

    assert!(Error::FailedToGenerateEmbeddings(Vec::new()).is_actionable());
    assert!(Error::InconsistentState(InconsistentStateError::NodeIndexNotFound).is_actionable());
    assert!(Error::Other(anyhow!("unexpected indexing failure")).is_actionable());
}
