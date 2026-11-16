use std::cmp::Ordering;
use std::fmt::{Display, Formatter, Result};

use uuid::Uuid;

use crate::server::ids::ObjectUid;

pub mod destructive_mcp_confirmation_dialog;
pub mod edit_page;
pub mod installation_modal;
pub mod list_page;
pub mod server_card;
pub mod style;
pub mod update_modal;

// TODO(aeybel/pei): In the future, to enable the re-use of ServerCard for different types of servers (eg. MCP, LSP, etc.)
// We should make ServerCardView and its corresponding events and actions generic
// And define different types of server card ids (eg. MCPId, LSPId) that can be used with this generic card
// As an example of what this might look like: https://github.com/warpdotdev/warp-internal/pull/19291/files
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerCardItemId {
    TemplatableMCP(Uuid),
    TemplatableMCPInstallation(Uuid),
    GalleryMCP(Uuid),
    FileBasedMCP(Uuid),
}

impl Ord for ServerCardItemId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let self_id = self.to_string();
        let other_id = other.to_string();
        self_id.cmp(&other_id)
    }
}

impl PartialOrd for ServerCardItemId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for ServerCardItemId {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            ServerCardItemId::TemplatableMCP(template_uuid) => {
                write!(f, "Templatable MCP Id: {template_uuid}")
            }
            ServerCardItemId::TemplatableMCPInstallation(uuid) => {
                write!(f, "Templatable MCP Installation Id: {uuid}")
            }
            ServerCardItemId::GalleryMCP(uuid) => write!(f, "Gallery MCP Id: {uuid}"),
            ServerCardItemId::FileBasedMCP(uuid) => write!(f, "File-Based MCP Id: {uuid}"),
        }
    }
}

impl ServerCardItemId {
    pub fn uid(&self) -> ObjectUid {
        match self {
            ServerCardItemId::TemplatableMCP(template_uuid) => template_uuid.to_string(),
            ServerCardItemId::TemplatableMCPInstallation(uuid) => uuid.to_string(),
            ServerCardItemId::GalleryMCP(uuid) => uuid.to_string(),
            ServerCardItemId::FileBasedMCP(uuid) => uuid.to_string(),
        }
    }
}
