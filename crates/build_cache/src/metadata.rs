use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, ErrorKind, Write as _};
use std::path::{Component, Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use tempfile::NamedTempFile;

const METADATA_FILE: &str = "cache-metadata.json";

/// Configured usage of a cache-volume path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheUsage {
    /// Path relative to the cache-volume root.
    pub path: PathBuf,
    pub cache_framework: Option<String>,
    pub mount_target: Vec<String>,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct CacheMetadata {
    version: u32,
    updated_at: String,
    user_request: BTreeMap<String, CachePathUsage>,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct CachePathUsage {
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_framework: Option<String>,
    mount_target: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheMetadataError {
    #[error("cache metadata directory is a symlink")]
    Symlink,
    #[error("cache metadata path is not volume-relative")]
    InvalidPath,
    #[error("failed to {operation} for cache metadata at {path:?}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cache metadata could not be serialized: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// Replace Namespace usage metadata with a complete snapshot for this instance.
///
/// Pathname operations do not protect against concurrent parent-directory replacement.
pub fn write_cache_metadata(
    cache_root: &Path,
    usages: impl IntoIterator<Item = CacheUsage>,
) -> Result<(), CacheMetadataError> {
    let document = CacheMetadata {
        version: 1,
        updated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        user_request: usage_entries(usages)?,
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(CacheMetadataError::Serialize)?;
    let directory = normalized_cache_root(cache_root)?.join(".ns");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(CacheMetadataError::Symlink);
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CacheMetadataError::Io {
                operation: "inspect directory",
                path: directory,
                source,
            });
        }
    }
    fs::create_dir_all(&directory).map_err(|source| CacheMetadataError::Io {
        operation: "create directory",
        path: directory.clone(),
        source,
    })?;

    let mut file = NamedTempFile::new_in(&directory).map_err(|source| CacheMetadataError::Io {
        operation: "create temporary file",
        path: directory.clone(),
        source,
    })?;
    file.write_all(&bytes)
        .map_err(|source| CacheMetadataError::Io {
            operation: "write temporary file",
            path: file.path().to_owned(),
            source,
        })?;
    file.flush().map_err(|source| CacheMetadataError::Io {
        operation: "flush temporary file",
        path: file.path().to_owned(),
        source,
    })?;
    let destination = directory.join(METADATA_FILE);
    file.persist(&destination)
        .map(|_| ())
        .map_err(|error| CacheMetadataError::Io {
            operation: "persist metadata file",
            path: destination,
            source: error.error,
        })
}

fn relative_key(path: &Path) -> Result<String, CacheMetadataError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                parts.push(value.to_str().ok_or(CacheMetadataError::InvalidPath)?);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CacheMetadataError::InvalidPath);
            }
        }
    }
    if parts.is_empty() {
        return Err(CacheMetadataError::InvalidPath);
    }
    Ok(parts.join("/"))
}

pub(crate) fn normalized_cache_root(path: &Path) -> Result<PathBuf, CacheMetadataError> {
    let mut normalized = PathBuf::new();
    for component in std::path::absolute(path)
        .map_err(|source| CacheMetadataError::Io {
            operation: "normalize cache root",
            path: path.to_owned(),
            source,
        })?
        .components()
    {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn usage_entries(
    usages: impl IntoIterator<Item = CacheUsage>,
) -> Result<BTreeMap<String, CachePathUsage>, CacheMetadataError> {
    let mut grouped = BTreeMap::<String, (BTreeSet<Option<String>>, BTreeSet<String>)>::new();
    for usage in usages {
        let key = relative_key(&usage.path)?;
        let (frameworks, targets) = grouped.entry(key).or_default();
        frameworks.insert(usage.cache_framework.filter(|mode| !mode.is_empty()));
        targets.extend(usage.mount_target);
    }
    Ok(grouped
        .into_iter()
        .map(|(path, (frameworks, targets))| {
            let framework = if frameworks.len() == 1 {
                frameworks.into_iter().next().flatten()
            } else {
                None
            };
            (
                path,
                CachePathUsage {
                    source: "warp".to_owned(),
                    cache_framework: framework,
                    mount_target: targets.into_iter().collect(),
                },
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
