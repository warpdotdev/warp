use float_cmp::ApproxEq;
use instant::Instant;
use parking_lot::Mutex;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use temporary_block::RenderableTemporaryBlock;
use vim::vim::VimMode;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::{
    AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext, ModelHandle,
    PaintContext, SizeConstraint, WeakViewHandle,
    color::ColorU,
    elements::{
        Axis, Border, Dash, Point, ScrollData, ScrollableElement, Vector2FExt, ZIndex,
        new_scrollable::{NewScrollableElement, ScrollableAxis},
    },
    event::{DispatchedEvent, ModifiersState},
    geometry::{
        rect::RectF,
        vector::{Vector2F, vec2f},
    },
    platform::Cursor,
    units::{IntoPixels, Pixels},
};

use super::model::{
    BlockItem, ElementUpdate, HitTestOptions, Location, RenderState, RichTextStyles, UNIT_MARGIN,
    viewport::{SizeInfo, ViewportItem},
};
use crate::{content::version::BufferVersion, editor::EditorView};
use string_offset::CharOffset;

use self::{
    empty::Empty, header::RenderableHeader, hidden_section::RenderableHiddenSection,
    horizontal_rule::HorizontalRule, image::RenderableImage, mermaid::RenderableMermaidDiagram,
    ordered_list::RenderableOrderedListItem, paragraph::RenderableParagraph,
    runnable_command::RenderableRunnableCommand, table::RenderableTable,
    task_list::RenderableTaskList, text_block::RenderableTextBlock,
    unordered_list::RenderableBulletList,
};

pub use self::paint::{CursorData, CursorDisplayType, RenderContext};

pub mod broken_embedding;
mod empty;
mod header;
mod hidden_section;
mod horizontal_rule;
mod image;
pub mod lens_element;
mod mermaid;
mod ordered_list;
mod paint;
mod paragraph;
mod placeholder;
mod runnable_command;
mod table;
mod task_list;
mod temporary_block;
mod text_block;
mod unordered_list;

const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);
const DRAG_POSITION_CLAMP_BUFFER: f32 = 40.;

#[derive(Debug, Clone, Copy, Default)]
pub enum VerticalExpansionBehavior {
    /// The rich text element will fill the max height available.
    #[default]
    FillMaxHeight,
    /// The rich text element will grow as content is added up to the max
    /// vertical height available.
    GrowToMaxHeight,
    /// The rich text element is not rendered with a max height. It grows as tall as
    /// is available.
    InfiniteHeight,
}

/// An element that renders rich text, with no additional UI or decorations.
///
/// This element caches the positions listed in [`super::model::saved_positions::SavedPositions`],
/// and the parent view can overlay UI controls on top of them using a [`warpui::elements::Stack`].
///
/// It additionally reserves horizontal gutters, which are considered in-bounds for content hit
/// testing.
pub struct RichTextElement<V: EditorView> {
    element_size: Option<Vector2F>,
    element_origin: Option<Point>,
    pub model: ModelHandle<RenderState>,
    blocks: Option<Vec<Box<dyn RenderableBlock>>>,
    /// The viewport's updated size info.
    viewport_size_info: Option<SizeInfo>,
    /// The content version corresponding to a viewport state update.
    buffer_version: Option<BufferVersion>,
    /// Whether pending lazy edits were flushed during this layout pass.
    pending_edits_flushed: bool,
    display_state: DisplayStateHandle,
    display_options: DisplayOptions,
    max_width: Option<Pixels>,
    /// We need some information about the view that contains this rich text element,
    /// in order to dispatch actions to it. Using a [`WeakViewHandle`] prevents
    /// reference cycles that make it impossible to dispose of old views, while
    /// also giving us a hook for view-dependent behavior. For example, [`RichTextAction`]
    /// implementations can use the parent view handle to fetch state from the
    /// editor layer.
    parent_view: WeakViewHandle<V>,
    /// This is helps us handling events properly on stacks. A stack will always
    /// put its children on higher z-indexes than its origin, so a hit test using the standard
    /// `z_index` method would always result in the event being covered (by the children of the
    /// stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    /// Then we use that upper bound to do the hit testing, which means a parent will always get
    /// events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,
    /// The z-index of the layer that content is painted on. This should be immediately above the
    /// origin z-index, since we create a layer for clipping. This helps us with hover/cursor
    /// behavior. Like a stack, [`RenderableBlock`]s put their child UI controls at a higher
    /// z-index. This lets us check if a position is covered by a child UI element, even if it's
    /// in bounds of the viewport. For hit testing, we still use `child_max_z_index`, since
    /// un-handled hits on a child element are still hits on the editor.
    content_z_index: Option<ZIndex>,
    /// Current VimMode of the rich text element, if there is one.
    vim_mode: Option<VimMode>,
    /// Vim visual tails - stored cursor positions when entering vim visual mode
    vim_visual_tails: Vec<CharOffset>,
}

