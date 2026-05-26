use std::ops::Range;

use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::units::IntoPixels;
use warpui::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, ModelHandle,
    PaintContext, SizeConstraint, WeakViewHandle,
};

use crate::editor::EditorView;
use crate::render::element::paragraph::RenderableParagraph;
use crate::render::element::temporary_block::RenderableTemporaryBlock;
use crate::render::element::{DisplayOptions, RenderContext, RenderableBlock};
use crate::render::model::{BlockItem, RenderLineLocation, RenderState};

pub struct RichTextElementLens<V: EditorView> {
    blocks: Option<Vec<Box<dyn RenderableBlock>>>,
    line_range: Range<RenderLineLocation>,
    pub model: ModelHandle<RenderState>,
    display_options: DisplayOptions,
    parent_view: WeakViewHandle<V>,
    element_size: Option<Vector2F>,
    element_origin: Option<Point>,
}

impl<V: EditorView> RichTextElementLens<V> {
    pub fn new(
        line_range: Range<RenderLineLocation>,
        model: ModelHandle<RenderState>,
        parent_view: WeakViewHandle<V>,
        display_options: DisplayOptions,
    ) -> Self {
        Self {
            element_size: None,
            element_origin: None,
            model,
            blocks: None,
            parent_view,
            display_options,
            line_range,
        }
    }

    pub fn blocks(&self) -> Option<&[Box<dyn RenderableBlock>]> {
        self.blocks.as_deref()
    }

    pub fn starting_renderable_block_offset(&self) -> Option<f32> {
        self.blocks
            .as_ref()
            .and_then(|blocks| blocks.first())
            .map(|block| block.viewport_item().content_offset.as_f32())
    }
}

impl<V: EditorView> Element for RichTextElementLens<V> {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let model = self.model.as_ref(app);
        let mut total_height = 0.;
        let blocks =
            model.blocks_in_line_range(self.line_range.clone(), constraint.max.x().into_pixels());

        // Only support paragraphs and temporary blocks for now. This should be extensible to more block types in the future.
        let mut renderable_blocks: Vec<Box<dyn RenderableBlock>> = blocks
            .into_iter()
            .filter_map(|(item, block)| match block {
                BlockItem::Paragraph(_) => Some(RenderableParagraph::new(item).finish()),
                BlockItem::TemporaryBlock {
                    decoration,
                    text_decoration,
                    ..
                } => {
                    Some(RenderableTemporaryBlock::new(item, decoration, text_decoration).finish())
                }
                _ => None, /* other block types not supported */
            })
            .collect();

        for block in renderable_blocks.iter_mut() {
            block.layout(model, ctx, app);
            total_height += block.viewport_item().content_size.y();
        }
        self.blocks = Some(renderable_blocks);

        let size = Vector2F::new(constraint.max.x(), total_height);
        self.element_size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for block in self.blocks.as_mut().unwrap().iter_mut() {
            block.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let parent = match self.parent_view.upgrade(app) {
            Some(handle) => handle.as_ref(app),
            None => {
                // TODO: This should really have been an error. But currently in code review it's possible
                // for the parent editor view to be dropped before the lens element.
                log::debug!("Parent rich-text editor view dropped before paint");
                return;
            }
        };

        let model = self.model.as_ref(app);
        let viewport_size = self.element_size.unwrap();
        self.element_origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let content_bounds = RectF::new(origin, viewport_size);

        // "Mock" the scroll top to be the first block's content offset here so the start of line range is displayed
        // top of the element.
        let scroll_top = self.starting_renderable_block_offset().unwrap_or(0.);

        let mut ctx = RenderContext::new(
            content_bounds,
            self.display_options.focused,
            self.display_options.editable,
            false,              /* no cursor blink */
            Default::default(), /* no cursor type */
            parent.text_decorations(
                model.viewport_charoffset_range(),
                model.next_render_buffer_version(),
                app,
            ),
            scroll_top,
            viewport_size,
            model,
            ctx,
            None,
            &[],
        );
        for block in self.blocks.as_mut().unwrap().iter_mut() {
            block.paint(model, &mut ctx, app);
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.element_size
    }

    fn origin(&self) -> Option<Point> {
        self.element_origin
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }
}
