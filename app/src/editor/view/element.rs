use super::super::soft_wrap::{
    ClampDirection, DisplayPointAndClampDirection, FrameLayouts, SoftWrapPoint, SoftWrapState,
};
use super::model::MarkedTextState;
use super::snapshot::VOICE_INPUT_ICON_CURSOR_GAP;
use super::{
    position_id_for_cached_point, snapshot::ViewSnapshot, CursorColors, DisplayPoint,
    DrawableSelection, EditorAction, ScrollState, SelectAction,
};
use super::{position_id_for_cursor, LocalDrawableSelectionData, ReplicaId};
use crate::appearance::Appearance;
use crate::editor::accept_autosuggestion_keybinding_view::{
    AcceptAutosuggestionKeybinding, AUTOSUGGESTION_HINT_MINIMUM_HEIGHT,
};
use crate::editor::autosuggestion_ignore_view::AutosuggestionIgnore;
use crate::editor::position_id_for_first_cursor;
use crate::settings::CursorDisplayType;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use itertools::Itertools;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use vim::vim::{MotionType, VimMode};
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::DEFAULT_UI_FONT_SIZE;
use warp_util::user_input::UserInput;
use warpui::event::KeyState;
use warpui::text_selection_utils::{
    calculate_tick_width, create_newline_tick_rect, selection_crosses_newline_row_based,
    NewlineTickParams,
};
use warpui::ViewHandle;
use warpui::{event::ModifiersState, text_layout::ComputeBaselinePositionArgs};

use crate::editor::view::AutosuggestionLocation;
use crate::themes::theme::Fill;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::{
    cmp, mem,
    ops::Range,
    sync::{Arc, Mutex},
    time::Duration,
};
use warpui::{
    elements::{
        AfterLayoutContext, CornerRadius, Element, Event, EventContext, LayoutContext,
        PaintContext, Point, SizeConstraint,
    },
    event::DispatchedEvent,
    keymap::Keystroke,
    text_layout::{self, LayoutCache, DEFAULT_TOP_BOTTOM_RATIO},
    ui_components::components::UiComponent,
    AppContext, SingletonEntity, TaskId,
};

use warpui::elements::{
    ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex, ParentElement, Text,
};
use warpui::platform::keyboard::KeyCode;

use instant::Instant;
use warpui::elements::{Radius, DEFAULT_UI_LINE_HEIGHT_RATIO};

// Similar to the terminal::model::ansi::CursorShape, this Editor Element has different cursor
// shapes. However, this element doesn't implement all the same variants, so we don't share that
// enum.
/// Width for cursor analogous to CursorShape::Beam.
const BEAM_CURSOR_WIDTH_PX: f32 = 3.;
/// Width for remote cursor is smaller than regular beam cursor to differentiate
/// between local and remote cursors.
const REMOTE_BEAM_CURSOR_WIDTH_PX: f32 = 2.;
/// Width for cursor analogous to CursorShape::Block.
const DEFAULT_BLOCK_CURSOR_WIDTH_PX: f32 = 8.;

const COMMAND_X_RAY_BOTTOM_PADDING_PX: f32 = 5.;
const COMMAND_X_RAY_HOVER_THRESHOLD_PX: f32 = 3.;
const COMMAND_X_RAY_HOVER_DELAY: Duration = Duration::from_millis(500);
const AUTOSUGGESTION_HINT_PADDING: f32 = 10.;

#[derive(Clone, Debug)]
pub struct CommandXRayMouseState {
    // The point at which the hover originated, in pixels
    pub hover_point: Vector2F,

    /// Whether the x-ray tooltip is visible
    pub visible: bool,

    /// The instant at which the x-ray should show
    pub hover_at: Instant,

    /// The timer id for the x-ray, in case we need to cancel it
    pub timer_id: TaskId,

    /// Whether the user has dismissed the hover through some action (e.g. esc or cmd-i or typing
    /// or scrolling)
    pub user_dismissed: bool,
}
pub type CommandXRayMouseStateHandle = Arc<Mutex<Option<CommandXRayMouseState>>>;

// Defined separately from LayoutState since LayoutState in used in broader
// methods such as paint_lines where we use the final computed line_height.
// In the future, we may add properties such as font kerning to this struct.
pub struct LineParameters {
    line_height: f32,
    cursor_height: f32,
}

/// Colors to use when rendering text in the editor.
#[derive(Clone, Debug)]
pub struct TextColors {
    /// The color to use for regular text.
    pub default_color: Fill,
    /// The color to use when the editor is disabled.
    pub disabled_color: Fill,
    /// The color to use for suggestions/hints.
    pub hint_color: Fill,
}

impl TextColors {
    /// Default text colors based on the theme.
    pub fn from_appearance(appearance: &Appearance) -> Self {
        let theme = appearance.theme();
        Self {
            default_color: theme.main_text_color(theme.background()),
            disabled_color: theme.disabled_text_color(theme.background()),
            hint_color: theme.hint_text_color(theme.background()),
        }
    }

    pub fn all_hint_color(appearance: &Appearance) -> Self {
        let theme = appearance.theme();
        let hint_color = theme.hint_text_color(theme.background());
        Self {
            default_color: hint_color,
            disabled_color: hint_color,
            hint_color,
        }
    }
}

#[derive(Default)]
pub struct EditorDecoratorElements {
    /// Arbitrary element that renders above the editor element. Currently used
    /// for the top n-1 lines of the lprompt.
    pub top_section: Option<Box<dyn Element>>,
    /// The left notch is an arbitrary element that is rendering at the
    /// top-left of the editor element. Note that we require this to be exactly
    /// 1 line high. The current use case for this is same-line prompt (this
    /// notch contains the nth, i.e. last, line of the lprompt).
    /// We start text painting right after this notch!
    pub left_notch: Option<Box<dyn Element>>,
    // Similarly, the right notch is currently used for the rprompt in the top-right,
    // but it could be any arbitrary element.
    pub right_notch: Option<Box<dyn Element>>,
    // The offset (in px) the right notch should be drawn, from the top-left of
    // the EditorElement (which is the top of the top section, if that exists).
    // If not specified, the default behavior is to draw the right notch at the right edge.
    pub right_notch_offset_px: Option<Vector2F>,
}

/// Structure holds the data necessary to draw cursors.
/// The origin and width of the cursor vary based on its position in the buffer.
struct CursorData {
    origin: Vector2F,
    /// The block cursor is used in Vim's normal mode and visual mode. The cursor should take on
    /// the exact width of the glyph it's on, so it may vary per-cursor.
    block_cursor_width: f32,
    color: Fill,
    /// Used to map the cursor data to its corresponding selections.
    replica_id: ReplicaId,
}

/// This type holds additional information about how to draw a remote peer's
/// selections and cursors, specifically within the editor element.
pub struct RemoteDrawableSelectionData {
    pub colors: CursorColors,
    pub should_draw_cursors: bool,
    // In contrast to [`super:: RemoteDrawableSelectionData`], the avatar is converted to an element to ensure it can be laid out.
    pub avatar: Box<dyn Element>,
}

impl From<super::RemoteDrawableSelectionData> for RemoteDrawableSelectionData {
    fn from(value: super::RemoteDrawableSelectionData) -> Self {
        Self {
            colors: value.colors,
            should_draw_cursors: value.should_draw_cursors,
            avatar: value.avatar.build().finish(),
        }
    }
}

/// Element that represents a text editor.
pub struct EditorElement {
    view_snapshot: ViewSnapshot,
    scroll_state: ScrollState,
    layout: Option<LayoutState>,
    paint: Option<PaintState>,
    mouse_state: CommandXRayMouseStateHandle,
    preferred_cursor_type: CursorDisplayType,
    soft_wrap: bool,
    /// Whether the placeholder text should soft wrap
    placeholder_soft_wrap: bool,
    /// Even when soft wrapping is off, we still use information about the
    /// laid out text in our code (the same logic just happens to work with
    /// or without soft wrapping).
    soft_wrap_state: SoftWrapState,
    autosuggestion_shortcut_icon: Option<Box<dyn Element>>,
    autosuggestion_ignore_icon: Option<Box<dyn Element>>,
    cycle_next_command_hint: Option<Box<dyn Element>>,

    vim_mode: Option<VimMode>,
    text_colors: TextColors,
    editor_decorator_elements: EditorDecoratorElements,
    local_selection_data: LocalDrawableSelectionData,
    remote_selections_data: HashMap<ReplicaId, RemoteDrawableSelectionData>,

    voice_input_cursor_icon: Option<Box<dyn Element>>,
    #[cfg_attr(not(feature = "voice_input"), allow(unused))]
    voice_input_toggle_key_code: Option<KeyCode>,
}

