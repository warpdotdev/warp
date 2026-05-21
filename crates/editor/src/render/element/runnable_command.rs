use warpui::{
    AppContext, Element, SizeConstraint,
    elements::{Border, CornerRadius, Empty, Radius},
    geometry::vector::vec2f,
};

use crate::{
    editor::RunnableCommandModel,
    extract_block,
    render::{
        BLOCK_FOOTER_HEIGHT,
        model::{BlockItem, RenderState, viewport::ViewportItem},
    },
};

use super::{RenderContext, RenderableBlock};

/// [`RenderableBlock`] implementation for runnable command blocks.
pub struct RenderableRunnableCommand {
    viewport_item: ViewportItem,
    footer: Box<dyn Element>,
    border: Option<Border>,
}

impl RenderableRunnableCommand {
    pub fn new(
        viewport_item: ViewportItem,
        model: Option<&dyn RunnableCommandModel>,
        editor_is_focused: bool,
        ctx: &AppContext,
    ) -> Self {
        let border = model.as_ref().and_then(|model| model.border(ctx));
        let footer = match model {
            Some(model) => model.render_block_footer(editor_is_focused, ctx),
            None => Empty::new().finish(),
        };

        Self {
            viewport_item,
            footer,
            border,
        }
    }
}

impl RenderableBlock for RenderableRunnableCommand {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _model: &RenderState, ctx: &mut warpui::LayoutContext, app: &AppContext) {
        self.footer.layout(
            SizeConstraint::strict(vec2f(
                self.viewport_item.content_size.x(),
                BLOCK_FOOTER_HEIGHT,
            )),
            ctx,
            app,
        );
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, app: &AppContext) {
        let content = model.content();
        let code_block = extract_block!(self.viewport_item, content, (block, BlockItem::RunnableCodeBlock{paragraph_block, ..}) => block.code_block(paragraph_block));

        let styles = model.styles();
        let code_style = &styles.code_text;

        let border = if ctx.focused {
            self.border.unwrap_or(styles.code_border)
        } else {
            styles.code_border
        };

        let background_rect = self.viewport_item.visible_bounds(ctx);

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(background_rect)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(border)
            .with_background(model.styles().code_background);

        for paragraph in code_block.paragraphs() {
            ctx.draw_paragraph(&paragraph, code_style, model);
        }

        // Place the button at a higher z-index for event handling. See the comment on
        // `RichTextElement::content_z_index` for context.
        ctx.paint.scene.start_layer(warpui::ClipBounds::ActiveLayer);

        // Position the block footer right below the content area, flush with its right-hand edge.
        // This gives the footer some padding relative to the visible area with a background.
        let content_rect = self.viewport_item.content_bounds(ctx);
        let button_origin = content_rect.lower_right()
            - vec2f(
                self.footer.size().expect("Footer should be laid out").x(),
                0.,
            );
        self.footer.paint(button_origin, ctx.paint, app);

        ctx.paint.scene.stop_layer();
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &warpui::AppContext) {
        self.footer.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &crate::render::model::RenderState,
        event: &warpui::event::DispatchedEvent,
        ctx: &mut warpui::EventContext,
        app: &AppContext,
    ) -> bool {
        self.footer.dispatch_event(event, ctx, app)
    }
}