/// State purely related to the display of this element. Generally, render-layer state should be
/// part of the [`RenderState`] model. However, some display state must be updated out-of-lifecycle,
/// so it goes here instead (such as hover states).
#[derive(Default)]
pub struct DisplayState {
    /// Whether or not we're currently using the editor (I-bar) mouse cursor.
    show_editor_cursor: AtomicBool,
    /// The most recently hovered-over location. We cache this so as to not spam events every time
    /// the mouse moves.
    hovered_location: Mutex<Option<Location>>,
    /// The next time the cursor blink state needs to be updated.
    next_blink_update: Mutex<Option<Instant>>,
    /// Whether or not the blinking cursor is currently visible.
    blink_cursor_visible: AtomicBool,
}

pub type DisplayStateHandle = Arc<DisplayState>;

impl DisplayState {
    /// Resets the cursor-blinking state.
    pub fn reset_cursor_blink_timer(&self) {
        *self.next_blink_update.lock() = Some(Instant::now() + CURSOR_BLINK_INTERVAL);
        self.blink_cursor_visible.store(true, Ordering::Relaxed);
    }
}

/// Flags for how to display rich text, passed down from the parent view.
#[derive(Debug, Clone)]
pub struct DisplayOptions {
    /// Whether or not the buffer is in an editable state.
    ///
    /// This controls whether or not the cursor is shown.
    pub editable: bool,

    /// Whether or not the buffer is focused (editable or otherwise).
    pub focused: bool,

    /// Whether or not cursor blinking is enabled.
    pub blink_cursors: bool,

    /// The block to record in the position cache as hovered. This is configurable so that the
    /// editor can choose when to update the hovered block based on mouse movement.
    pub hovered_block_start: Option<CharOffset>,

    /// Whether or not to paint the boundaries of each block for debugging.
    pub debug_bounds: bool,

    /// Additional margin to reserve on the left side of the editor.
    pub left_gutter: f32,
    /// Additional margin to reserve on the right side of the editor.
    pub right_gutter: f32,

    /// Whether to expand to take all available vertical space.
    pub vertical_expansion_behavior: VerticalExpansionBehavior,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            editable: true,
            blink_cursors: true,
            debug_bounds: true,
            hovered_block_start: None,
            focused: true,
            left_gutter: 0.,
            right_gutter: 0.,
            vertical_expansion_behavior: Default::default(),
        }
    }
}

/// A renderable block of rich text. Each block in the rendering model is painted by a dedicated
/// [`RenderableBlock`], which also manages UI elements for that block.
pub trait RenderableBlock {
    /// The [`ViewportItem`] identifying the model-layer block that backs this one.
    /// This determines the size and content of this block.
    fn viewport_item(&self) -> &ViewportItem;

    /// Callback to lay out any child [`Element`]s. The block is sized according to its
    /// [`ViewportItem`].
    fn layout(&mut self, _model: &RenderState, _ctx: &mut LayoutContext, _app: &AppContext);

    /// Paints this block's content, selections, and any associated UI elements.
    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, app: &AppContext);

    /// Callback to dispatch `after_layout` updates to child [`Element`]s.
    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    /// Handle events for UI controls (per-block buttons and menus, for example). Implementations
    /// must _only_ handle events for their controls. Selection and text entry are delegated to the
    /// editor model layer by the parent [`RichTextElement`].
    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    /// The visible bounds of this block, based on its viewport location.
    ///
    /// If the block is fully out of view, this will return `None`. In practice, this should
    /// not happen as we only construct [`RenderableBlock`]s for in-viewport blocks.
    fn visible_bounds(&self, ctx: &RenderContext) -> Option<RectF> {
        self.viewport_item()
            .visible_bounds(ctx)
            .intersection(ctx.bounds)
    }

    /// Paints the bounding boxes of this item, for debugging.
    fn paint_bounds(&self, ctx: &mut RenderContext) {
        let border = Border::all(1.).with_dashed_border(Dash {
            dash_length: 8.,
            gap_length: 8.,
            ..Default::default()
        });

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(self.viewport_item().reserved_bounds(ctx))
            .with_border(border.with_border_color(ColorU::new(255, 0, 0, 255)));

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(self.viewport_item().visible_bounds(ctx))
            .with_border(border.with_border_color(ColorU::new(0, 255, 0, 255)));

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(self.viewport_item().content_bounds(ctx))
            .with_border(border.with_border_color(ColorU::new(0, 0, 255, 255)));
    }

    /// The overlay decoration color for this block, if any.
    ///
    /// Used by `EditorWrapper` to draw consolidated full-width background rects
    /// for diff highlighting, replacing the per-block drawing.
    fn overlay_decoration(&self) -> Option<ThemeFill> {
        None
    }

    /// Whether the renderable block is a temporary block.
    fn is_temporary(&self) -> bool {
        false
    }

    fn is_hidden_section(&self) -> bool {
        false
    }

    fn is_embedded_comment(&self) -> bool {
        false
    }

    fn finish(self) -> Box<dyn RenderableBlock>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Conversion trait for editor action types that are compatible with rich text
