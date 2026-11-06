//! This crate contains generic utilities and helpers available for use across all internal warp
//! crates.
//!
//! Generally, if a given function/abstraction is useful outside of a single warp-internal crate
//! but isn't large/complex enough to warrant its own crate, it belongs here.
use std::fmt;
pub mod assets;
pub mod content_version;
pub mod file;
pub mod file_type;
pub mod git;
pub mod hashed;
pub mod host_id;
pub mod lazy;
pub mod local_or_remote_path;
pub mod on_cancel;
pub mod path;
pub mod remote_path;
pub mod standardized_path;
pub mod sync;
pub mod user_input;
pub mod worktree_names;
/// AsciiDebug is intended to make it easy to inspect the contents of byte slices that are mostly ASCII
/// characters (but may not be valid unicode). It changes the output of the wrapped byte slice to
/// a human readable string with non-ASCII characters written as hex escapes.
///
/// E.g. `log::info!("{:?}", &AsciiDebug(some_byte_slice));`
pub struct AsciiDebug<'a>(pub &'a [u8]);

impl fmt::Debug for AsciiDebug<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"")?;
        for &byte in self.0 {
            // Check if the byte is a standard printable character.
            if (32..126).contains(&byte) {
                write!(f, "{}", byte as char)?;
            } else {
                write!(f, "\\{{{byte:02X}}}")?;
            }
        }
        write!(f, "\"")?;
        Ok(())
    }
}

#[cfg(windows)]
pub mod windows;
