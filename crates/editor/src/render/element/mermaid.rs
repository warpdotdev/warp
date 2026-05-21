use std::time::Duration;

use warpui::{
    AppContext, Element, SizeConstraint,
    elements::{Align, CacheOption, CornerRadius, Empty, Image, Radius, Text},
    geometry::vector::vec2f,
};

use crate::{
    editor::RunnableCommandModel,
    extract_block,
    render::{
        BLOCK_FOOTER_HEIGHT,
        element::paint::{CursorData, CursorDisplayType},
        model::{BlockItem, RenderState, bounds, viewport::ViewportItem},
    },
};

use super::{RenderContext, RenderableBlock};
const MERMAID_RENDER_TIMEOUT: Duration = Duration::from_secs(10);

pub struct RenderableMermaidDiagram {
    viewport_item: ViewportItem,
    image_element: Option<Box<dyn Element>>,
    footer: Box<dyn Element>,
}

impl RenderableMermaidDiagram {
    pub fn new(
        viewport_item: ViewportItem,
        model: Option<&dyn RunnableCommandModel>,
        editor_is_focused: bool,
        ctx: &AppContext,
    ) -> Self {
        let footer = match model {
            Some(model) => model.render_block_footer(editor_is_focused, ctx),
            None => Empty::new().finish(),
        };
        Self {
            viewport_item,
            image_element: None,
            footer,
        }
    }
}

impl RenderableBlock for RenderableMermaidDiagram {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, model: &RenderState, ctx: &mut warpui::LayoutContext, app: &AppContext) {
        let content = model.content();
        let (asset_source, config) = extract_block!(
            self.viewport_item,
            content,
            (_block, BlockItem::MermaidDiagram { asset_source, config, .. }) => (asset_source.clone(), *config)
        );

        self.footer.layout(
            SizeConstraint::strict(vec2f(
                self.viewport_item.content_size.x(),
                BLOCK_FOOTER_HEIGHT,
            )),
            ctx,
            app,
        );

        let code_text = model.styles().code_text;
        let placeholder_color = model.styles().placeholder_color;
        let placeholder = Align::new(
            Text::new(
                "Rendering Mermaid diagram…",
                code_text.font_family,
                code_text.font_size,
            )
            .with_color(placeholder_color)
            .with_line_height_ratio(code_text.line_height_ratio)
            .soft_wrap(false)
            .finish(),
        )
        .finish();
        let failure_notice = Align::new(
            Text::new(
                "Error rendering Mermaid diagram. Please check syntax.",
                code_text.font_family,
                code_text.font_size,
            )
            .with_color(placeholder_color)
            .with_line_height_ratio(code_text.line_height_ratio)
            .soft_wrap(true)
            .finish(),
        )
        .finish();
        let timeout_notice = Align::new(
            Text::new(
                "Failed to render Mermaid diagram",
                code_text.font_family,
                code_text.font_size,
            )
            .with_color(placeholder_color)
            .with_line_height_ratio(code_text.line_height_ratio)
            .soft_wrap(false)
            .finish(),
        )
        .finish();

        let size = vec2f(config.width.as_f32(), config.height.as_f32());
        let mut image = Image::new(asset_source, CacheOption::BySize)
            .contain()
            .before_load(placeholder)
            .on_load_failure(failure_notice)
            .on_load_timeout(MERMAID_RENDER_TIMEOUT, timeout_notice);
        image.layout(SizeConstraint::strict(size), ctx, app);

        self.image_element = Some(Box::new(image));
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, app: &AppContext) {
        let content = model.content();
        let (start_offset, end_offset) = extract_block!(
            self.viewport_item,
            content,
            (block, BlockItem::MermaidDiagram { .. }) => (block.start_char_offset, block.end_char_offset())
        );

        let visible_rect = self.viewport_item.visible_bounds(ctx);
        let content_rect = self.viewport_item.content_bounds(ctx);

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(visible_rect)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(model.styles().code_border)
            .with_background(model.styles().code_background);

        if let Some(ref mut image_element) = self.image_element {
            image_element.paint(content_rect.origin(), ctx.paint, app);
        }

        let selected = model
            .selections()
            .iter()
            .any(|selection| selection.start() < end_offset && selection.end() + 1 > start_offset);
        if selected {
            ctx.paint
                .scene
                .draw_rect_with_hit_recording(content_rect)
                .with_background(model.styles().selection_fill);
        }
        if model.is_selection_head(start_offset) {
            let content_position = bounds::content_origin(
                self.viewport_item.content_offset,
                &self.viewport_item.spacing,
            );
            let end_of_line_position =
                content_position + vec2f(self.viewport_item.content_size.x(), 0.);
            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                end_of_line_position,
                vec2f(
                    model.styles().cursor_width,
                    self.viewport_item.content_size.y(),
                ),
                CursorData::default(),
                model.styles(),
            );
        }
        ctx.paint.scene.start_layer(warpui::ClipBounds::ActiveLayer);
        let button_origin = content_rect.lower_right()
            - vec2f(
                self.footer.size().expect("Footer should be laid out").x(),
                0.,
            );
        self.footer.paint(button_origin, ctx.paint, app);
        ctx.paint.scene.stop_layer();
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &warpui::AppContext) {
        if let Some(ref mut image_element) = self.image_element {
            image_element.after_layout(ctx, app);
        }
        self.footer.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        event: &warpui::event::DispatchedEvent,
        ctx: &mut warpui::EventContext,
        app: &AppContext,
    ) -> bool {
        self.footer.dispatch_event(event, ctx, app)
    }
}
