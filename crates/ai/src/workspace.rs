use std::path::PathBuf;

use chrono::{DateTime, Days, Utc};

/// Public-facing metadata persisted in SQLite
#[derive(Debug, Default, Clone)]
pub struct WorkspaceMetadata {
    pub path: PathBuf,
    pub navigated_ts: Option<DateTime<Utc>>,
    pub modified_ts: Option<DateTime<Utc>>,
    pub queried_ts: Option<DateTime<Utc>>,
}

impl WorkspaceMetadata {
    /// Surface most recently navigated first
    pub fn most_recently_navigated(a: &Self, b: &Self) -> std::cmp::Ordering {
        match (a.navigated_ts, b.navigated_ts) {
            (Some(a_ts), Some(b_ts)) => b_ts.cmp(&a_ts),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.path.cmp(&b.path),
        }
    }

    /// Surface most recently touched first
    pub fn most_recently_touched(a: &Self, b: &Self) -> std::cmp::Ordering {
        match (a.last_touched(), b.last_touched()) {
            (Some(a_ts), Some(b_ts)) => b_ts.cmp(&a_ts),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.path.cmp(&b.path),
        }
    }

    /// The most recent time this codebase index was navigated to, queried or modified.
    pub fn last_touched(&self) -> Option<DateTime<Utc>> {
        let mut last_access_time: Option<DateTime<Utc>> = None;
        if let Some(nav_ts) = self.navigated_ts {
            last_access_time = last_access_time
                .map(|old_time| old_time.max(nav_ts))
                .or(Some(nav_ts));
        }
        if let Some(mod_ts) = self.modified_ts {
            last_access_time = last_access_time
                .map(|old_time| old_time.max(mod_ts))
                .or(Some(mod_ts));
        }
        if let Some(query_ts) = self.queried_ts {
            last_access_time = last_access_time
                .map(|old_time| old_time.max(query_ts))
                .or(Some(query_ts));
        }
        last_access_time
    }

    pub fn is_expired(&self, current_time: DateTime<Utc>, shelf_life_days: u64) -> bool {
        let Some(last_touch) = self.last_touched() else {
            return true;
        };
        last_touch
            .checked_add_days(Days::new(shelf_life_days))
            .unwrap_or_default()
            < current_time
    }
}

/// An event to update the workspace metadata.
#[derive(Debug, Clone, Copy)]
pub enum WorkspaceMetadataEvent {
    Queried,
    Modified,
    Created,
}

impl From<WorkspaceMetadata> for persistence::model::NewWorkspaceMetadata {
    fn from(value: WorkspaceMetadata) -> Self {
        Self {
            repo_path: value.path.to_string_lossy().into_owned(),
            navigated_ts: value.navigated_ts.map(|utc_dt| utc_dt.naive_utc()),
            modified_ts: value.modified_ts.map(|utc_dt| utc_dt.naive_utc()),
            queried_ts: value.queried_ts.map(|utc_dt| utc_dt.naive_utc()),
        }
    }
}

impl From<persistence::model::WorkspaceMetadata> for WorkspaceMetadata {
    fn from(value: persistence::model::WorkspaceMetadata) -> Self {
        Self {
            path: PathBuf::from(value.repo_path),
            navigated_ts: value.navigated_ts.map(|naive_ts| naive_ts.and_utc()),
            modified_ts: value.modified_ts.map(|naive_ts| naive_ts.and_utc()),
            queried_ts: value.queried_ts.map(|naive_ts| naive_ts.and_utc()),
        }
    }
}