/// rendering. [`RichTextElement`] uses this to produce view-specific typed actions
/// for generic editor events.
///
/// If a conversion function returns `None`, no event is dispatched. Editors can
/// use this to easily ignore events that don't apply to them (such as scrolling
/// in a single-line editor).
pub trait RichTextAction<V>: Sized {
    /// Create a new event that represents scrolling by `delta` pixels. In almost
    /// all cases, the event handler should dispatch to [`RenderState::scroll`].
    ///
    /// The parent view and [`AppContext`] are not available when handling scroll events.
    fn scroll(delta: Pixels, axis: Axis) -> Option<Self>;

    /// Create a new event that represents the user typing characters.
    fn user_typed(chars: String, parent_view: &WeakViewHandle<V>, ctx: &AppContext)
    -> Option<Self>;

    /// Create a new event that represents a user typing characters in vim mode.
    fn vim_user_typed(
        chars: String,
        parent_view: &WeakViewHandle<V>,
        ctx: &AppContext,
    ) -> Option<Self>;

    /// Create a new event for a left mouse press. This is only fired for mouse events
    /// that successfully hit-test. Usually, this maps to a selection event, but
    /// the implementation and/or event receiver is responsible for determining
    /// what kind (e.g. cursor vs. word vs. line).
    fn left_mouse_down(
        location: Location,
        modifiers: ModifiersState,
        click_count: u32,
        is_first_mouse: bool,
        parent_view: &WeakViewHandle<V>,
        ctx: &AppContext,
    ) -> Option<Self>;

    fn left_mouse_dragged(
        location: Location,
        cmd: bool,
        shift: bool,
        parent_view: &WeakViewHandle<V>,
        ctx: &AppContext,
    ) -> Option<Self>;

    fn left_mouse_up(
        location: Location,
        cmd: bool,
        shift: bool,
        parent_view: &WeakViewHandle<V>,
        ctx: &AppContext,
    ) -> Vec<Self>;

    /// Dispatch an event when the mouse hover location has changed. This is called at [`Location`]
    /// granularity, and not every time the mouse moves. If the location is `None`, then the mouse
    /// is no longer hovering over any content.
    fn mouse_hovered(
        location: Option<Location>,
        parent_view: &WeakViewHandle<V>,
        cmd: bool,
        is_covered: bool,
        ctx: &AppContext,
    ) -> Option<Self>;

    /// Dispatch an event when the checkbox for a task list item was clicked.
    fn task_list_clicked(
        block_start: CharOffset,
        parent_view: &WeakViewHandle<V>,
        ctx: &AppContext,
    ) -> Option<Self>;

    /// Dispatch an event when the mouse wheel is clicked.
    fn middle_mouse_down(ctx: &AppContext) -> Option<Self>;

    /// Create a new event for a right mouse press. This is only fired for mouse events
    /// that successfully hit-test. Used for context menu support in editors.
    /// Default implementation returns None (no action needed).
    fn right_mouse_down(
        _location: Location,
        _parent_view: &WeakViewHandle<V>,
        _ctx: &AppContext,
    ) -> Option<Self> {
        None
    }
}

/// Whether mouse events should be clamped to within the element bound.
enum ClampBehavior {
    ClampToBound,
    // Clamp to (bound - delta, bound + delta). This is useful for drag to
    // select to prevent scrolling stuck at a position with the next block
    // item being completely off the viewport.
    ClampWithBuffer(f32),
    NoneIfOutBound,
}

/// Result of a hit-test from [`RichTextElement::position_to_location`].
///
/// Distinguishes between a successful hit, the mouse being outside the element
/// bounds, and a transient state where layout data is not yet available.
enum HitTestResult {
    /// Successfully resolved a location in the buffer.
    Hit(Location),
    /// The mouse position is outside the element bounds.
    OutOfBounds,
    /// Viewport/layout state is not yet available (e.g., the element was
    /// recreated during a re-render but hasn't been laid out yet). Callers
    /// should avoid acting on this — the state is transient.
    LayoutPending,
}

impl<V: EditorView> RichTextElement<V> {
    /// Creates a new `RichTextElement` which will render the content defined
    /// by `model`. The element needs a weak handle to its containing view in
    /// order to dispatch events appropriately.
    pub fn new(
        model: ModelHandle<RenderState>,
        parent_view: WeakViewHandle<V>,
        display_options: DisplayOptions,
        display_state: DisplayStateHandle,
        vim_mode: Option<VimMode>,
        vim_visual_tails: Vec<CharOffset>,
    ) -> Self {
        Self {
            element_size: None,
            element_origin: None,
            model,
            blocks: None,
            viewport_size_info: None,
            display_state,
            parent_view,
            display_options,
            child_max_z_index: None,
            content_z_index: None,
            max_width: None,
            buffer_version: None,
            pending_edits_flushed: false,
            vim_mode,
            vim_visual_tails,
        }
    }

    /// Returns an immutable reference to the underlying renderable blocks in the viewport.
    pub fn blocks(&self) -> Option<&[Box<dyn RenderableBlock>]> {
        self.blocks.as_deref()
    }

    /*
     * For an example rich-text layout, see:
     * https://docs.google.com/drawings/d/15_Rx_GTWJTvLX8_R_Lfph5sCdcugh5MHP6xY4Ws-McM/edit
     */

