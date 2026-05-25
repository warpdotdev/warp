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
use warp_localization::{replace_placeholders, LocaleId};

use crate::localization;

fn text(key: &str) -> String {
    localization::text_for_locale(LocaleId::EnUs, key)
}

fn text_with_args(key: &str, args: &[(&str, &str)]) -> String {
    replace_placeholders(&text(key), args)
        .expect("localized text template arguments must match the catalog")
}

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
            return Err(anyhow::Error::from(e).context(text_with_args(
                "agent_sdk.driver.harness.json_utils.error.read",
                &[("path", &path.display().to_string())],
            )));
        }
    };
    serde_json::from_str(&content).with_context(|| {
        text_with_args(
            "agent_sdk.driver.harness.json_utils.error.parse",
            &[("path", &path.display().to_string())],
        )
    })
}

/// Serialize `value` as pretty JSON and write it to `path`, creating parent
/// directories as needed. `serialize_error` is used as the context for the
/// serialization step so the caller-facing error is specific to the config
/// file being written.
pub(super) fn write_json_file<T>(path: &Path, value: &T, serialize_error: String) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            text_with_args(
                "agent_sdk.driver.harness.json_utils.error.create",
                &[("path", &parent.display().to_string())],
            )
        })?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).context(serialize_error)?,
    )
    .with_context(|| {
        text_with_args(
            "agent_sdk.driver.harness.json_utils.error.write",
            &[("path", &path.display().to_string())],
        )
    })
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
