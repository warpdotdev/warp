use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::elements::{ScrollData, ScrollableElement, ZIndex};
use warpui::event::ModifiersState;
use warpui::units::{IntoLines, IntoPixels, Pixels};
use warpui::{AppContext, Element, EventContext, ModelHandle, SizeConstraint};

use super::block_list_element::BlockListMenuSource;
use super::view::TerminalAction;
use crate::terminal::input::inline_menu::InlineMenuPositioner;

/// An element that renders the input, block_list and "gap" created after a
/// clear or ctrl-l when in waterfall input mode.
/// It needs its own element because when there is a gap in waterfall mode
/// the input scrolls with the blocklist rather than staying fixed.
///
/// Important - in order for this rendering to work, we need to maintain the model
/// invariant that the height of the gap + all block_heights after the gap
/// equals the height of the current viewport.  That means that on resize
/// the gap height needs to be adjusted so that this invariant holds.
pub struct WaterfallGapElement {
    /// The block list element to render.  Note that only a portion of this
    /// will be rendered depending on the scroll position.
    block_list_element: Box<dyn Element>,

    /// The input box element to render
    input_element: Box<dyn Element>,

    /// The total height of the blocklist from the terminal model, including
    /// the height of the gap, in pixels.
    block_list_height_px: Pixels,

    /// The size of the gap in pixels.
    gap_size_px: Vector2F,

    /// The size of the input - set after layout.
    laid_out_input_size_px: Option<Vector2F>,

    /// The size of the input - set after layout.
    laid_out_block_list_size_px: Option<Vector2F>,

    /// The current line height
    line_height_px: Pixels,

    /// The current scroll top in pixels
    scroll_top_px: Pixels,

    /// The pane height in pixels
    pane_height_px: Pixels,

    // This is helps us handling events properly on stacks. A stack will always
    // put its children on higher z-indexes than its origin, so a hit test using the standard
    // `z_index` method would always result in the event being covered (by the children of the
    // stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    // Then we use that upper bound to do the hit testing, which means a parent will always get
    // events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,

    /// Standard element size and origin fields
    origin: Option<warpui::elements::Point>,
    size: Option<Vector2F>,

    inline_menu_positioner: ModelHandle<InlineMenuPositioner>,
}

impl WaterfallGapElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_list_element: Box<dyn Element>,
        input_element: Box<dyn Element>,
        block_list_height_px: Pixels,
        gap_size_px: Vector2F,
        line_height_px: Pixels,
        scroll_top_px: Pixels,
        pane_height_px: Pixels,
        inline_menu_positioner: ModelHandle<InlineMenuPositioner>,
    ) -> Self {
        Self {
            block_list_element,
            input_element,
            block_list_height_px,
            gap_size_px,
            laid_out_input_size_px: None,
            laid_out_block_list_size_px: None,
            line_height_px,
            origin: None,
            size: None,
            child_max_z_index: None,
            scroll_top_px,
            pane_height_px,
            inline_menu_positioner,
        }
    }

    fn scroll_internal(
        &self,
        position: Vector2F,
        delta: Vector2F,
        precise: bool,
        ctx: &mut EventContext,
    ) -> bool {
        if self
            .bounds()
            .expect("Bounds should be set before event dispatching")
            .contains_point(position)
        {
            if precise {
                // Handle Trackpad Scroll by converting pixel height into fractional lines.
                ctx.dispatch_typed_action(TerminalAction::Scroll {
                    delta: delta.y().into_pixels().to_lines(self.line_height_px),
                });
            } else {
                // Handle Mouse Scroll, whose delta is already in terms of lines.
                ctx.dispatch_typed_action(TerminalAction::Scroll {
                    delta: delta.y().into_lines(),
                });
            }
            true
        } else {
            false
        }
    }

    fn mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        if self
            .bounds()
            .expect("Bounds should be set before event dispatching")
            .contains_point(position)
        {
            ctx.dispatch_typed_action(TerminalAction::Focus);
            true
        } else {
            false
        }
    }
}

