use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    ChildAnchor, Container, CrossAxisAlignment, Flex, Highlight, Hoverable, MouseStateHandle,
    OffsetPositioning, ParentElement, PositionedElementAnchor, PositionedElementOffsetBounds,
    SavePosition, Stack, Text,
};
use warpui_core::geometry::vector::vec2f;
use warpui_core::platform::Cursor;
use warpui_core::text_layout::TextStyle;
use warpui_core::ui_components::components::UiComponent;
use warpui_core::{
    AfterLayoutContext, AppContext, Element, LayoutContext, SingletonEntity, SizeConstraint,
    WeakViewHandle,
};

use super::super::model::RenderState;
use super::super::model::viewport::ViewportItem;
use super::{RenderContext, RenderableBlock, RichTextAction};
use crate::editor::EditorView;
use crate::extract_block;
use crate::render::model::{BlockItem, LineCount, RichTextStyles};

const LABEL_LEFT_PADDING: f32 = 8.;
/// Slightly smaller than the default UI font size.
const LABEL_FONT_SIZE: f32 = 11.;
const TOOLTIP_HOVER_DELAY: Duration = Duration::from_millis(500);
/// A hidden-section bar with a link to expand all lines.
pub struct RenderableHiddenSection {
    element: Box<dyn Element>,
    viewport_item: ViewportItem,
}

impl RenderableHiddenSection {
    pub fn new<V: EditorView>(
        viewport_item: ViewportItem,
        mouse_state: MouseStateHandle,
        line_count: LineCount,
        full_line_range: Option<Range<LineCount>>,
        styles: &RichTextStyles,
        parent_view: WeakViewHandle<V>,
        app: &AppContext,
    ) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let base_background = internal_colors::fg_overlay_1(theme);

        // UI chrome rather than code content.
        let font_family = appearance.ui_font_family();
        let base_color = styles.placeholder_color;
        let hover_color = styles.base_text.text_color;
        let label_position_id = format!("hidden_section_label_{:p}", Arc::as_ptr(&mouse_state));

        let label = Hoverable::new(mouse_state, move |state| {
            // Immediate hover style; tooltip waits for TOOLTIP_HOVER_DELAY.
            let mouse_over = state.is_mouse_over_element();

            let lines = line_count.as_usize();
            let text_content = if lines == 1 {
                "1 unmodified line".to_string()
            } else {
                format!("{lines} unmodified lines")
            };
            let char_count = text_content.chars().count();

            let mut text = Text::new_inline(text_content, font_family, LABEL_FONT_SIZE)
                .with_color(if mouse_over { hover_color } else { base_color });
            if mouse_over {
                text = text.with_single_highlight(
                    Highlight::new().with_text_style(
                        TextStyle::new()
                            .with_foreground_color(hover_color)
                            .with_underline_color(hover_color),
                    ),
                    (0..char_count).collect(),
                );
            }

            let mut stack = Stack::new()
                .with_child(SavePosition::new(text.finish(), &label_position_id).finish());

            if state.is_hovered() {
                let tooltip = appearance
                    .ui_builder()
                    .tool_tip("Expand all lines".to_string())
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_save_position_element(
                        label_position_id.clone(),
                        vec2f(0., 4.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }

            stack.finish()
        })
        .on_click(move |ctx, app, _| {
            if let Some(line_range) = full_line_range.clone()
                && let Some(action) =
                    V::Action::hidden_section_clicked(line_range, &parent_view, app)
            {
                ctx.dispatch_typed_action(action);
            }
        })
        .with_hover_in_delay(TOOLTIP_HOVER_DELAY)
        .with_cursor(Cursor::PointingHand)
        .finish();

        let element = Container::new(
            Flex::row()
                .with_child(label)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_background(base_background)
        .with_padding_left(LABEL_LEFT_PADDING)
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