    pub fn with_max_width(mut self, max_width: Option<Pixels>) -> Self {
        self.max_width = max_width;
        self
    }

    /// The size of the viewport.
    fn viewport_size(&self) -> Option<Vector2F> {
        self.viewport_size_info.map(|info| info.viewport_size)
    }

    /// The offset of the viewport from the element origin, to center it within the element.
    fn viewport_offset(&self) -> Option<Vector2F> {
        let element_width = self.element_size?.x();
        let viewport_width = self.viewport_size()?.x();
        // Keep the content itself centered if there's lots of space, but ensure we at least leave
        // room for the gutter.
        let x_axis_offset =
            ((element_width - viewport_width) / 2.).max(self.display_options.left_gutter);
        Some(vec2f(x_axis_offset, 0.))
    }

    /// The paint origin for the viewport. This may be offset from the element's own origin.
    fn viewport_origin(&self) -> Option<Vector2F> {
        let element_origin = self.element_origin?;
        Some(element_origin.xy() + self.viewport_offset()?)
    }

    /// Calculate the scroll data within the element instead of deferring to the render state.
    /// This is because the viewport state in render state is updated via an async channel, which
    /// could be output of date.
    fn vertical_scroll_data(&self, app: &AppContext) -> ScrollData {
        let model = self.model.as_ref(app);
        let mut visible_px = self
            .viewport_size_info
            .expect("Viewport size should exist")
            .viewport_size
            .y()
            .into_pixels();
        let total_size = model.height();
        if visible_px.approx_eq(total_size, UNIT_MARGIN) {
            // This is a hack copied from the BlockListElement. Due to floating-point
            // errors, total_size and visible_px may be slightly different,
            // even if they should be the same. In that case, set them to be
            // equal so that a useless scroll bar isn't shown.
            visible_px = total_size;
        }

        let scroll_top = model
            .viewport()
            .scroll_top()
            .min(total_size - visible_px)
            .max(Pixels::zero());

        ScrollData {
            scroll_start: scroll_top,
            visible_px,
            total_size,
        }
    }

    /// Tests if a given position is in this element's bounds.
    fn is_in_bounds(&self, position: Vector2F, ctx: &EventContext) -> bool {
        let Some(origin) = self.element_origin else {
            return false;
        };
        let Some(size) = self.element_size else {
            return false;
        };
        // Bounds checks should use the element's own layer. Child layers may have narrower clip
        // bounds (for example, a wide table's clipped content layer), and using those clip bounds
        // can incorrectly reject valid clicks within the editor before block-level hit testing runs.
        ctx.visible_rect(origin, size)
            .is_some_and(|bounds| bounds.contains_point(position))
    }

    /// Tests if the given position is covered at the content layer (i.e. by a child UI element).
    fn is_content_covered(&self, position: Vector2F, ctx: &EventContext) -> bool {
        self.content_z_index
            .is_some_and(|z_index| ctx.is_covered(Point::from_vec2f(position, z_index)))
    }

    /// Resolve a pixel position to a location in the buffer as best as possible.
    ///
    /// This is only possible after layout and painting.
    fn position_to_location(
        &self,
        position: Vector2F,
        clamp: ClampBehavior,
        options: &HitTestOptions,
        app: &AppContext,
        ctx: &EventContext,
    ) -> HitTestResult {
        // If the pixel position is outside our paint bounds and should not be clamped, don't bother hit testing.
        if matches!(clamp, ClampBehavior::NoneIfOutBound) && !self.is_in_bounds(position, ctx) {
            return HitTestResult::OutOfBounds;
        }

        // If viewport state is not available yet (e.g., during a re-render before layout),
        // report that layout is pending so callers can avoid acting on stale state.
        let Some(viewport_origin) = self.viewport_origin() else {
            return HitTestResult::LayoutPending;
        };
        let Some(viewport_size) = self.viewport_size() else {
            return HitTestResult::LayoutPending;
        };
        let relative_position = position - viewport_origin;
        let viewport_position = match clamp {
            ClampBehavior::ClampToBound => relative_position.clamp(Vector2F::zero(), viewport_size),
            ClampBehavior::NoneIfOutBound => relative_position,
            ClampBehavior::ClampWithBuffer(buffer) => relative_position.clamp(
                Vector2F::zero() - vec2f(0., buffer),
                viewport_size + vec2f(0., buffer),
            ),
        };
        let model = self.model.as_ref(app);
        let mut location = model.viewport_coordinates_to_location(
            viewport_position.x().into_pixels(),
            viewport_position.y().into_pixels(),
            options,
        );
        // If we clamped to the viewport here, override the `clamped` field in `Location` - in
        // `viewport_coordinates_to_location`, we won't know this happened.
        if relative_position != viewport_position
            && let Location::Text { clamped, .. } = &mut location
        {
            *clamped = true;
        }

        HitTestResult::Hit(location)
    }

