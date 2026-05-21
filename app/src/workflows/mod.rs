use serde::{Deserialize, Serialize};
use warp_core::context_flag::ContextFlag;
use warpui::AppContext;

use workflow::Workflow;

pub mod aliases;
pub mod categories;
pub mod command_parser;
pub mod info_box;
pub mod local_workflows;
pub mod workflow;
pub mod workflow_enum;

use crate::notebooks::{NotebookId, NotebookLocation};
use warp_server_client::ids::{HashableId, SyncId};

pub use categories::{CategoriesView, CategoriesViewEvent};

pub fn init(_app: &mut AppContext) {}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
pub enum WorkflowSource {
    Global,
    Local,
    Project,
    AI,
    Notebook {
        notebook_id: Option<NotebookId>,
        location: NotebookLocation,
    },

    /// A hardcoded workflow type that allows Warp to surface features as Workflows (e.g.
    /// a command to see our network log)
    App,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash, PartialOrd)]
pub enum WorkflowSelectionSource {
    CommandPalette,
    UniversalSearch,
    Voltron,
    AI,
    Notebook,
    SlashMenu,
    UpArrowHistory,
    WorkflowView,
    AgentMode,
    Undefined,
    Alias,
}

#[derive(Debug, Clone, Copy)]
pub enum WorkflowViewMode {
    View,
    Edit,
    Create,
}

impl WorkflowViewMode {
    /// The editing mode supported for a workflow.
    ///
    /// Editing is disabled if the user does not have edit permissions.
    pub fn supported_edit_mode(
        _workflow_id: Option<warp_server_client::ids::SyncId>,
        _app: &AppContext,
    ) -> Self {
        Self::Edit
    }

    /// The viewing mode supported for this workflow.
    ///
    /// Viewing is disabled if the user is allowed to edit the workflow and in a context where
    /// running workflows is supported.
    pub fn supported_view_mode(
        _workflow_id: Option<warp_server_client::ids::SyncId>,
        _app: &AppContext,
    ) -> Self {
        if ContextFlag::RunWorkflow.is_enabled() {
            Self::Edit
        } else {
            Self::View
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct WorkflowId(String);

#[cfg(any(test, feature = "test-util"))]
impl From<i64> for WorkflowId {
    fn from(id: i64) -> Self {
        Self(format!("test_uid{}", id.abs()))
    }
}

impl From<String> for WorkflowId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<WorkflowId> for String {
    fn from(id: WorkflowId) -> String {
        id.0
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.0)
    }
}

impl HashableId for WorkflowId {
    fn to_hash(&self) -> String {
        format!("Workflow-{}", self)
    }

    fn from_hash(hash: &str) -> Option<Self> {
        hash.strip_prefix("Workflow-")
            .map(|id| Self(id.to_string()))
    }
}

impl From<WorkflowId> for SyncId {
    fn from(id: WorkflowId) -> Self {
        Self::LegacyObjectId(id.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AIWorkflowOrigin {
    CommandSearch,
    AgentMode,
}

/// Wrapper type for a workflow from local files, generated AI output, or a notebook.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowType {
    /// Saved workflows sourced from local, global, project, app collections, saved locally.
    Local(Workflow),
    /// Ephemeral/transient workflows created from AI output.
    AIGenerated {
        workflow: Workflow,
        origin: AIWorkflowOrigin,
    },
    /// A workflow that's part of a notebook.
    Notebook(Workflow),
}

impl WorkflowType {
    pub fn as_workflow(&self) -> &Workflow {
        match self {
            WorkflowType::Local(workflow) => workflow,
            WorkflowType::AIGenerated { workflow, .. } => workflow,
            WorkflowType::Notebook(workflow) => workflow,
        }
    }

    /// Returns the contained [`Workflow`], consuming `self`.
    pub fn take_workflow(self) -> Workflow {
        match self {
            WorkflowType::Local(workflow) => workflow,
            WorkflowType::AIGenerated { workflow, .. } => workflow,
            WorkflowType::Notebook(workflow) => workflow,
        }
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
