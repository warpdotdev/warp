//! This crate contains generic utilities and helpers available for use across all internal warp
//! crates.
//!
//! Generally, if a given function/abstraction is useful outside of a single warp-internal crate
//! but isn't large/complex enough to warrant its own crate, it belongs here.
pub mod assets;
pub mod content_version;
pub mod file;
pub mod file_type;
pub mod git;
pub mod host_id;
pub mod local_or_remote_path;
pub mod on_cancel;
pub mod path;
pub mod remote_path;
pub mod standardized_path;
pub mod sync;
pub mod user_input;
pub mod worktree_names;

#[cfg(windows)]
pub mod windows;
