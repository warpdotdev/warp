use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

const TRUST_STORE_RELATIVE_PATH: &str = ".warp/oz-hook-trust.json";
const TRUST_STORE_SCHEMA_VERSION: &str = "warp.oz_hook_trust.v1";
const MAX_TRUST_RECORDS: usize = 1024;
const MAX_TRUST_STORE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HookTrustKey {
    pub(crate) git_root: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) definition_hash: String,
}

pub(crate) struct PersistentHookTrustStore {
    path: PathBuf,
    trusted: RwLock<HashSet<HookTrustKey>>,
}

impl PersistentHookTrustStore {
    pub(crate) fn load_default() -> Result<Self, PersistentTrustError> {
        let path = dirs::home_dir()
            .ok_or(PersistentTrustError::MissingHome)?
            .join(TRUST_STORE_RELATIVE_PATH);
        Self::load(path)
    }

    pub(crate) fn load(path: PathBuf) -> Result<Self, PersistentTrustError> {
        let trusted = match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > MAX_TRUST_STORE_BYTES {
                    return Err(PersistentTrustError::Oversized);
                }
                let file: PersistentTrustFile = serde_json::from_slice(&bytes)?;
                if file.schema_version != TRUST_STORE_SCHEMA_VERSION {
                    return Err(PersistentTrustError::UnsupportedSchema);
                }
                if file.records.len() > MAX_TRUST_RECORDS
                    || file.records.iter().any(|record| !valid_trust_key(record))
                {
                    return Err(PersistentTrustError::InvalidRecord);
                }
                file.records.into_iter().collect()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            trusted: RwLock::new(trusted),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn trust(&self, key: HookTrustKey) -> Result<(), PersistentTrustError> {
        let key = canonical_trust_key(key)?;
        let mut trusted = self.trusted.write().unwrap();
        if trusted.len() >= MAX_TRUST_RECORDS && !trusted.contains(&key) {
            return Err(PersistentTrustError::TooManyRecords);
        }
        let mut updated = trusted.clone();
        updated.insert(key);
        self.persist(&updated)?;
        *trusted = updated;
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn revoke(&self, key: &HookTrustKey) -> Result<(), PersistentTrustError> {
        let key = canonical_trust_key(key.clone())?;
        let mut trusted = self.trusted.write().unwrap();
        let mut updated = trusted.clone();
        updated.remove(&key);
        self.persist(&updated)?;
        *trusted = updated;
        Ok(())
    }

    fn persist(&self, trusted: &HashSet<HookTrustKey>) -> Result<(), PersistentTrustError> {
        let parent = self
            .path
            .parent()
            .ok_or(PersistentTrustError::InvalidPath)?;
        fs::create_dir_all(parent)?;
        let mut records = trusted.iter().cloned().collect::<Vec<_>>();
        records.sort_by(|left, right| {
            (&left.git_root, &left.config_path, &left.definition_hash).cmp(&(
                &right.git_root,
                &right.config_path,
                &right.definition_hash,
            ))
        });
        let bytes = serde_json::to_vec_pretty(&PersistentTrustFile {
            schema_version: TRUST_STORE_SCHEMA_VERSION.into(),
            records,
        })?;
        if bytes.len() > MAX_TRUST_STORE_BYTES {
            return Err(PersistentTrustError::Oversized);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.flush()?;
        temporary
            .persist(&self.path)
            .map_err(|error| PersistentTrustError::Io(error.error))?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;

impl HookTrustStore for PersistentHookTrustStore {
    fn is_trusted(&self, key: &HookTrustKey) -> bool {
        self.trusted.read().unwrap().contains(key)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistentTrustFile {
    schema_version: String,
    records: Vec<HookTrustKey>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PersistentTrustError {
    #[error("home directory is unavailable")]
    MissingHome,
    #[error("trust store path has no parent")]
    InvalidPath,
    #[error("trust store exceeds its size limit")]
    Oversized,
    #[error("trust store schema version is unsupported")]
    UnsupportedSchema,
    #[error("trust store contains an invalid record")]
    InvalidRecord,
    #[error("trust store contains too many records")]
    TooManyRecords,
    #[error("failed to access trust store: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse trust store: {0}")]
    Json(#[from] serde_json::Error),
}

pub(crate) fn is_hook_trust_store_path(path: &Path) -> bool {
    path.ends_with(Path::new(TRUST_STORE_RELATIVE_PATH))
}
fn canonical_trust_key(mut key: HookTrustKey) -> Result<HookTrustKey, PersistentTrustError> {
    key.git_root = fs::canonicalize(key.git_root)?;
    key.config_path = fs::canonicalize(key.config_path)?;
    if valid_trust_key(&key) {
        Ok(key)
    } else {
        Err(PersistentTrustError::InvalidRecord)
    }
}

fn valid_trust_key(key: &HookTrustKey) -> bool {
    key.git_root.is_absolute()
        && key.config_path.is_absolute()
        && key.config_path.starts_with(&key.git_root)
        && key.definition_hash.len() == 64
        && key
            .definition_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) trait HookTrustStore: Send + Sync {
    fn is_trusted(&self, key: &HookTrustKey) -> bool;
}

#[derive(Default)]
pub(crate) struct DenyProjectHookTrust;

impl HookTrustStore for DenyProjectHookTrust {
    fn is_trusted(&self, _key: &HookTrustKey) -> bool {
        false
    }
}

#[derive(Default)]
pub(crate) struct ExactHookTrustStore {
    trusted: RwLock<HashSet<HookTrustKey>>,
}

impl ExactHookTrustStore {
    pub(crate) fn trust(&self, key: HookTrustKey) {
        self.trusted.write().unwrap().insert(key);
    }

    #[allow(dead_code)]
    pub(crate) fn revoke(&self, key: &HookTrustKey) {
        self.trusted.write().unwrap().remove(key);
    }
}

impl HookTrustStore for ExactHookTrustStore {
    fn is_trusted(&self, key: &HookTrustKey) -> bool {
        self.trusted.read().unwrap().contains(key)
    }
}
