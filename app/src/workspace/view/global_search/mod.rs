use warp_ripgrep::search::Submatch;
use warp_util::local_or_remote_path::LocalOrRemotePath;

pub struct SearchConfig {
    pub use_regex: bool,
    pub use_case_sensitivity: bool,
}

/// A single global search match: one line in one file, which may live on
/// the local filesystem or on a remote host.
#[derive(Clone, Debug)]
pub struct GlobalSearchMatch {
    pub location: LocalOrRemotePath,
    pub line_number: u32,
    /// Original 1-based character column in the file. This is captured
    /// before display-only whitespace trimming so opening a result navigates
    /// to the correct location.
    pub column_num: Option<usize>,
    pub line_text: String,
    pub submatches: Vec<Submatch>,
}

#[cfg_attr(not(target_family = "wasm"), path = "model.rs")]
#[cfg_attr(target_family = "wasm", path = "model_wasm.rs")]
pub mod model;
pub mod view;
