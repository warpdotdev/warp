use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{Container, CrossAxisAlignment, Empty, Flex, ParentElement};
use warpui_core::geometry::vector::vec2f;
use warpui_core::{
    AfterLayoutContext, AppContext, Element, LayoutContext, SingletonEntity, SizeConstraint,
};

use super::super::model::RenderState;
use super::super::model::viewport::ViewportItem;
use super::{RenderContext, RenderableBlock};
use crate::extract_block;
use crate::render::model::BlockItem;

/// A renderable block for hidden sections that renders a single- or double-line-height rectangle.
/// This is used for BlockItem::Hidden items that need to be visually indicated.
pub struct RenderableHiddenSection {
    element: Box<dyn Element>,
    viewport_item: ViewportItem,
}

impl RenderableHiddenSection {
    pub fn new(viewport_item: ViewportItem, app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let row = Flex::row()
            .with_child(Empty::new().finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let element = Container::new(row.finish())
            .with_background(internal_colors::fg_overlay_1(theme))
            .finish();

        Self {
            viewport_item,
            element,
        }
    }
}

impl RenderableBlock for RenderableHiddenSection {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, model: &RenderState, ctx: &mut LayoutContext, app: &AppContext) {
        let content = model.content();
        let hidden_section = extract_block!(self.viewport_item, content, (_block, BlockItem::Hidden(config)) => config);

        self.element.layout(
            SizeConstraint::strict(vec2f(
                model.viewport().width().as_f32(),
                hidden_section.height().as_f32(),
            )),
            ctx,
            app,
        );
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, app: &AppContext) {
        // Paint the single- or double-line-height rectangle element
        let content_origin = self.viewport_item.content_bounds(ctx).origin()
            + vec2f(model.viewport().scroll_left().as_f32(), 0.);
        self.element.paint(content_origin, ctx.paint, app);
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.element.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        event: &warpui_core::event::DispatchedEvent,
        ctx: &mut warpui_core::EventContext,
        app: &AppContext,
    ) -> bool {
        self.element.dispatch_event(event, ctx, app)
    }

    fn is_hidden_section(&self) -> bool {
        true
    }
}