impl Element for WaterfallGapElement {
    fn layout(
        &mut self,
        constraint: warpui::SizeConstraint,
        ctx: &mut warpui::LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // To layout the gap element, we first layout the input element and then
        // use its size, the gap size and the scroll_top to figure out how much
        // space is left to lay out the block list element.
        let input_size = self.input_element.layout(constraint, ctx, app);
        self.laid_out_input_size_px = Some(input_size);

        // See the doc comments on `blocklist_top_inset_when_in_waterfall_mode` method for context.
        //
        // Basically, when the inline menu is open, the visible height of the blocklist should be
        // reduced by the height of the inline menu.
        let blocklist_inset_accounting_for_inline_menu = self
            .inline_menu_positioner
            .as_ref(app)
            .blocklist_top_inset_when_in_waterfall_mode(app);

        // Calculate the height after the scroll position of the blocklist without
        // the gap - this is the height the block list element would like to take
        // up in the viewport.
        let visible_block_list_height_px = self.block_list_height_px
            - self.scroll_top_px
            - self.gap_size_px.y().into_pixels()
            - blocklist_inset_accounting_for_inline_menu.unwrap_or_default();

        // Calculate the max height it could take up, which is a function of the pane height
        // and input size.
        let max_height_for_blocklist_element_px =
            self.pane_height_px - input_size.y().into_pixels();

        let block_list_height_constraint_px = visible_block_list_height_px
            .max(Pixels::zero())
            .min(max_height_for_blocklist_element_px);

        let block_list_constraint = SizeConstraint {
            min: constraint.min,
            max: vec2f(constraint.max.x(), block_list_height_constraint_px.as_f32()),
        };

        // Layout the block list with the new constraint
        let block_list_size = self
            .block_list_element
            .layout(block_list_constraint, ctx, app);
        self.laid_out_block_list_size_px = Some(block_list_size);
        self.size = Some(constraint.max);
        constraint.max
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &AppContext) {
        self.input_element.after_layout(ctx, app);
        self.block_list_element.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut warpui::PaintContext, app: &AppContext) {
        self.origin = Some(warpui::elements::Point::from_vec2f(
            origin,
            ctx.scene.z_index(),
        ));
        self.block_list_element.paint(origin, ctx, app);
        self.input_element.paint(
            origin
                + vec2f(
                    0.,
                    self.laid_out_block_list_size_px
                        .expect("block list size set during layout")
                        .y(),
                ),
            ctx,
            app,
        );

        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<warpui::elements::Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &warpui::event::DispatchedEvent,
        ctx: &mut warpui::EventContext,
        app: &AppContext,
    ) -> bool {
        let mut handled = false;
        handled |= self.input_element.dispatch_event(event, ctx, app);
        handled |= self.block_list_element.dispatch_event(event, ctx, app);
        if !handled {
            let event_at_z_index = event.at_z_index(
                self.child_max_z_index.expect("waterfall gap z-index set"),
                ctx,
            );
            handled |= match event_at_z_index {
                Some(warpui::Event::LeftMouseDown { position, .. }) => {
                    self.mouse_down(*position, ctx)
                }
                Some(warpui::Event::RightMouseDown { position, .. }) => {
                    let position_in_terminal_view =
                        *position - self.origin().expect("origin set after paint").xy();
                    ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(
                        BlockListMenuSource::OutsideBlockRightClick {
                            position_in_terminal_view,
                        },
                    ));
                    true
                }
                Some(warpui::Event::ScrollWheel {
                    position,
                    delta,
                    precise,
                    modifiers: ModifiersState { ctrl: false, .. },
                }) => self.scroll_internal(*position, *delta, *precise, ctx),
                _ => false,
            };
        };

        handled
    }
}

impl ScrollableElement for WaterfallGapElement {
    fn scroll_data(&self, app: &AppContext) -> Option<warpui::elements::ScrollData> {
        // You might be wondering - 'what is this blocklist top inset'?
        //
        // This is the height of the inline menu when it is open and rendered above the input.
        //
        // When the inline menu is open, we simulate scrolling the blocklist downwards by the
        // height of the inline menu, which effectively 'slides' the blocklist upwards, keeping the
        // input position fixed.
        //
        // This 'scrolling'/'slide' effect is applied at _paint_ time - the height of the inline
        // menu is _not_ factored into the total blocklist height. However, the height of the menu
        // _is_ factored into the laid out input size. Thus, when calculating the total scrollable
        // distance (total_size), we need to subtract the inline menu height (which is fixed).
        //
        // Basically, for the purposes of scroll logic, we "pretend" that the inline menu is not
        // there, and things work as intended.
        let total_size = self.block_list_height_px + self.laid_out_input_size_px?.y().into_pixels()
            - self
                .inline_menu_positioner
                .as_ref(app)
                .blocklist_top_inset_when_in_waterfall_mode(app)
                .unwrap_or_default();
        Some(ScrollData {
            scroll_start: self.scroll_top_px,
            visible_px: self.size?.y().into_pixels(),
            total_size,
        })
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        ctx.dispatch_typed_action(TerminalAction::Scroll {
            delta: delta.to_lines(self.line_height_px),
        });
    }

    fn should_handle_scroll_wheel(&self) -> bool {
        false
    }
}