impl EditorElement {
    /// Creates a new editor element.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        view_snapshot: ViewSnapshot,
        scroll_state: ScrollState,
        mouse_state: CommandXRayMouseStateHandle,
        soft_wrap: bool,
        soft_wrap_state: SoftWrapState,
        placeholder_soft_wrap: bool,
        vim_mode: Option<VimMode>,
        text_colors: TextColors,
        editor_decorator_elements: EditorDecoratorElements,
        local_selection_data: LocalDrawableSelectionData,
        remote_selections_data: HashMap<ReplicaId, super::RemoteDrawableSelectionData>,
        cursor_display_type: Option<CursorDisplayType>,
        voice_input_toggle_key_code: Option<KeyCode>,
    ) -> Self {
        let remote_selections_data = HashMap::from_iter(remote_selections_data.into_iter().map(
            |(replica_id, drawable_selections_data)| {
                (
                    replica_id,
                    RemoteDrawableSelectionData::from(drawable_selections_data),
                )
            },
        ));

        Self {
            view_snapshot,
            scroll_state,
            layout: None,
            paint: None,
            mouse_state,
            soft_wrap,
            placeholder_soft_wrap,
            soft_wrap_state,
            autosuggestion_shortcut_icon: None,
            autosuggestion_ignore_icon: None,
            vim_mode,
            text_colors,
            editor_decorator_elements,
            local_selection_data,
            remote_selections_data,
            preferred_cursor_type: cursor_display_type.unwrap_or_default(),
            cycle_next_command_hint: None,
            voice_input_cursor_icon: None,
            voice_input_toggle_key_code,
        }
    }

    /// Returns whether or not a given replica id is a local peer.
    fn is_local_replica(&self, replica_id: &ReplicaId, app: &AppContext) -> bool {
        replica_id == &self.view_snapshot.editor_model.as_ref(app).replica_id(app)
    }

    fn scroll_position(&self) -> Vector2F {
        *self.scroll_state.scroll_position.lock()
    }

    /// Handles selection actions.
    fn mouse_down(
        &self,
        position: Vector2F,
        modifiers: &ModifiersState,
        is_first_mouse: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if !self.view_snapshot.can_select {
            return false;
        }
        let layout = self.layout.as_ref().unwrap();
        let paint = self.paint.as_ref().unwrap();
        if paint.rect.contains_point(position) {
            ctx.dispatch_typed_action(EditorAction::Focus);
            ctx.dispatch_typed_action(EditorAction::ClearParentSelections);

            // On mobile WASM, request the soft keyboard when tapping on an editable editor.
            #[cfg(target_family = "wasm")]
            if self.view_snapshot.editor_model.as_ref(app).can_edit() {
                ctx.request_soft_keyboard();
            }

            if is_first_mouse {
                // If the editor is receiving the first mouse click on activation
                // we want to focus the editor but avoid starting any selections.
                // No mouse up event is sent for the first mouse click, so we would
                // otherwise get stuck in a selecting state.
                return true;
            }
            let point_for_position = paint.possible_point_for_position(
                &self.view_snapshot,
                self.scroll_position(),
                layout,
                position,
            );
            if modifiers.shift {
                ctx.dispatch_typed_action(EditorAction::Select(SelectAction::Extend {
                    position: point_for_position.display_point,
                    scroll_position: paint.scroll_position(
                        &self.view_snapshot,
                        &self.scroll_state,
                        layout,
                        position,
                        ctx,
                        app,
                    ),
                }));
            } else {
                let position = DisplayPointAndClampDirection {
                    point: point_for_position.display_point,
                    clamp_direction: point_for_position.clamp_direction,
                };
                ctx.dispatch_typed_action(EditorAction::Select(SelectAction::Begin {
                    position,
                    add: if cfg!(target_os = "macos") {
                        modifiers.cmd
                    } else {
                        modifiers.alt
                    },
                }));
            }
            true
        } else {
            false
        }
    }

    /// Handles selection actions.
    fn double_mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        if !self.view_snapshot.can_select {
            return false;
        }
        let layout = self.layout.as_ref().unwrap();
        let paint = self.paint.as_ref().unwrap();
        if paint.rect.contains_point(position) {
            let position = paint.point_for_position(
                &self.view_snapshot,
                self.scroll_position(),
                layout,
                position,
            );
            ctx.dispatch_typed_action(EditorAction::SelectWord(position));
            true
        } else {
            false
        }
    }

    /// Handles selection actions.
    fn triple_mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        if !self.view_snapshot.can_select {
            return false;
        }
        let layout = self.layout.as_ref().unwrap();
        let paint = self.paint.as_ref().unwrap();
        if paint.rect.contains_point(position) {
            let position = paint.point_for_position(
                &self.view_snapshot,
                self.scroll_position(),
                layout,
                position,
            );
            ctx.dispatch_typed_action(EditorAction::SelectLine(position));
            true
        } else {
            false
        }
    }

    fn mouse_up(&self, _position: Vector2F, ctx: &mut EventContext, app: &AppContext) -> bool {
        if self.view_snapshot.is_selecting(app) {
            ctx.dispatch_typed_action(EditorAction::Select(SelectAction::End));
            true
        } else {
            false
        }
    }

    /// Handles selection actions.
    fn mouse_dragged(&self, position: Vector2F, ctx: &mut EventContext, app: &AppContext) -> bool {
        let layout = self.layout.as_ref().unwrap();
        let paint = self.paint.as_ref().unwrap();

        if self.view_snapshot.is_selecting(app) {
            ctx.dispatch_typed_action(EditorAction::Select(SelectAction::Update {
                position: paint.point_for_position(
                    &self.view_snapshot,
                    self.scroll_position(),
                    layout,
                    position,
                ),
                scroll_position: paint.scroll_position(
                    &self.view_snapshot,
                    &self.scroll_state,
                    layout,
                    position,
                    ctx,
                    app,
                ),
            }));
            true
        } else {
            false
        }
    }

    fn key_down(&self, keystroke: &Keystroke, ctx: &mut EventContext) -> bool {
        if self.view_snapshot.is_focused && (keystroke.cmd || keystroke.ctrl) {
            // Ctrl and cmd should be handled via key bindings
            ctx.dispatch_typed_action(EditorAction::UnhandledModifierKey(Arc::new(
                keystroke.normalized(),
            )));
        }
        false
    }

    fn modifier_key_change(
        &self,
        key_code: &KeyCode,
        state: &KeyState,
        ctx: &mut EventContext,
    ) -> bool {
        cfg_if::cfg_if! {
            if #[cfg(feature = "voice_input")] {
                self.maybe_handle_voice_toggle(key_code, state, ctx);
            } else {
                // Silence unused param warnings when voice_input is disabled.
                let _ = (key_code, state, ctx);
            }
        }
        false
    }

    #[cfg(feature = "voice_input")]
    fn maybe_handle_voice_toggle(
        &self,
        key_code: &KeyCode,
        state: &KeyState,
        ctx: &mut EventContext,
    ) {
        if let Some(voice_input_toggle_key_code) = self.voice_input_toggle_key_code {
            if *key_code == voice_input_toggle_key_code {
                ctx.dispatch_typed_action(EditorAction::ToggleVoiceInput(
                    voice_input::VoiceInputToggledFrom::Key { state: *state },
                ));
            }
        }
    }

    fn mouse_moved(
        &mut self,
        position: Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let layout = self
            .layout
            .as_ref()
            .expect("layout should be set at event handling");
        let paint = self
            .paint
            .as_ref()
            .expect("paint should be set at event handling");

        let mut state_guard = self.mouse_state.lock().expect("mouse state lock");
        if let Some(state) = &mut *state_guard {
            if state.user_dismissed {
                if (state.hover_point - position).length() < COMMAND_X_RAY_HOVER_THRESHOLD_PX {
                    // Early exit if some user action has caused the x-ray tooltip to be dismissed
                    // and the mouse hasn't moved.
                    return false;
                } else {
                    return self.reset_x_ray(&mut state_guard, Some(position), ctx);
                }
            }
        }

        if !paint.rect.contains_point(position) {
            // Mouse is outside of the editor, so clear any pending x-ray and exit early
            return self.reset_x_ray(&mut state_guard, None, ctx);
        }

        let possible_point = paint.possible_point_for_position(
            &self.view_snapshot,
            self.scroll_position(),
            layout,
            position,
        );

        if let Some(state) = &mut *state_guard {
            let within_last_mouse_move_radius =
                (state.hover_point - position).length() < COMMAND_X_RAY_HOVER_THRESHOLD_PX;
            let is_within_word_boundary =
                self.is_position_within_x_ray_token_bounds(&possible_point, app);

            // Case 1: the command xray tooltip is open. We only want to close it if:
            //  - the cursor is not within the word boundary for the token being described
            //  - and the cursor is more than a radius away from the token
            // The latter condition ensures that we only close the tooltip when
            // there is a substantial mouse movement (if the cursor is only slightly
            // moved, even outside of the word boundary, we still want to keep it open).
            if state.visible && !is_within_word_boundary && !within_last_mouse_move_radius {
                return self.reset_x_ray(&mut state_guard, Some(position), ctx);
            } else if !state.visible {
                // Case 2: the command xray tooltip is not open yet. We only want to open it if
                //  - enough time has elapsed
                //  - the mouse is still within the same radius as it was before
                //  - the point we are considering isn't a clamped point (since we don't
                //    want to describe the last token if the cursor is actually well past the buffer text)
                let timer_elapsed = Instant::now() >= state.hover_at;
                if timer_elapsed && within_last_mouse_move_radius && !possible_point.is_clamped {
                    ctx.dispatch_typed_action(EditorAction::TryToShowXRay(
                        possible_point.display_point,
                    ));
                    ctx.clear_notify_timer(state.timer_id);
                    state.visible = true;
                    return true;
                } else if !within_last_mouse_move_radius {
                    // Case 3: the command xray tooltip is not open yet. We should reset the
                    // state as long as the mouse has moved more than a radius away since the
                    // last tracked mouse position.
                    return self.reset_x_ray(&mut state_guard, Some(position), ctx);
                }
            }
        } else {
            // Case 4: No timer set, set a new one
            return self.reset_x_ray(&mut state_guard, Some(position), ctx);
        }

        false
    }

    fn middle_mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        if self
            .paint
            .as_ref()
            .is_some_and(|paint| paint.rect.contains_point(position))
        {
            ctx.dispatch_typed_action(EditorAction::MiddleClickPaste);
            return true;
        }
        false
    }

    /// Returns whether the given mouse position is within the bounds of the current
    /// token being used for command x-ray
    fn is_position_within_x_ray_token_bounds(
        &self,
        point: &PossibleDisplayPoint,
        app: &AppContext,
    ) -> bool {
        if point.is_clamped {
            return false;
        }

        if let Some(description) = &self.view_snapshot.command_xray {
            let start_byte = description.token.span.start();
            let end_byte = description.token.span.end();
            let position_offset = self
                .view_snapshot
                .byte_offset_at_point(&point.display_point, app);
            if let Some(position_offset) = position_offset {
                return position_offset.as_usize() >= start_byte
                    && position_offset.as_usize() < end_byte;
            }
        }
        false
    }

    /// Resets the timer and (optional) position for triggering command x-ray
    fn reset_x_ray(
        &self,
        x_ray_state: &mut Option<CommandXRayMouseState>,
        new_position: Option<Vector2F>,
        ctx: &mut EventContext,
    ) -> bool {
        let mut updated = false;
        if let Some(state) = &mut *x_ray_state {
            if state.visible {
                ctx.dispatch_typed_action(EditorAction::HideXRay);
                updated = true;
            }
            ctx.clear_notify_timer(state.timer_id);
        }

        if let Some(position) = new_position {
            let (timer_id, hover_at) = ctx.notify_after(COMMAND_X_RAY_HOVER_DELAY);
            *x_ray_state = Some(CommandXRayMouseState {
                visible: false,
                hover_point: position,
                hover_at,
                timer_id,
                user_dismissed: false,
            });
        } else {
            *x_ray_state = None;
        }
        updated
    }

    fn typed_characters(&self, chars: &str, ctx: &mut EventContext) -> bool {
        if self.view_snapshot.is_focused {
            if self.vim_mode.is_some() {
                ctx.dispatch_typed_action(EditorAction::VimUserInsert(UserInput::new(chars)));
            } else {
                ctx.dispatch_typed_action(EditorAction::UserInsert(UserInput::new(chars)));
            }

            return true;
        }
        false
    }

    fn set_marked_text(
        &mut self,
        marked_text: &str,
        selected_range: &Range<usize>,
        ctx: &mut EventContext,
    ) -> bool {
        if self.view_snapshot.is_focused {
            ctx.dispatch_typed_action(EditorAction::SetMarkedText {
                marked_text: UserInput::new(marked_text),
                selected_range: selected_range.clone(),
            });
            return true;
        }
        false
    }

    fn clear_marked_text(&mut self, ctx: &mut EventContext) -> bool {
        if self.view_snapshot.is_focused {
            ctx.dispatch_typed_action(EditorAction::ClearMarkedText);
            return true;
        }
        false
    }

    fn drag_and_drop_file(
        &self,
        paths: Vec<String>,
        location: Vector2F,
        ctx: &mut EventContext,
    ) -> bool {
        if self
            .paint
            .as_ref()
            .is_some_and(|paint| paint.rect.contains_point(location))
            && self.view_snapshot.is_focused
        {
            let paths = paths.into_iter().map(UserInput::new).collect();

            ctx.dispatch_typed_action(EditorAction::DragAndDropFiles(paths));
            return true;
        }
        false
    }

    /// Handles mouse wheel and trackpad events
    fn scroll(
        &self,
        position: Vector2F,
        delta: Vector2F,
        precise: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let paint = self.paint.as_ref().unwrap();

        if !paint.rect.contains_point(position) {
            return false;
        }

        let view_snapshot = &self.view_snapshot;
        let layout_cache = &ctx.text_layout_cache;

        let y;
        let x;
        if precise {
            let max_glyph_width = view_snapshot.em_width;
            let line_height = view_snapshot.line_height;
            x = (self.scroll_position().x() * max_glyph_width - delta.x()) / max_glyph_width;
            y = (self.scroll_position().y() * line_height - delta.y()) / line_height;
        } else {
            x = self.scroll_position().x() - delta.x();
            y = self.scroll_position().y() - delta.y()
        }

        let scroll_position = vec2f(x, y).clamp(
            Vector2F::zero(),
            self.layout
                .as_ref()
                .expect("should have layout")
                .scroll_max(view_snapshot, layout_cache, app),
        );

        // Don't handle this event if this would be a noop so that a parent element
        // (such as the WaterFallGapElement) can properly handle scrolling
        if scroll_position != self.scroll_position() {
            ctx.dispatch_typed_action(EditorAction::Scroll(scroll_position));
            true
        } else {
            false
        }
    }

    fn x_ray_position_id(&self) -> String {
        format!("editor:command_x_ray_{}", self.view_snapshot.view_id)
    }

    fn visible_selection_range(
        selection: &mut Range<SoftWrapPoint>,
        visible_start: SoftWrapPoint,
        visible_end: SoftWrapPoint,
    ) -> Range<u32> {
        if selection.start > selection.end {
            mem::swap(&mut selection.start, &mut selection.end)
        }
        let visible_selection_start_row = cmp::max(selection.start.row(), visible_start.row());
        let visible_selection_end_row = cmp::min(selection.end.row(), visible_end.row());
        // If selection.end evaluates to visible_end (column 0), its row is not visible.
        // So we can skip painting it.
        if visible_selection_end_row == visible_end.row() && selection.end.column() == 0 {
            visible_selection_start_row..visible_selection_end_row
        } else {
            visible_selection_start_row..visible_selection_end_row + 1
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_selection(
        &self,
        content_origin: Vector2F,
        row: u32,
        first_visible_row: u32,
        color: Fill,
        selection: &Range<SoftWrapPoint>,
        line_height: f32,
        line_layout: &text_layout::Line,
        layout: &LayoutState,
        ctx: &mut PaintContext,
    ) {
        // Text content origin defines where we actually began painting the text, as opposed to content
        // origin which is the origin of the entire EditorElement. We push over the text content origin
        // if we're looking at the 0th row by the width of the left notch!
        let text_content_origin = if row == 0 {
            content_origin + vec2f(layout.left_notch_layout_width_px, 0.)
        } else {
            content_origin
        };

        let start_x = if row == selection.start.row() {
            line_layout.x_for_index(selection.start.column() as usize)
        } else {
            0.
        };
        let end_x = if row == selection.end.row() {
            line_layout.x_for_index(selection.end.column() as usize)
        } else {
            line_layout.width
        };

        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(
                text_content_origin
                    + vec2f(start_x, (row - first_visible_row) as f32 * line_height),
                vec2f(end_x - start_x, line_height),
            ))
            .with_background(color);

        let max_soft_wrap_row = layout.frame_layouts.num_lines().saturating_sub(1) as u32;
        let is_last_line = row == max_soft_wrap_row;
        let selection_crosses_newline = selection_crosses_newline_row_based(
            row as usize,
            is_last_line,
            selection.start.row() as usize,
            selection.end.row() as usize,
            selection.end.column() as usize,
            line_layout.end_index(),
        );
        if selection_crosses_newline {
            let tick_width = calculate_tick_width(line_layout.font_size);
            let tick_origin = text_content_origin
                + vec2f(
                    line_layout.width,
                    (row - first_visible_row) as f32 * line_height,
                );
            ctx.scene
                .draw_rect_with_hit_recording(create_newline_tick_rect(NewlineTickParams {
                    tick_origin,
                    tick_width,
                    tick_height: line_height,
                }))
                .with_background(color);
        }
    }

    /// Returns the height that the cursor should be.
    /// Note that the cursor's origin is NOT the top of the line - it is the "correct"
    /// spot at the top of the text. This means the cursor's height should be no larger than
    /// it would be at DEFAULT_UI_LINE_HEIGHT_RATIO.
    /// We do need to account for smaller line heights to prevent the cursor from extending too far down.
    pub fn cursor_height(font_size: f32, line_height_ratio: f32) -> f32 {
        font_size * DEFAULT_UI_LINE_HEIGHT_RATIO.min(line_height_ratio)
    }

    /// Draws cursors and avatars for local and remote peers.
    fn draw_cursors(
        cursor_display_type: CursorDisplayType,
        cursors: SmallVec<[CursorData; 32]>,
        view_snapshot: &ViewSnapshot,
        remote_selections_data: &mut HashMap<ReplicaId, RemoteDrawableSelectionData>,
        voice_input_icon: &mut Option<Box<dyn Element>>,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        let cursor_height =
            Self::cursor_height(view_snapshot.font_size, view_snapshot.line_height_ratio);
        let cursor_corner_radius = match cursor_display_type {
            CursorDisplayType::Block | CursorDisplayType::Underline => Radius::Pixels(0.),
            _ => Radius::Percentage(50.),
        };
        for cursor in cursors {
            let is_local =
                cursor.replica_id == view_snapshot.editor_model.as_ref(app).replica_id(app);
            let cursor_width = match cursor_display_type {
                CursorDisplayType::Block | CursorDisplayType::Underline => {
                    cursor.block_cursor_width
                }
                _ if !is_local => REMOTE_BEAM_CURSOR_WIDTH_PX,
                _ => BEAM_CURSOR_WIDTH_PX,
            };
            let mut cursor_rect = RectF::new(cursor.origin, vec2f(cursor_width, cursor_height));

            if cursor_display_type == CursorDisplayType::Underline {
                cursor_rect.set_origin_y(cursor_rect.origin_y() + view_snapshot.font_size);
            };

            ctx.scene
                .draw_rect_with_hit_recording(cursor_rect)
                .with_background(cursor.color)
                .with_corner_radius(CornerRadius::with_all(cursor_corner_radius));

            // Draw cursor avatars for remote selections
            if !is_local {
                if let Some(drawable_selections_data) =
                    remote_selections_data.get_mut(&cursor.replica_id)
                {
                    // Offset for avatar's x origin is calculated based on avatar's size, border and cursor width.
                    // We include half of the cursor's width to ensure the avatar is centered with the cursor's center
                    // and not just the cursor's x origin.
                    let avatar_size = view_snapshot.cursor_avatar_size() + 2.;
                    let avatar_offset = avatar_size / 2. - cursor_width / 2.;
                    let avatar_origin = vec2f(
                        cursor.origin.x() - avatar_offset,
                        cursor.origin.y() - cursor_height,
                    );
                    // New layer is started so avatars are rendered over text and prompt
                    ctx.scene.start_layer(warpui::ClipBounds::None);
                    drawable_selections_data
                        .avatar
                        .paint(avatar_origin, ctx, app);
                    ctx.scene.stop_layer();
                }
            }

            if let Some(element) = voice_input_icon {
                let icon_size = view_snapshot.voice_input_icon_size();
                let icon_x_offset = icon_size.x() / 2. - cursor_width / 2.;
                let icon_origin = vec2f(
                    cursor.origin.x() - icon_x_offset,
                    cursor.origin.y() - icon_size.y() - VOICE_INPUT_ICON_CURSOR_GAP,
                );
                // New layer is started so voice icon is rendered over text and prompt
                ctx.scene.start_layer(warpui::ClipBounds::None);
                element.paint(icon_origin, ctx, app);
                ctx.scene.stop_layer();
            }
        }
    }

    fn cursor_origin_vertical_adjustment_from_text_content_origin(
        view_snapshot: &ViewSnapshot,
        compute_baseline_position_args: ComputeBaselinePositionArgs,
    ) -> Vector2F {
        vec2f(
            0.,
            // 1. Go down by baseline position (which is 80% of line height due to top bottom ratio).
            // Note that we do NOT have a particular Run (ie font ID) here, so we ALWAYS use the default Line-style
            // baseline offset calculation for text selections.
            (view_snapshot.baseline_position_fn()(
                compute_baseline_position_args,
            )
            // 2. Go up by font size * 80% (top bottom ratio) * 1.2 (default line height ratio) which
            // gets us to a good point for the cursor origin (the same exact point in default case).
                - (view_snapshot.font_size
                    * DEFAULT_UI_LINE_HEIGHT_RATIO
                    * DEFAULT_TOP_BOTTOM_RATIO))
                .max(0.0),
        )
    }
    /// This is a helper method responsible for drawing lines within the editor (split into
    /// multiple selections). It returns vector of cursor positions (1 for every selection).
    fn draw_selections_and_get_cursor_data(
        &self,
        content_origin: Vector2F,
        bounds: RectF,
        layout: &LayoutState,
        line_parameters: LineParameters,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) -> SmallVec<[CursorData; 32]> {
        let appearance = Appearance::as_ref(app);
        let fallback_block_cursor_width = DEFAULT_BLOCK_CURSOR_WIDTH_PX
            * (appearance.monospace_font_size() / DEFAULT_UI_FONT_SIZE);
        let view_snapshot = &self.view_snapshot;
        let marked_text_state = view_snapshot
            .editor_model
            .as_ref(app)
            .buffer(app)
            .marked_text_state();
        let first_visible_row = Self::first_visible_row(
            self.scroll_position().y(),
            view_snapshot.line_height,
            layout.top_section_height_px,
        );

        let scroll_top = self.scroll_position().y() * line_parameters.line_height;
        let visible_end_row =
            ((scroll_top + bounds.height()) / line_parameters.line_height).ceil() as u32;
        let vim_visual_tails = view_snapshot.vim_visual_tails(app).collect_vec();

        let mut cursors = SmallVec::<[CursorData; 32]>::new();

        // Looping over all selections.
        for (
            i,
            DrawableSelection {
                range,
                clamp_direction,
                replica_id,
            },
        ) in view_snapshot
            .all_drawable_selections_intersecting_range(
                DisplayPoint::new(0, 0)..view_snapshot.max_point(app),
                app,
            )
            .enumerate()
        {
            let start = layout
                .frame_layouts
                .to_soft_wrap_point(range.start, clamp_direction);
            let end = layout
                .frame_layouts
                .to_soft_wrap_point(range.end, clamp_direction);

            let is_local_replica = self.is_local_replica(&replica_id, app);
            let (should_draw_cursors, colors) = if is_local_replica {
                let data = &self.local_selection_data;
                (data.should_draw_cursors, data.colors)
            } else if let Some(data) = self.remote_selections_data.get(&replica_id) {
                (data.should_draw_cursors, data.colors)
            } else {
                log::warn!("Failed to access selections data with replica id");
                continue;
            };

            // Only attempt to draw the selection if both the start and end points exist.
            // There shouldn't be a case where we don't, but it's better to be safe.
            if let (Some(start), Some(end)) = (start, end) {
                let mut selection = start..end;
                let cursor_position = selection.end;
                let text_content_origin = if cursor_position.row() == 0 {
                    content_origin + vec2f(layout.left_notch_layout_width_px, 0.)
                } else {
                    content_origin
                };

                if (first_visible_row..visible_end_row).contains(&cursor_position.row()) {
                    let index = selection.end.row() as usize;
                    let cursor_row_layout = match layout.frame_layouts.get_line(index) {
                        Some(layout) => layout,
                        None => {
                            log::warn!("Attempting to access line {index}, but there are fewer lines in the layout.");
                            continue;
                        }
                    };
                    let cursor_x_index = match &marked_text_state {
                        MarkedTextState::Active { selected_range } => {
                            // The marked text selected range assumes that the marked text starts at 0.
                            // Adjust the cursor position to match what the IME tells us.
                            let ime_offset = selected_range.end;
                            selection.start.column() as usize + ime_offset
                        }
                        MarkedTextState::Inactive => selection.end.column() as usize,
                    };
                    // Use baseline position to get to bottom of text line, then subtract the font size to
                    // get to top of text. We have the multipliers of default line height ratio and top bottom ratio
                    // to get to the "correct" spot above the normal characters within a font.
                    // Note that we don't want to start from top of line (don't want
                    // a massive cursor for large line heights).
                    let cursor_origin = text_content_origin
                        + vec2f(
                            0.,
                            // 1. Go down by baseline position (which is 80% of line height due to top bottom ratio).
                            // Note that we do NOT have a particular Run (ie font ID) here, so we ALWAYS use the default Line-style
                            // baseline offset calculation for text selections.
                            (view_snapshot.baseline_position_fn()(
                                ComputeBaselinePositionArgs {
                                    font_cache: ctx.font_cache,
                                    font_size: cursor_row_layout.font_size,
                                    line_height_ratio: cursor_row_layout.line_height_ratio,
                                    baseline_ratio: cursor_row_layout.baseline_ratio,
                                    ascent: cursor_row_layout.ascent,
                                    descent: cursor_row_layout.descent,
                                },
                            )
                            // 2. Go up by font size * 80% (top bottom ratio) * 1.2 (default line height ratio) which
                            // gets us to a good point for the cursor origin (the same exact point in default case).
                                - (view_snapshot.font_size
                                    * DEFAULT_UI_LINE_HEIGHT_RATIO
                                    * DEFAULT_TOP_BOTTOM_RATIO))
                                .max(0.0),
                        )
                        + vec2f(
                            cursor_row_layout.x_for_index(cursor_x_index),
                            (selection.end.row() - first_visible_row) as f32
                                * line_parameters.line_height,
                        );

                    let block_cursor_width = cursor_row_layout
                        .width_for_index(selection.end.column() as usize)
                        .filter(|value| *value > 0.)
                        .unwrap_or(fallback_block_cursor_width);

                    // `should_draw_cursors` is set differently for local and remote cursors:
                    // * remote cursors are drawn based on their block selections
                    // * local cursors are drawn based on their blink setting,
                    //   focus on active window, and interaction state
                    if should_draw_cursors {
                        cursors.push(CursorData {
                            origin: cursor_origin,
                            block_cursor_width,
                            color: colors.cursor,
                            replica_id,
                        });
                    }
                    // Ensure cursor position is always cached when local
                    if is_local_replica {
                        let cursor_size = vec2f(block_cursor_width, line_parameters.cursor_height);
                        ctx.position_cache.cache_position_indefinitely(
                            position_id_for_cursor(view_snapshot.view_id),
                            RectF::new(cursor_origin, cursor_size),
                        );

                        if i == 0 {
                            ctx.position_cache.cache_position_indefinitely(
                                position_id_for_first_cursor(view_snapshot.view_id),
                                RectF::new(cursor_origin, cursor_size),
                            );
                        }
                    }
                }

                let visible_start = SoftWrapPoint::new(first_visible_row, 0);
                let visible_end = SoftWrapPoint::new(visible_end_row, 0);
                if selection.start != selection.end {
                    let visible_selection_range =
                        Self::visible_selection_range(&mut selection, visible_start, visible_end);

                    for row in visible_selection_range {
                        let Some(line_layout) = layout.frame_layouts.get_line(row as usize) else {
                            continue;
                        };

                        let selection_to_draw = match &marked_text_state {
                            MarkedTextState::Active { selected_range }
                                if !selected_range.is_empty() =>
                            {
                                // This is the case where we have a selected range within marked text.
                                // This selected range needs to be highlighted.
                                // Since we model the marked text itself as a selection in the editor,
                                // we need to generate the "real" selection here.
                                let marked_text_selection_start = SoftWrapPoint::new(
                                    selection.start.row(),
                                    selection.start.column() + selected_range.start as u32,
                                );
                                let marked_text_selection_end = SoftWrapPoint::new(
                                    selection.start.row(),
                                    selection.start.column() + selected_range.end as u32,
                                );
                                let marked_text_selection =
                                    marked_text_selection_start..marked_text_selection_end;
                                Some(marked_text_selection)
                            }
                            MarkedTextState::Inactive => Some(selection.clone()),
                            _ => None,
                        };

                        if let Some(selection_to_draw) = selection_to_draw {
                            self.draw_selection(
                                content_origin,
                                row,
                                first_visible_row,
                                colors.selection,
                                &selection_to_draw,
                                line_parameters.line_height,
                                line_layout,
                                layout,
                                ctx,
                            );
                        }
                    }
                } else if let Some(VimMode::Visual(motion_type)) = self.vim_mode {
                    // If we're in Vim visual mode, render the visual mode selection which isn't
                    // the same as the [`crate::editor::view::Selection`].
                    let Some(visual_tail) = vim_visual_tails.get(i) else {
                        continue;
                    };
                    let Some(visual_tail) = layout
                        .frame_layouts
                        .to_soft_wrap_point(*visual_tail, clamp_direction)
                    else {
                        continue;
                    };
                    let mut visual_range = visual_tail..selection.end;
                    if visual_range.start > visual_range.end {
                        mem::swap(&mut visual_range.start, &mut visual_range.end)
                    }
                    match motion_type {
                        // In charwise visual mode, add 1 to account for the fact that the char
                        // under the block cursor gets included in the selection range.
                        MotionType::Charwise => *visual_range.end.column_mut() += 1,
                        // In linewise visual mode, extend the start and end points to the start
                        // and end of the line respectively.
                        MotionType::Linewise => {
                            // This calculation must be done in "DisplayPoint-space" since visual
                            // line mode doesn't respect soft wrap in Vim.
                            let mut start =
                                layout.frame_layouts.to_display_point(visual_range.start);
                            let mut end = layout.frame_layouts.to_display_point(visual_range.end);

                            // Move the start to the beginning of the line, always column 0.
                            *start.point.column_mut() = 0;

                            // Moving to the line end is the same as setting the column to the
                            // "line length" value.
                            let buffer = self.view_snapshot.editor_model.as_ref(app).buffer(app);
                            if let Ok(len) = buffer.line_len(end.point.row()) {
                                *end.point.column_mut() = len;
                            }

                            // Back to "soft-wrap-space" for the visual range painting.
                            if let Some(start) = layout
                                .frame_layouts
                                .to_soft_wrap_point(start.point, start.clamp_direction)
                            {
                                if let Some(end) = layout
                                    .frame_layouts
                                    .to_soft_wrap_point(end.point, end.clamp_direction)
                                {
                                    visual_range = start..end;
                                }
                            }
                        }
                    }

                    let visible_selection_range = Self::visible_selection_range(
                        &mut visual_range,
                        visible_start,
                        visible_end,
                    );
                    for row in visible_selection_range {
                        let Some(line_layout) = layout.frame_layouts.get_line(row as usize) else {
                            continue;
                        };
                        if marked_text_state == MarkedTextState::Inactive {
                            self.draw_selection(
                                content_origin,
                                row,
                                first_visible_row,
                                colors.selection,
                                &visual_range,
                                line_parameters.line_height,
                                line_layout,
                                layout,
                                ctx,
                            );
                        }
                    }
                }
            }
        }

        cursors
    }

    /// Returns the index of the first visible row, given the current scroll position, taking into
    /// account the top section element, if relevant.
    fn first_visible_row(
        scroll_position_y: f32,
        line_height: f32,
        top_section_height_px: f32,
    ) -> u32 {
        let top_section_height_lines = top_section_height_px / line_height;
        if scroll_position_y <= top_section_height_lines {
            return 0;
        }
        // Calculate the scroll position relative to the start of the user typed editor content.
        let adjusted_scroll_position_y = scroll_position_y - top_section_height_lines;
        adjusted_scroll_position_y.floor() as u32
    }

    /// Returns the fraction of the scroll position (y) that is drawn "above" the visible content area.
    fn scroll_position_y_fract(
        scroll_position_y: f32,
        line_height: f32,
        top_section_height_px: f32,
    ) -> f32 {
        // We draw the top section until the first visible row is no longer 0. Hence, we need to account for the first
        // row, which is beside the left notch.
        if Self::first_visible_row(scroll_position_y, line_height, top_section_height_px) == 0 {
            return scroll_position_y * line_height;
        }

        let top_section_height_lines = top_section_height_px / line_height;
        let adjusted_scroll_position_y = scroll_position_y - top_section_height_lines;
        // Otherwise, we're simply drawing the first visible row, which can go slightly outside of the visible area.
        adjusted_scroll_position_y.fract() * line_height
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_lines(
        &self,
        content_origin: Vector2F,
        layout: &LayoutState,
        line_height: f32,
        autosuggestion_soft_wrapped_line_ix: Option<usize>,
        ctx: &mut PaintContext,
        app: &AppContext,
        last_autosuggestion_glyph_position: &mut Option<Vector2F>,
    ) {
        let view_snapshot = &self.view_snapshot;
        let first_visible_row = Self::first_visible_row(
            self.scroll_position().y(),
            line_height,
            layout.top_section_height_px,
        );
        let x_ray_start = view_snapshot
            .command_xray
            .as_ref()
            .map(|desc| desc.token.span.start());
        let x_ray_display_point = x_ray_start
            .and_then(|start| view_snapshot.display_point_at_byte_offset(&start.into(), app));
        if x_ray_start.is_none() {
            ctx.position_cache
                .clear_position(self.x_ray_position_id().as_str());
        }

        let color = if view_snapshot.can_select {
            self.text_colors.default_color
        } else {
            self.text_colors.disabled_color
        }
        .into();

        let baseline_position_fn = view_snapshot.baseline_position_fn();

        for (ix, line) in layout.frame_layouts.displayed_lines().enumerate() {
            // Push over text content origin due to left notch, if the first line is visible.
            let text_content_origin = if ix == 0 && first_visible_row == 0 {
                content_origin + vec2f(layout.left_notch_layout_width_px, 0.)
            } else {
                content_origin
            };
            let line_origin = text_content_origin + vec2f(0., ix as f32 * line_height);

            if let Some(point) =
                x_ray_display_point.filter(|point| point.row() == first_visible_row + ix as u32)
            {
                let x = line.x_for_index(point.column() as usize);
                ctx.position_cache.cache_position_indefinitely(
                    self.x_ray_position_id(),
                    RectF::new(
                        line_origin + vec2f(x, COMMAND_X_RAY_BOTTOM_PADDING_PX),
                        Vector2F::zero(),
                    ),
                );
            }

            line.paint_with_baseline_position(
                RectF::from_points(
                    line_origin,
                    self.bounds()
                        .expect("layout() should have been called before paint()")
                        .lower_right(),
                ),
                &Default::default(),
                color,
                ctx.font_cache,
                ctx.scene,
                &baseline_position_fn,
            );

            // Determine if the autosuggestion belongs on this line.
            // If the location of the autosuggestion is not specified, it belongs at the very end
            // If the autosuggestion is attached to a location, it belongs on the last TextFrame associated
            // with the logical line.
            let paint_autosuggestion_here = match autosuggestion_soft_wrapped_line_ix {
                None => ix == layout.frame_layouts.displayed_lines().count() - 1,
                Some(line_ix) => line_ix == ix,
            };

            // Render first line of autosuggestion text, if it exists. If not, render
            // first line of placeholder text, if it exists.
            if paint_autosuggestion_here {
                if let Some(suggestion_line) =
                    layout.placeholder_suggestion_text_line_layouts.first()
                {
                    let line_origin = line_origin + vec2f(line.width, 0.);

                    suggestion_line.paint_with_baseline_position(
                        RectF::from_points(
                            line_origin,
                            self.bounds()
                                .expect("layout() should have been called before paint()")
                                .lower_right(),
                        ),
                        &Default::default(),
                        self.text_colors.hint_color.into(),
                        ctx.font_cache,
                        ctx.scene,
                        &baseline_position_fn,
                    );
                    if let Some(pos) = last_autosuggestion_glyph_position {
                        *pos = line_origin + vec2f(suggestion_line.width, 0.);
                    }
                }
            }
        }
    }

    /// Computes right notch origin, if it should be drawn. Otherwise, returns None.
    fn compute_right_notch_origin(
        &self,
        layout: &LayoutState,
        first_visible_row: u32,
        element_origin: Vector2F,
        content_origin: Vector2F,
        last_autosuggestion_glyph_position: Option<Vector2F>,
    ) -> Option<Vector2F> {
        if first_visible_row != 0 {
            return None;
        }

        // Use the right notch offset, to compute the right notch origin, if it exists,
        // otherwise, default to the right edge of the EditorElement.
        let right_notch_origin = self
            .editor_decorator_elements
            .right_notch_offset_px
            .map(|right_notch_offset_px| element_origin + right_notch_offset_px)
            .unwrap_or_else(|| {
                vec2f(
                    layout.size.x() - layout.right_notch_layout_width_px + content_origin.x(),
                    content_origin.y(),
                )
            });

        let first_line_intersects_right_notch = layout
            .frame_layouts
            .get_line(0)
            .map(|first_line| {
                content_origin.x() + first_line.width + layout.left_notch_layout_width_px
                    > right_notch_origin.x()
            })
            .unwrap_or(false);

        let autosuggestion_intersects_right_notch = last_autosuggestion_glyph_position
            .map(|position| {
                // Autosuggestion must be on the same line as the right notch, to intersect it.
                position.y() == right_notch_origin.y() && position.x() > right_notch_origin.x()
            })
            .unwrap_or(false);

        // We only want to paint the right notch, if there is no intersecting first line of text or autosuggestion.
        (!first_line_intersects_right_notch && !autosuggestion_intersects_right_notch)
            .then_some(right_notch_origin)
    }

    /// Note that we lay out all of the rows instead of only the visible ones.
    fn layout_text_frames(
        &self,
        view_snapshot: &ViewSnapshot,
        size: &Vector2F,
        left_notch_layout_width_px: f32,
        top_section_height_px: f32,
        ctx: &LayoutContext,
        app: &AppContext,
    ) -> anyhow::Result<FrameLayouts> {
        let line_height = view_snapshot.line_height;

        let max_width = if self.soft_wrap { size.x() } else { f32::MAX };
        let frames = view_snapshot.layout_text_frames(
            0..view_snapshot.max_point(app).row() + 1,
            ctx.text_layout_cache,
            max_width,
            left_notch_layout_width_px,
            app,
        )?;

        // Determine a limit for the end line. If the size is infinite,
        // this is infinite. Otherwise, depending on the scrolling position
        // and the number of lines that fit in the element, calculate the line index.
        let end_line = if size.y().is_infinite() {
            u32::MAX
        } else {
            let scroll_top = self.scroll_position().y() * line_height + top_section_height_px;
            ((scroll_top + size.y()) / line_height).ceil() as u32
        };
        let start_line = Self::first_visible_row(
            self.scroll_position().y(),
            line_height,
            top_section_height_px,
        );
        Ok(FrameLayouts::new(frames, start_line, end_line))
    }

    fn should_show_cycle_next_command_hint(&self, is_cycling: bool, ctx: &AppContext) -> bool {
        FeatureFlag::CycleNextCommandSuggestion.is_enabled()
            && self
                .view_snapshot
                .editor_model
                .as_ref(ctx)
                .buffer(ctx)
                .is_empty()
            && (self.view_snapshot.active_next_command_suggestion() || is_cycling)
    }

    /// Adds icons used in the input editor but not other editors.
    pub fn with_input_editor_icons(
        mut self,
        accept_autosuggestion_keybinding: &ViewHandle<AcceptAutosuggestionKeybinding>,
        autosuggestion_ignore: &ViewHandle<AutosuggestionIgnore>,
        show_autosuggestion_keybinding_hint: bool,
        show_autosuggestion_ignore_button: bool,
        is_cycling: bool,
        ctx: &AppContext,
    ) -> Self {
        // Text in the input is cut off at line heights < 1, so it's not possible
        // to render the keybinding shortcut / ignore button at these small line heights.
        if self.view_snapshot.is_focused
            && self.view_snapshot.active_autosuggestion()
            && self.view_snapshot.line_height_ratio >= 1.
        {
            if show_autosuggestion_keybinding_hint {
                self.autosuggestion_shortcut_icon =
                    Some(ChildView::new(accept_autosuggestion_keybinding).finish());
            }
            if show_autosuggestion_ignore_button
                && FeatureFlag::AllowIgnoringInputSuggestions.is_enabled()
            {
                self.autosuggestion_ignore_icon =
                    Some(ChildView::new(autosuggestion_ignore).finish());
            }
        }
        // If the input buffer is empty, down arrow cycles suggestions.
        let cycle_next_command_hint = if self.should_show_cycle_next_command_hint(is_cycling, ctx) {
            let appearance = Appearance::as_ref(ctx);
            Some(
                self.render_cycle_next_command_hint(warp_core::ui::theme::Fill::Solid(
                    blended_colors::semantic_text_disabled(appearance.theme()),
                )),
            )
        } else {
            None
        };
        Self {
            cycle_next_command_hint,
            ..self
        }
    }

    fn render_cycle_next_command_hint(&self, color: Fill) -> Box<dyn Element + 'static> {
        let font_size = self.view_snapshot.font_size - 2.;
        let icon_height = Self::cursor_height(font_size, self.view_snapshot.line_height_ratio);
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::End)
            .with_children([
                Container::new(
                    ConstrainedBox::new(Icon::ArrowDown.to_warpui_icon(color).finish())
                        .with_max_height(icon_height)
                        .with_max_width(icon_height)
                        .finish(),
                )
                .with_margin_right(self.view_snapshot.em_width)
                .finish(),
                Text::new(
                    "Cycle suggestions",
                    self.view_snapshot.font_family,
                    font_size,
                )
                .with_color(self.text_colors.hint_color.into())
                .finish(),
            ])
            .finish()
    }

    #[cfg(feature = "voice_input")]
    pub fn with_voice_input_cursor_icon(self, element: Box<dyn Element>) -> Self {
        // If voice input is not active, don't render the icon.
        if !self.view_snapshot.voice_input_state.is_active() {
            return self;
        }
        Self {
            voice_input_cursor_icon: Some(element),
            ..self
        }
    }

    /// Takes into account whether or not we're in Vim mode to determine the cursor type.
    fn get_cursor_type(&self) -> CursorDisplayType {
        self.vim_mode
            .map(|vim_mode| match vim_mode {
                VimMode::Normal | VimMode::Visual(_) => CursorDisplayType::Block,
                VimMode::Replace => CursorDisplayType::Underline,
                VimMode::Insert => CursorDisplayType::Bar,
            })
            .unwrap_or(self.preferred_cursor_type)
    }
}

