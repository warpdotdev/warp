//! Notebook-specific local action metadata.

use serde::{Deserialize, Serialize};

use crate::workflows::WorkflowId;

/// Generic entrypoint information for actions that might be keyboard or mouse driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionEntrypoint {
    /// A keyboard shortcut.
    Keyboard,
    /// A button in the UI.
    Button,
    /// A menu item.
    Menu,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "object_type")]
pub enum EmbeddedObjectInfo {
    Workflow { workflow_id: Option<WorkflowId> },
}

/// Information about a block in the notebook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "block_type")]
pub enum BlockInfo {
    /// A workflow embedded in the notebook.
    EmbeddedWorkflow { workflow_id: Option<WorkflowId> },
    /// A code or command block within the notebook.
    CodeBlock,
}

/// A selection/navigation mode within the notebook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    /// Navigate between command/code blocks and embedded workflows.
    Command,
    /// Navigate with a text cursor/selection.
    Text,
}
