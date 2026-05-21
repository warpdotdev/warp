use crate::ai::agent::conversation::AIConversationId;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::search::command_palette::new_session::{NewSessionOption, NewSessionOptionId};
use crate::search::mixer::SearchMixer;
use crate::util::bindings::CommandBinding;
use crate::workspace::PaneViewLocator;
use std::sync::Arc;
use strum_macros::IntoStaticStr;
use warp_server_client::ids::SyncId;
use warp_util::path::LineAndColumnArg;
use warpui::keymap::BindingId;
use warpui::{EntityId, WindowId};

pub type CommandPaletteMixer = SearchMixer<CommandPaletteItemAction>;

#[derive(Clone, Debug)]
pub enum CommandPaletteItemAction {
    /// A binding result was clicked.
    AcceptBinding {
        binding: Arc<CommandBinding>,
    },
    ExecuteWorkflow {
        id: SyncId,
    },
    OpenNotebook {
        id: SyncId,
    },
    InvokeEnvironmentVariables {
        id: SyncId,
    },
    /// Navigate to the session identified by `pane_view`.
    NavigateToSession {
        pane_view_locator: PaneViewLocator,
        window_id: WindowId,
    },
    /// Navigate to a specific conversation.
    NavigateToConversation {
        pane_view_locator: Option<PaneViewLocator>,
        window_id: Option<WindowId>,
        conversation_id: AIConversationId,
        terminal_view_id: Option<EntityId>,
    },
    ForkConversation {
        conversation_id: AIConversationId,
    },
    OpenLaunchConfiguration {
        config: Arc<LaunchConfig>,
        /// See [`OpenLaunchConfigArg::open_in_active_window`].
        open_in_active_window: bool,
    },
    NewSession {
        source: Arc<NewSessionOption>,
    },
    OpenFile {
        path: String,
        project_directory: String,
        line_and_column_arg: Option<LineAndColumnArg>,
    },
    OpenDirectory {
        path: String,
        project_directory: String,
    },
    CreateFile {
        file_name: String,
        current_directory: String,
    },
    NewConversationInProject {
        path: String,
        project_name: String,
    },
    /// Start a new AI conversation
    NewConversation,
    /// No-op action (used for non-interactable separator items that don't do anything on click).
    NoOp,
}

impl CommandPaletteItemAction {
    pub fn to_summary(&self) -> ItemSummary {
        match self {
            CommandPaletteItemAction::AcceptBinding { binding } => ItemSummary::Action {
                binding_id: binding.id,
            },
            CommandPaletteItemAction::OpenNotebook { id } => {
                ItemSummary::Notebook { id: id.clone() }
            }
            CommandPaletteItemAction::ExecuteWorkflow { id } => {
                ItemSummary::Workflow { id: id.clone() }
            }
            CommandPaletteItemAction::InvokeEnvironmentVariables { id } => {
                ItemSummary::EnvVarCollection { id: id.clone() }
            }
            CommandPaletteItemAction::NavigateToSession {
                pane_view_locator, ..
            } => ItemSummary::Session {
                pane_view_locator: *pane_view_locator,
            },
            CommandPaletteItemAction::NavigateToConversation {
                conversation_id, ..
            } => ItemSummary::Conversation {
                id: *conversation_id,
            },
            CommandPaletteItemAction::ForkConversation { .. } => ItemSummary::ForkConversation,
            CommandPaletteItemAction::NewSession { source } => ItemSummary::NewSession {
                id: source.id().clone(),
            },
            CommandPaletteItemAction::OpenLaunchConfiguration { .. } => {
                ItemSummary::LaunchConfiguration
            }
            CommandPaletteItemAction::OpenFile {
                path,
                project_directory,
                line_and_column_arg,
            } => ItemSummary::File {
                path: path.clone(),
                project_directory: project_directory.clone(),
                line_and_column_arg: *line_and_column_arg,
            },
            CommandPaletteItemAction::OpenDirectory {
                path,
                project_directory,
            } => ItemSummary::Directory {
                path: path.clone(),
                project_directory: project_directory.clone(),
            },
            CommandPaletteItemAction::CreateFile { .. } => {
                // CreateFile actions should not show up in recent items
                ItemSummary::NoOp
            }
            CommandPaletteItemAction::NewConversationInProject { path, .. } => {
                ItemSummary::Project { path: path.clone() }
            }
            CommandPaletteItemAction::NewConversation => ItemSummary::NewConversation,
            CommandPaletteItemAction::NoOp => ItemSummary::NoOp,
        }
    }

    pub fn result_type(&self) -> &'static str {
        self.to_summary().into()
    }
}

/// Summary of items that were selected via the command palette. This is needed so that we have a
/// unique way to identify a selected item  so  we can show it in the "recent" section of the
/// palette. We choose to not use the entire [`CommandPaletteItemAction`] since we only need a
/// unique identifier to store. Additionally, parts of the `CommandPaletteItemAction` could change
/// in between invocations of the command palette (such as the content or title of a workflow or the
/// trigger for a keybinding) that should not be factored in when determining whether to show it in
/// the recent section of the palette.
#[derive(Clone, Debug, PartialEq, IntoStaticStr)]
pub enum ItemSummary {
    Action {
        binding_id: BindingId,
    },
    Workflow {
        id: SyncId,
    },
    EnvVarCollection {
        id: SyncId,
    },
    Notebook {
        id: SyncId,
    },
    Session {
        pane_view_locator: PaneViewLocator,
    },
    NewSession {
        id: NewSessionOptionId,
    },
    /// Dummy enum variant for launch configurations until we support showing them in recent section
    /// of the zero state
    LaunchConfiguration,
    File {
        path: String,
        project_directory: String,
        line_and_column_arg: Option<LineAndColumnArg>,
    },
    Directory {
        path: String,
        project_directory: String,
    },
    Project {
        path: String,
    },
    Conversation {
        id: AIConversationId,
    },
    ForkConversation,
    NewConversation,
    /// No-op action (used for non-interactable separator items that don't do anything on click).
    NoOp,
}
