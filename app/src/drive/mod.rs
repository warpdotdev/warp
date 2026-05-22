pub mod cloud_action_confirmation_dialog;
mod cloud_object_naming_dialog;
pub mod cloud_object_styling;
pub mod drive_helpers;
pub mod empty_trash_confirmation_dialog;
pub mod export;
pub mod folders;
pub mod import;
pub(crate) mod index;
pub mod items;
pub mod panel;
pub mod settings;
pub mod sharing;
pub mod workflows;

use std::cmp::Ordering;
use std::fmt;

pub use index::DriveIndexVariant;
pub use panel::{DrivePanel, DrivePanelEvent};
use serde::{Deserialize, Serialize};
use warp_core::user_preferences::GetUserPreferences as _;
pub use warp_server_client::drive::CloudObjectTypeAndId;
use warpui::AppContext;

use crate::cloud_object::model::view::{CloudViewModel, UpdateTimestamp};
use crate::cloud_object::{CloudObject, ObjectType};
use crate::server::ids::ServerId;
use crate::ui_components::icons::Icon;
use crate::workflows::CloudWorkflow;

type SortByComparator<'a> = dyn FnMut(&&dyn CloudObject, &&dyn CloudObject) -> Ordering + 'a;

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct OpenWarpDriveObjectSettings {
    /// The folder that should be focused in the Warp Drive when the object is opened.
    pub focused_folder_id: Option<ServerId>,
    /// The email of the user to invite to the object, if the object is being opened via the request access flow.
    pub invitee_email: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OpenWarpDriveObjectArgs {
    pub object_type: ObjectType,
    pub server_id: ServerId,
    pub settings: OpenWarpDriveObjectSettings,
}

#[derive(Copy, Clone, Debug)]
pub enum DriveObjectType {
    Workflow,
    AgentModeWorkflow,
    AIFact,
    AIFactCollection,
    Notebook {
        /// Whether the notebook was created as an AI Document (plan)
        is_ai_document: bool,
    },
    Folder,
    EnvVarCollection,
    MCPServer,
    MCPServerCollection,
}

impl From<DriveObjectType> for Icon {
    fn from(cloud_object_type: DriveObjectType) -> Icon {
        match cloud_object_type {
            DriveObjectType::Workflow => Icon::Workflow,
            DriveObjectType::AgentModeWorkflow => Icon::Prompt,
            DriveObjectType::AIFact => Icon::BookOpen,
            DriveObjectType::AIFactCollection => Icon::BookOpen,
            DriveObjectType::Notebook { is_ai_document } => {
                if is_ai_document {
                    Icon::Compass
                } else {
                    Icon::Notebook
                }
            }
            DriveObjectType::Folder => Icon::Folder,
            DriveObjectType::EnvVarCollection => Icon::EnvVarCollection,
            DriveObjectType::MCPServer => Icon::Dataflow,
            DriveObjectType::MCPServerCollection => Icon::Dataflow,
        }
    }
}

impl fmt::Display for DriveObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriveObjectType::Notebook { .. } => write!(f, "notebook"),
            DriveObjectType::Workflow => write!(f, "workflow"),
            DriveObjectType::Folder => write!(f, "folder"),
            DriveObjectType::EnvVarCollection => write!(f, "env var collection"),
            DriveObjectType::AgentModeWorkflow => write!(f, "prompt"),
            DriveObjectType::AIFact => write!(f, "ai fact"),
            DriveObjectType::AIFactCollection => write!(f, "ai fact collection"),
            DriveObjectType::MCPServer => write!(f, "mcp server"),
            DriveObjectType::MCPServerCollection => write!(f, "mcp server collection"),
        }
    }
}

pub fn should_auto_open_welcome_folder(app: &mut AppContext) -> bool {
    app.private_user_preferences()
        .read_value(settings::HAS_AUTO_OPENED_WELCOME_FOLDER)
        .unwrap_or_default()
        .and_then(|s| serde_json::from_str(&s).ok())
        .map(|has_opened: bool| !has_opened)
        .unwrap_or(true)
}

