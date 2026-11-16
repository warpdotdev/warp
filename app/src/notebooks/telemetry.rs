//! Notebook-specific telemetry definitions.

use serde::{Deserialize, Serialize};

use super::editor::BlockInsertionSource;
use crate::server::ids::ServerId;
use crate::workflows::WorkflowId;

/// A user action within a notebook. Some actions, like running a command, are not included here
/// because they're covered by existing telemetry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum NotebookTelemetryAction {
    /// The user manually took edit control.
    GrabEditingBaton,
    /// An object was embedded into the notebook.
    InsertEmbeddedObject(EmbeddedObjectInfo),
    /// A block within the notebook was copied to the clipboard.
    /// Currently, this only applies to command-like blocks.
    CopyBlock {
        #[serde(flatten)]
        block: BlockInfo,
        entrypoint: ActionEntrypoint,
    },
    /// The user opened the block insertion menu.
    OpenBlockInsertionMenu { source: BlockInsertionSource },
    /// The user opened the search menu for embedded objects.
    OpenEmbeddedObjectSearch,
    /// The user opened the find bar.
    OpenFindBar,
    /// The user opened the right-click context menu.
    OpenContextMenu,
    /// The selection mode changed.
    ChangeSelectionMode { mode: SelectionMode },
    /// The user navigated between command/code blocks or embedded workflows with the keyboard.
    CommandKeyboardNavigation,
}

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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "object_type")]
pub enum EmbeddedObjectInfo {
    Workflow {
        workflow_id: Option<WorkflowId>,
        team_uid: Option<ServerId>,
    },
}

/// Information about a block in the notebook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "block_type")]
pub enum BlockInfo {
    /// A workflow embedded in the notebook.
    EmbeddedWorkflow {
        workflow_id: Option<WorkflowId>,
        team_uid: Option<ServerId>,
    },
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
