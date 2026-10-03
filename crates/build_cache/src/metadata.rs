use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use cap_tempfile::TempFile;
use chrono::{DateTime, Datelike as _, SecondsFormat, Timelike as _, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::spacectl::Mount;

const METADATA_FILE: &str = "cache-metadata.json";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachePathUsage {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_framework: Option<String>,
    pub mount_target: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheMetadataError {
    #[error("cache metadata is malformed")]
    Malformed,
    #[error("cache metadata version is unsupported")]
    UnsupportedVersion,
    #[error("cache metadata location is a symlink")]
    Symlink,
    #[error("cache metadata document is not a regular file")]
    NonRegular,
    #[error("cache metadata path is not volume-relative")]
    InvalidPath,
    #[error("cache metadata could not be read or written")]
    Io,
}

/// Merge cache usage into Namespace's metadata document, preserving unrelated entries and fields.
pub fn write_cache_metadata(
    cache_root: &Path,
    mounts: &[Mount],
    additional: impl IntoIterator<Item = (PathBuf, CachePathUsage)>,
) -> Result<(), CacheMetadataError> {
    let cache_root = normalized_cache_root(cache_root)?;
    let entries = usage_entries(&cache_root, mounts, additional)?;
    MetadataDirectory::open(&cache_root)?.update(entries)
}

// Keep the directory handle alive so a pathname swap cannot redirect reads or replacement.
struct MetadataDirectory(Dir);

impl MetadataDirectory {
    fn open(cache_root: &Path) -> Result<Self, CacheMetadataError> {
        fs::create_dir_all(cache_root).map_err(|_| CacheMetadataError::Io)?;
        let root = Dir::open_ambient_dir(cache_root, ambient_authority())
            .map_err(|_| CacheMetadataError::Io)?;
        reject_symlink(&root, ".ns")?;
        match root.create_dir(".ns") {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(_) => return Err(CacheMetadataError::Io),
        }
        root.open_dir_nofollow(".ns")
            .map(Self)
            .map_err(|_| CacheMetadataError::Io)
    }

    fn read(&self) -> Result<Map<String, Value>, CacheMetadataError> {
        reject_symlink(&self.0, METADATA_FILE)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            // A FIFO must not block before the opened handle can be checked for a regular file.
            options.custom_flags(nix::libc::O_NONBLOCK);
        }
        let mut file = match self.0.open_with(METADATA_FILE, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Map::from_iter([("version".to_owned(), json!(1))]));
            }
            Err(_) => return Err(CacheMetadataError::Io),
        };
        if !file
            .metadata()
            .map_err(|_| CacheMetadataError::Io)?
            .is_file()
        {
            return Err(CacheMetadataError::NonRegular);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| CacheMetadataError::Io)?;
        serde_json::from_slice(&bytes).map_err(|_| CacheMetadataError::Malformed)
    }

    fn update(&self, entries: BTreeMap<String, CachePathUsage>) -> Result<(), CacheMetadataError> {
        let mut document = self.read()?;
        validate_document(&document)?;
        let map_name = if document.contains_key("user_request") {
            "user_request"
        } else {
            "userRequest"
        };
        let user_request = document.entry(map_name).or_insert_with(|| json!({}));
        if user_request.is_null() {
            *user_request = json!({});
        }
        let user_request = user_request
            .as_object_mut()
            .ok_or(CacheMetadataError::Malformed)?;
        for (path, usage) in entries {
            let entry = user_request.entry(path).or_insert_with(|| json!({}));
            let entry = entry.as_object_mut().ok_or(CacheMetadataError::Malformed)?;
            entry.remove("cache_framework");
            entry.remove("mount_target");
            entry.remove("cacheFramework");
            entry.extend(
                serde_json::to_value(usage)
                    .map_err(|_| CacheMetadataError::Malformed)?
                    .as_object()
                    .ok_or(CacheMetadataError::Malformed)?
                    .clone(),
            );
        }
        document.insert("version".to_owned(), json!(1));
        document.remove("updated_at");
        document.insert(
            "updatedAt".to_owned(),
            json!(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
        );
        let mut temporary = TempFile::new(&self.0).map_err(|_| CacheMetadataError::Io)?;
        #[cfg(unix)]
        {
            use cap_std::fs::{Permissions, PermissionsExt as _};
            temporary
                .as_file()
                .set_permissions(Permissions::from_mode(0o600))
                .map_err(|_| CacheMetadataError::Io)?;
        }
        serde_json::to_writer_pretty(&mut temporary, &document)
            .map_err(|_| CacheMetadataError::Io)?;
        temporary.flush().map_err(|_| CacheMetadataError::Io)?;
        reject_symlink(&self.0, METADATA_FILE)?;
        temporary
            .replace(METADATA_FILE)
            .map_err(|_| CacheMetadataError::Io)?;
        Ok(())
    }
}

fn aliased_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    alias: &str,
) -> Result<Option<&'a Value>, CacheMetadataError> {
    if object.contains_key(name) && object.contains_key(alias) {
        return Err(CacheMetadataError::Malformed);
    }
    Ok(object
        .get(name)
        .or_else(|| object.get(alias))
        .filter(|value| !value.is_null()))
}