impl Element for EditorElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let mut size = constraint.max;
        let view_snapshot = &self.view_snapshot;

        let font_cache = app.font_cache();
        let line_height = view_snapshot.line_height;

        let top_section_height_px = self
            .editor_decorator_elements
            .top_section
            .as_mut()
            .map(|top_section_element: &mut Box<dyn Element>| {
                top_section_element.layout(constraint, ctx, app).y()
            })
            .unwrap_or(0.0);
        // We require the left notch to be exactly 1 line_height high! We do this
        // to avoid tracking y-offset logic in the EditorElement (for aligning to
        // the bottom of the left notch). Hence, we only track the width.
        let left_notch_layout_width_px = self
            .editor_decorator_elements
            .left_notch
            .as_mut()
            .map(|left_notch_element| left_notch_element.layout(constraint, ctx, app).x())
            .unwrap_or(0.0);
        let right_notch_layout_width_px = self
            .editor_decorator_elements
            .right_notch
            .as_mut()
            .map(|right_notch_element| right_notch_element.layout(constraint, ctx, app).x())
            .unwrap_or(0.0);

        let frame_layouts = match self.layout_text_frames(
            view_snapshot,
            &size,
            left_notch_layout_width_px,
            top_section_height_px,
            ctx,
            app,
        ) {
            Err(error) => {
                log::error!("error laying out lines: {error}");
                return size;
            }
            Ok(layouts) => layouts,
        };

        let last_text_line_width = frame_layouts
            .last_frame()
            .and_then(|frame| frame.lines().last().map(|line| line.width))
            .unwrap_or(0.);

        // Choose what to render: placeholder takes precedence over autosuggestion
        let placeholder_suggestion_text_line_layouts = if let Some(placeholder_text) = view_snapshot
            .matching_placeholder_text(&view_snapshot.editor_model.as_ref(app).buffer_text(app))
        {
            view_snapshot.layout_placeholder_text(
                &placeholder_text,
                last_text_line_width + left_notch_layout_width_px,
                font_cache,
                ctx.text_layout_cache,
                &size,
                self.placeholder_soft_wrap,
            )
        } else if view_snapshot.active_autosuggestion() {
            view_snapshot.layout_autosuggestion(
                last_text_line_width + left_notch_layout_width_px,
                font_cache,
                ctx.text_layout_cache,
                &size,
                self.soft_wrap,
            )
        } else {
            vec![]
        };
        // Set the height of the element, in case we don't have enough text to fill
        // in all the space.
        if size.y().is_infinite() || view_snapshot.autogrow {
            let num_lines_total = if view_snapshot.is_empty
                && view_snapshot.placeholder_text_exists()
            {
                placeholder_suggestion_text_line_layouts.len() as u32
            } else {
                let num_lines = frame_layouts
                    .frames()
                    .fold(0, |acc, layout| acc + layout.lines().len() as u32)
                    .max(1);
                // The first line of the autosuggestion is rendered on the last line of the buffer so
                // don't include it when calculating the height of the editor.
                let num_autosuggestion_lines = placeholder_suggestion_text_line_layouts
                    .len()
                    .saturating_sub(1) as u32;

                num_lines + num_autosuggestion_lines
            };

            // We add 1. to the height because when we paint the text content, we force pixel-alignment of the Editor's content
            // to avoid rendering artifacts with text. This is done by taking the ceil of the content_origin.y().
            // This can increase the required height by up to 1, and we don't know if we're going to need to do this until paint time.
            let computed_y =
                num_lines_total as f32 * view_snapshot.line_height + top_section_height_px + 1.;

            let y_before_shrink_delay = computed_y.clamp(constraint.min.y(), size.y());
            size.set_y(
                self.view_snapshot
                    .get_editor_height_with_shrink_delay(y_before_shrink_delay),
            );
        }
        if size.x().is_infinite() {
            unimplemented!("we don't yet handle an infinite width constraint on buffer elements");
        }

        let top_section_height_lines = top_section_height_px / view_snapshot.line_height;
        let total_lines = frame_layouts
            .frames()
            .fold(0, |acc, frame| acc + frame.lines().len()) as f32
            + top_section_height_lines;

        let autoscroll_horizontally = view_snapshot.autoscroll_vertically(
            &self.scroll_state,
            total_lines,
            size.y() / line_height,
            top_section_height_lines,
            &frame_layouts,
            app,
        ) && !self.soft_wrap;

        let mut max_visible_line_width = frame_layouts
            .frames()
            .map(|x| x.max_width())
            .reduce(f32::max)
            .unwrap_or(0.);

        max_visible_line_width = max_visible_line_width.max(
            placeholder_suggestion_text_line_layouts
                .iter()
                .map(|line| line.width)
                .reduce(f32::max)
                .unwrap_or(0.),
        );

        if let Some(element) = self.cycle_next_command_hint.as_mut() {
            element.layout(constraint, ctx, app);
        }

        let cursor_height =
            Self::cursor_height(view_snapshot.font_size, view_snapshot.line_height_ratio);

        // Show the autosuggestion hint & ignore button if there is enough space.
        if cursor_height >= AUTOSUGGESTION_HINT_MINIMUM_HEIGHT {
            if let Some(element) = self.autosuggestion_shortcut_icon.as_mut() {
                element.layout(constraint, ctx, app);
            }
            if let Some(element) = self.autosuggestion_ignore_icon.as_mut() {
                element.layout(constraint, ctx, app);
            }
        } else {
            self.autosuggestion_shortcut_icon = None;
            self.autosuggestion_ignore_icon = None;
        }

        // The first line of the suggestion lines is rendered on the same line as the last line
        // of the buffer, so update the max width to include it.
        if let Some(autosuggestion_line) = placeholder_suggestion_text_line_layouts.first() {
            let line_with_both_text_and_autosuggestion_width =
                last_text_line_width + autosuggestion_line.width;

            max_visible_line_width =
                max_visible_line_width.max(line_with_both_text_and_autosuggestion_width);
        }
        // The last line of the autosuggestion may have icons that add to width.
        if let Some(autosuggestion_line) = placeholder_suggestion_text_line_layouts.last() {
            let mut width_with_icons = autosuggestion_line.width;
            if let Some(autosuggestion_shortcut_icon) = self.autosuggestion_shortcut_icon.as_ref() {
                // Add width for the padding and the icon width.
                width_with_icons += AUTOSUGGESTION_HINT_PADDING
                    + autosuggestion_shortcut_icon
                        .size()
                        .expect("should have size")
                        .x();
            }
            if let Some(autosuggestion_ignore_icon) = self.autosuggestion_ignore_icon.as_ref() {
                // Add width for the padding and the ignore icon width.
                width_with_icons += AUTOSUGGESTION_HINT_PADDING
                    + autosuggestion_ignore_icon
                        .size()
                        .expect("should have size")
                        .x();
            }
            max_visible_line_width = max_visible_line_width.max(width_with_icons);
        }

        // Layout all avatars
        let avatar_size = view_snapshot.cursor_avatar_size() + 2.; // 2. to account for border on both sides
        for drawable_selections_data in self.remote_selections_data.values_mut() {
            drawable_selections_data.avatar.layout(
                SizeConstraint::new(
                    vec2f(avatar_size, avatar_size),
                    vec2f(avatar_size, avatar_size),
                ),
                ctx,
                app,
            );
        }

        if let Some(element) = self.voice_input_cursor_icon.as_mut() {
            let voice_input_icon_size = view_snapshot.voice_input_icon_size();
            element.layout(
                SizeConstraint::new(voice_input_icon_size, voice_input_icon_size),
                ctx,
                app,
            );
        }

        self.soft_wrap_state.update(frame_layouts.clone());

        self.layout = Some(LayoutState {
            size,
            frame_layouts,
            placeholder_suggestion_text_line_layouts,
            max_visible_line_width,
            autoscroll_horizontally,
            top_section_height_px,
            left_notch_layout_width_px,
            right_notch_layout_width_px,
        });
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        if let Some(layout) = &self.layout {
            if let Some(top_section) = &mut self.editor_decorator_elements.top_section {
                top_section.after_layout(ctx, app);
            }

            if let Some(left_notch) = &mut self.editor_decorator_elements.left_notch {
                left_notch.after_layout(ctx, app);
            }

            if let Some(right_notch) = &mut self.editor_decorator_elements.right_notch {
                right_notch.after_layout(ctx, app);
            }

            let view_snapshot = &self.view_snapshot;
            view_snapshot.clamp_scroll_left(
                &self.scroll_state,
                layout
                    .scroll_max(view_snapshot, ctx.text_layout_cache, app)
                    .x(),
            );

            if layout.autoscroll_horizontally {
                view_snapshot.autoscroll_horizontally(
                    &self.scroll_state,
                    self.scroll_position().y() as u32,
                    layout.size.x(),
                    layout.scroll_width(view_snapshot, ctx.text_layout_cache, app),
                    view_snapshot.em_width,
                    layout
                        .frame_layouts
                        .frames()
                        .flat_map(|frame| frame.lines().first())
                        .collect(),
                    app,
                );
            }
        }
    }

    // TODO: refactor this paint code to make it more readable.
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        if let Some(layout) = &self.layout {
            let bounds = RectF::new(origin, layout.size);
            self.paint = Some(PaintState {
                rect: bounds,
                origin: Point::from_vec2f(origin, ctx.scene.z_index()),
            });
            let view_snapshot = &self.view_snapshot;
            let cursor_height =
                Self::cursor_height(view_snapshot.font_size, view_snapshot.line_height_ratio);
            let line_height = view_snapshot.line_height;
            // The start of the element (top-left of top section).
            let element_origin = bounds.origin()
                - vec2f(
                    self.scroll_position().x() * view_snapshot.em_width,
                    Self::scroll_position_y_fract(
                        self.scroll_position().y(),
                        line_height,
                        layout.top_section_height_px,
                    ),
                );

            let first_visible_row = Self::first_visible_row(
                self.scroll_position().y(),
                line_height,
                layout.top_section_height_px,
            );

            // The start of the "core" content, below the top section.
            // Note that the Grid is always pixel-aligned to avoid rendering artifacts that cause text to look misaligned.
            // Similarly, we force pixel-alignment of the Editor's content to avoid rendering artifacts with text.
            let content_origin;
            if first_visible_row == 0 {
                if let Some(top_section_element) = &mut self.editor_decorator_elements.top_section {
                    top_section_element.paint(element_origin, ctx, app);
                    content_origin = vec2f(
                        element_origin.x(),
                        element_origin.y() + layout.top_section_height_px,
                    )
                    .ceil();
                } else {
                    content_origin = element_origin.ceil();
                }

                if let Some(left_notch_element) = &mut self.editor_decorator_elements.left_notch {
                    left_notch_element.paint(content_origin, ctx, app);
                }
            } else {
                content_origin = element_origin.ceil();
            }

            let cursors = self.draw_selections_and_get_cursor_data(
                content_origin,
                bounds,
                layout,
                LineParameters {
                    line_height,
                    cursor_height,
                },
                ctx,
                app,
            );

            Self::draw_cursors(
                self.get_cursor_type(),
                cursors,
                view_snapshot,
                &mut self.remote_selections_data,
                &mut self.voice_input_cursor_icon,
                ctx,
                app,
            );

            // Determine where to place the autosuggestion hint.
            // It's mutable and updated according to the width of autosuggestion lines. And it is only drawn at the last line of autosuggestion.
            let mut last_autosuggestion_glyph_position = Some(vec2f(0., 0.));

            // The autosuggestion location should be expressed in a soft-wrapped row coordinate
            // In other words, in what soft-wrapped row can we find the last character in the logical row that has the autosuggestion?
            let autosuggestion_soft_wrapped_line = view_snapshot
                .autosuggestion_location()
                .as_ref()
                .and_then(|l| match l {
                    AutosuggestionLocation::EndOfBuffer => None,
                    AutosuggestionLocation::Inline(logical_line_ix) => {
                        Some(layout.frame_layouts.end_of_logical_row(*logical_line_ix))
                    }
                });

            self.paint_lines(
                content_origin,
                layout,
                line_height,
                autosuggestion_soft_wrapped_line,
                ctx,
                app,
                &mut last_autosuggestion_glyph_position,
            );

            let right_notch_origin = self.compute_right_notch_origin(
                layout,
                first_visible_row,
                element_origin,
                content_origin,
                last_autosuggestion_glyph_position,
            );

            right_notch_origin.and_then(|right_notch_origin| {
                self.editor_decorator_elements
                    .right_notch
                    .as_mut()
                    .map(|right_notch_element| {
                        right_notch_element.paint(right_notch_origin, ctx, app);
                    })
            });

            let line_origin = content_origin
                + vec2f(
                    0.,
                    layout.frame_layouts.num_displayed_lines() as f32 * line_height,
                );

            // Draw the rest of the autosuggestion lines--we skip the first line as it is handled
            // by paint_lines since it's painted on the same line as the buffer.
            layout
                .placeholder_suggestion_text_line_layouts
                .iter()
                .skip(1)
                .enumerate()
                .for_each(|(ix, line)| {
                    let line_origin = line_origin + vec2f(0., ix as f32 * line_height);
                    line.paint_with_baseline_position(
                        RectF::from_points(
                            line_origin,
                            self.bounds()
                                .expect("layout() should have been called before paint()")
                                .lower_right(),
                        ),
                        &Default::default(),
                        self.text_colors.hint_color.into(),
                        ctx.font_cache,
                        ctx.scene,
                        &view_snapshot.baseline_position_fn(),
                    );
                    last_autosuggestion_glyph_position = Some(line_origin + vec2f(line.width, 0.));
                });

            let last_line = layout
                .frame_layouts
                .get_line(layout.frame_layouts.num_lines() - 1);
            let next_icon_origin_without_padding = last_autosuggestion_glyph_position
                .zip(last_line)
                .map(|(p, last_line)| {
                    p + Self::cursor_origin_vertical_adjustment_from_text_content_origin(
                        view_snapshot,
                        ComputeBaselinePositionArgs {
                            font_cache: ctx.font_cache,
                            font_size: last_line.font_size,
                            line_height_ratio: last_line.line_height_ratio,
                            baseline_ratio: last_line.baseline_ratio,
                            ascent: last_line.ascent,
                            descent: last_line.descent,
                        },
                    )
                });

            let mut current_icon_x_offset = 0.;
            if let Some((position, element)) =
                next_icon_origin_without_padding.zip(self.autosuggestion_shortcut_icon.as_mut())
            {
                element.paint(position + vec2f(AUTOSUGGESTION_HINT_PADDING, 0.), ctx, app);
                current_icon_x_offset +=
                    AUTOSUGGESTION_HINT_PADDING + element.size().expect("should have size").x();
            }

            // Paint the ignore button after the keybinding hint (if it exists)
            if let Some((position, ignore_element)) =
                next_icon_origin_without_padding.zip(self.autosuggestion_ignore_icon.as_mut())
            {
                ignore_element.paint(
                    position + vec2f(current_icon_x_offset + AUTOSUGGESTION_HINT_PADDING, 0.),
                    ctx,
                    app,
                );
            }

            if let Some(cycle_next_command_hint) = self.cycle_next_command_hint.as_mut() {
                let hint_size = cycle_next_command_hint
                    .size()
                    .expect("should have element size at paint");

                let origin = vec2f(
                    bounds.max_x() - hint_size.x(),
                    bounds.max_y() - hint_size.y(),
                );
                // Only render the hint if it wouldn't overlap with the rightmost icon.
                if next_icon_origin_without_padding.is_none_or(|p| p.x() < origin.x()) {
                    cycle_next_command_hint.paint(origin, ctx, app);
                }
            }

            let cursor_size = vec2f(BEAM_CURSOR_WIDTH_PX, cursor_height);
            for (id, point) in &view_snapshot.cached_buffer_points {
                // `cached_buffer_points` are expressed in buffer (DisplayPoint) coordinates, but
                // the editor's line layouts are indexed by soft-wrapped row. Convert before
                // looking up a line / computing pixel offsets.
                let display_point = DisplayPoint::new(point.row, point.column);
                let Some(soft_wrap_point) = layout
                    .frame_layouts
                    .to_soft_wrap_point(display_point, ClampDirection::Down)
                else {
                    continue;
                };

                let row = soft_wrap_point.row();
                let Some(line) = &layout.frame_layouts.get_line(row as usize) else {
                    continue;
                };

                // `content_origin` is the origin of the first visible row.
                let relative_row = row as f32 - first_visible_row as f32;
                let y_offset = relative_row * line_height;

                // Match paint_lines: the left notch only affects the first visible line.
                let text_content_origin = if row == 0 && first_visible_row == 0 {
                    content_origin + vec2f(layout.left_notch_layout_width_px, 0.)
                } else {
                    content_origin
                };

                let x_offset = line.x_for_index(soft_wrap_point.column() as usize);
                let bounds = text_content_origin + vec2f(x_offset, y_offset);

                ctx.position_cache.cache_position_indefinitely(
                    position_id_for_cached_point(self.view_snapshot.view_id, id),
                    RectF::new(bounds, cursor_size),
                );
            }
        }
        if let Some(editor_repaint_at) = self.view_snapshot.get_editor_repaint_at() {
            ctx.repaint_at(editor_repaint_at);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let Some(z_index) = self.z_index() else {
            return false;
        };

        // These icons have tooltips in overlay layers, so they need to be dispatched before checking event.at_z_index below.
        // This is because event.at_z_index will filter the event due to the overlay layer above.
        if let Some(icon) = &mut self.autosuggestion_shortcut_icon {
            if icon.bounds().is_some() && icon.dispatch_event(event, ctx, app) {
                return true;
            }
        }

        if let Some(ignore_icon) = &mut self.autosuggestion_ignore_icon {
            if ignore_icon.bounds().is_some() && ignore_icon.dispatch_event(event, ctx, app) {
                return true;
            }
        }

        // Since editor decorator elements may be clickable, we need to prioritize dispatching events from them.
        if let Some(top_section) = &mut self.editor_decorator_elements.top_section {
            if top_section.dispatch_event(event, ctx, app) {
                return true;
            }
        }

        if let Some(left_notch) = &mut self.editor_decorator_elements.left_notch {
            if left_notch.dispatch_event(event, ctx, app) {
                return true;
            }
        }

        if let Some(right_notch) = &mut self.editor_decorator_elements.right_notch {
            if right_notch.dispatch_event(event, ctx, app) {
                return true;
            }
        }

        let Some(event_at_z_index) = event.at_z_index(z_index, ctx) else {
            return false;
        };

        let events_to_propagate_on = matches!(
            event_at_z_index,
            Event::MouseMoved { .. }
                | Event::LeftMouseDragged { .. }
                | Event::LeftMouseDown { .. }
                | Event::LeftMouseUp { .. }
                | Event::RightMouseDown { .. }
        );

        // Intercept mouse events for floating elements,
        // similar to how it is done in dispatch_event() in block_list_element.rs
        if events_to_propagate_on {
            // Only pass through events to decorator elements which have been painted (this avoids cases such as the right prompt
            // not being painted, where the EventHandler will fail to unwrap a Z-index option)!
            if let Some(top_section) = self
                .editor_decorator_elements
                .top_section
                .as_mut()
                .filter(|n| n.z_index().is_some())
            {
                if top_section.dispatch_event(event, ctx, app) {
                    return true;
                }
            }
            if let Some(left_notch) = self
                .editor_decorator_elements
                .left_notch
                .as_mut()
                .filter(|n| n.z_index().is_some())
            {
                if left_notch.dispatch_event(event, ctx, app) {
                    return true;
                }
            }
            if let Some(right_notch) = self
                .editor_decorator_elements
                .right_notch
                .as_mut()
                .filter(|n| n.z_index().is_some())
            {
                if right_notch.dispatch_event(event, ctx, app) {
                    return true;
                }
            }
        }

        match event_at_z_index {
            Event::LeftMouseDown {
                position,
                modifiers,
                click_count,
                is_first_mouse,
            } => match *click_count {
                1 => self.mouse_down(*position, modifiers, *is_first_mouse, ctx, app),
                2 => self.double_mouse_down(*position, ctx),
                3 => self.triple_mouse_down(*position, ctx),
                _ => false,
            },
            Event::LeftMouseUp { position, .. } => self.mouse_up(*position, ctx, app),
            Event::LeftMouseDragged { position, .. } => self.mouse_dragged(*position, ctx, app),
            Event::ScrollWheel {
                position,
                delta,
                precise,
                modifiers: ModifiersState { ctrl: false, .. },
            } => self.scroll(*position, *delta, *precise, ctx, app),
            Event::KeyDown { keystroke, .. } => self.key_down(keystroke, ctx),
            Event::ModifierKeyChanged {
                key_code, state, ..
            } => self.modifier_key_change(key_code, state, ctx),
            Event::TypedCharacters { chars } => self.typed_characters(chars, ctx),
            Event::DragAndDropFiles { paths, location } => {
                self.drag_and_drop_file(paths.clone(), *location, ctx)
            }
            Event::MouseMoved { position, .. } => self.mouse_moved(*position, ctx, app),
            Event::MiddleMouseDown { position, .. } => self.middle_mouse_down(*position, ctx),
            Event::SetMarkedText {
                marked_text,
                selected_range,
            } => self.set_marked_text(marked_text, selected_range, ctx),
            Event::ClearMarkedText => self.clear_marked_text(ctx),
            _ => false,
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.layout.as_ref().map(|layout| layout.size)
    }

    fn origin(&self) -> Option<Point> {
        self.paint.as_ref().map(|paint| paint.origin)
    }
}

