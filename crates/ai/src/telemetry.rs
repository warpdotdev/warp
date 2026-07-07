use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::features::FeatureFlag;
use warp_core::register_telemetry_event;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Clone, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum AITelemetryEvent {
    MerkleTreeSnapshotRebuildSuccess {
        duration: Duration,
    },
    MerkleTreeSnapshotRebuildFailed {
        error: String,
    },
    MerkleTreeSnapshotDiffSuccess {
        duration: Duration,
    },
    MerkleTreeSnapshotDiffFailed {
        error: String,
    },
    SyncCodebaseContextSuccess {
        total_sync_duration: Duration,
        flushed_node_count: usize,
        flushed_fragment_count: usize,
        total_fragment_size_bytes: usize,
        sync_type: CodebaseContextSyncType,
        cache_population_error: Option<String>,
    },
    SyncCodebaseContextFailed {
        error: String,
        sync_type: CodebaseContextSyncType,
    },
    BuildTreeFailed {
        error: String,
    },
    BuildTreeSuccess {
        file_traversal_duration: Duration,
        merkle_tree_parse_duration: Duration,
    },
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Clone, Serialize)]
pub enum CodebaseContextSyncType {
    Full,
    Initial,
    Incremental,
}

impl TelemetryEvent for AITelemetryEvent {
    fn name(&self) -> &'static str {
        AITelemetryEventDiscriminants::from(self).name()
    }

    fn description(&self) -> &'static str {
        AITelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        AITelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::MerkleTreeSnapshotRebuildSuccess { duration } => Some(json!({
                "duration": duration,
            })),
            Self::MerkleTreeSnapshotRebuildFailed { error } => Some(json!({
                "error": error,
            })),
            Self::MerkleTreeSnapshotDiffSuccess { duration } => Some(json!({
                "duration": duration,
            })),
            Self::MerkleTreeSnapshotDiffFailed { error } => Some(json!({
                "error": error,
            })),
            Self::SyncCodebaseContextSuccess {
                total_sync_duration,
                sync_type,
                flushed_node_count,
                flushed_fragment_count,
                total_fragment_size_bytes,
                cache_population_error,
            } => Some(json!({
                "total_sync_duration": total_sync_duration,
                "sync_type": sync_type,
                "flushed_node_count": flushed_node_count,
                "flushed_fragment_count": flushed_fragment_count,
                "total_fragment_size_bytes": total_fragment_size_bytes,
                "cache_population_error": cache_population_error
            })),
            Self::SyncCodebaseContextFailed { error, sync_type } => Some(json!({
                "error": error,
                "sync_type": sync_type
            })),
            Self::BuildTreeFailed { error } => Some(json!({
                "error": error
            })),
            Self::BuildTreeSuccess {
                file_traversal_duration,
                merkle_tree_parse_duration,
            } => Some(json!({
                "file_traversal_duration": file_traversal_duration,
                "merkle_tree_parse_duration": merkle_tree_parse_duration
            })),
        }
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::MerkleTreeSnapshotRebuildSuccess { .. }
            | Self::MerkleTreeSnapshotRebuildFailed { .. }
            | Self::MerkleTreeSnapshotDiffSuccess { .. }
            | Self::MerkleTreeSnapshotDiffFailed { .. }
            | Self::SyncCodebaseContextFailed { .. }
            | Self::SyncCodebaseContextSuccess { .. }
            | Self::BuildTreeFailed { .. }
            | Self::BuildTreeSuccess { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for AITelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::MerkleTreeSnapshotRebuildSuccess => {
                "AgentMode.MerkleTreeSnapshot.Rebuild.Success"
            }
            Self::MerkleTreeSnapshotRebuildFailed => "AgentMode.MerkleTreeSnapshot.Rebuild.Failed",
            Self::MerkleTreeSnapshotDiffSuccess => "AgentMode.MerkleTreeSnapshot.Diff.Success",
            Self::MerkleTreeSnapshotDiffFailed => "AgentMode.MerkleTreeSnapshot.Diff.Failed",
            Self::SyncCodebaseContextSuccess => "AgentMode.SyncCodebaseContext.Success",
            Self::SyncCodebaseContextFailed => "AgentMode.SyncCodebaseContext.Failed",
            Self::BuildTreeFailed => "AgentMode.SyncCodebaseContext.BuildTree.Failed",
            Self::BuildTreeSuccess => "AgentMode.SyncCodebaseContext.BuildTree.Success",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::MerkleTreeSnapshotRebuildSuccess => {
                "Successfully rebuilt merkle tree from snapshot"
            }
            Self::MerkleTreeSnapshotRebuildFailed => "Failed to rebuild merkle tree from snapshot",
            Self::MerkleTreeSnapshotDiffSuccess => "Successfully diffed merkle tree snapshot",
            Self::MerkleTreeSnapshotDiffFailed => "Failed to diff merkle tree snapshot",
            Self::SyncCodebaseContextSuccess => "Successfully synced codebase context",
            Self::SyncCodebaseContextFailed => "Failed to sync codebase context",
            Self::BuildTreeFailed => "Failed to build merkle tree for codebase context",
            Self::BuildTreeSuccess => "Successfully built merkle tree for codebase context",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::MerkleTreeSnapshotRebuildSuccess
            | Self::MerkleTreeSnapshotRebuildFailed
            | Self::MerkleTreeSnapshotDiffSuccess
            | Self::MerkleTreeSnapshotDiffFailed
            | Self::SyncCodebaseContextFailed
            | Self::SyncCodebaseContextSuccess
            | Self::BuildTreeFailed
            | Self::BuildTreeSuccess => EnablementState::Flag(FeatureFlag::FullSourceCodeEmbedding),
        }
    }
}

register_telemetry_event!(AITelemetryEvent);
