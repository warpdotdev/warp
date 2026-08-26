pub use warp_terminal::model::secrets::*;
use warpui::EntityId;
use warpui::elements::SecretRange;

use crate::ai::blocklist::block::TextLocation;
#[derive(Clone, Debug)]
pub struct RichContentSecretTooltipInfo {
    pub secret: String,
    pub secret_range: SecretRange,
    pub location: TextLocation,
    pub is_obfuscated: bool,
    pub position_id: String,
    pub view_id: EntityId,
    pub secret_level: SecretLevel,
}
