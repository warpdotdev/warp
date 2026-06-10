//! View zone hosting the orchestration config card inside the plan editor.
//!
//! The card is anchored before the first line of the plan document, so it scrolls away with plan
//! content while never being part of the document text. Its reserved height converges through
//! measurement: the zone reserves the card's last measured height, the hosting renderable
//! re-measures the real rendered element each layout pass, and `AIDocumentView` re-reconciles
//! the zone whenever the measured size differs from the reserved size.

use std::any::Any;

use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_editor::editor::EmbeddedItemModel;
use warp_editor::render::element::RenderableBlock;
use warp_editor::render::model::viewport::ViewportItem;
use warp_editor::render::model::{
    BlockSpacing, LaidOutEmbeddedItem, RenderState, EMBEDDED_ITEM_FIRST_LINE_HEIGHT,
};
use warpui::elements::{ChildView, Margin, Padding};
use warpui::units::Pixels;
use warpui::{AppContext, Element, EntityId, ViewHandle, WindowId};

use crate::ai::document::orchestration_config_block::OrchestrationConfigBlockView;
use crate::editor::view_zone::{RenderableViewZone, RenderableViewZoneSpacer, ViewZoneXOrigin};

/// Reserved height for the card before its first measurement (roughly the collapsed card).
/// Corrected by the measure write-back after the first layout pass.
const ORCHESTRATION_CONFIG_FALLBACK_HEIGHT: f32 = 88.;

/// Height cap when measuring the card. The fully expanded card (mode toggle, all pickers, and a
/// wrapped validation message) stays well under this, so the measured height is never clipped.
const ORCHESTRATION_CONFIG_MAX_HEIGHT: f32 = 2_000.;

const ORCHESTRATION_CONFIG_ZONE_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(0.)
        .with_top(8.)
        .with_bottom(12.)
        .with_right(16.),
    padding: Padding::uniform(0.),
};

/// An already-laid-out view zone hosting the [`OrchestrationConfigBlockView`] card. The view is
/// resolved by its window + entity id so the zone stays independent of app-crate view types at
/// the `warp_editor` boundary.
#[derive(Debug)]
pub(crate) struct LaidOutOrchestrationConfig {
    size: Vector2F,
    view_entity_id: EntityId,
    window_id: WindowId,
}

impl LaidOutOrchestrationConfig {
    /// Creates a zone item reserving `size` for the card identified by window + entity id.
    pub(crate) fn new(view_entity_id: EntityId, window_id: WindowId, size: Vector2F) -> Self {
        Self {
            size,
            view_entity_id,
            window_id,
        }
    }

    fn config_view(&self, app: &AppContext) -> Option<ViewHandle<OrchestrationConfigBlockView>> {
        app.view_with_id::<OrchestrationConfigBlockView>(self.window_id, self.view_entity_id)
    }
}

impl LaidOutEmbeddedItem for LaidOutOrchestrationConfig {
    fn height(&self) -> Pixels {
        Pixels::new(self.size.y())
    }

    fn size(&self) -> Vector2F {
        self.size
    }

    fn first_line_bound(&self) -> Vector2F {
        vec2f(self.size.x(), EMBEDDED_ITEM_FIRST_LINE_HEIGHT)
    }

    fn element(
        &self,
        _state: &RenderState,
        viewport_item: ViewportItem,
        _model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Box<dyn RenderableBlock> {
        match self.config_view(ctx) {
            Some(view) => {
                let child = ChildView::new(&view).finish();
                let window_id = self.window_id;
                let entity_id = self.view_entity_id;
                Box::new(RenderableViewZone::new(
                    viewport_item,
                    child,
                    ORCHESTRATION_CONFIG_MAX_HEIGHT,
                    ViewZoneXOrigin::Content,
                    Box::new(move |measured, app| {
                        if let Some(view) =
                            app.view_with_id::<OrchestrationConfigBlockView>(window_id, entity_id)
                        {
                            view.read(app, |view, _| view.set_laid_out_size(measured));
                        }
                    }),
                ))
            }
            None => Box::new(RenderableViewZoneSpacer::new(viewport_item)),
        }
    }

    fn spacing(&self) -> BlockSpacing {
        ORCHESTRATION_CONFIG_ZONE_SPACING
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The size the zone should reserve for the card: the editor's content width (viewport width
/// minus zone margins) at the card's last measured height.
pub(crate) fn orchestration_config_zone_size(
    view: &ViewHandle<OrchestrationConfigBlockView>,
    viewport_width: Pixels,
    app: &AppContext,
) -> Vector2F {
    let width =
        (viewport_width - ORCHESTRATION_CONFIG_ZONE_SPACING.x_axis_offset()).max(Pixels::zero());
    let height = view
        .as_ref(app)
        .laid_out_size()
        .map(|size| size.y())
        .unwrap_or(ORCHESTRATION_CONFIG_FALLBACK_HEIGHT);
    vec2f(width.as_f32(), height)
}
