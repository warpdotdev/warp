//! Client for the factory public API endpoints.

use std::collections::BTreeMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(test)]
use mockall::automock;

use super::ServerApi;

/// The only code forge supported for factory config sources.
pub const GITHUB_CODE_FORGE: &str = "GITHUB";

/// A repository scoped to a factory.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactoryRepository {
    pub owner: String,
    pub repo: String,
}

/// The repository directory that drives reconciliation for a file-managed factory.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactorySource {
    pub code_forge: String,
    pub repository: FactoryRepository,
    pub r#ref: String,
    pub path: String,
}

/// JSON payload sent to `POST /factory/{uid}/source`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct FactorySourceRequest {
    pub code_forge: String,
    pub repository: FactoryRepository,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The subset of the public factory representation rendered by the CLI.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactoryResponse {
    pub uid: String,
    pub name: String,
    #[serde(default)]
    pub management_mode: Option<String>,
    #[serde(default)]
    pub source: Option<FactorySource>,
}

/// Response body of `GET /factory/{uid}/sync-status`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactorySyncStatusResponse {
    pub management_mode: String,
    #[serde(default)]
    pub source: Option<FactorySource>,
    #[serde(default)]
    pub last_synced_commit: Option<String>,
    #[serde(default)]
    pub latest_sync: Option<FactorySyncSummary>,
}

/// Lifecycle status of a sync attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactorySyncState {
    Pending,
    Running,
    Success,
    Partial,
    Failed,
    Noop,
}

impl FactorySyncState {
    pub fn is_terminal(&self) -> bool {
        match self {
            FactorySyncState::Pending | FactorySyncState::Running => false,
            FactorySyncState::Success
            | FactorySyncState::Partial
            | FactorySyncState::Failed
            | FactorySyncState::Noop => true,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FactorySyncState::Pending => "pending",
            FactorySyncState::Running => "running",
            FactorySyncState::Success => "success",
            FactorySyncState::Partial => "partial",
            FactorySyncState::Failed => "failed",
            FactorySyncState::Noop => "noop",
        }
    }
}

/// One sync attempt from a file-managed factory's sync ledger.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactorySyncSummary {
    pub commit_sha: String,
    pub status: FactorySyncState,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub resource_errors: Vec<FactoryResourceError>,
    #[serde(default)]
    pub degraded_reasons: Vec<String>,
}

/// JSON payload sent to `POST /factory/{uid}/sync`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct FactorySyncRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
}

/// 202 response body of a non-dry-run `POST /factory/{uid}/sync`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactorySyncAccepted {
    pub commit_sha: String,
}

/// 200 response body of a dry-run `POST /factory/{uid}/sync`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactorySyncDryRunResult {
    pub commit_sha: String,
    #[serde(default)]
    pub plan: Option<FactorySyncPlan>,
    #[serde(default)]
    pub resource_errors: Vec<FactoryResourceError>,
}

/// The diff a sync would apply, grouped by change type.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactorySyncPlan {
    pub creates: Vec<FactoryPlannedChange>,
    pub updates: Vec<FactoryPlannedChange>,
    pub deletes: Vec<FactoryPlannedChange>,
    pub no_ops: i64,
}

/// One resource a sync would create, update, or delete.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactoryPlannedChange {
    pub path: String,
    pub kind: String,
    pub reason: String,
}

/// A per-resource failure captured by a factory sync.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactoryResourceError {
    pub resource_path: String,
    #[serde(default)]
    pub line: Option<i64>,
    pub message: String,
}

/// Response body of `GET /factory/{uid}/export`: rendered config files keyed
/// by path relative to the factory source root.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FactoryExportResponse {
    pub files: BTreeMap<String, String>,
}

pub(crate) fn build_factory_source_url(factory_uid: &str) -> String {
    format!("factory/{}/source", urlencoding::encode(factory_uid))
}

pub(crate) fn build_factory_sync_status_url(factory_uid: &str) -> String {
    format!("factory/{}/sync-status", urlencoding::encode(factory_uid))
}

pub(crate) fn build_factory_sync_url(factory_uid: &str) -> String {
    format!("factory/{}/sync", urlencoding::encode(factory_uid))
}

pub(crate) fn build_factory_export_url(factory_uid: &str) -> String {
    format!("factory/{}/export", urlencoding::encode(factory_uid))
}

/// Trait for the factory public API endpoints used by the CLI.
#[cfg_attr(test, automock)]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait FactoryClient: 'static + Send + Sync {
    /// Register a repository directory as the factory's config source.
    async fn link_factory_source(
        &self,
        factory_uid: &str,
        request: FactorySourceRequest,
    ) -> Result<FactoryResponse>;

    /// Clear the factory's config source, returning it to live-managed.
    async fn unlink_factory_source(&self, factory_uid: &str) -> Result<()>;

    /// Report the factory's management mode, config source, and sync ledger state.
    async fn get_factory_sync_status(
        &self,
        factory_uid: &str,
    ) -> Result<FactorySyncStatusResponse>;

    /// Compute the plan a sync would apply without applying or recording anything.
    async fn sync_factory_dry_run(
        &self,
        factory_uid: &str,
        sha: Option<String>,
    ) -> Result<FactorySyncDryRunResult>;

    /// Start an asynchronous sync of the factory from its config source.
    async fn sync_factory(
        &self,
        factory_uid: &str,
        sha: Option<String>,
    ) -> Result<FactorySyncAccepted>;

    /// Render the factory's current projection as factory configuration files.
    async fn export_factory(&self, factory_uid: &str) -> Result<FactoryExportResponse>;
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl FactoryClient for ServerApi {
    async fn link_factory_source(
        &self,
        factory_uid: &str,
        request: FactorySourceRequest,
    ) -> Result<FactoryResponse> {
        self.post_public_api(&build_factory_source_url(factory_uid), &request)
            .await
    }

    async fn unlink_factory_source(&self, factory_uid: &str) -> Result<()> {
        self.delete_public_api_unit(&build_factory_source_url(factory_uid))
            .await
    }

    async fn get_factory_sync_status(
        &self,
        factory_uid: &str,
    ) -> Result<FactorySyncStatusResponse> {
        self.get_public_api(&build_factory_sync_status_url(factory_uid))
            .await
    }

    async fn sync_factory_dry_run(
        &self,
        factory_uid: &str,
        sha: Option<String>,
    ) -> Result<FactorySyncDryRunResult> {
        let request = FactorySyncRequest {
            sha,
            dry_run: Some(true),
        };
        self.post_public_api(&build_factory_sync_url(factory_uid), &request)
            .await
    }

    async fn sync_factory(
        &self,
        factory_uid: &str,
        sha: Option<String>,
    ) -> Result<FactorySyncAccepted> {
        let request = FactorySyncRequest {
            sha,
            dry_run: None,
        };
        self.post_public_api(&build_factory_sync_url(factory_uid), &request)
            .await
    }

    async fn export_factory(&self, factory_uid: &str) -> Result<FactoryExportResponse> {
        self.get_public_api(&build_factory_export_url(factory_uid))
            .await
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