    /// Performs hit-testing and dispatches mouse-click events to the parent view.
    #[allow(clippy::too_many_arguments)]
    fn handle_left_mouse_down(
        &mut self,
        position: Vector2F,
        modifiers: ModifiersState,
        click_count: u32,
        is_first_mouse: bool,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        if self.is_content_covered(position, ctx) {
            return false;
        }

        // On mobile WASM, request the soft keyboard when tapping on an editable text input.
        if self.display_options.editable && self.is_in_bounds(position, ctx) {
            ctx.request_soft_keyboard();
        }

        if let HitTestResult::Hit(location) = self.position_to_location(
            position,
            ClampBehavior::NoneIfOutBound,
            &Default::default(),
            app,
            ctx,
        ) && let Some(action) = V::Action::left_mouse_down(
            location,
            modifiers,
            click_count,
            is_first_mouse,
            &self.parent_view,
            app,
        ) {
            ctx.dispatch_typed_action(action);
            return true;
        }
        false
    }

    /// Performs hit-testing and dispatches mouse-drag events to the parent view.
    fn handle_left_mouse_dragged(
        &mut self,
        position: Vector2F,
        cmd: bool,
        shift: bool,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        if self.is_content_covered(position, ctx) {
            return false;
        }

        if let HitTestResult::Hit(location) = self.position_to_location(
            position,
            ClampBehavior::ClampWithBuffer(DRAG_POSITION_CLAMP_BUFFER),
            &HitTestOptions {
                // When dragging, we want to stick to text rather than converting to a block
                // selection.
                force_text_selection: true,
            },
            app,
            ctx,
        ) && let Some(action) =
            V::Action::left_mouse_dragged(location, cmd, shift, &self.parent_view, app)
        {
            ctx.dispatch_typed_action(action);
            return true;
        }
        false
    }

    /// Performs hit-testing and dispatches mouse-up events to the parent view.
    fn handle_left_mouse_up(
        &mut self,
        position: Vector2F,
        modifiers: &ModifiersState,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        if self.is_content_covered(position, ctx) {
            return false;
        }

        if let HitTestResult::Hit(location) = self.position_to_location(
            position,
            ClampBehavior::ClampToBound,
            &Default::default(),
            app,
            ctx,
        ) {
            let actions = V::Action::left_mouse_up(
                location,
                modifiers.cmd,
                modifiers.shift,
                &self.parent_view,
                app,
            );

            let handled = !actions.is_empty();
            for action in actions {
                ctx.dispatch_typed_action(action);
            }

            return handled;
        }
        false
    }

    /// Performs hit-testing and dispatches right-mouse-click events to the parent view.
    /// Used for context menu support.
    fn handle_right_mouse_down(
        &mut self,
        position: Vector2F,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        if self.is_content_covered(position, ctx) {
            return false;
        }

        if let HitTestResult::Hit(location) = self.position_to_location(
            position,
            ClampBehavior::NoneIfOutBound,
            &Default::default(),
            app,
            ctx,
        ) && let Some(action) = V::Action::right_mouse_down(location, &self.parent_view, app)
        {
            ctx.dispatch_typed_action(action);
            return true;
        }
        false
    }

    /// Dispatches user-typed characters to the parent view.
    fn handle_typed_characters(
        &mut self,
        chars: String,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        if self.vim_mode.is_some() {
            match V::Action::vim_user_typed(chars, &self.parent_view, app) {
                Some(action) => {
                    ctx.dispatch_typed_action(action);
                    true
                }
                None => false,
            }
        } else {
            match V::Action::user_typed(chars, &self.parent_view, app) {
                Some(action) => {
                    ctx.dispatch_typed_action(action);
                    true
                }
                None => false,
            }
        }
    }

    /// Updates display states in response to mouse movement.
    fn handle_mouse_moved(
        &mut self,
        position: Vector2F,
        cmd: bool,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        // Use the content z-index, rather than the max child z-index we use for hit testing.
        // This lets child UI elements cover the content layer, so that they can set their
        // own cursor.
        let Some(z_index) = self.content_z_index else {
            return false;
        };

        let in_bounds = self.is_in_bounds(position, ctx);
        // If the position is covered by a child UI element, do not consider the editor hovered.
        let is_covered = self.is_content_covered(position, ctx);

        let show_editor_cursor = in_bounds
            && !is_covered
            && self.display_options.editable
            && self.display_options.focused;
        let was_showing_cursor = self
            .display_state
            .show_editor_cursor
            .swap(show_editor_cursor, Ordering::Relaxed);

        if show_editor_cursor && !was_showing_cursor {
            ctx.set_cursor(Cursor::IBeam, z_index);
        } else if !show_editor_cursor && was_showing_cursor {
            ctx.reset_cursor();
        }

        // Store the entire location to support per-character hover states (e.g. hyperlinks).
        let hit_test = self.position_to_location(
            position,
            ClampBehavior::NoneIfOutBound,
            &Default::default(),
            app,
            ctx,
        );

        // If layout state is not yet available (transient during re-render), skip hover
        // processing entirely. This avoids dispatching spurious events that would
        // incorrectly clear hover state (e.g., the cmd-hover underline for goto-definition).
        if matches!(hit_test, HitTestResult::LayoutPending) {
            return false;
        }

        let new_location = match hit_test {
            HitTestResult::Hit(location) => Some(location),
            HitTestResult::OutOfBounds | HitTestResult::LayoutPending => None,
        };
        let mut location = self.display_state.hovered_location.lock();
        if location.as_ref() != new_location.as_ref() {
            location.clone_from(&new_location);
            if let Some(action) =
                V::Action::mouse_hovered(new_location, &self.parent_view, cmd, is_covered, app)
            {
                ctx.dispatch_typed_action(action);
            }
        }

        // Allow the event to continue propagating
        false
    }

