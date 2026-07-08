use std::ops::Range;
use std::time::Duration;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    ChildAnchor, Container, CrossAxisAlignment, Empty, Flex, Hoverable, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Stack,
};
use warpui_core::geometry::vector::vec2f;
use warpui_core::platform::Cursor;
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
use crate::render::model::{BlockItem, LineCount};

/// A renderable block for hidden sections: a single- or double-line-height full-width bar.
/// Double-clicking the bar fully expands the hidden section it represents; a single click is
/// consumed so it does not start a text selection. The gutter expand buttons (rendered
/// elsewhere) keep their chunk-at-a-time behavior.
pub struct RenderableHiddenSection {
    element: Box<dyn Element>,
    viewport_item: ViewportItem,
}

impl RenderableHiddenSection {
    pub fn new<V: EditorView>(
        viewport_item: ViewportItem,
        mouse_state: MouseStateHandle,
        full_line_range: Option<Range<LineCount>>,
        parent_view: WeakViewHandle<V>,
        app: &AppContext,
    ) -> Self {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let base_background = internal_colors::fg_overlay_1(theme);
        let ui_builder = appearance.ui_builder();

        let element = Hoverable::new(mouse_state, move |state| {
            let row = Flex::row()
                .with_child(Empty::new().finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center);
            let bar = Container::new(row.finish())
                .with_background(base_background)
                .finish();

            if !state.is_hovered() {
                return bar;
            }

            // On hover, float a tooltip explaining the double-click gesture,
            // centered just below the bar (mirrors how `Button` shows its tooltip).
            let mut stack = Stack::new().with_child(bar);
            let tooltip = ui_builder
                .tool_tip("Double-click to expand all lines".to_string())
                .build()
                .finish();
            stack.add_positioned_overlay_child(
                tooltip,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 8.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                ),
            );
            stack.finish()
        })
        // A single click on the bar does nothing, but registering the handler makes the
        // Hoverable consume the press so it does not fall through to text selection.
        .on_click(|_, _, _| {})
        .on_double_click(move |ctx, app, _| {
            if let Some(line_range) = full_line_range.clone()
                && let Some(action) =
                    V::Action::hidden_section_double_clicked(line_range, &parent_view, app)
            {
                ctx.dispatch_typed_action(action);
            }
        })
        .with_hover_in_delay(Duration::from_millis(500))
        .with_cursor(Cursor::PointingHand)
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
        // Paint the single- or double-line-height bar element.
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
