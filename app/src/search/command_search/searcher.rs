use crate::env_vars::CloudEnvVarCollection;
use crate::search::mixer::SearchMixer;
use crate::server::ids::SyncId;
use crate::terminal::history::LinkedWorkflowData;
use crate::workflows::{WorkflowSource, WorkflowType};

pub type CommandSearchMixer = SearchMixer<CommandSearchItemAction>;

#[derive(Clone, Debug)]
pub struct AcceptedHistoryItem {
    pub command: String,

    /// The workflow used to construct the command, if any.
    pub linked_workflow_data: Option<LinkedWorkflowData>,
}

/// Payload for `AcceptWorkflow`: identifies which workflow was selected.
///
/// Cloud workflows carry only a `SyncId` so the handler can resolve the full
/// object from `CloudModel` at accept time (produced by the async
/// `cloud_workflows_data_source`). Local/AI-generated workflows are produced
/// by separate sync data sources and carry owned data since they don't live
/// in `CloudModel`.
#[derive(Clone, Debug)]
pub enum AcceptedWorkflow {
    Cloud {
        id: SyncId,
        source: WorkflowSource,
    },
    Local {
        workflow: Box<WorkflowType>,
        source: WorkflowSource,
    },
}

/// The set of events that may be produced by accepting or executing a search
/// result.
#[derive(Clone, Debug)]
pub enum CommandSearchItemAction {
    /// The user accepted a history search item. The contained string is the
    /// command they accepted.
    AcceptHistory(AcceptedHistoryItem),

    /// The user requested the re-execution of a history search item. The
    /// contained string is the command they accepted.
    ExecuteHistory(String),

    /// The user accepted a workflow search item.
    AcceptWorkflow(AcceptedWorkflow),

    /// The user accepted the notebook search item.
    AcceptNotebook(SyncId),

    /// The user accepted an EVC search item.
    AcceptEnvVarCollection(Box<CloudEnvVarCollection>),

    /// The user accepted the AI query search item with this query text.
    AcceptAIQuery(String),

    /// The user requested to run the AI query search item with this query text.
    RunAIQuery(String),

    /// The user accepted the search item to open Warp AI.
    OpenWarpAI,

    /// The user accepted the search item to translate the query to a command using Warp AI.
    TranslateUsingWarpAI,
}

#[cfg(test)]
#[path = "searcher_tests.rs"]
mod tests;
