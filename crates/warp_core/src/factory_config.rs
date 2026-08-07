//! The shared on-disk contract for a user's **default factory**.
//!
//! Warp, the Warp TUI, and third-party harness plugins all read and write the
//! same `<warp home config dir>/factory/config.json` file so a chosen factory
//! survives across surfaces. This module is the canonical implementation of that
//! contract for `warpdotdev/warp`; the `oz factory default` CLI subcommand is its
//! caller, and third-party harnesses either invoke that CLI or reimplement the
//! same behavior against the same file.
//!
//! The load-bearing rule is preserve-unknown-keys: a write is a read-modify-write
//! that retains every key the current version does not recognize, so the file can
//! grow into a wider preferences store later without a breaking migration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::factory_config_file_path;

/// The parsed contents of `factory/config.json` (schema v1).
///
/// `default_factory_uid` is authoritative; `default_factory_name` is advisory and
/// never participates in resolution. Every other key is captured in `extra` and
/// preserved verbatim across writes.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactoryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_factory_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_factory_name: Option<String>,
    /// Keys this version does not recognize. Retained on write so a newer
    /// consumer's preferences are never dropped by an older one.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A resolved, usable default factory.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultFactory {
    /// The authoritative uid, passed directly as `factory_uid`.
    pub uid: String,
    /// The advisory display name, if the file recorded one. Never used to match.
    pub name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FactoryConfigError {
    #[error("failed to read factory config at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("factory config at {path} is malformed: {source}")]
    Malformed {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to write factory config at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not resolve the factory config path (no home directory)")]
    NoConfigPath,
}

fn config_path() -> Result<PathBuf, FactoryConfigError> {
    factory_config_file_path().ok_or(FactoryConfigError::NoConfigPath)
}

/// Reads and parses the config at `path`.
///
/// Returns `Ok(None)` when the file is absent (an unset default, not an error),
/// `Ok(Some(_))` when it parses, and `Err(Malformed)` when it exists but is not a
/// valid v1 object. A malformed file is never modified by a read.
pub fn read_at(path: &Path) -> Result<Option<FactoryConfig>, FactoryConfigError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(FactoryConfigError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|source| FactoryConfigError::Malformed {
            path: path.to_owned(),
            source,
        })
}

/// Resolves the default factory at `path` to a usable uid, or `None` when no
/// usable default is set (absent file, or present with no non-empty uid).
///
/// A present-but-unset file (e.g. after a clear, or one holding only unknown
/// keys) is a silent no-default, matching an absent file. A file that exists but
/// cannot be parsed is surfaced as `Err(Malformed)` so the caller can warn.
pub fn resolve_at(path: &Path) -> Result<Option<DefaultFactory>, FactoryConfigError> {
    let Some(config) = read_at(path)? else {
        return Ok(None);
    };
    Ok(config
        .default_factory_uid
        .filter(|uid| !uid.is_empty())
        .map(|uid| DefaultFactory {
            uid,
            name: config.default_factory_name,
        }))
}

/// Writes `config` to `path` atomically, creating the `factory/` directory when
/// missing. The write goes to a sibling temp file that is renamed into place, so
/// a reader never observes a torn file.
fn write_at(path: &Path, config: &FactoryConfig) -> Result<(), FactoryConfigError> {
    let write_err = |source| FactoryConfigError::Write {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(write_err)?;
    }
    let mut json = serde_json::to_string_pretty(config).map_err(|source| {
        // A serialization failure is a programming error, not a user-file issue,
        // but reuse the write path so the caller sees a single failure surface.
        FactoryConfigError::Write {
            path: path.to_owned(),
            source: std::io::Error::other(source),
        }
    })?;
    json.push('\n');
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json).map_err(write_err)?;
    std::fs::rename(&tmp_path, path).map_err(write_err)
}

/// Sets the default factory at `path`, preserving every unknown key already in
/// the file. Refuses to touch a malformed file so a hand-edited config is never
/// clobbered behind the user's back.
pub fn set_default_at(
    path: &Path,
    uid: &str,
    name: Option<&str>,
) -> Result<(), FactoryConfigError> {
    let mut config = read_at(path)?.unwrap_or_default();
    config.default_factory_uid = Some(uid.to_owned());
    config.default_factory_name = name.map(str::to_owned);
    write_at(path, &config)
}

/// Clears the default factory at `path`, preserving every unknown key. An absent
/// file is a no-op (no file is created); a malformed file is left untouched.
pub fn clear_default_at(path: &Path) -> Result<(), FactoryConfigError> {
    let Some(mut config) = read_at(path)? else {
        return Ok(());
    };
    config.default_factory_uid = None;
    config.default_factory_name = None;
    write_at(path, &config)
}

/// Resolves the default factory from the real channel-aware config path.
pub fn resolve_default() -> Result<Option<DefaultFactory>, FactoryConfigError> {
    resolve_at(&config_path()?)
}

/// Sets the default factory at the real channel-aware config path.
pub fn set_default(uid: &str, name: Option<&str>) -> Result<(), FactoryConfigError> {
    set_default_at(&config_path()?, uid, name)
}

/// Clears the default factory at the real channel-aware config path.
pub fn clear_default() -> Result<(), FactoryConfigError> {
    clear_default_at(&config_path()?)
}

#[cfg(all(test, feature = "local_fs"))]
#[path = "factory_config_tests.rs"]
mod tests;