struct LayoutState {
    size: Vector2F,
    // Each frame is a row of text from the buffer. If the soft wrap option is on and a row
    // is too long to fit on the screen, it'll be split into multiple lines within
    // a frame. Otherwise, the frame is a singleton Line.
    frame_layouts: FrameLayouts,
    // Will hold either the suggestion text or placeholder text or empty vector, if neither exist.
    // Suggestion text should take precedence.
    placeholder_suggestion_text_line_layouts: Vec<Arc<text_layout::Line>>,
    // This contains the shortcut icon that shows new users how to accept the autosuggestion.
    max_visible_line_width: f32,
    /// True if the `autoscroll_vertically` function on the editor view returns true
    /// and the soft wrap setting is off.
    autoscroll_horizontally: bool,
    /// Height of top section element, rendered above the EditorElement (defaults to 0
    /// if no top element).
    top_section_height_px: f32,
    /// The width of the left notch and right notch elements when laid out (default to 0 if
    /// they doesn't exist).
    left_notch_layout_width_px: f32,
    right_notch_layout_width_px: f32,
}

impl LayoutState {
    /// Computes the scroll width of this editor in pixels.
    fn scroll_width(
        &self,
        view_snapshot: &ViewSnapshot,
        text_layout_cache: &LayoutCache,
        app: &AppContext,
    ) -> f32 {
        if self.autoscroll_horizontally {
            let row = view_snapshot.rightmost_point(app).row();
            let longest_line_width = view_snapshot
                .layout_line(row, text_layout_cache, app)
                .expect("Should have layout line")
                .width;
            longest_line_width.max(self.max_visible_line_width) + view_snapshot.em_width
        } else {
            let last_line_width = self
                .frame_layouts
                .displayed_lines()
                .last()
                .map_or(0., |line| line.width);
            let placeholder_or_suggestion_width = self
                .placeholder_suggestion_text_line_layouts
                .first()
                .map_or(0., |line| line.width);
            last_line_width + placeholder_or_suggestion_width + view_snapshot.em_width
        }
    }