    fn middle_click(&self, position: Vector2F, app: &AppContext, ctx: &mut EventContext) -> bool {
        let in_bounds = self.is_in_bounds(position, ctx);
        if in_bounds && let Some(action) = V::Action::middle_mouse_down(app) {
            ctx.dispatch_typed_action(action);
        }

        in_bounds
    }

    /// Builds a [`RenderableBlock`] for each block in the current viewport. This is done lazily,
    /// during layout.
    fn renderable_blocks(&mut self, styles: &RichTextStyles, ctx: &AppContext) {
        let parent = match self.parent_view.upgrade(ctx) {
            Some(handle) => handle.as_ref(ctx),
            None => {
                log::error!("Parent rich-text editor view dropped before layout");
                return;
            }
        };

        let model = self.model.as_ref(ctx);
        let mut ordered_list_numbering = model.viewport_list_numbering();
        let scroll_data = self.vertical_scroll_data(ctx);

        let content = model.content();
        let viewport_items = content.viewport_items(
            scroll_data.visible_px,
            model.viewport().width(),
            scroll_data.scroll_start,
        );

        let blocks = viewport_items
            .map(|(item, block)| {
                let renderable_block = match block {
                    BlockItem::Paragraph(_) => RenderableParagraph::new(item).finish(),
                    BlockItem::TextBlock { .. } => RenderableTextBlock::new(item).finish(),
                    BlockItem::Header { .. } => RenderableHeader::new(item).finish(),
                    BlockItem::UnorderedList { indent_level, .. } => {
                        RenderableBulletList::new(*indent_level, styles, item).finish()
                    }
                    BlockItem::TaskList {
                        complete,
                        mouse_state,
                        ..
                    } => RenderableTaskList::new(
                        *complete,
                        styles,
                        item,
                        mouse_state.clone(),
                        self.parent_view.clone(),
                    )
                    .finish(),
                    BlockItem::OrderedList {
                        indent_level,
                        number,
                        ..
                    } => {
                        let indent = indent_level.as_usize();
                        let number = ordered_list_numbering.advance(indent, *number).label_index;
                        RenderableOrderedListItem::new(*indent_level, item, number).finish()
                    }
                    BlockItem::RunnableCodeBlock { .. } => {
                        // For layout purposes, the start marker for the command block is considered
                        // the ending newline of the _previous_ block.
                        let start_offset = item.block_offset;
                        let runnable_command = parent.runnable_command_at(start_offset, ctx);
                        RenderableRunnableCommand::new(
                            item,
                            runnable_command,
                            self.display_options.focused,
                            ctx,
                        )
                        .finish()
                    }
                    BlockItem::MermaidDiagram { .. } => {
                        let start_offset = item.block_offset;
                        let runnable_command = parent.runnable_command_at(start_offset, ctx);
                        RenderableMermaidDiagram::new(
                            item,
                            runnable_command,
                            self.display_options.focused,
                            ctx,
                        )
                        .finish()
                    }
                    BlockItem::TemporaryBlock {
                        decoration,
                        text_decoration,
                        ..
                    } => RenderableTemporaryBlock::new(item, *decoration, text_decoration.clone())
                        .finish(),
                    BlockItem::HorizontalRule(_) => HorizontalRule::new(item).finish(),
                    BlockItem::Image { .. } => RenderableImage::new(item).finish(),
                    BlockItem::Table { .. } => RenderableTable::new(item).finish(),
                    BlockItem::TrailingNewLine(_) => Empty::new(item).finish(),
                    BlockItem::Hidden { .. } => RenderableHiddenSection::new(item, ctx).finish(),
                    BlockItem::Embedded(embed) => {
                        let start_offset = item.block_offset;
                        let child_model = parent.embedded_item_at(start_offset, ctx);
                        embed.element(model, item, child_model, ctx)
                    }
                };

                if !matches!(block, BlockItem::OrderedList { .. }) {
                    ordered_list_numbering.reset();
                }

                renderable_block
            })
            .collect();

        self.blocks = Some(blocks);
    }

    /// Updates the cursor-blinking state.
    fn update_blink_state(&self, ctx: &mut PaintContext) {
        if !self.display_options.blink_cursors {
            // Short-circuit if cursor blinking is disabled, since we won't use the blink state.
            return;
        }

        let now = Instant::now();
        let mut timer_guard = self.display_state.next_blink_update.lock();
        let update_deadline = timer_guard.unwrap_or(now);

        let next_update = if now >= update_deadline {
            // Every update interval, toggle the blink flag.
            self.display_state
                .blink_cursor_visible
                .fetch_xor(true, Ordering::Relaxed);

            now + CURSOR_BLINK_INTERVAL
        } else {
            update_deadline
        };

        *timer_guard = Some(next_update);
        ctx.repaint_after(next_update - now);
    }

