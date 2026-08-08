//! Filesystem-resolved containment for plugin package paths.
//!
//! Agent Plugins §4.1 requires that every package path a client discovers, reads, or executes
//! resolves inside the filesystem-resolved plugin root. Containment is centralized here so that
//! symlinks on Unix and junctions/reparse points on Windows are handled the same way at every
//! boundary, and so the narrowest applicable failure boundary stays a caller decision.
//!
//! Containment is a correctness rule about which package files Warp will touch. It is not a
//! sandbox: it does not constrain what a launched subprocess subsequently does.
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginPathError {
    #[error("plugin root could not be resolved: {0}")]
    UnresolvableRoot(io::Error),
    #[error("path could not be resolved: {0}")]
    Unresolvable(io::Error),
    #[error("'{0}' is not a plugin-relative path; it must begin with './'")]
    NotPluginRelative(String),
    #[error("'{0}' contains a parent-directory component")]
    ParentTraversal(String),
    #[error("'{resolved}' resolves outside '{root}'")]
    EscapesRoot { resolved: PathBuf, root: PathBuf },
}

/// Returns whether `value` is a plugin-relative path in the sense of Agent Plugins §4.1: it
/// begins with `./`.
///
/// Windows-style `.\` is deliberately not accepted. The standard defines the portable form, and
/// accepting a second spelling would make an unportable package look valid on one platform.
pub fn is_plugin_relative(value: &str) -> bool {
    value.starts_with("./")
}

/// Canonicalizes the longest existing ancestor of `path` and re-appends the remaining
/// components.
///
/// Plain canonicalization fails for a path that does not exist yet, which is legitimate for a
/// working directory a server creates on first run. Resolving the existing prefix still forces
/// every symlink on the way to be followed, so a link that points out of the plugin root cannot
/// be hidden behind a not-yet-created leaf.
pub fn resolve_partial(path: &Path) -> io::Result<PathBuf> {
    let mut trailing: Vec<OsString> = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        match dunce::canonicalize(&current) {
            Ok(resolved) => {
                let mut out = resolved;
                for component in trailing.iter().rev() {
                    out.push(component);
                }
                return Ok(out);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(file_name) = current.file_name().map(OsString::from) else {
                    return Err(error);
                };
                let Some(parent) = current.parent().map(Path::to_path_buf) else {
                    return Err(error);
                };
                if parent == current {
                    return Err(error);
                }
                trailing.push(file_name);
                current = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Resolves `relative` against `root` and verifies the result stays inside the
/// filesystem-resolved `root`.
///
/// `relative` must be a plugin-relative path (`./...`) and must not contain a parent-directory
/// component. Rejecting `..` lexically before touching the filesystem matters: resolving it
/// afterwards would let a symlinked intermediate directory silently relocate the escape.
pub fn resolve_contained(root: &Path, relative: &str) -> Result<PathBuf, PluginPathError> {
    if !is_plugin_relative(relative) {
        return Err(PluginPathError::NotPluginRelative(relative.to_owned()));
    }
    let suffix = Path::new(&relative[2..]);
    resolve_joined(root, suffix, relative)
}

/// Resolves an already-absolute or root-relative `candidate` and verifies containment in `root`.
///
/// Used for paths Warp itself derives, such as `skills/<name>/SKILL.md`, and for a `cwd` whose
/// placeholder has already been expanded to an absolute path.
pub(crate) fn verify_contained(root: &Path, candidate: &Path) -> Result<PathBuf, PluginPathError> {
    let resolved_root = dunce::canonicalize(root).map_err(PluginPathError::UnresolvableRoot)?;
    let resolved = resolve_partial(candidate).map_err(PluginPathError::Unresolvable)?;
    if !resolved.starts_with(&resolved_root) {
        return Err(PluginPathError::EscapesRoot {
            resolved,
            root: resolved_root,
        });
    }
    Ok(resolved)
}

fn resolve_joined(root: &Path, suffix: &Path, original: &str) -> Result<PathBuf, PluginPathError> {
    for component in suffix.components() {
        match component {
            Component::ParentDir => {
                return Err(PluginPathError::ParentTraversal(original.to_owned()));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(PluginPathError::NotPluginRelative(original.to_owned()));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    verify_contained(root, &root.join(suffix))
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod tests;
