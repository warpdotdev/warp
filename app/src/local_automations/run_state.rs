//! On-disk schedule state for local automations (separate from user TOML).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// Filename under `data_dir()` for scheduler bookkeeping.
pub const STATE_FILE_NAME: &str = "local_automations_state.json";

pub fn state_file_path() -> PathBuf {
    warp_core::paths::data_dir().join(STATE_FILE_NAME)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalAutomationsRunState {
    #[serde(default = "state_version")]
    pub version: u32,
    #[serde(default)]
    pub by_path: HashMap<String, AutomationRunState>,
}

fn state_version() -> u32 {
    1
}

impl Default for LocalAutomationsRunState {
    fn default() -> Self {
        Self {
            version: state_version(),
            by_path: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationRunState {
    #[serde(default, with = "optional_local_datetime")]
    pub last_scheduled_fire_at: Option<DateTime<Local>>,
    #[serde(default, with = "optional_local_datetime")]
    pub last_missed_at: Option<DateTime<Local>>,
    #[serde(default)]
    pub missed_count: u32,
    #[serde(default, with = "optional_local_datetime")]
    pub in_flight_since: Option<DateTime<Local>>,
}

mod optional_local_datetime {
    use chrono::{DateTime, Local};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<DateTime<Local>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(dt) => serializer.serialize_some(&dt.to_rfc3339()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Local>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<String>::deserialize(deserializer)?;
        match opt {
            None => Ok(None),
            Some(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| Some(dt.with_timezone(&Local)))
                .or_else(|_| {
                    // Accept bare UTC / offset-naive fallbacks via chrono's parse
                    s.parse::<DateTime<Local>>()
                        .map(Some)
                        .map_err(serde::de::Error::custom)
                }),
        }
    }
}

impl LocalAutomationsRunState {
    pub fn load_from_path(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(state) => state,
                Err(e) => {
                    log::warn!(
                        "Failed to parse local automations state at {}: {e}; starting fresh",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log::warn!(
                    "Failed to read local automations state at {}: {e}; starting fresh",
                    path.display()
                );
                Self::default()
            }
        }
    }

    pub fn load() -> Self {
        Self::load_from_path(&state_file_path())
    }

    pub fn save_to_path(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn save(&self) {
        let path = state_file_path();
        if let Err(e) = self.save_to_path(&path) {
            log::warn!(
                "Failed to persist local automations state to {}: {e}",
                path.display()
            );
        }
    }

    pub fn entry_mut(&mut self, path_key: &str) -> &mut AutomationRunState {
        self.by_path.entry(path_key.to_string()).or_default()
    }

    pub fn entry(&self, path_key: &str) -> Option<&AutomationRunState> {
        self.by_path.get(path_key)
    }

    /// Drop state for paths that are no longer present in the loaded set.
    pub fn prune_to_paths(&mut self, keep: &std::collections::HashSet<String>) {
        self.by_path.retain(|k, _| keep.contains(k));
    }
}

/// Stable key for an automation's on-disk path.
pub fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "run_state_tests.rs"]
mod tests;