pub fn write_has_auto_opened_welcome_folder_to_user_defaults(app: &mut AppContext) {
    let _ = app
        .private_user_preferences()
        .write_value(settings::HAS_AUTO_OPENED_WELCOME_FOLDER, true.to_string());
}

/// Enum used for sorting elements in the Warp Drive Index (and potentially other places).
/// In the future it can be used to add other options (like, by name or by author), and exposed to
/// users in the index.
#[derive(
    Default,
    PartialEq,
    Eq,
    Hash,
    Clone,
    Copy,
    Debug,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Sort order for Warp Drive items.",
    rename_all = "snake_case"
)]
pub enum DriveSortOrder {
    /// Sort by newest revision first in main index, most recently trashed in trash index
    #[default]
    ByTimestamp,
    /// A => Z
    AlphabeticalDescending,
    /// Z => A
    AlphabeticalAscending,
    /// Sort by object type, with folders first
    ByObjectType,
}

impl DriveSortOrder {
    /// Returns the comparator that can be used for sorting items returned by
    /// CloudModel::cloud_objects_in_space, for example (so more specifically, on the iterator of
    /// type Iterator<Item = &'_ dyn CloudObject>)
    pub fn sort_by<'a>(
        &self,
        cloud_model: &'a CloudViewModel,
        update_timestamp: UpdateTimestamp,
        app: &'a AppContext,
    ) -> Box<SortByComparator<'a>> {
        match self {
            // Sorts newly-created objects to be at the top of the list
            Self::ByTimestamp => Box::new(
                move |a: &&dyn CloudObject, b: &&dyn CloudObject| -> Ordering {
                    cloud_model
                        .object_sorting_timestamp(*a, update_timestamp, app)
                        .cmp(&cloud_model.object_sorting_timestamp(*b, update_timestamp, app))
                        .reverse()
                },
            ),
            Self::AlphabeticalDescending => Box::new(
                move |a: &&dyn CloudObject, b: &&dyn CloudObject| -> Ordering {
                    a.display_name()
                        .to_lowercase()
                        .cmp(&b.display_name().to_lowercase())
                },
            ),
            Self::AlphabeticalAscending => Box::new(
                move |a: &&dyn CloudObject, b: &&dyn CloudObject| -> Ordering {
                    b.display_name()
                        .to_lowercase()
                        .cmp(&a.display_name().to_lowercase())
                },
            ),
            Self::ByObjectType => Box::new(
                move |a: &&dyn CloudObject, b: &&dyn CloudObject| -> Ordering {
                    let order = |obj: &&dyn CloudObject| match obj.object_type() {
                        ObjectType::Folder => 0,
                        ObjectType::GenericStringObject(_) => 1,
                        ObjectType::Notebook => 2,
                        ObjectType::Workflow => {
                            let Some(workflow) = obj.as_any().downcast_ref::<CloudWorkflow>()
                            else {
                                return 3;
                            };

                            if workflow.model().data.is_agent_mode_workflow() {
                                4
                            } else {
                                3
                            }
                        }
                    };

                    // First compare by object type ordering, then by display name alphabetically if equal
                    order(a).cmp(&order(b)).then_with(|| {
                        a.display_name()
                            .to_lowercase()
                            .cmp(&b.display_name().to_lowercase())
                    })
                },
            ),
        }
    }

    /// Returns the text that is used to display the sorting option in the KnowledgeIndex's sorting menu
    pub fn menu_text(&self, index_variant: DriveIndexVariant) -> &str {
        match (self, index_variant) {
            (DriveSortOrder::ByTimestamp, DriveIndexVariant::MainIndex) => "Last updated",
            (DriveSortOrder::ByTimestamp, DriveIndexVariant::Trash) => "Last trashed",
            (DriveSortOrder::AlphabeticalDescending, _) => "A to Z",
            (DriveSortOrder::AlphabeticalAscending, _) => "Z to A",
            (DriveSortOrder::ByObjectType, _) => "Type",
        }
    }
}
