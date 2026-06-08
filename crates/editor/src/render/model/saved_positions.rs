use warpui_core::EntityId;

/// Utility for consistently creating and referencing saved position IDs for
/// rich text.
#[derive(Debug, Clone)]
pub struct SavedPositions {
    /// Entity ID for the [`super::RenderState`] that owns these positions. This
    /// disambiguates IDs across multiple rich text editors.
    model_id: EntityId,
}

impl SavedPositions {
    /// Create a new `SavedPositions` given the parent model ID.
    pub(super) fn new(model_id: EntityId) -> Self {
        Self { model_id }
    }

    /// Saved position ID for the cursor location.
    pub fn cursor_id(&self) -> String {
        format!("warp_editor:cursor_{}", self.model_id)
    }

    /// The bounding box for the text selection.
    pub fn text_selection_id(&self) -> String {
        format!("warp_editor:text_selection_{}", self.model_id)
    }

    /// The first line of the block that the mouse is currently hovered over.
    pub fn hovered_block_start(&self) -> String {
        format!("warp_editor:hovered_block_start_{}", self.model_id)
    }
}