    /// The maximum bottom-right scrolling position in pixels.
    fn scroll_max(
        &self,
        view_snapshot: &ViewSnapshot,
        text_layout_cache: &LayoutCache,
        app: &AppContext,
    ) -> Vector2F {
        let horizontal_scrolling_max = ((self.scroll_width(view_snapshot, text_layout_cache, app)
            - self.size.x())
            / view_snapshot.em_width)
            .max(0.0);

        // Total number of lines, including the top section.
        let total_lines = self
            .frame_layouts
            .frames()
            .fold(0, |acc, frame| acc + frame.lines().len()) as f32
            + self.top_section_height_px / view_snapshot.line_height;

        let max_scroll_top =
            ViewSnapshot::max_scroll_top(total_lines, self.size.y() / view_snapshot.line_height);
        vec2f(horizontal_scrolling_max, max_scroll_top)
    }
}

struct PaintState {
    rect: RectF,
    origin: Point,
}

/// The display point might be clamped at either end of a line.
pub struct PossibleDisplayPoint {
    pub display_point: DisplayPoint,
    /// Whether the display point is clamped to either the beginning or end of the line.
    pub is_clamped: bool,
    pub clamp_direction: ClampDirection,
}

impl PaintState {
    /// Returns the display point for the given position,
    /// clamping the row/col if necessary.
    fn point_for_position(
        &self,
        view_snapshot: &ViewSnapshot,
        scroll_position: Vector2F,
        layout: &LayoutState,
        position: Vector2F,
    ) -> DisplayPoint {
        self.possible_point_for_position(view_snapshot, scroll_position, layout, position)
            .display_point
    }