    /// Whether or not blinking cursors are visible
    fn blinking_cursors_visible(&self) -> bool {
        !self.display_options.blink_cursors
            || self
                .display_state
                .blink_cursor_visible
                .load(Ordering::Relaxed)
    }

    /// The type of cursor to render depending on the vim mode
    fn cursor_display_type(&self) -> CursorDisplayType {
        self.vim_mode
            .map(|vim_mode| match vim_mode {
                VimMode::Normal | VimMode::Visual(_) => CursorDisplayType::Block,
                VimMode::Replace => CursorDisplayType::Underline,
                VimMode::Insert => CursorDisplayType::Bar,
            })
            .unwrap_or(CursorDisplayType::Bar)
    }
}

impl<V: EditorView> Element for RichTextElement<V> {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let model = self.model.as_ref(app);
        self.buffer_version = model.next_render_buffer_version();
        // Try flushing any pending edits at layout time (if we are performing layout lazily).
        self.pending_edits_flushed = model.try_layout_pending_edits(app);

        let size_buffer = vec2f(
            self.display_options.left_gutter + self.display_options.right_gutter,
            0.,
        );

        let mut constraint = constraint;

        if matches!(
            self.display_options.vertical_expansion_behavior,
            VerticalExpansionBehavior::GrowToMaxHeight | VerticalExpansionBehavior::InfiniteHeight
        ) {
            // If we should grow to the max height as more code is added, instead of filling all available space,
            // we should set the max height to be the shorter of the content height and the max height.
            constraint
                .max
                .set_y(constraint.max.y().min(model.height().as_f32()))
        }

        let size_info = model
            .viewport()
            .viewport_size(constraint, size_buffer, self.max_width);
        log::trace!(
            "Viewport size is {} within {}",
            size_info.viewport_size.display_size(),
            constraint.max.display_size()
        );
        self.viewport_size_info = Some(size_info);

        // The size of the editor element should always take up the entire constraint.
        let size = constraint.max;
        self.element_size = Some(size);

        self.renderable_blocks(model.styles(), app);
        match self.blocks.as_mut() {
            Some(blocks) => {
                for block in blocks.iter_mut() {
                    block.layout(model, ctx, app);
                }
            }
            None => log::error!("Rich-text blocks missing for layout"),
        }

        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        match self.blocks.as_mut() {
            Some(blocks) => {
                for block in blocks.iter_mut() {
                    block.after_layout(ctx, app);
                }
            }
            None => log::error!("Rich-text blocks missing after layout"),
        }

        // Even though this state is calculated in Self::layout, don't submit it until after all
        // views have been laid out.
        let layout_update = ElementUpdate {
            viewport_size: self.viewport_size_info,
            buffer_version: self.buffer_version,
            pending_edits_flushed: self.pending_edits_flushed,
        };
        self.model.as_ref(app).submit_element_update(layout_update);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let parent = match self.parent_view.upgrade(app) {
            Some(handle) => handle.as_ref(app),
            None => {
                log::error!("Parent rich-text editor view dropped before layout");
                return;
            }
        };
        if self.display_options.editable {
            self.update_blink_state(ctx);
        }

        let element_size = self
            .element_size
            .expect("Rich text element must have a size before painting");

        let viewport_size = self
            .viewport_size()
            .expect("Viewport size must be set before painting");

        let element_origin = Point::from_vec2f(origin, ctx.scene.z_index());
        // Save the element origin for hit testing.
        self.element_origin = Some(element_origin);

        // We clip to the element's own bounds, which include the gutters and margin, because this
        // simplifies hit-testing logic (the scene accounts for clipping when checking if a
        // position is in bounds). For rendering individual content, we shift to the viewport area.

        // Clip to the viewport bounds by creating a separate layer, similar to
        // the Clipped element. This clips content that's partially out of the
        // viewport.
        let Some(clip_bounds) = ctx.scene.visible_rect(element_origin, element_size) else {
            // If visible_rect returns None, we shouldn't paint anything.
            return;
        };
        ctx.scene
            .start_layer(warpui::ClipBounds::BoundedBy(clip_bounds));
        // Save the clipped content layer z-index for hover detection.
        self.content_z_index = Some(ctx.scene.z_index());

        let content_bounds = RectF::new(
            origin
                + self
                    .viewport_offset()
                    .expect("Viewport state should be set"),
            viewport_size,
        );

        if self.display_options.debug_bounds {
            let element_bounds = RectF::new(origin, element_size);

            ctx.scene
                .draw_rect_without_hit_recording(element_bounds)
                .with_border(Border::all(1.).with_border_color(ColorU::new(252, 144, 3, 255)));
            ctx.scene
                .draw_rect_without_hit_recording(content_bounds)
                .with_border(Border::all(1.).with_border_color(ColorU::new(252, 3, 232, 255)));
        }

        let model = self.model.as_ref(app);

