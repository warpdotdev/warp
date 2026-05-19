use crate::appearance::Appearance;
use crate::settings::EnforceMinimumContrast;
use crate::terminal::blockgrid_renderer::BlockGridParams;
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::{color, SizeInfo};
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warpui::elements::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint,
};
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;

use super::blockgrid_renderer::GridRenderParams;

pub struct BlockGridElement {
    block_grid: BlockGrid,
    block_grid_params: BlockGridParams,
    size: Vector2F,
    origin: Option<Point>,
    bounds: Option<RectF>,
}

impl BlockGridElement {
    pub fn new(
        block_grid: &BlockGrid,
        appearance: &Appearance,
        enforce_minimum_contrast: EnforceMinimumContrast,
        obfuscate_secrets: ObfuscateSecrets,
        size_info: SizeInfo,
        font_size: f32,
    ) -> Self {
        let theme = appearance.theme();
        let cell_size = Vector2F::new(
            size_info.cell_width_px().as_f32(),
            size_info.cell_height_px().as_f32(),
        );
        let size = vec2f(
            block_grid.grid_handler().columns() as f32,
            block_grid.len_displayed() as f32,
        ) * cell_size;
        Self {
            block_grid: block_grid.clone(),
            block_grid_params: BlockGridParams {
                grid_render_params: GridRenderParams {
                    warp_theme: theme.clone(),
                    font_family: appearance.monospace_font_family(),
                    font_size,
                    font_weight: appearance.monospace_font_weight(),
                    line_height_ratio: appearance.ui_builder().line_height_ratio(),
                    enforce_minimum_contrast,
                    obfuscate_secrets,
                    size_info,
                    cell_size,
                    use_ligature_rendering: false,
                    hide_cursor_cell: false,
                },
                colors: color::List::from(&color::Colors::from(theme.clone())),
                override_colors: color::OverrideList::empty(),
                bounds: Default::default(),
            },
            size,
            origin: None,
            bounds: None,
        }
    }

    pub fn with_ligature_rendering(mut self) -> Self {
        self.block_grid_params
            .grid_render_params
            .use_ligature_rendering = true;
        self
    }

    /// Returns the underlying text of the `BlockGrid`.
    pub fn text(&self) -> String {
        self.block_grid.contents_to_string(false, None)
    }
}

impl Element for BlockGridElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        self.size = self.size.min(constraint.max);
        self.size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let bounds = RectF::new(origin, self.size);

        self.block_grid_params.bounds = bounds;
        self.bounds = Some(bounds);
        self.block_grid
            .draw_with_default_params(origin, origin, &self.block_grid_params, ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        Some(self.size)
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