fn validate_document(document: &Map<String, Value>) -> Result<(), CacheMetadataError> {
    let version = match document.get("version") {
        Some(Value::Number(number)) => number
            .as_f64()
            .filter(|value| value.fract() == 0.0 && *value >= 0.0 && *value <= u32::MAX as f64),
        Some(Value::String(value)) => value.parse::<u32>().ok().map(f64::from),
        Some(Value::Null) | None => Some(0.0),
        Some(Value::Bool(_) | Value::Array(_) | Value::Object(_)) => None,
    }
    .ok_or(CacheMetadataError::Malformed)?;
    if version != 1.0 {
        return Err(CacheMetadataError::UnsupportedVersion);
    }
    if let Some(value) = aliased_field(document, "updatedAt", "updated_at")? {
        validate_timestamp(value)?;
    }
    if let Some(value) = aliased_field(document, "userRequest", "user_request")? {
        for entry in value
            .as_object()
            .ok_or(CacheMetadataError::Malformed)?
            .values()
        {
            let entry = entry.as_object().ok_or(CacheMetadataError::Malformed)?;
            if entry
                .get("source")
                .filter(|value| !value.is_null())
                .is_some_and(|value| !value.is_string())
                || aliased_field(entry, "cacheFramework", "cache_framework")?
                    .is_some_and(|value| !value.is_string())
                || aliased_field(entry, "mountTarget", "mount_target")?.is_some_and(|value| {
                    !value
                        .as_array()
                        .is_some_and(|values| values.iter().all(Value::is_string))
                })
            {
                return Err(CacheMetadataError::Malformed);
            }
        }
    }
    Ok(())
}

fn validate_timestamp(value: &Value) -> Result<(), CacheMetadataError> {
    let value = value.as_str().ok_or(CacheMetadataError::Malformed)?;
    let timestamp =
        DateTime::parse_from_rfc3339(value).map_err(|_| CacheMetadataError::Malformed)?;
    let fractional = value.get(19..).ok_or(CacheMetadataError::Malformed)?;
    let fractional = if let Some(fractional) = fractional.strip_suffix('Z') {
        fractional
    } else {
        let split = fractional
            .len()
            .checked_sub(6)
            .ok_or(CacheMetadataError::Malformed)?;
        let zone = fractional
            .get(split..)
            .ok_or(CacheMetadataError::Malformed)?;
        if !zone.starts_with(['+', '-']) || zone.as_bytes().get(3) != Some(&b':') {
            return Err(CacheMetadataError::Malformed);
        }
        &fractional[..split]
    };
    if value.as_bytes().get(10) != Some(&b'T')
        || !(1..=9999).contains(&timestamp.with_timezone(&Utc).year())
        || timestamp.nanosecond() >= 1_000_000_000
        || (!fractional.is_empty()
            && !fractional.strip_prefix('.').is_some_and(|digits| {
                (1..=9).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
            }))
    {
        return Err(CacheMetadataError::Malformed);
    }
    Ok(())
}

fn reject_symlink(directory: &Dir, path: &str) -> Result<(), CacheMetadataError> {
    match directory.symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CacheMetadataError::Symlink),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CacheMetadataError::Io),
    }
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

fn normalized_cache_root(path: &Path) -> Result<PathBuf, CacheMetadataError> {
    let mut normalized = PathBuf::new();
    for component in std::path::absolute(path)
        .map_err(|_| CacheMetadataError::Io)?
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
    cache_root: &Path,
    mounts: &[Mount],
    additional: impl IntoIterator<Item = (PathBuf, CachePathUsage)>,
) -> Result<BTreeMap<String, CachePathUsage>, CacheMetadataError> {
    let mut grouped = BTreeMap::<String, (BTreeSet<&str>, BTreeSet<String>)>::new();
    for mount in mounts {
        // Older spacectl responses may omit paths; they cannot be attributed to a volume entry.
        if mount.cache_path.as_os_str().is_empty() || mount.mount_path.as_os_str().is_empty() {
            continue;
        }
        let relative = mount
            .cache_path
            .strip_prefix(cache_root)
            .map_err(|_| CacheMetadataError::InvalidPath)?;
        let key = relative_key(relative)?;
        let (modes, targets) = grouped.entry(key).or_default();
        modes.insert(&mount.mode);
        targets.insert(
            mount
                .mount_path
                .to_str()
                .ok_or(CacheMetadataError::InvalidPath)?
                .to_owned(),
        );
    }
    let mut entries = grouped
        .into_iter()
        .map(|(path, (modes, targets))| {
            let framework = if modes.len() == 1 {
                modes.first().filter(|mode| !mode.is_empty())
            } else {
                None
            };
            (
                path,
                CachePathUsage {
                    source: "warp".to_owned(),
                    cache_framework: framework.map(|mode| (*mode).to_owned()),
                    mount_target: targets.into_iter().collect(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (path, usage) in additional {
        entries.insert(relative_key(&path)?, usage);
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