    /// Returns a possible display point for the given position,
    /// which includes whether or not the display_point has been clamped.
    ///
    /// This method supports soft-wrapping.
    fn possible_point_for_position(
        &self,
        view_snapshot: &ViewSnapshot,
        scroll_position: Vector2F,
        layout: &LayoutState,
        position: Vector2F,
    ) -> PossibleDisplayPoint {
        let mut is_clamped = false;

        let position = position - self.rect.origin();
        let y = position.y().max(0.0).min(layout.size.y());

        let num_rows = layout.frame_layouts.num_lines();
        let max_row = num_rows as u32 - 1;
        // Offset by the top section, which shouldn't be considered for purposes of position -> display point.
        let shifted_scroll_position_y =
            scroll_position.y() - layout.top_section_height_px / view_snapshot.line_height;
        let mut row: u32 = ((y / view_snapshot.line_height) + shifted_scroll_position_y) as u32;
        if row > max_row {
            is_clamped = true;
            row = max_row;
        }
        let shifted_position = if row == 0 {
            // Note that the given position is shifted for the left notch's width, but we don't
            // want the DisplayPoint col to be shifted over, hence we subtract the width.
            vec2f(
                position.x() - layout.left_notch_layout_width_px,
                position.y(),
            )
        } else {
            position
        };

        let line = layout
            .frame_layouts
            .get_line(row as usize)
            .expect("Should be clamped to last possible line");

        let x = shifted_position.x() + (scroll_position.x() * view_snapshot.em_width);

        let col = if let Some(col) = line.index_for_x(x).map(|ix| ix as u32) {
            col
        } else {
            // Clamp to the left or right if the x pos is before or after the buffer text, respectively.
            is_clamped = true;
            if x >= 0. {
                // If the line has no glyphs, the index is zero. Otherwise, we take one more index
                // beyond the last start index in the line to get the last position.
                line.last_glyph()
                    .map_or(0, |glyph| (glyph.index + 1) as u32)
            } else {
                0
            }
        };
        // Now convert from SoftWrapPoint to DisplayPoint.
        let soft_wrap_point = SoftWrapPoint::new(row, col);

        let display_point_and_clamp_direction =
            layout.frame_layouts.to_display_point(soft_wrap_point);
        PossibleDisplayPoint {
            display_point: display_point_and_clamp_direction.point,
            is_clamped,
            clamp_direction: display_point_and_clamp_direction.clamp_direction,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scroll_position(
        &self,
        view_snapshot: &ViewSnapshot,
        scroll_state: &ScrollState,
        layout: &LayoutState,
        position: Vector2F,
        ctx: &EventContext,
        app: &AppContext,
    ) -> Vector2F {
        let rect = self.rect;
        let mut scroll_delta = Vector2F::zero();

        let vertical_margin = view_snapshot.line_height.min(rect.height() / 3.0);
        let top = rect.origin_y() + vertical_margin;
        let bottom = rect.lower_left().y() - vertical_margin;
        if position.y() < top {
            scroll_delta.set_y(-scale_vertical_mouse_autoscroll_delta(top - position.y()))
        }
        if position.y() > bottom {
            scroll_delta.set_y(scale_vertical_mouse_autoscroll_delta(position.y() - bottom))
        }

        let horizontal_margin = view_snapshot.line_height.min(rect.width() / 3.0);
        let left = rect.origin_x() + horizontal_margin;
        let right = rect.upper_right().x() - horizontal_margin;
        if position.x() < left {
            scroll_delta.set_x(-scale_horizontal_mouse_autoscroll_delta(
                left - position.x(),
            ))
        }
        if position.x() > right {
            scroll_delta.set_x(scale_horizontal_mouse_autoscroll_delta(
                position.x() - right,
            ))
        }
        (scroll_state.scroll_position() + scroll_delta).clamp(
            Vector2F::zero(),
            layout.scroll_max(view_snapshot, ctx.text_layout_cache, app),
        )
    }
}

fn scale_vertical_mouse_autoscroll_delta(delta: f32) -> f32 {
    delta.powf(1.5) / 100.0
}

fn scale_horizontal_mouse_autoscroll_delta(delta: f32) -> f32 {
    delta.powf(1.2) / 300.0
}

#[cfg(test)]
#[path = "element_tests.rs"]
mod tests;
