//! Shared JSON read/merge/write helpers for third-party harness config prep.
//!
//! Third-party CLIs like Claude Code and Gemini CLI persist onboarding, trust,
//! and auth state in JSON files. The harness preparation step
//! needs to set a few keys on those files without clobbering
//! user-owned state. These helpers allow us to read and merge with existing
//! JSON file state easily.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Read a JSON file as `T`, or return `T::default()` if the file does not exist.
///
/// Returns an error if the file exists but cannot be read or parsed.
pub(super) fn read_json_file_or_default<T>(path: &Path) -> Result<T>
where
    T: Default + for<'de> Deserialize<'de>,
{
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(T::default());
        }
        Err(e) => {
            return Err(
                anyhow::Error::from(e).context(format!("Failed to read {}", path.display()))
            );
        }
    };
    serde_json::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Serialize `value` as pretty JSON and write it to `path`, creating parent
/// directories as needed. `serialize_error` is used as the context for the
/// serialization step so the caller-facing error is specific to the config
/// file being written.
pub(super) fn write_json_file<T>(
    path: &Path,
    value: &T,
    serialize_error: &'static str,
) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).context(serialize_error)?,
    )
    .with_context(|| format!("Failed to write {}", path.display()))
}

/// Serialize a slice of JSON values as a JSONL byte string (one value per line).
pub(super) fn entries_to_jsonl(entries: &[Value]) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut buf, entry)?;
        buf.push(b'\n');
    }
    Ok(buf)
}
