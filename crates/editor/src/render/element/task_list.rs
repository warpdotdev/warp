use pathfinder_color::ColorU;
use warpui_core::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, Hoverable, Icon, ListIndentLevel,
    MouseStateHandle, Radius, Rect,
};
use warpui_core::geometry::vector::vec2f;
use warpui_core::platform::Cursor;
use warpui_core::{AppContext, Element, SizeConstraint, WeakViewHandle};

use super::paint::RenderContext;
use super::placeholder::{self, BlockPlaceholder};
use super::{RenderableBlock, RichTextAction};
use crate::content::text::BufferBlockStyle;
use crate::editor::EditorView;
use crate::extract_block;
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState, RichTextStyles, bounds};

// Minimum size constraint for the checkbox point. If the size is smaller than the constraint,
// the svg won't render.
const MIN_CHECK_BOX_SIZE: f32 = 12.;

pub struct RenderableTaskList {
    viewport_item: ViewportItem,
    task_list_icon: Box<dyn Element>,
    icon_size: f32,
    placeholder: BlockPlaceholder,
}

impl RenderableTaskList {
    pub fn new<V: EditorView>(
        complete: bool,
        styles: &RichTextStyles,
        viewport_item: ViewportItem,
        mouse_state: MouseStateHandle,
        parent_view: WeakViewHandle<V>,
    ) -> Self {
        let checkmark_icon =
            Icon::new(styles.check_box_style.icon_path, ColorU::white()).with_opacity(1.0);
        let checkbox_size = styles.base_text.font_size.max(MIN_CHECK_BOX_SIZE);

        let (inner, border_width) = if complete {
            (checkmark_icon.finish(), 0.)
        } else {
            (Rect::new().finish(), styles.check_box_style.border_width)
        };

        let checkbox_length_without_border = checkbox_size - border_width * 2.;
        let checkmark_size = checkbox_length_without_border - 3.;

        let checkbox = Container::new(
            ConstrainedBox::new(
                Align::new(
                    ConstrainedBox::new(inner)
                        .with_height(checkmark_size)
                        .with_width(checkmark_size)
                        .finish(),
                )
                .finish(),
            )
            .with_width(checkbox_length_without_border)
            .with_height(checkbox_length_without_border)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
        .with_border(
            Border::all(border_width).with_border_fill(styles.check_box_style.border_color),
        );

        let block_start = viewport_item.block_offset;
        let hoverable = Hoverable::new(mouse_state, |state| {
            if complete {
                checkbox
                    .with_background(styles.check_box_style.background)
                    .finish()
            } else if state.is_hovered() {
                checkbox
                    .with_background(styles.check_box_style.hover_background)
                    .finish()
            } else {
                checkbox.finish()
            }
        })
        .on_click(move |ctx, app, _| {
            if let Some(action) = V::Action::task_list_clicked(block_start, &parent_view, app) {
                ctx.dispatch_typed_action(action)
            }
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        Self {
            viewport_item,
            task_list_icon: hoverable,
            icon_size: checkbox_size,
            placeholder: BlockPlaceholder::new(true),
        }
    }
}

impl RenderableBlock for RenderableTaskList {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &RenderState,
        ctx: &mut warpui_core::LayoutContext,
        app: &warpui_core::AppContext,
    ) {
        self.task_list_icon.layout(
            SizeConstraint::strict(vec2f(self.icon_size, self.icon_size)),
            ctx,
            app,
        );
        self.placeholder
            .layout(&self.viewport_item, model, ctx, app, |block| {
                placeholder::Options {
                    text: "To-do list",
                    block_style: match block {
                        BlockItem::TaskList {
                            indent_level,
                            complete,
                            ..
                        } => BufferBlockStyle::TaskList {
                            indent_level: *indent_level,
                            complete: *complete,
                        },
                        _ => BufferBlockStyle::TaskList {
                            indent_level: ListIndentLevel::One,
                            complete: false,
                        },
                    },
                }
            })
    }

    fn paint(
        &mut self,
        model: &RenderState,
        ctx: &mut RenderContext,
        app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let task_list = extract_block!(self.viewport_item, content, (block, BlockItem::TaskList{ paragraph: inner, ..}) => block.task_list(inner));
        let text_styling = &model.styles().base_text;

        let line_origin = ctx.content_to_screen(bounds::visible_origin(
            task_list.start_y_offset,
            &self.viewport_item.spacing,
        ));

        let content_origin = task_list.content_origin();
        let space_width = ctx
            .paint
            .font_cache
            .em_width(text_styling.font_family, text_styling.font_size)
            / 2.;

        let checkbox_origin = vec2f(
            ctx.content_to_screen(content_origin).x() - space_width - self.icon_size,
            // Center the checkbox with respect to the first line of text.
            line_origin.y() + (text_styling.line_height().as_f32() - self.icon_size) / 2.,
        );
        self.task_list_icon.paint(checkbox_origin, ctx.paint, app);

        if !self.placeholder.paint(content_origin, model, ctx) {
            ctx.draw_paragraph(&task_list, text_styling, model);
        }
    }

    fn dispatch_event(
        &mut self,
        _model: &crate::render::model::RenderState,
        event: &warpui_core::event::DispatchedEvent,
        ctx: &mut warpui_core::EventContext,
        app: &AppContext,
    ) -> bool {
        self.task_list_icon.dispatch_event(event, ctx, app)
    }
}