        let mut ctx = RenderContext::new(
            content_bounds,
            self.display_options.focused,
            self.display_options.editable,
            self.blinking_cursors_visible(),
            self.cursor_display_type(),
            parent.text_decorations(
                model.viewport_charoffset_range(),
                model.next_render_buffer_version(),
                app,
            ),
            self.vertical_scroll_data(app).scroll_start.as_f32(),
            viewport_size,
            model,
            ctx,
            self.vim_mode,
            &self.vim_visual_tails,
        );

        model.record_text_selection(&mut ctx);

        match self.blocks.as_mut() {
            Some(blocks) => {
                for block in blocks.iter_mut() {
                    block.paint(model, &mut ctx, app);
                    if self.display_options.debug_bounds {
                        block.paint_bounds(&mut ctx);
                    }

                    if Some(block.viewport_item().block_offset)
                        == self.display_options.hovered_block_start
                        && let Some(block_bounds) = model.first_line_bounds(&**block, &ctx)
                    {
                        ctx.paint.position_cache.cache_position_indefinitely(
                            model.saved_positions().hovered_block_start(),
                            block_bounds,
                        );
                    }
                }
            }
            None => log::error!("Rich-text blocks missing after layout"),
        }

        ctx.paint.scene.stop_layer();
        self.child_max_z_index = Some(ctx.paint.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.element_size
    }

    fn origin(&self) -> Option<Point> {
        self.element_origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let Some(z_index) = self
            .child_max_z_index
            .or_else(|| self.element_origin.map(|origin| origin.z_index()))
        else {
            return false;
        };

        let mut block_handled = false;
        match self.blocks.as_mut() {
            Some(blocks) => {
                for block in blocks.iter_mut() {
                    block_handled |= block.dispatch_event(self.model.as_ref(app), event, ctx, app);
                }
            }
            None => log::error!("Rich-text blocks missing for event dispatching"),
        }

        match event.at_z_index(z_index, ctx) {
            Some(Event::LeftMouseDown {
                position,
                modifiers,
                click_count,
                is_first_mouse,
                ..
            }) if !block_handled => self.handle_left_mouse_down(
                *position,
                *modifiers,
                *click_count,
                *is_first_mouse,
                app,
                ctx,
            ),
            Some(Event::LeftMouseDragged {
                position,
                modifiers,
            }) if !block_handled => {
                self.handle_left_mouse_dragged(*position, modifiers.cmd, modifiers.shift, app, ctx)
            }
            Some(Event::LeftMouseUp {
                position,
                modifiers,
            }) if !block_handled => self.handle_left_mouse_up(*position, modifiers, app, ctx),
            Some(Event::TypedCharacters { chars }) if !block_handled => {
                self.handle_typed_characters(chars.clone(), app, ctx)
            }
            // For editor, we treat modifier state change as a mouse moved event. This allows us to trigger
            // any modifier-hover events (e.g. cmd-hover).
            Some(Event::ModifierStateChanged {
                mouse_position,
                modifiers,
                ..
            }) => self.handle_mouse_moved(*mouse_position, modifiers.cmd, app, ctx),
            // Always handle mouse-moved events, even if a block did. This is important for cursor state.
            Some(Event::MouseMoved { position, cmd, .. }) => {
                self.handle_mouse_moved(*position, *cmd, app, ctx)
            }
            Some(Event::MiddleMouseDown { position, .. }) => self.middle_click(*position, app, ctx),
            Some(Event::RightMouseDown { position, .. }) if !block_handled => {
                self.handle_right_mouse_down(*position, app, ctx)
            }
            _ => block_handled,
        }
    }
}

impl<V: EditorView> NewScrollableElement for RichTextElement<V> {
    fn scroll_data(&self, axis: Axis, app: &AppContext) -> Option<ScrollData> {
        Some(match axis {
            Axis::Horizontal => self.model.as_ref(app).scroll_data_horizontal(),
            Axis::Vertical => self.vertical_scroll_data(app),
        })
    }

    fn scroll(&mut self, delta: warpui::units::Pixels, axis: Axis, ctx: &mut EventContext) {
        if let Some(action) = V::Action::scroll(delta, axis) {
            ctx.dispatch_typed_action(action);
        }
    }

    fn axis_should_handle_scroll_wheel(&self, _axis: Axis) -> bool {
        true
    }

    fn axis(&self) -> ScrollableAxis {
        ScrollableAxis::Both
    }
}

// TODO(kevin): Deprecate the following once the new scrollable element stablized.
impl<V: EditorView> ScrollableElement for RichTextElement<V> {
    fn scroll_data(&self, app: &AppContext) -> Option<ScrollData> {
        Some(self.vertical_scroll_data(app))
    }

    fn scroll(&mut self, delta: warpui::units::Pixels, ctx: &mut EventContext) {
        if let Some(action) = V::Action::scroll(delta, Axis::Vertical) {
            ctx.dispatch_typed_action(action);
        }
    }

    fn should_handle_scroll_wheel(&self) -> bool {
        // For now, use the default scroll wheel behavior. We may need to revisit
        // this for horizontal scroll support.
        true
    }
}

impl fmt::Debug for Box<dyn RenderableBlock> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(self.type_name())
            .field("viewport_item", self.viewport_item())
            .finish()
    }
}
