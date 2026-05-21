use crate::ai::blocklist::agent_view::{agent_view_bg_fill, AgentViewState};
use crate::ai::blocklist::{ai_brand_color, ATTACH_AS_AGENT_MODE_CONTEXT_TEXT};
use crate::ai_assistant::{AI_ASSISTANT_SVG_PATH, ASK_AI_ASSISTANT_TEXT};
use crate::appearance::Appearance;
use crate::drive::settings::WarpDriveSettings;
use crate::features::FeatureFlag;
use crate::pane_group::SplitPaneState;
use crate::settings::{
    AISettings, DebugSettings, EnforceMinimumContrast, PrivacySettings, TerminalSpacing,
};
use crate::terminal::alt_screen::{should_intercept_mouse, should_intercept_scroll};
use crate::terminal::block_list_viewport::AutoscrollBehavior;
use crate::terminal::input::inline_menu::InlineMenuPositioner;
use crate::terminal::model::block::{Block, BlockSection};
use crate::terminal::model::blocks::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, BlockListPoint, TotalIndex,
};
use crate::terminal::model::index::Point as IndexPoint;
use crate::terminal::model::selection::{SelectAction, SelectionPoint};
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::view::TerminalAction;
use crate::terminal::{grid_renderer, SizeInfo};
use crate::themes::theme::{Fill, WarpTheme};
use crate::ui_components::{self, icons as UIIcon};
use crate::util::color::Opacity;
use enum_iterator::Sequence;
use itertools::Itertools;
use parking_lot::FairMutex;
use vec1::Vec1;
use warp_core::semantic_selection::SemanticSelection;
use warp_core::ui::builder::UiBuilder;
use warp_core::ui::theme::AnsiColorIdentifier;
use warp_util::user_input::UserInput;
use warpui::platform::Cursor;
use warpui::text::SelectionType;

use pathfinder_color::ColorU;
use session_sharing_protocol::common::{ParticipantId, Selection};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::ops::{Deref, Range, RangeInclusive};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use warpui::elements::new_scrollable::{NewScrollableElement, ScrollableAxis};
use warpui::elements::{
    Axis, Border, ChildAnchor, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Point, Radius, ScrollData, ScrollableElement, Stack, Text, ZIndex,
};
use warpui::event::{KeyState, ModifiersState};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::platform::keyboard::KeyCode;
use warpui::ui_components::components::UiComponent;
use warpui::units::{IntoLines, IntoPixels, Lines, Pixels};
use warpui::{elements::Icon, ClipBounds};
use warpui::{
    elements::SavePosition, event::DispatchedEvent, AfterLayoutContext, AppContext, Element, Event,
    EventContext, LayoutContext, PaintContext, SizeConstraint,
};
use warpui::{EntityId, ModelHandle, SingletonEntity as _};

use super::block_list_viewport::{ClampingMode, InputMode, ScrollPosition, ViewportState};
use super::blockgrid_renderer::GridRenderParams;
use super::find::{BlockFindRenderData, TerminalFindModel};
use super::grid_renderer::CellGlyphCache;

use super::meta_shortcuts::handle_keystroke_despite_composing;
use super::model::block::BlockId;
use super::model::blocks::{RichContentItem, SelectionRange};
use super::model::grid::grid_handler::{Link, TermMode};
use super::model::image_map::StoredImageMetadata;
use super::model::mouse::{MouseAction, MouseButton, MouseState};
use super::model::session::SessionId;
use super::model::terminal_model::{SelectedBlocks, WithinBlock, WithinModel};
use super::model::SecretHandle;
use super::shared_session::presence_manager::{
    text_selection_color, PresenceManager, MUTED_PARTICIPANT_COLOR,
};
use super::shared_session::render_util::SHARED_SESSION_AVATAR_DIAMETER;
use super::view::{
    BlocklistAIRenderContext, InlineBannerId, RichContentMetadata, SeparatorId,
    SharedSessionBanners, TerminalEditor, TerminalViewRenderContext, BLOCK_BANNER_HEIGHT,
};
use super::warpify::render::{draw_flag_pole, render_subshell_flag};
use super::TerminalModel;
use super::{heights_approx_eq, HEIGHT_FUDGE_FACTOR_LINES};
use crate::terminal::blockgrid_renderer::BlockGridParams;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::warpify::SubshellSource;

use crate::terminal::model::escape_sequences::{
    maybe_kitty_keyboard_escape_sequence, KeystrokeWithDetails, ToEscapeSequence,
};

/// The number of pixels at the bottom of padding where selection scrolling is performed.
const BOTTOM_VERTICAL_MARGIN: f32 = 10.0;

/// The alpha of text in the command grid.
const COMMAND_ALPHA: u8 = 255;

const SEPARATOR_TO_MONOSPACE_FONT_SIZE_RATIO: f32 = 0.8;
const SEPARATOR_LEFT_OFFSET: f32 = 20.;

/// Border widths for selected blocks
#[derive(Debug, Clone, Copy)]
struct SelectionBorderWidth {
    single: f32,
    tail_multi: f32,
    reg_multi: f32,
}

impl Default for SelectionBorderWidth {
    fn default() -> Self {
        if FeatureFlag::MinimalistUI.is_enabled() {
            Self {
                single: 0.0,
                tail_multi: 0.0,
                reg_multi: 0.0,
            }
        } else {
            Self {
                single: 2.0,
                tail_multi: 3.0,
                reg_multi: 1.5,
            }
        }
    }
}

const SHARED_SESSION_PARTICIPANT_SELECTION_BORDER_WIDTH: f32 = 1.5;

const SNACKBAR_HOVER_OPACITY: Opacity = 60;

// If the header is more than SNACKBAR_HEADER_MAX_RATIO percent of the entire
// grid height, we disable the "sticky" behavior and have the block scroll normally.
const SNACKBAR_HEADER_MAX_RATIO: f32 = 0.5;

/// Scrolling speed increases by this exponent curve the further out of bounds the
/// mouse has been dragged
const SELECTION_SCROLLING_ACCELERATION: f32 = 1.5;

const POLYNOMIAL_SCROLLING: ScrollingAcceleration =
    ScrollingAcceleration::Polynomial(SELECTION_SCROLLING_ACCELERATION);

const LINEAR_SCROLLING: ScrollingAcceleration = ScrollingAcceleration::Polynomial(1.);

/// Height for a block hover button element.
/// Without making the vertical size fixed, for some reason some elements (bookmark, block filter, shared session avatar)
/// have a height that extends down to the bottom of the window when there's a horizontal scroll bar, which messes with the on-hover behavior.
const BLOCK_HOVER_BUTTON_HEIGHT: f32 = 28.;

const TAG_AGENT_FOR_ASSISTANCE_TEXT: &str = "Tag agent for assistance";

const SAVE_AS_WORKFLOW_TEXT: &str = "Save as Workflow";
const SAVE_AS_WORKFLOW_SECRETS_TEXT: &str = "Blocks containing secrets cannot be saved.";

enum ScrollingAcceleration {
    Polynomial(f32),
}

impl ScrollingAcceleration {
    fn accelerated_delta(&self, delta: f32) -> f32 {
        match *self {
            ScrollingAcceleration::Polynomial(degree) => delta.powf(degree) / 100.0,
        }
    }
}

enum SelectionCursorRenderLocation {
    None,
    Start,
    End,
}

const OVERFLOW_BUTTON_ICON_PATH: &str = "bundled/svg/overflow.svg";
/// The number of lines from the top of the blocklist where we should show the snackbar toggle
/// button on mouse hover when the snackbar is collapsed.
const SNACKBAR_TOGGLE_BUTTON_HOVER_LINES: f32 = 4.;
const SNACKBAR_TOGGLE_BUTTON_WIDTH: f32 = 30.;
const SNACKBAR_TOGGLE_BUTTON_HEIGHT: f32 = 16.;

/// How far away from the right edge of the blocklist the selected block avatar should be
const SELECTED_BLOCK_AVATAR_EDGE_OFFSET: f32 = 25.;
/// Space between multiple avatars on a selected block.
const SPACE_BETWEEN_SELECTED_BLOCK_AVATARS: f32 = 2.;

const CLI_SUBAGENT_HORIZONTAL_MARGIN: f32 = 8.;
const CLI_SUBAGENT_VERTICAL_MARGIN: f32 = 8.;

pub type LabelBuilderFn = dyn Fn(
    Vec<BlockIndex>,
    &HashMap<BlockIndex, MouseStateHandle>,
    &TerminalModel,
    &AppContext,
) -> Vec<Box<dyn Element>>;

/// Note that the returned element length has to be the same as the length of block indices
/// passed in. Otherwise it will cause panicking in layout.
pub type BookmarkBuilderFn = dyn Fn(
    Vec<BlockIndex>,
    Option<BlockIndex>,
    &HashMap<BlockIndex, MouseStateHandle>,
    &AppContext,
) -> Vec<Option<Box<dyn Element>>>;

pub type FilterBuilderFn = dyn Fn(
    Vec<BlockIndex>,
    Option<BlockIndex>,           /* hovered_block_index */
    Option<BlockIndex>,           /* active_filter_editor_block_index */
    Option<&HashSet<BlockIndex>>, /* filtered_blocks */
    &HashMap<BlockIndex, MouseStateHandle>,
    &AppContext,
) -> Vec<Option<Box<dyn Element>>>;

#[derive(Debug, PartialEq, Copy, Clone, Eq, PartialOrd, Sequence, Hash)]
pub enum GridType {
    Prompt,
    Rprompt,          // Right side prompt
    PromptAndCommand, // Combined prompt/command grid.
    Output,
}

#[derive(Debug, Default, Clone)]
pub struct SnackbarHeaderState {
    pub snackbar_enabled: bool,
    pub show_snackbar: bool,
    pub hover_near_snackbar_area: bool,

    pub state_handle: SnackbarHeaderStateHandle,
}

pub type SnackbarHeaderStateHandle = Arc<Mutex<SnackbarHeader>>;

#[derive(Debug, Default, PartialEq, Copy, Clone)]
pub struct SnackbarHeaderPosition {
    // The origin that the header would have were it not pinned to the
    // top of the screen at the element origin; e.g. how much it should be
    // positioned above the element origin.
    // This is relative to the element origin at the time the snackbar was rendered.
    // E.g. the element origin will typically be something like (0, 35),
    // and if we are scrolled halfway through a big block, the header origin
    // will have a negative y coord.
    pub header_origin: Vector2F,

    // The position of the header in pixel coordinates
    pub rect: RectF,

    // The block index for the header.
    pub block_index: BlockIndex,
}

/// How to interpret a point that is within the snackbar
pub enum SnackbarTranslationMode {
    /// Interprets the point as being within the snackbar
    WithinSnackbar,

    /// Interprets the point as referring to whatever point would conceptually
    /// be "beneath" the snackbar at the given point
    UnderneathSnackbar,
}

pub struct SnackbarPoint {
    pub coord: Vector2F,
    pub translation_mode: SnackbarTranslationMode,
}

impl SnackbarPoint {
    fn within_snackbar(coord: Vector2F) -> Self {
        Self {
            coord,
            translation_mode: SnackbarTranslationMode::WithinSnackbar,
        }
    }

    fn underneath_snackbar(coord: Vector2F) -> Self {
        Self {
            coord,
            translation_mode: SnackbarTranslationMode::UnderneathSnackbar,
        }
    }
}

/// A struct that holds position and state information about the snackbar
/// based header, if there is one.  It's a wrapper around an optional header position
/// struct, plus additional info like its hover state.
#[derive(Default, Debug, PartialEq, Copy, Clone)]
pub struct SnackbarHeader {
    // The position of the snackbar header, if there is one.
    pub header_position: Option<SnackbarHeaderPosition>,

    // Whether the header is hovered
    pub hovered: bool,

    // Whether the header would've been shown if it wasn't toggled off by the snackbar toggle
    // button. This is used to know when the snackbar expand button should be drawn.
    pub hidden_by_toggle: bool,
}

// Treat the header as being in screen if the grid origin is within
// this distance from the element origin.  Prevents flickering of the
// header border when the header is close to being in screen.
// Flickering seems to occur because as you resize the grid, sometimes
// accumulated rounding errors for the size of space above the element add up
// to where a grid_origin.y() - element_origin.y() > 0 (and also greater than
// f32::EPSILON).  This fix is a bit of a cludge that ideally would go away
// if/when we switch to using f64 for coordinates.
const HEADER_IN_SCREEN_THRESHOLD_PX: f32 = 1.0;

impl SnackbarHeader {
    /// Updates the snackbar state for the given block and element and grid
    /// positions.
    /// Returns a copy of the latest version of the header state.
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        snackbar_enabled: bool,
        show_snackbar: bool,
        block: &Block,
        grid_origin: Vector2F,
        blocklist_element_bounds: RectF,
        params: &BlockGridParams,
        scroll_position: ScrollPosition,
    ) -> Option<SnackbarHeader> {
        if !snackbar_enabled {
            self.clear_state();
            return None;
        }

        if block.is_active_and_long_running()
            && Self::should_hide_snackbar_during_long_running_command(
                block,
                params,
                scroll_position,
                blocklist_element_bounds.size(),
            )
        {
            self.clear_state();
            return None;
        }

        // Don't show the snackbar for background blocks, since they have no
        // associated command.
        if block.is_background() {
            self.clear_state();
            return None;
        }

        let cell_size_height = params.grid_render_params.cell_size.y();
        let mut header_height = cell_size_height
            * block
                .block_section_offset_from_top(BlockSection::OutputGrid(Lines::zero()))
                .as_f64() as f32;

        let prompt_height_offset = cell_size_height * block.padding_top().as_f64() as f32;
        // Note that we need the snackbar header to AT LEAST be tall enough to contain the toolbelt icons!
        header_height = header_height.max(prompt_height_offset + BLOCK_HOVER_BUTTON_HEIGHT);

        // Don't show the snackbar if the header is more than SNACKBAR_HEADER_MAX_RATIO
        // percent of the entire grid height.
        if header_height
            > SNACKBAR_HEADER_MAX_RATIO
                * params
                    .grid_render_params
                    .size_info
                    .pane_height_px()
                    .as_f32()
        {
            self.clear_state();
            return None;
        }

        // Calculate where the bottom of the block is
        let block_bottom_y = grid_origin.y()
            + cell_size_height
                * (block.block_section_offset_from_top(BlockSection::EndOfBlock)).as_f64() as f32;

        let top_of_output = block_bottom_y - header_height;
        let element_origin = blocklist_element_bounds.origin();
        self.header_position =
            if grid_origin.y() + HEADER_IN_SCREEN_THRESHOLD_PX >= element_origin.y() {
                // If the block is totally in screen, there's no need to set a header, so clear it.
                self.clear_state();
                None
            } else if element_origin.y() < top_of_output {
                // The block intersects the screen and has some portion of its output
                // showing beneath the header
                Some(SnackbarHeaderPosition {
                    block_index: block.index(),
                    rect: RectF::new(element_origin, vec2f(params.bounds.width(), header_height)),
                    header_origin: grid_origin,
                })
            } else {
                // The block is about to be scrolled offscreen and only the header is left,
                // so we need to start scrolling the header itself.
                Some(SnackbarHeaderPosition {
                    block_index: block.index(),
                    rect: RectF::new(
                        vec2f(element_origin.x(), block_bottom_y - header_height),
                        vec2f(params.bounds.width(), header_height),
                    ),
                    header_origin: grid_origin,
                })
            };
        if self.header_position.is_some() && !show_snackbar {
            // Snackbar is collapsed by the toggle button. Hide snackbar, but mark that we would've
            // drawn it otherwise.
            self.clear_state();
            self.hidden_by_toggle = true;
            return None;
        }
        Some(*self)
    }

    /// Returns whether the snackbar should be hidden when there is a long running command.
    /// While a command is executing, we hide the snackbar if:
    /// 1) The user has scrolled OR
    /// 2) The height of the block's _output_ grid is taller than the content area.
    /// In the latter case, we hide the snackbar to make sure that commands like `git log` and
    /// `less` (which simulate full screen apps) don't have the snackbar shown on top of them.
    fn should_hide_snackbar_during_long_running_command(
        block: &Block,
        params: &BlockGridParams,
        scroll_position: ScrollPosition,
        element_size: Vector2F,
    ) -> bool {
        // A user has scrolled iff they are fixed at a pixel position. Otherwise, they are fixed to
        // the bottom of a block.
        let has_scrolled = matches!(scroll_position, ScrollPosition::FixedAtPosition { .. });

        let block_output_taller_than_content_area =
            block.output_grid_displayed_height().into_lines()
                - Pixels::new(element_size.y())
                    .to_lines(params.grid_render_params.size_info.cell_height_px)
                > HEIGHT_FUDGE_FACTOR_LINES;

        !has_scrolled && !block_output_taller_than_content_area
    }

    fn clear_state(&mut self) {
        self.header_position = None;
        self.hovered = false;
        self.hidden_by_toggle = false;
    }

    /// Returns the number of lines needed to translate a coord that is contained
    /// in the snackbar to where the coord would lie if the header was not
    /// pinned.
    pub fn header_translation_for_coord(
        &self,
        size: SizeInfo,
        snackbar_point: SnackbarPoint,
    ) -> Lines {
        if matches!(
            snackbar_point.translation_mode,
            SnackbarTranslationMode::UnderneathSnackbar
        ) {
            // There's no translation if we are looking for the coordinate "underneath" the snackbar
            return Lines::zero();
        }
        self.header_position
            .filter(|position| position.rect.contains_point(snackbar_point.coord))
            .map_or(Lines::zero(), |position| {
                (position.rect.min_y() - position.header_origin.y())
                    .into_pixels()
                    .to_lines(size.cell_height_px())
            })
    }

    /// Handles a mouse down event, possibly dispatching an event to scroll to the top
    /// of the header, if the header is set and the click is within the header
    fn mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        self.header_position
            .filter(|header_position| header_position.rect.contains_point(position))
            .is_some_and(|header_position| {
                ctx.dispatch_typed_action(TerminalAction::ScrollToTopOfBlock {
                    topmost_block: header_position.block_index,
                });
                true
            })
    }

    fn contains_point(&self, position: Vector2F) -> bool {
        self.header_position
            .is_some_and(|p| p.rect.contains_point(position))
    }

    fn header_rect(&self) -> Option<RectF> {
        self.header_position.map(|p| p.rect)
    }

    /// Determines the adjusted `scroll_top` value used for rendering snackbar text selections.
    /// Returns `None` when there is no snackbar header.
    fn calculate_scroll_top_for_selection(
        &self,
        size_info: &SizeInfo,
        scroll_top: Lines,
        origin: Vector2F,
        bounds: RectF,
    ) -> Option<Lines> {
        let (Some(header_position), Some(header_rect)) = (self.header_position, self.header_rect())
        else {
            return None;
        };

        // Render the snackbar header portion of the selection as though we were scrolled
        // to exactly the start of the snackbar header. Another way of looking at it is that
        // |header_displacement - origin_displacement| equals the absolute distance between
        // the top of the snackbar header and the top of the block on which it sits.
        let header_displacement = header_position
            .header_origin
            .y()
            .into_pixels()
            .to_lines(size_info.cell_height_px());
        let origin_displacement = origin
            .y()
            .into_pixels()
            .to_lines(size_info.cell_height_px());

        // If the block is about to be scrolled offscreen, the height of the snackbar header will
        // become larger than the height of the remaining visible portion of the block. In response
        // to this, the snackbar header itself will need to be scrolled as well.
        //
        // As an example, assume the snackbar header is 100px tall but only 80px of the block remains
        // on screen. Then, the snackbar header will need to be displaced upward by 100px - 80px = 20px.
        let offscreen_block_displacement = (bounds.min_y() - header_rect.min_y())
            .into_pixels()
            .to_lines(size_info.cell_height_px());

        Some(scroll_top + header_displacement - origin_displacement + offscreen_block_displacement)
    }

    /// Renders the given selection range across the snackbar header.
    /// Returns true if the selection was rendered, in which case the selection
    /// layer will need to later be closed by the caller.
    #[allow(clippy::too_many_arguments)]
    fn render_selection(
        &self,
        start: &SelectionPoint,
        end: &SelectionPoint,
        size_info: &SizeInfo,
        scroll_top: Lines,
        origin: Vector2F,
        bounds: RectF,
        color: ColorU,
        ctx: &mut PaintContext,
    ) -> bool {
        let Some(header_position) = &self.header_position else {
            return false;
        };
        let Some(clip_rect) = header_position.rect.intersection(bounds) else {
            return false;
        };
        let Some(adjusted_scroll_top) =
            self.calculate_scroll_top_for_selection(size_info, scroll_top, origin, bounds)
        else {
            return false;
        };

        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(clip_rect));

        // Note that we render the entire selection here and let the clipping
        // logic restrict it to just the snackbar. The alternative would
        // be to try and intersect start / end with the snackbar positions
        // but handling it at the render layer is simpler (if possibly slightly
        // slower, although not necessarily so)
        grid_renderer::render_selection(
            start,
            end,
            size_info,
            adjusted_scroll_top,
            origin,
            color,
            ctx,
        );
        ctx.scene.stop_layer();

        // In the case of a block based header, we need to clip beneath
        // the header in order to make the selection not overflow into the snackbar
        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(RectF::new(
                clip_rect.lower_left(),
                vec2f(bounds.width(), bounds.height() - clip_rect.height()),
            )));
        true
    }
}

/// Helper type for tracking mouseover state across frames
#[derive(Clone, Default)]
struct BlockListHoverState(Arc<AtomicBool>);

impl BlockListHoverState {
    /// Updates the hover state with a new value and returns the old value for comparison
    fn read_and_update(&self, new_value: bool) -> bool {
        self.0.swap(new_value, Ordering::Relaxed)
    }
}

pub struct BlockListElement {
    model: Arc<FairMutex<TerminalModel>>,
    size_info: SizeInfo,
    input_mode: InputMode,
    scroll_position: ScrollPosition,
    is_terminal_focused: bool,
    is_terminal_selecting: bool,
    /// This map contains the IDs of sessions that were subshells as keys. Their corresponding
    /// values are the command that spawned the subshell, which is needed to paint the "flag"
    subshell_sessions: HashMap<SessionId, SubshellSource>,
    size: Option<Vector2F>,
    /// These are the bounds the UI framework paints in, which are NOT necessarily the same as the visible bounds of the blocklist element.
    /// If we have a horizontal scroll bar (see horizontal_clipped_scroll_state), the UI bounds can go beyond the actually visible bounds.
    /// Use [`Self::mouse_position_is_in_bounds`] to check if a mouse position is within the visible bounds.
    ///
    /// Window with split pane:
    ///  ___________
    /// |     |     |
    /// |     |     |
    /// |     |     |
    /// |_____|_____|
    ///
    /// |-----| pane bounds
    ///
    /// |---------| block list element bounds can go beyond pane
    bounds: Option<RectF>,
    origin: Option<Point>,
    ui_font_family: FamilyId,
    font_family: FamilyId,
    font_size: f32,
    font_weight: Weight,
    line_height_ratio: f32,
    warp_theme: WarpTheme,
    ui_builder: UiBuilder,
    block_borders_enabled: bool,
    overflow_offset: f32,
    hovered_block_index: Option<BlockIndex>,
    overflow_menu_button: Option<Box<dyn Element>>,
    snackbar_toggle_button: Option<Box<dyn Element>>,
    ask_ai_assistant_button: Option<Box<dyn Element>>,
    save_as_workflow_button: Option<Box<dyn Element>>,
    restored_session_separator: Option<Box<dyn Element>>,
    inline_banners: HashMap<InlineBannerId, Box<dyn Element>>,
    /// Subshell separators are similar to banners, except they are smaller and only meant to show
    /// in compact mode. Setting Self::subshell_separator_height to 0 will effectively hide the
    /// flags.
    subshell_separators: HashMap<SeparatorId, Box<dyn Element>>,
    cli_subagent_views: HashMap<BlockId, Box<dyn Element>>,
    subshell_separator_height: f32,

    selected_blocks: SelectedBlocks,

    label_elements_builder: Box<LabelBuilderFn>,
    label_elements: HashMap<BlockIndex, Box<dyn Element>>,

    bookmark_element_builder: Box<BookmarkBuilderFn>,
    bookmark_elements: HashMap<BlockIndex, Box<dyn Element>>,

    /// The block whose filter we are actively editing.
    active_filter_editor_block_index: Option<BlockIndex>,
    filtered_blocks: Option<HashSet<BlockIndex>>,
    filter_elements_builder: Box<FilterBuilderFn>,
    filter_elements: HashMap<BlockIndex, Box<dyn Element>>,

    mouse_states: BlockListMouseStates,

    /// The range of block indices that are currently visible.
    visible_blocks: Option<Range<BlockIndex>>,

    /// All the items within the block list that are currently visible.
    visible_items: Option<Rc<Vec<VisibleItem>>>,

    /// This map stores the subshell flag Element for each Block that needs one. This is the flag
    /// that renders inside the block padding when spacing is not in compact mode.
    subshell_flags: HashMap<BlockIndex, Box<dyn Element>>,

    /// The snackbar header for the block list - this is the fixed "header block"
    /// that shows up when we are partially scrolled through a block.
    snackbar_header_state: SnackbarHeaderState,

    line_height: Option<Pixels>,
    scroll_top: Option<Lines>,

    disable_scroll: bool,

    /// An optional secret that is currently being hovered.
    hovered_secret: Option<SecretHandle>,

    highlighted_url: Option<WithinBlock<Link>>,
    link_tool_tip: Option<WithinBlock<Link>>,

    /// Used to save the position of the active cursor.
    terminal_view_id: EntityId,

    pane_state: SplitPaneState,

    enforce_minimum_contrast: EnforceMinimumContrast,

    /// This is helps us handling events properly on stacks. A stack will always
    /// put its children on higher z-indexes than its origin, so a hit test using the standard
    /// `z_index` method would always result in the event being covered (by the children of the
    /// stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    /// Then we use that upper bound to do the hit testing, which means a parent will always get
    /// events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,
    selection_ranges: Option<Vec1<SelectionRange>>,

    /// This banner is nested inside one of the blocks. Currently we only support 1 block banner in
    /// the BlockList, and it has to be in the active Block.
    block_banner: Option<Box<dyn Element>>,

    use_ligature_rendering: bool,

    /// When true, suppresses cursor rendering for CLI agents when rich input is open. For agents that draw their own cursor (SHOW_CURSOR off),
    /// the cursor cell is skipped. For agents that let Warp draw the cursor
    /// (SHOW_CURSOR on), the `draw_cursor` call and cursor contrast colouring
    /// are suppressed instead.
    hide_cursor_cell: bool,

    /// Child Elements to use for rich content inserted into the block list
    rich_content_elements: HashMap<EntityId, Box<dyn Element>>,
    rich_content_metadata: HashMap<EntityId, RichContentMetadata>,

    shared_session_banner_state: SharedSessionBanners,
    presence_manager: Option<ModelHandle<PresenceManager>>,
    presence_avatars: HashMap<ParticipantId, Box<dyn Element>>,

    horizontal_clipped_scroll_state: ClippedScrollStateHandle,

    /// Information about blocks and AI blocks used to render blocklist AI-specific decoration.
    ai_render_context: Rc<RefCell<BlocklistAIRenderContext>>,

    /// The last laid out size of the input view.
    input_size_at_last_frame: Vector2F,

    block_footer_elements: HashMap<BlockIndex, Box<dyn Element>>,

    find_model: ModelHandle<TerminalFindModel>,

    /// If `Some()`, lays out and renders the element next to the cursor.
    cursor_hint_text_element: Option<Box<dyn Element>>,

    /// Voice input toggle key code for CLI agent footer integration.
    #[cfg(feature = "voice_input")]
    voice_input_toggle_key_code: Option<KeyCode>,

    inline_menu_positioner: ModelHandle<InlineMenuPositioner>,
}

#[derive(Debug)]
pub enum VisibleItem {
    Block {
        // The index of the block.
        block_index: BlockIndex,
        // The index of the item within the block list sum tree.
        index: TotalIndex,
        subshell_session_id: Option<SessionId>,
    },
    RestoredBlockSeparator {
        index: TotalIndex,
        height_px: f32,
    },
    Gap {
        height_px: f32,
        index: TotalIndex,
    },
    Banner {
        height_px: f32,
        index: TotalIndex,
        banner_id: InlineBannerId,
    },
    SubshellSeparator {
        height_px: f32,
        index: TotalIndex,
        separator_id: SeparatorId,
    },
    RichContent {
        view_id: EntityId,
        height_px: f32,
        index: TotalIndex,
    },
}

impl VisibleItem {
    pub fn index(&self) -> TotalIndex {
        match self {
            VisibleItem::Block { index, .. } => *index,
            VisibleItem::Gap { index, .. } => *index,
            VisibleItem::RestoredBlockSeparator { index, .. } => *index,
            VisibleItem::Banner { index, .. } => *index,
            VisibleItem::SubshellSeparator { index, .. } => *index,
            VisibleItem::RichContent { index, .. } => *index,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum BlockSelectAction {
    ClearAllBlocks,
    /// Takes an [`Option<BlockIndex>`] because rich content blocks don't have a [`BlockIndex`]
    /// but still dispatch a [`MouseDown`] event on click.
    MouseDown(Option<BlockIndex>),
    MouseUp {
        block_index: BlockIndex,
        is_ctrl_down: bool,
        is_cmd_down: bool,
        is_shift_down: bool,
    },
}

pub type BlockTextSelectAction = SelectAction<BlockListPoint>;

#[derive(Debug, Clone, Copy)]
pub enum BlockHoverAction {
    Begin {
        position: Vector2F,
        block_index: BlockIndex,
    },
    Clear,
}

/// Every possible reason for which a [`BlockListContextMenu`] may be surfaced. It should never be
/// possible to trigger more than one [`BlockListMenuSource`] using a single action.
#[derive(Clone, Copy, Debug)]
pub enum BlockListMenuSource {
    BlockOverflowButton {
        block_index: BlockIndex,
    },
    BlockKeybinding {
        block_index: BlockIndex,
    },
    RegularBlockRightClick {
        block_index: BlockIndex,
        position_in_terminal_view: Vector2F,
    },
    RegularTextRightClick {
        position_in_terminal_view: Vector2F,
    },
    RichContentBlockRightClick {
        rich_content_view_id: EntityId,
        position_in_terminal_view: Vector2F,
    },
    /// We use [`position_in_rich_content`] here because text selection right-click logic for rich
    /// content views is handled in the [`SelectableArea`] element, within which there is no way of
    /// determining the origin of the entire [`BlockListElement`].
    RichContentTextRightClick {
        rich_content_view_id: EntityId,
        position_in_rich_content: Vector2F,
    },
    /// Catches all right-clicks that don't fall within the bounds of any type of block. This mostly
    /// refers to the empty space that is exposed when existing blocks have yet to fill the window.
    OutsideBlockRightClick {
        position_in_terminal_view: Vector2F,
    },
}

#[derive(Clone, Default)]
pub struct BlockListMouseStates {
    hover_state: BlockListHoverState,
    pub label_mouse_states: HashMap<BlockIndex, MouseStateHandle>,
    pub bookmark_mouse_states: HashMap<BlockIndex, MouseStateHandle>,
    pub overflow_menu_button_mouse_state: MouseStateHandle,
    pub ai_assistant_button_mouse_state: MouseStateHandle,
    pub save_as_workflow_button_mouse_state: MouseStateHandle,
    pub filter_mouse_states: HashMap<BlockIndex, MouseStateHandle>,
    pub snackbar_toggle_button_mouse_state: MouseStateHandle,
}

impl BlockListElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Arc<FairMutex<TerminalModel>>,
        find_model: ModelHandle<TerminalFindModel>,
        input_mode: InputMode,
        terminal_view_render_context: TerminalViewRenderContext,
        mouse_states: BlockListMouseStates,
        last_snackbar_header_state: SnackbarHeaderState,
        terminal_spacing: &TerminalSpacing,
        enforce_minimum_contrast: EnforceMinimumContrast,
        appearance: &Appearance,
        label_elements_builder: Box<LabelBuilderFn>,
        bookmark_element_builder: Box<BookmarkBuilderFn>,
        filter_elements_builder: Box<FilterBuilderFn>,
        inline_banners: HashMap<InlineBannerId, Box<dyn Element>>,
        subshell_separators: HashMap<SeparatorId, Box<dyn Element>>,
        cli_subagent_views: HashMap<BlockId, Box<dyn Element>>,
        selection_ranges: Option<Vec1<SelectionRange>>,
        block_banner: Option<Box<dyn Element>>,
        shared_session_banners: SharedSessionBanners,
        input_size_at_last_frame: Vector2F,
        inline_menu_positioner: ModelHandle<InlineMenuPositioner>,
        cursor_hint_text_element: Option<Box<dyn Element>>,
    ) -> Self {
        let highlighted_url = terminal_view_render_context
            .highlighted_url
            .map(TryInto::try_into)
            .and_then(Result::ok);

        let link_tool_tip = terminal_view_render_context
            .link_tool_tip
            .map(TryInto::try_into)
            .and_then(Result::ok);

        Self {
            model,
            find_model,
            input_mode,
            terminal_view_id: terminal_view_render_context.terminal_view_id,
            size_info: terminal_view_render_context.size_info,
            scroll_position: terminal_view_render_context.scroll_position,
            is_terminal_focused: terminal_view_render_context.is_terminal_focused,
            is_terminal_selecting: terminal_view_render_context.is_terminal_selecting,
            subshell_sessions: terminal_view_render_context.spawning_command_for_subshell_sessions,
            subshell_flags: HashMap::new(),
            size: None,
            bounds: None,
            origin: None,
            snackbar_header_state: last_snackbar_header_state,
            ui_font_family: appearance.ui_font_family(),
            font_family: appearance.monospace_font_family(),
            font_size: appearance.monospace_font_size(),
            font_weight: appearance.monospace_font_weight(),
            line_height_ratio: appearance.ui_builder().line_height_ratio(),
            warp_theme: appearance.theme().clone(),
            ui_builder: appearance.ui_builder().clone(),
            block_borders_enabled: terminal_spacing.block_borders_enabled,
            overflow_offset: terminal_spacing.overflow_offset,
            subshell_separator_height: terminal_spacing.subshell_separator_height,
            hovered_block_index: None,
            overflow_menu_button: None,
            ask_ai_assistant_button: None,
            save_as_workflow_button: None,
            snackbar_toggle_button: None,
            restored_session_separator: None,
            inline_banners,
            subshell_separators,
            selected_blocks: terminal_view_render_context.selected_blocks,
            label_elements: HashMap::new(),
            label_elements_builder,
            bookmark_element_builder,
            bookmark_elements: HashMap::new(),
            filter_elements_builder,
            filter_elements: HashMap::new(),
            mouse_states,
            visible_blocks: None,
            visible_items: None,
            line_height: None,
            scroll_top: None,
            disable_scroll: terminal_view_render_context.is_context_menu_open
                || terminal_view_render_context.is_waterfall_gap_mode,
            highlighted_url,
            link_tool_tip,
            pane_state: terminal_view_render_context.pane_state,
            enforce_minimum_contrast,
            child_max_z_index: None,
            selection_ranges,
            block_banner,
            hovered_secret: terminal_view_render_context.hovered_secret,
            use_ligature_rendering: false,
            hide_cursor_cell: false,
            active_filter_editor_block_index: None,
            filtered_blocks: None,
            rich_content_elements: HashMap::new(),
            rich_content_metadata: HashMap::new(),
            shared_session_banner_state: shared_session_banners,
            presence_manager: None,
            presence_avatars: HashMap::new(),
            horizontal_clipped_scroll_state: terminal_view_render_context
                .horizontal_clipped_scroll_state,
            ai_render_context: terminal_view_render_context.ai_render_context,
            input_size_at_last_frame,
            block_footer_elements: HashMap::new(),
            cursor_hint_text_element,
            cli_subagent_views,
            inline_menu_positioner,
            #[cfg(feature = "voice_input")]
            voice_input_toggle_key_code: None,
        }
    }

    /// Sets the voice input toggle key code for CLI agent footer integration.
    #[cfg(feature = "voice_input")]
    pub fn with_voice_input_toggle_key(mut self, key_code: Option<KeyCode>) -> Self {
        self.voice_input_toggle_key_code = key_code;
        self
    }

    pub fn with_ligature_rendering(mut self) -> Self {
        self.use_ligature_rendering = true;
        self
    }

    pub fn with_hide_cursor_cell(mut self) -> Self {
        self.hide_cursor_cell = true;
        self
    }

    pub fn with_rich_content<I>(mut self, content: I) -> Self
    where
        I: IntoIterator<Item = (EntityId, Box<dyn Element>, Option<RichContentMetadata>)>,
    {
        for (id, element, metadata) in content.into_iter() {
            self.rich_content_elements.insert(id, element);
            if let Some(metadata) = metadata {
                self.rich_content_metadata.insert(id, metadata);
            }
        }
        self
    }

    /// Returns `ViewportState` for the element after layout.
    ///
    /// This _must_ be called after `BlockListElement::layout` (which determines the element size
    /// required for viewport logic), otherwise will panic.
    fn viewport_state_after_layout<'a>(&self, block_list: &'a BlockList) -> ViewportState<'a> {
        ViewportState::new(
            block_list,
            self.snackbar_header_state.clone(),
            self.input_mode,
            self.size_info,
            self.scroll_position,
            self.visible_items.clone(),
            self.horizontal_clipped_scroll_state.clone(),
            self.size
                .expect("Cannot construct ViewportState prior to element layout."),
            self.input_size_at_last_frame,
            if self.ai_render_context.borrow().has_active_conversation() {
                AutoscrollBehavior::WhenScrolledToEnd
            } else {
                AutoscrollBehavior::Always
            },
            self.inline_menu_positioner.clone(),
        )
    }

    fn snackbar_header_state(&self) -> MutexGuard<'_, SnackbarHeader> {
        self.snackbar_header_state
            .state_handle
            .lock()
            .expect("locking snackbar header state")
    }

    pub fn with_filtered_blocks(mut self, filtered_blocks: HashSet<BlockIndex>) -> Self {
        self.filtered_blocks = Some(filtered_blocks);
        self
    }

    pub fn with_active_block_filter_editor(
        mut self,
        active_filter_editor_block_index: BlockIndex,
    ) -> Self {
        self.active_filter_editor_block_index = Some(active_filter_editor_block_index);
        self
    }

    /// Returns an updated version of the [`BlockListElement`] that renders the toolbelt on top of the hovered block.
    pub fn with_hovered_index(
        mut self,
        block_index: BlockIndex,
        model: &TerminalModel,
        should_render_tooltip_below_button: bool,
        app: &AppContext,
    ) -> Self {
        self.hovered_block_index = Some(block_index);
        let icon_color = self
            .warp_theme
            .sub_text_color(self.warp_theme.surface_2())
            .into_solid();

        let icon = Container::new(
            ConstrainedBox::new(Icon::new(OVERFLOW_BUTTON_ICON_PATH, icon_color).finish())
                .with_height(26.)
                .with_width(26.)
                .finish(),
        );

        self.overflow_menu_button = Some(
            SavePosition::new(
                render_hoverable_block_button(
                    icon,
                    None,
                    false,
                    true,
                    self.mouse_states.overflow_menu_button_mouse_state.clone(),
                    &self.warp_theme,
                    &self.ui_builder,
                    move |ctx, _, _| {
                        ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(
                            BlockListMenuSource::BlockOverflowButton { block_index },
                        ));
                    },
                ),
                format!("context_menu_button_{block_index}").as_str(),
            )
            .finish(),
        );

        let snackbar_toggle_icon;
        let rounded_corners;
        if self.snackbar_header_state.show_snackbar {
            // Snackbar expanded. Show collapse button attached to bottom of snackbar.
            snackbar_toggle_icon = UIIcon::Icon::ChevronUp;
            rounded_corners = CornerRadius::with_top(Radius::Pixels(8.));
        } else {
            // Snackbar collapsed. Show expand button attached to top of blocklist.
            snackbar_toggle_icon = UIIcon::Icon::ChevronDown;
            rounded_corners = CornerRadius::with_bottom(Radius::Pixels(8.));
        };
        let icon = Container::new(
            ConstrainedBox::new(Icon::new(snackbar_toggle_icon.into(), icon_color).finish())
                .with_width(SNACKBAR_TOGGLE_BUTTON_WIDTH)
                .with_height(SNACKBAR_TOGGLE_BUTTON_HEIGHT)
                .finish(),
        );

        self.snackbar_toggle_button = Some(
            Hoverable::new(
                self.mouse_states.snackbar_toggle_button_mouse_state.clone(),
                |state| {
                    let background = if state.is_clicked() || state.is_hovered() {
                        self.warp_theme.surface_2()
                    } else {
                        self.warp_theme.surface_1()
                    };
                    icon.with_corner_radius(rounded_corners)
                        .with_background(background)
                        .finish()
                },
            )
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::ToggleSnackbarInActivePane);
            })
            .finish(),
        );

        if AISettings::as_ref(app).is_any_ai_enabled(app) {
            let icon = Container::new(
                ConstrainedBox::new(if FeatureFlag::AgentView.is_enabled() {
                    UIIcon::Icon::Paperclip
                        .to_warpui_icon(icon_color.into())
                        .finish()
                } else if FeatureFlag::AgentMode.is_enabled() {
                    UIIcon::Icon::Stars
                        .to_warpui_icon(icon_color.into())
                        .finish()
                } else {
                    Icon::new(AI_ASSISTANT_SVG_PATH, icon_color).finish()
                })
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
            .with_vertical_padding(5.)
            .with_padding_left(6.)
            .with_padding_right(4.);

            let (ai_button_action, ai_button_tooltip) = if FeatureFlag::AgentMode.is_enabled() {
                let active_block = model.block_list().active_block();
                let has_active_long_running_command = active_block.is_active_and_long_running();

                if has_active_long_running_command && active_block.index() == block_index {
                    (
                        Some(TerminalAction::SetInputModeAgent),
                        TAG_AGENT_FOR_ASSISTANCE_TEXT,
                    )
                } else {
                    (
                        Some(TerminalAction::AskAIAssistant { block_index }),
                        *ATTACH_AS_AGENT_MODE_CONTEXT_TEXT,
                    )
                }
            } else {
                (
                    Some(TerminalAction::AskAIAssistant { block_index }),
                    ASK_AI_ASSISTANT_TEXT,
                )
            };

            let tooltip = ToolbeltButtonTooltip {
                label: ai_button_tooltip.to_owned(),
                tool_tip_below_button: should_render_tooltip_below_button,
            };

            let element = render_hoverable_block_button(
                icon,
                Some(tooltip),
                false,
                true,
                self.mouse_states.ai_assistant_button_mouse_state.clone(),
                &self.warp_theme,
                &self.ui_builder,
                move |ctx: &mut EventContext, _, _| {
                    if let Some(action) = ai_button_action.clone() {
                        ctx.dispatch_typed_action(action);
                    }
                },
            );
            self.ask_ai_assistant_button = Some(element);
        }

        if WarpDriveSettings::is_warp_drive_enabled(app) {
            let icon = Container::new(
                ConstrainedBox::new(
                    ui_components::icons::Icon::Save
                        .to_warpui_icon(icon_color.into())
                        .finish(),
                )
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
            .with_uniform_padding(4.);

            let element = if PrivacySettings::as_ref(app).is_enterprise_secret_redaction_enabled()
                && model
                    .block_list()
                    .block_at(block_index)
                    .is_some_and(|block| block.num_secrets_obfuscated() > 0)
            {
                // If enterprise secret redaction is enabled and the block contains secrets,
                // disable save as workflow button + show different tooltip messaging.
                render_hoverable_block_button(
                    icon,
                    Some(ToolbeltButtonTooltip {
                        label: SAVE_AS_WORKFLOW_SECRETS_TEXT.to_owned(),
                        tool_tip_below_button: should_render_tooltip_below_button,
                    }),
                    false,
                    false,
                    self.mouse_states
                        .save_as_workflow_button_mouse_state
                        .clone(),
                    &self.warp_theme,
                    &self.ui_builder,
                    move |ctx: &mut EventContext, _, _| {
                        ctx.dispatch_typed_action(TerminalAction::OpenWorkflowModalForBlock(
                            block_index,
                        ));
                    },
                )
            } else {
                render_hoverable_block_button(
                    icon,
                    Some(ToolbeltButtonTooltip {
                        label: SAVE_AS_WORKFLOW_TEXT.to_owned(),
                        tool_tip_below_button: should_render_tooltip_below_button,
                    }),
                    false,
                    true,
                    self.mouse_states
                        .save_as_workflow_button_mouse_state
                        .clone(),
                    &self.warp_theme,
                    &self.ui_builder,
                    move |ctx: &mut EventContext, _, _| {
                        ctx.dispatch_typed_action(TerminalAction::OpenWorkflowModalForBlock(
                            block_index,
                        ));
                    },
                )
            };

            self.save_as_workflow_button = Some(element);
        }

        self
    }

    pub fn with_shared_session_presence(
        mut self,
        presence_avatars: HashMap<ParticipantId, Box<dyn Element>>,
        presence_manager: ModelHandle<PresenceManager>,
    ) -> Self {
        self.presence_avatars = presence_avatars;
        self.presence_manager = Some(presence_manager);
        self
    }

    /// We only want to process control characters here and return `false` for everything else.
    /// That way, we'll receive a `warpui::Event::TypedCharacters` event for printable characters.
    /// So `TerminalAction::KeyDown` is for control characters only while
    /// `TerminalAction::TypedCharacters` is for characters that can go into the editor.
    fn key_down(&mut self, chars: &str, ctx: &mut EventContext) -> bool {
        if self.is_terminal_focused && !chars.is_empty() && chars.chars().all(|c| c.is_control()) {
            ctx.dispatch_typed_action(TerminalAction::KeyDown(chars.to_string()));
            true
        } else {
            false
        }
    }

    fn typed_characters(&mut self, chars: &str, ctx: &mut EventContext) -> bool {
        if self.is_terminal_focused && !chars.is_empty() {
            ctx.dispatch_typed_action(TerminalAction::TypedCharacters(chars.to_string()));
        }
        true
    }

    fn set_marked_text(
        &mut self,
        marked_text: &str,
        selected_range: &Range<usize>,
        ctx: &mut EventContext,
    ) -> bool {
        if self.is_terminal_focused {
            ctx.dispatch_typed_action(TerminalAction::SetMarkedText {
                marked_text: UserInput::new(marked_text),
                selected_range: selected_range.clone(),
            });
        }
        true
    }

    fn clear_marked_text(&mut self, ctx: &mut EventContext) -> bool {
        if self.is_terminal_focused {
            ctx.dispatch_typed_action(TerminalAction::ClearMarkedText);
        }
        true
    }

    fn ctrl_d(&self, ctx: &mut EventContext) -> bool {
        if self.is_terminal_focused {
            ctx.dispatch_typed_action(TerminalAction::CtrlD);
            true
        } else {
            false
        }
    }

    fn scroll_internal(
        &self,
        position: Vector2F,
        delta: Vector2F,
        precise: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.disable_scroll {
            return false;
        }
        if self.is_mouse_position_within_bounds(position) {
            let cell_height = self.size_info.cell_height_px;
            let delta_lines = if precise {
                // Handle Trackpad Scroll by converting pixel height into fractional lines.
                delta.y().into_pixels().to_lines(cell_height)
            } else {
                // Handle Mouse Scroll, whose delta is already in terms of lines.
                delta.y().into_lines()
            };

            let blocklist_point = self.coord_to_point(
                SnackbarPoint::within_snackbar(position),
                ClampingMode::ClampToGridIfWithinBlock,
            );

            let model = self.model.lock();
            let viewport = self.viewport_state_after_layout(model.block_list());

            if let Some(blocklist_point) = blocklist_point {
                if let Some(block_index) = viewport.block_index_from_point(blocklist_point) {
                    let on_long_running_block = model
                        .block_list()
                        .block_at(block_index)
                        .is_some_and(|block| block.is_active_and_long_running());

                    if on_long_running_block && !should_intercept_scroll(&model, app) {
                        // Send scroll event to PTY as mouse wheel action.
                        // Convert Lines to i32 by rounding to nearest non-zero integer.
                        let delta = round_nonzero(delta_lines.as_f64());
                        if delta != 0 {
                            let mouse_state = MouseState::new(
                                MouseButton::Wheel,
                                MouseAction::Scrolled { delta },
                                Default::default(),
                            );
                            let grid_point = IndexPoint::new(
                                blocklist_point.row.as_f64().round() as usize,
                                blocklist_point.column,
                            );

                            drop(model);
                            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(
                                mouse_state.set_point(grid_point),
                            ));
                            return true;
                        }
                    }
                }
            }
            ctx.dispatch_typed_action(TerminalAction::Scroll { delta: delta_lines });
            true
        } else {
            false
        }
    }

    /// Converts a pixel coordinate to a pixel in the `BlockList` coordinate space
    fn coord_to_point(
        &self,
        snackbar_point: SnackbarPoint,
        clamping_mode: ClampingMode,
    ) -> Option<BlockListPoint> {
        let model = self.model.lock();
        let viewport = self.viewport_state_after_layout(model.block_list());
        viewport.screen_coord_to_blocklist_point(
            self.bounds?.origin(),
            snackbar_point,
            clamping_mode,
        )
    }

    fn right_mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        if self.is_mouse_position_within_bounds(position) {
            let position_in_terminal_view = self.position_in_terminal_view(position);

            if self.is_mouse_position_within_selection(position) {
                ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(
                    BlockListMenuSource::RegularTextRightClick {
                        position_in_terminal_view,
                    },
                ));
                return true;
            }

            let blocklist_point = self.coord_to_point(
                SnackbarPoint::within_snackbar(position),
                ClampingMode::ClampToGridIfWithinBlock,
            );

            let block_index = blocklist_point.and_then(|point| {
                let model = self.model.lock();
                let viewport = self.viewport_state_after_layout(model.block_list());
                viewport.block_index_from_point(point)
            });

            let source = match block_index {
                Some(index) => BlockListMenuSource::RegularBlockRightClick {
                    block_index: index,
                    position_in_terminal_view,
                },
                None => {
                    let rich_content_view_id = blocklist_point.and_then(|point| {
                        let model = self.model.lock();
                        let viewport = self.viewport_state_after_layout(model.block_list());
                        match viewport.block_height_item_from_point(point) {
                            Some(BlockHeightItem::RichContent(item)) => Some(item.view_id),
                            _ => None,
                        }
                    });
                    match rich_content_view_id {
                        Some(rich_content_view_id) => {
                            BlockListMenuSource::RichContentBlockRightClick {
                                rich_content_view_id,
                                position_in_terminal_view,
                            }
                        }
                        None => BlockListMenuSource::OutsideBlockRightClick {
                            position_in_terminal_view,
                        },
                    }
                }
            };

            ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(source));
            true
        } else {
            false
        }
    }

    fn middle_mouse_down(&self, position: Vector2F, ctx: &mut EventContext) -> bool {
        let mut dispatched_screen_click = false;
        let handled = if self.is_mouse_position_within_bounds(position) {
            let position = self
                .coord_to_point(
                    SnackbarPoint::within_snackbar(position),
                    ClampingMode::ReturnNoneIfNotInGrid,
                )
                .and_then(|point| {
                    let model = self.model.lock();
                    let viewport = self.viewport_state_after_layout(model.block_list());
                    let within_block = viewport.block_list_point_to_grid_point(point);
                    drop(model);

                    if within_block.is_some() {
                        dispatched_screen_click = true;
                    }
                    within_block.map(WithinModel::BlockList)
                });
            ctx.dispatch_typed_action(TerminalAction::MiddleClickOnGrid { position });
            true
        } else {
            false
        };

        // If we haven't dispatched screen click, we need to dispatch MaybeDismissToolTip
        // to clear the tooltip in case user is not clicking inside a block.
        if !dispatched_screen_click {
            ctx.dispatch_typed_action(TerminalAction::MaybeDismissToolTip {
                from_keybinding: false,
            });
        }

        handled
    }

    fn mouse_down(
        &self,
        position: Vector2F,
        click_count: u32,
        is_first_mouse: bool,
        modifiers: &ModifiersState,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if is_first_mouse {
            // If the block list is receiving the first mouse click on activation, we should
            // not handle the event directly (typically by selecting a block). Instead,
            // let a view higher up the view tree decide how to handle it.
            return false;
        }

        if !self.pane_state.is_focused() {
            return false;
        }

        if self.is_mouse_position_within_bounds(position) {
            ctx.dispatch_typed_action(TerminalAction::CloseContextMenu);
            let mut should_redetermine_focus = true;

            match self.coord_to_point(
                SnackbarPoint::within_snackbar(position),
                ClampingMode::ClampToGridIfWithinBlock,
            ) {
                Some(point) => {
                    let model = self.model.lock();
                    let viewport = self.viewport_state_after_layout(model.block_list());

                    match viewport.block_height_item_from_point(point) {
                        Some(BlockHeightItem::Block { .. }) => {
                            // Offset it by the origin to make sure we are passing in the relative position
                            // to the terminal bounds.
                            let bounds = self
                                .bounds
                                .expect("Bounds should be set before event dispatching");
                            let side = self.size_info.get_mouse_side(position - bounds.origin());
                            let selection_type = if FeatureFlag::RectSelection.is_enabled() {
                                SelectionType::from_mouse_event(*modifiers, click_count)
                            } else {
                                SelectionType::from_click_count(click_count)
                            };

                            let block_index = match viewport.block_index_from_point(point) {
                                None => {
                                    ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                                        action: BlockSelectAction::ClearAllBlocks,
                                        should_redetermine_focus,
                                    });
                                    return true;
                                }
                                Some(block_index) => block_index,
                            };

                            if self.snackbar_header_state().mouse_down(position, ctx) {
                                return true;
                            }

                            let on_long_running_block = model
                                .block_list()
                                .block_at(block_index)
                                .is_some_and(|block| block.is_active_and_long_running());

                            // On mobile, request soft keyboard so users can input
                            if warpui::platform::is_mobile_device() && on_long_running_block {
                                ctx.request_soft_keyboard();
                            }

                            if on_long_running_block
                                && !should_intercept_mouse(&model, modifiers.shift, app)
                            {
                                let within_block = viewport.block_list_point_to_grid_point(point);

                                if let Some(within_block) = within_block {
                                    let grid_point =
                                        point_from_first_visible_row(&viewport, within_block);
                                    let mouse_state = MouseState::new(
                                        MouseButton::Left,
                                        MouseAction::Pressed,
                                        *modifiers,
                                    );
                                    drop(model);
                                    ctx.dispatch_typed_action(TerminalAction::AltMouseAction(
                                        mouse_state.set_point(grid_point),
                                    ));
                                    return true;
                                }
                            }

                            // If the find bar is open, allow selecting the active block again so users can
                            // scope find to that block.
                            let find_bar_open = self.find_model.as_ref(app).is_find_bar_open();
                            // If there's only a non-simple selection, clear the clicked block to avoid a
                            // text selection and a block selection being rendered at the same time.
                            // Note we should only dispatch block selection actions when the mouse is not
                            // clicking on highlighted links or a rich block or if this mouse_down is unselecting
                            // text.
                            if selection_type == SelectionType::Simple
                                && self.highlighted_url.is_none()
                                && self.hovered_secret.is_none()
                                && model.block_list().selection().is_none()
                                // Clicking on an active long running block should focus that block instead,
                                // except when the find bar is open, in which case we allow selecting it.
                                && (!on_long_running_block || find_bar_open)
                            {
                                ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                                    action: BlockSelectAction::MouseDown(Some(block_index)),
                                    should_redetermine_focus,
                                });
                            } else {
                                ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                                    action: BlockSelectAction::ClearAllBlocks,
                                    should_redetermine_focus,
                                });
                            }

                            ctx.dispatch_typed_action(TerminalAction::BlockTextSelect(
                                BlockTextSelectAction::Begin {
                                    point,
                                    side,
                                    selection_type,
                                    position,
                                },
                            ));
                        }
                        // While rich content blocks can't be selected like command blocks,
                        // text selections can still originate in them (i.e. with AI blocks)
                        Some(BlockHeightItem::RichContent(RichContentItem { view_id, .. })) => {
                            let bounds = self
                                .bounds
                                .expect("Bounds should be set before event dispatching");
                            let side = self.size_info.get_mouse_side(position - bounds.origin());
                            let selection_type = if FeatureFlag::RectSelection.is_enabled() {
                                SelectionType::from_mouse_event(*modifiers, click_count)
                            } else {
                                SelectionType::from_click_count(click_count)
                            };

                            if self.snackbar_header_state().mouse_down(position, ctx) {
                                return true;
                            }

                            if let Some(RichContentMetadata::AIBlock { .. }) =
                                self.rich_content_metadata.get(view_id)
                            {
                                should_redetermine_focus = false;
                            }

                            ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                                action: BlockSelectAction::MouseDown(None),
                                should_redetermine_focus,
                            });
                            ctx.dispatch_typed_action(TerminalAction::BlockTextSelect(
                                BlockTextSelectAction::Begin {
                                    point,
                                    side,
                                    selection_type,
                                    position,
                                },
                            ));
                        }
                        _ => {
                            ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                                action: BlockSelectAction::ClearAllBlocks,
                                should_redetermine_focus,
                            });
                        }
                    }

                    drop(model);
                }
                None => {
                    ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                        action: BlockSelectAction::ClearAllBlocks,
                        should_redetermine_focus,
                    });
                }
            }

            if should_redetermine_focus {
                ctx.dispatch_typed_action(TerminalAction::Focus);
            }

            true
        } else {
            false
        }
    }

    fn mouse_up(
        &self,
        position: Vector2F,
        modifiers: &ModifiersState,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.is_terminal_selecting {
            ctx.dispatch_typed_action(TerminalAction::BlockTextSelect(BlockTextSelectAction::End));
        }

        let handled = if self.is_mouse_position_within_bounds(position) {
            if let Some(point) = self.coord_to_point(
                SnackbarPoint::within_snackbar(position),
                ClampingMode::ReturnNoneIfNotInGrid,
            ) {
                let model = self.model.lock();
                let viewport = self.viewport_state_after_layout(model.block_list());

                if let Some(within_block) = viewport.block_list_point_to_grid_point(point) {
                    let on_long_running_block = model
                        .block_list()
                        .block_at(within_block.block_index)
                        .is_some_and(|block| block.is_active_and_long_running());

                    let alt_mouse_action = on_long_running_block
                        && !should_intercept_mouse(&model, modifiers.shift, app);

                    if alt_mouse_action {
                        let grid_point = point_from_first_visible_row(&viewport, within_block);
                        let mouse_state =
                            MouseState::new(MouseButton::Left, MouseAction::Released, *modifiers);
                        drop(model);
                        ctx.dispatch_typed_action(TerminalAction::AltMouseAction(
                            mouse_state.set_point(grid_point),
                        ));
                        return true;
                    }

                    drop(model);
                    ctx.dispatch_typed_action(TerminalAction::ClickOnGrid {
                        position: WithinModel::BlockList(within_block),
                        modifiers: *modifiers,
                    });
                }
            }

            if let Some(point) = self.coord_to_point(
                SnackbarPoint::within_snackbar(position),
                ClampingMode::ClampToGrid,
            ) {
                let model = self.model.lock();
                let viewport = self.viewport_state_after_layout(model.block_list());
                let block_index = match viewport.block_index_from_point(point) {
                    None => {
                        return true;
                    }
                    Some(block_index) => block_index,
                };

                if self.highlighted_url.is_none()
                    && self.hovered_secret.is_none()
                    && !self.snackbar_header_state().contains_point(position)
                {
                    ctx.dispatch_typed_action(TerminalAction::BlockSelect {
                        action: BlockSelectAction::MouseUp {
                            block_index,
                            is_ctrl_down: modifiers.ctrl,
                            is_cmd_down: modifiers.cmd,
                            is_shift_down: modifiers.shift,
                        },
                        should_redetermine_focus: true,
                    });
                }
            }
            true
        } else {
            false
        };

        handled
    }

    /// Handle a mouse move event when we've determined the mouse is over the block list (and not
    /// obscured by any modals or other elements).
    fn mouse_over(&self, position: Vector2F, app: &AppContext, ctx: &mut EventContext) {
        let snackbar_hovered = self
            .snackbar_header_state()
            .header_rect()
            .is_some_and(|rect| rect.contains_point(position));
        let was_snackbar_hovered =
            mem::replace(&mut self.snackbar_header_state().hovered, snackbar_hovered);
        if snackbar_hovered != was_snackbar_hovered {
            ctx.dispatch_typed_action(TerminalAction::BlockSnackbarHover {
                is_hovered: snackbar_hovered,
            });
        }

        if !self.snackbar_header_state.show_snackbar {
            // The snackbar is collapsed. If the mouse is near the top of the blocklist, we might want
            // to display the expand button.
            let hover_near_snackbar_area = self.origin().is_some_and(|origin| {
                position.y() - origin.y()
                    < self.size_info.cell_height_px().as_f32() * SNACKBAR_TOGGLE_BUTTON_HOVER_LINES
            });
            if hover_near_snackbar_area != self.snackbar_header_state.hover_near_snackbar_area {
                ctx.dispatch_typed_action(TerminalAction::BlockNearSnackbarHover {
                    is_hovered: hover_near_snackbar_area,
                });
            }
        }

        let block_list_point = self.coord_to_point(
            SnackbarPoint::within_snackbar(position),
            ClampingMode::ReturnNoneIfNotInGrid,
        );

        let grid_point = {
            let model = self.model.lock();
            let viewport = self.viewport_state_after_layout(model.block_list());
            block_list_point.and_then(|point| viewport.block_list_point_to_grid_point(point))
        };

        let secret_redaction = get_secret_obfuscation_mode(app);
        let secret_handle = if secret_redaction.should_redact_secret() {
            grid_point.and_then(|grid_point| {
                self.model
                    .lock()
                    .secret_at_point(&WithinModel::BlockList(grid_point))
                    .map(|(handle, _)| handle)
            })
        } else {
            None
        };

        ctx.dispatch_typed_action(TerminalAction::MaybeHoverSecret { secret_handle });

        if secret_handle.is_some() {
            // Clear any link hover by dispatching with None position
            ctx.dispatch_typed_action(TerminalAction::MaybeLinkHover {
                position: None,
                from_editor: TerminalEditor::No,
            });
        } else {
            // Dispatch normal link hover logic
            ctx.dispatch_typed_action(TerminalAction::MaybeLinkHover {
                position: grid_point.map(WithinModel::BlockList),
                from_editor: TerminalEditor::No,
            });
        }

        if let Some(point) = self.coord_to_point(
            SnackbarPoint::within_snackbar(position),
            ClampingMode::ClampToGrid,
        ) {
            let block_index = {
                let model = self.model.lock();
                let viewport = self.viewport_state_after_layout(model.block_list());
                match viewport.block_index_from_point(point) {
                    Some(block_index) => block_index,
                    None => return,
                }
            };

            ctx.dispatch_typed_action(TerminalAction::BlockHover(BlockHoverAction::Begin {
                position,
                block_index,
            }));
        } else {
            ctx.dispatch_typed_action(TerminalAction::BlockHover(BlockHoverAction::Clear));
        }
    }

    /// Determine if the mouse is directly hovering over the block list.
    ///
    /// If the mouse is over the block list but there is another element (e.g. a modal) obscuring
    /// the block list, then that will be treated as _not_ hovering.
    fn is_directly_hovering(&self, mouse_position: Vector2F, ctx: &mut EventContext) -> bool {
        let is_hovering = self.is_mouse_position_within_bounds(mouse_position);
        let is_covered = ctx.is_covered(Point::from_vec2f(
            mouse_position,
            self.child_max_z_index
                .expect("child z index should be set before dispatching"),
        ));

        is_hovering && !is_covered
    }

    fn mouse_moved(&self, position: Vector2F, app: &AppContext, ctx: &mut EventContext) -> bool {
        let is_hovering = self.is_directly_hovering(position, ctx);
        let was_hovering = self.mouse_states.hover_state.read_and_update(is_hovering);

        if is_hovering {
            // If the mouse is over the block list, we need to further process the MouseMove event
            // in order to properly react.
            self.mouse_over(position, app, ctx);

            // Allow the event to propagate to the parent in case it also wants to handle it.
            false
        } else {
            // If the mouse is not over the block list, then we should clear any outstanding hover
            // state (e.g. block hover or link hover). However, we only need to do this when the
            // mouse moves from hovering to not hovering, not on every mouse move.
            if was_hovering {
                ctx.dispatch_typed_action(TerminalAction::BlockHover(BlockHoverAction::Clear));
                ctx.dispatch_typed_action(TerminalAction::MaybeLinkHover {
                    position: None,
                    from_editor: TerminalEditor::No,
                });

                // Clear the snackbar hovered state
                if self.snackbar_header_state().hovered {
                    self.snackbar_header_state().hovered = false;
                    ctx.dispatch_typed_action(TerminalAction::BlockSnackbarHover {
                        is_hovered: false,
                    });
                }
                if self.snackbar_header_state.hover_near_snackbar_area {
                    ctx.dispatch_typed_action(TerminalAction::BlockNearSnackbarHover {
                        is_hovered: false,
                    });
                }
            }
            false
        }
    }

    fn mouse_dragged(
        &self,
        position: Vector2F,
        is_selecting_blocks: bool,
        modifiers: &ModifiersState,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.is_terminal_selecting && self.bounds.is_some() {
            let bounds = self.bounds.unwrap();

            let snackbar_height = self
                .snackbar_header_state()
                .header_rect()
                .map_or(0., |rect| rect.height());
            let bounds_top = bounds.origin_y();
            let snackbar_bottom = bounds_top + snackbar_height;
            let bottom = bounds.lower_left().y() - BOTTOM_VERTICAL_MARGIN;

            // Adjust the scroll delta if the mouse position is not within the current element.
            let delta_y = if position.y() < snackbar_bottom {
                // In order to make scrolling feel smooth when there is a block based
                // snackbar, we only calculate exponential scroll acceleration on the
                // portion of the scroll delta that is above the bounds of the element.
                // Otherwise, if you exponentially scroll on the snackbar portion of the
                // delta, you end up with "jumps" in scroll once one snackbar scrolls offscreen
                // and the next header shows.
                let (delta_outside_bounds, delta_from_snackbar_bottom) = (
                    (bounds_top - position.y()).max(0.),
                    (snackbar_bottom - position.y())
                        .min(snackbar_height)
                        .max(0.),
                );
                POLYNOMIAL_SCROLLING.accelerated_delta(delta_outside_bounds)
                    + LINEAR_SCROLLING.accelerated_delta(delta_from_snackbar_bottom)
            } else if position.y() > bottom {
                -POLYNOMIAL_SCROLLING.accelerated_delta(position.y() - bottom)
            } else {
                0.0
            };

            let side = self
                .size_info
                .get_mouse_side(position - vec2f(bounds.origin().x(), snackbar_bottom));
            if !is_selecting_blocks {
                if let Some(point) = self.coord_to_point(
                    SnackbarPoint::underneath_snackbar(position),
                    ClampingMode::ClampToGrid,
                ) {
                    ctx.dispatch_typed_action(TerminalAction::BlockTextSelect(
                        BlockTextSelectAction::Update {
                            point,
                            delta: delta_y.into_lines(),
                            side,
                            position,
                        },
                    ));
                }
            }

            if let Some(point) = self.coord_to_point(
                SnackbarPoint::within_snackbar(position),
                ClampingMode::ReturnNoneIfNotInGrid,
            ) {
                let model = self.model.lock();
                let viewport = self.viewport_state_after_layout(model.block_list());

                if let Some(within_block) = viewport.block_list_point_to_grid_point(point) {
                    let on_long_running_block = model
                        .block_list()
                        .block_at(within_block.block_index)
                        .is_some_and(|block| block.is_active_and_long_running());

                    if on_long_running_block
                        && !should_intercept_mouse(&model, modifiers.shift, app)
                    {
                        let grid_point = point_from_first_visible_row(&viewport, within_block);
                        let mouse_state = MouseState::new(
                            MouseButton::LeftDrag,
                            MouseAction::Pressed,
                            *modifiers,
                        );
                        drop(model);
                        ctx.dispatch_typed_action(TerminalAction::AltMouseAction(
                            mouse_state.set_point(grid_point),
                        ));
                    }
                }
            }

            true
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_selection(
        &self,
        range: &SelectionRange,
        origin: Vector2F,
        block_list: &BlockList,
        color: ColorU,
        selection_cursor_render_location: SelectionCursorRenderLocation,
        ctx: &mut PaintContext,
    ) {
        let total_block_heights = block_list.block_heights().summary().height;

        let viewport = self.viewport_state_after_layout(block_list);
        let (start, end) = viewport.selection_as_viewport_points(range);

        let cell_height = self.size_info.cell_height_px;
        let visible_rows = self.size().unwrap().y().into_pixels().to_lines(cell_height);
        // Offset vertically if the blocks do not take the entire screen, so we render the correct selections.
        // This offset is only necessary for the MostRecentAtBottom block ordering because
        // there is no gap at the top in the MostRecentAtTop ordering
        let selection_origin = match self.input_mode {
            InputMode::PinnedToBottom => {
                origin
                    + vec2f(
                        0.,
                        (visible_rows - total_block_heights)
                            .max(Lines::zero())
                            .to_pixels(cell_height)
                            .as_f32(),
                    )
            }
            InputMode::Waterfall | InputMode::PinnedToTop => origin,
        };

        let rendered_snackbar_selection = self.snackbar_header_state().render_selection(
            &start,
            &end,
            &self.size_info,
            viewport.scroll_top_in_lines(),
            origin,
            self.bounds
                .expect("Bounds should be set before rendering selection"),
            color,
            ctx,
        );

        grid_renderer::render_selection(
            &start,
            &end,
            &self.size_info,
            viewport.scroll_top_in_lines(),
            selection_origin,
            color,
            ctx,
        );
        match selection_cursor_render_location {
            SelectionCursorRenderLocation::Start => {
                let mut cursor_color = color;
                cursor_color.a = crate::util::color::OPAQUE;
                grid_renderer::render_selection_cursor(
                    &start,
                    &self.size_info,
                    viewport.scroll_top_in_lines(),
                    selection_origin,
                    cursor_color,
                    false,
                    ctx,
                );
            }
            SelectionCursorRenderLocation::End => {
                let mut cursor_color = color;
                cursor_color.a = crate::util::color::OPAQUE;
                grid_renderer::render_selection_cursor(
                    &end,
                    &self.size_info,
                    viewport.scroll_top_in_lines(),
                    selection_origin,
                    cursor_color,
                    true,
                    ctx,
                );
            }
            _ => (),
        }
        if rendered_snackbar_selection {
            // Rendering the snackbar creates a layer that we need to close.
            ctx.scene.stop_layer();
        }
    }

    fn render_shared_session_participants_selections(
        &self,
        origin: Vector2F,
        block_list: &BlockList,
        app: &AppContext,
        ctx: &mut PaintContext<'_>,
    ) {
        // Render other shared session participants'
        if let Some(presence_manager) = &self.presence_manager {
            let is_self_reconnecting = presence_manager.as_ref(app).is_reconnecting();
            for participant in presence_manager.as_ref(app).all_present_participants() {
                let Selection::BlockText {
                    start,
                    end,
                    is_reversed,
                } = &participant.info.selection
                else {
                    continue;
                };
                let start = WithinBlock::<IndexPoint>::from_session_sharing_block_point(
                    start.clone(),
                    block_list,
                );
                let end = WithinBlock::<IndexPoint>::from_session_sharing_block_point(
                    end.clone(),
                    block_list,
                );

                // TODO: if either of these are None, we should probably find the closest, relevant point.
                // For example, if there is displayed output in our grid and a remote selection is made
                // starting at an undisplayed row and ends somewhere in a displayed row, the selection should
                // probably be from the start of the displayed row to its end, rather than no selection at all.
                let Some((start, end)) = start.zip(end) else {
                    continue;
                };
                let start_block_index = start.block_index;
                let end_block_index = end.block_index;

                // Don't show highlight ui if this block is hidden.
                let mut any_hidden = false;
                for block_index in
                    BlockIndex::range_as_iter(start_block_index..end_block_index.next())
                {
                    if block_list
                        .block_at(block_index)
                        .map(|block| block.should_hide_block(block_list.agent_view_state()))
                        .unwrap_or(true)
                    {
                        any_hidden = true;
                        break;
                    }
                }
                if any_hidden {
                    continue;
                }

                let start_block_list_point =
                    BlockListPoint::from_within_block_point(&start, block_list);
                let end_block_list_point =
                    BlockListPoint::from_within_block_point(&end, block_list);
                let range = SelectionRange::new(start_block_list_point, end_block_list_point);
                let participant_color = if is_self_reconnecting {
                    MUTED_PARTICIPANT_COLOR
                } else {
                    participant.color
                };
                let viewport = self.viewport_state_after_layout(block_list);
                if viewport.is_range_in_order_in_viewport(&range) {
                    let selection_cursor_render_location = if *is_reversed {
                        SelectionCursorRenderLocation::Start
                    } else {
                        SelectionCursorRenderLocation::End
                    };
                    self.render_selection(
                        &range,
                        origin,
                        block_list,
                        text_selection_color(participant_color),
                        selection_cursor_render_location,
                        ctx,
                    );
                } else {
                    // The start is always before the end from the participant's perspective before it's sent to the server.
                    // If the start is after the end for us, it means our blocklist is inverted relative to the participant's.
                    // For example, this happens if the selection spans multiple blocks and they are using waterfall mode but we are not, or vice versa.
                    self.render_shared_session_participant_selection_relative_inverted_blocklist(
                        start_block_list_point,
                        start_block_index,
                        end_block_list_point,
                        end_block_index,
                        *is_reversed,
                        participant_color,
                        origin,
                        block_list,
                        ctx,
                    );
                }
            }
        }
    }

    /// Render a participant's selection when it spans multiple blocks and their blocklist is inverted relative to ours.
    /// Say the participant selected from S to E across 4 blocks on their screen:
    ///
    /// 1.  [ ][ ][S][X][X]
    ///     [X][X][X][X][X]
    /// 2.  [X][X][X][X][X]
    ///     [X][X][X][X][X]
    /// 3.  [X][X][X][X][X]
    ///     [X][X][X][X][X]
    /// 4.  [X][X][E][ ][ ]
    ///     [ ][ ][ ][ ][ ]
    ///
    /// But our blocklist is inverted relative to the participant's. On our screen, the selection should appear like:
    ///
    /// 4.  [X][X][E][ ][ ]
    ///     [ ][ ][ ][ ][ ]
    /// 3.  [X][X][X][X][X]
    ///     [X][X][X][X][X]
    /// 2.  [X][X][X][X][X]
    ///     [X][X][X][X][X]
    /// 1.  [ ][ ][S][X][X]
    ///     [X][X][X][X][X]
    ///
    /// Returns Some(()) if the selection was rendered, which will happen as long as the block indices are in bounds.
    #[allow(clippy::too_many_arguments)]
    fn render_shared_session_participant_selection_relative_inverted_blocklist(
        &self,
        start_block_list_point: BlockListPoint,
        start_block_index: BlockIndex,
        end_block_list_point: BlockListPoint,
        end_block_index: BlockIndex,
        is_reversed: bool,
        participant_color: ColorU,
        origin: Vector2F,
        block_list: &BlockList,
        ctx: &mut PaintContext<'_>,
    ) -> Option<()> {
        // Render a selection from the start point of the same block that the end point is in, to the end point.
        // 4.  [X][X][E][ ][ ]
        //     [ ][ ][ ][ ][ ]
        let block_start = block_list
            .block_at(end_block_index)
            .map(|b| b.start_point().to_within_block_point(end_block_index))?;
        let range = SelectionRange::new(
            BlockListPoint::from_within_block_point(&block_start, block_list),
            end_block_list_point,
        );
        let selection_cursor_render_location = if is_reversed {
            SelectionCursorRenderLocation::None
        } else {
            SelectionCursorRenderLocation::End
        };
        self.render_selection(
            &range,
            origin,
            block_list,
            text_selection_color(participant_color),
            selection_cursor_render_location,
            ctx,
        );

        // Any intermediate blocks between start and end points are fully selected.
        // 3.  [X][X][X][X][X]
        //     [X][X][X][X][X]
        // 2.  [X][X][X][X][X]
        //     [X][X][X][X][X]
        let (larger_block_index, smaller_block_index) = if start_block_index > end_block_index {
            (start_block_index, end_block_index)
        } else {
            (end_block_index, start_block_index)
        };
        if larger_block_index - smaller_block_index > 1.into() {
            // The intermediate start block index should be whichever is on top in the viewport.
            // The intermediate end block index is whichever is on bottom in the viewport.
            let (intermediate_start_block_index, intermediate_end_block_index) =
                if !self.input_mode.is_inverted_blocklist() {
                    (
                        smaller_block_index + 1.into(),
                        larger_block_index - 1.into(),
                    )
                } else {
                    (
                        larger_block_index - 1.into(),
                        smaller_block_index + 1.into(),
                    )
                };
            let intermediate_start =
                block_list
                    .block_at(intermediate_start_block_index)
                    .map(|b| {
                        b.start_point()
                            .to_within_block_point(intermediate_start_block_index)
                    })?;

            let intermediate_end = block_list.block_at(intermediate_end_block_index).map(|b| {
                b.end_point()
                    .to_within_block_point(intermediate_end_block_index)
            })?;

            let range = SelectionRange::new(
                BlockListPoint::from_within_block_point(&intermediate_start, block_list),
                BlockListPoint::from_within_block_point(&intermediate_end, block_list),
            );
            self.render_selection(
                &range,
                origin,
                block_list,
                text_selection_color(participant_color),
                SelectionCursorRenderLocation::None,
                ctx,
            );
        }

        // Render a selection from the start point to the end of the block that the start point is in.
        // 1.  [ ][ ][S][X][X]
        //     [X][X][X][X][X]
        let block_end = block_list
            .block_at(start_block_index)
            .map(|b| b.end_point().to_within_block_point(start_block_index))?;
        let range = SelectionRange::new(
            start_block_list_point,
            BlockListPoint::from_within_block_point(&block_end, block_list),
        );
        let selection_cursor_render_location = if is_reversed {
            SelectionCursorRenderLocation::Start
        } else {
            SelectionCursorRenderLocation::None
        };
        self.render_selection(
            &range,
            origin,
            block_list,
            text_selection_color(participant_color),
            selection_cursor_render_location,
            ctx,
        );
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_block_background(
        cell_size: Vector2F,
        grid_origin: Vector2F,
        block: &Block,
        is_selected_by_anyone: bool,
        bounds: RectF,
        warp_theme: &WarpTheme,
        block_borders_enabled: bool,
        snackbar_header: &Option<SnackbarHeader>,
        ai_render_context: &BlocklistAIRenderContext,
        agent_view_state: &AgentViewState,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        let block_height = block.height(agent_view_state).as_f64() as f32 * cell_size.y();
        if block.is_restored()
            && (!FeatureFlag::AgentView.is_enabled() || !agent_view_state.is_fullscreen())
        {
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    grid_origin,
                    Vector2F::new(bounds.width(), block_height),
                ))
                .with_background(warp_theme.restored_blocks_overlay());
        }

        // Update the background for the current active long running command when the inline agent view is active.
        if agent_view_state.is_inline() && block.is_active_and_long_running() {
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    grid_origin,
                    Vector2F::new(bounds.width(), block_height),
                ))
                .with_background(agent_view_bg_fill(app));
        }

        let mut did_render_ai_stripe = false;
        if !FeatureFlag::AgentView.is_enabled() {
            if let Some(ai_context_stripe_color) =
                ai_render_context.context_color_for_block(block, warp_theme)
            {
                draw_flag_pole(grid_origin, block_height, ai_context_stripe_color, ctx);
                did_render_ai_stripe = true;
            }
        }

        if block.has_failed() {
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    grid_origin,
                    Vector2F::new(bounds.width(), block_height),
                ))
                .with_background(warp_theme.failed_block_color().with_opacity(10));

            if !is_selected_by_anyone && !did_render_ai_stripe {
                draw_flag_pole(
                    grid_origin,
                    block_height,
                    warp_theme.failed_block_color(),
                    ctx,
                );
            }
        }

        if let Some((is_hovered, header_rect)) =
            snackbar_header.and_then(|h| h.header_rect().map(|rect| (h.hovered, rect)))
        {
            // Draw the hover effect if the header is being hovered
            if is_hovered {
                ctx.scene
                    .draw_rect_with_hit_recording(header_rect)
                    .with_background(
                        warp_theme
                            .accent_overlay()
                            .with_opacity(SNACKBAR_HOVER_OPACITY),
                    );
            }

            // Draw a bottom border if there is content scrolled underneath. We only need to do this block
            // border if the block borders are not enabled.
            if grid_origin.y() + block_height > header_rect.max_y() && !block_borders_enabled {
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(
                        vec2f(header_rect.min_x(), header_rect.max_y() - 1.0),
                        vec2f(header_rect.width(), 1.0),
                    ))
                    .with_background(warp_theme.outline());
            }
        }
    }

    fn draw_border_between_blocks(
        grid_origin: Vector2F,
        block_grid_params: &BlockGridParams,
        ctx: &mut PaintContext,
    ) {
        let rect = ctx.scene.draw_rect_with_hit_recording(RectF::new(
            grid_origin,
            vec2f(block_grid_params.bounds.width(), 1.),
        ));

        rect.with_background(block_grid_params.grid_render_params.warp_theme.outline());
    }

    // TODO(alokedesai): Clean this up even more by pulling out parameters into various structs.
    #[allow(clippy::too_many_arguments)]
    fn draw_block(
        block: &Block,
        grid_origin: &mut Vector2F,
        element_origin: Vector2F,
        find_render_data: Option<BlockFindRenderData>,
        highlighted_url: Option<&WithinBlock<Link>>,
        link_tool_tip: Option<&WithinBlock<Link>>,
        hovered_secret: Option<SecretHandle>,
        glyphs: &mut CellGlyphCache,
        label_element: Option<&mut Box<dyn Element>>,
        footer_element: Option<&mut Box<dyn Element>>,
        block_index: BlockIndex,
        block_borders_enabled: bool,
        is_current_block_selected_by_anyone: bool,
        block_grid_params: &BlockGridParams,
        snackbar_header: &Option<SnackbarHeader>,
        terminal_view_id: EntityId,
        draw_border_between_blocks: bool,
        ai_render_context: &BlocklistAIRenderContext,
        cursor_hint_text: Option<&mut Box<dyn Element>>,
        image_metadata: &HashMap<u32, StoredImageMetadata>,
        agent_view_state: &AgentViewState,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        Self::draw_block_background(
            block_grid_params.grid_render_params.cell_size,
            *grid_origin,
            block,
            is_current_block_selected_by_anyone,
            block_grid_params.bounds,
            &block_grid_params.grid_render_params.warp_theme,
            block_borders_enabled,
            snackbar_header,
            ai_render_context,
            agent_view_state,
            ctx,
            app,
        );

        let cell_size_height = block_grid_params.grid_render_params.cell_size.y();
        let block_banner_height = block.block_banner_height().as_f64() as f32 * cell_size_height;

        if draw_border_between_blocks && block_borders_enabled {
            // The border belongs *above* the block banner, if there is one. The grid_origin has
            // already been updated to point to below that banner, so we do the subtraction to go
            // up and draw the border above it.
            let border_origin = *grid_origin - vec2f(0., block_banner_height);
            Self::draw_border_between_blocks(border_origin, block_grid_params, ctx);
        }

        let cursor_visible = block.is_mode_set(TermMode::SHOW_CURSOR);
        let command_origin = if !block.should_hide_command_grid() {
            let prompt_height_offset = cell_size_height * block.padding_top().as_f64() as f32;

            *grid_origin += vec2f(0., prompt_height_offset);

            let prompt_origin = snackbar_header
                .and_then(|header| header.header_rect())
                .map_or(*grid_origin, |r| {
                    let y = r.origin().y() + prompt_height_offset + block_banner_height;
                    vec2f(grid_origin.x(), y)
                });

            // Draw prompt
            if let Some(label_element) = label_element {
                label_element.paint(prompt_origin, ctx, app);
            } else {
                let size_info = &block_grid_params.grid_render_params.size_info;
                if block.should_display_rprompt(size_info) {
                    let rprompt_origin = prompt_origin + block.rprompt_render_offset(size_info);
                    block.rprompt_grid().draw(
                        rprompt_origin,
                        element_origin,
                        glyphs,
                        COMMAND_ALPHA,
                        None,
                        None,
                        hovered_secret,
                        None::<std::iter::Empty<&RangeInclusive<IndexPoint>>>,
                        None,
                        Properties::default(),
                        block_grid_params,
                        None,
                        image_metadata,
                        ctx,
                        app,
                    );
                }
            }

            // If Warp prompt (non-PS1) is being used, the command is drawn below the prompt,
            // hence we account for the prompt's vertical offset.
            let prompt_vertical_offset_px = if !block.honor_ps1() {
                cell_size_height
                    * (block.command_padding_top() + block.prompt_height()).as_f64() as f32
            } else {
                // Otherwise, the prompt/command are drawn together, in a single grid. Hence, we haven't
                // drawn the prompt above and we do not account for the offset.
                0.0
            };

            *grid_origin += vec2f(0.0, prompt_vertical_offset_px);

            // Determine command_origin based on snackbar_header.
            let command_origin = if snackbar_header.is_some() {
                prompt_origin + vec2f(0.0, prompt_vertical_offset_px)
            } else {
                *grid_origin
            };

            // Update grid_origin and draw command.
            let command_grid_properties = Properties::default();
            let command_focused_range =
                find_render_data
                    .as_ref()
                    .and_then(|data: &BlockFindRenderData<'_>| {
                        data.focused_range_for_grid(GridType::PromptAndCommand)
                    });
            block.prompt_and_command_grid().draw(
                command_origin,
                element_origin,
                glyphs,
                COMMAND_ALPHA,
                highlighted_url
                    .filter(|url| url.is_in_command_content() && url.block_index == block_index)
                    .map(|url| &url.inner),
                link_tool_tip
                    .filter(|url| url.is_in_command_content() && url.block_index == block_index)
                    .map(|url| &url.inner),
                hovered_secret,
                find_render_data
                    .as_ref()
                    .and_then(|data: &BlockFindRenderData<'_>| data.command_grid_matches()),
                command_focused_range.as_ref(),
                command_grid_properties,
                block_grid_params,
                cursor_visible.then(|| block.prompt_and_command_grid().cursor_style().shape),
                image_metadata,
                ctx,
                app,
            );

            // Only render the cursor in the command grid if the command grid is active and if it's
            // long running. This is to avoid jitter where a cursor just flickers while the pty is
            // initializing.
            if block.is_active_and_long_running()
                && block.is_command_grid_active()
                // Check if the "hide cursor" escape sequence is present.
                && block.is_mode_set(TermMode::SHOW_CURSOR)
            {
                block.prompt_and_command_grid().draw_cursor(
                    command_origin,
                    &block_grid_params.grid_render_params,
                    ctx,
                    terminal_view_id,
                    None,
                    block_grid_params
                        .grid_render_params
                        .warp_theme
                        .cursor()
                        .into(),
                    app,
                );
            }

            // Update grid_origin & draw output
            *grid_origin += vec2f(
                0.,
                cell_size_height
                    * (block.padding_middle() + block.prompt_and_command_grid().len().into_lines())
                        .as_f64() as f32,
            );

            command_origin
        } else {
            *grid_origin
        };

        let block_middle_lines =
            block.padding_middle() + block.prompt_and_command_number_of_rows().into_lines();
        if let Some(header_rect) = snackbar_header.map(|h| h.header_rect()).flatten() {
            // In the case of a block based header, we need to clip beneath
            // the header in order to make scrolling the block output work.
            // Note that we always need the dividing border line to be drawn below the toolbelt icons.
            let bottom_border_origin = header_rect.lower_left();

            #[cfg(feature = "integration_tests")]
            {
                ctx.position_cache.cache_position_for_one_frame(
                    format!("block_list_snackbar:{terminal_view_id}"),
                    header_rect,
                );
            }

            if block_borders_enabled {
                Self::draw_border_between_blocks(bottom_border_origin, block_grid_params, ctx);
            }
            ctx.scene
                .start_layer(ClipBounds::BoundedByActiveLayerAnd(RectF::new(
                    header_rect.lower_left(),
                    vec2f(
                        block_grid_params.bounds.width(),
                        block_grid_params.bounds.height() - header_rect.height(),
                    ),
                )));
        }

        if !block.should_hide_output_grid() {
            let viewport_origin = snackbar_header.map_or(element_origin, |_| {
                command_origin + vec2f(0., cell_size_height * block_middle_lines.as_f64() as f32)
            });

            let output_grid_properties =
                Properties::default().weight(block_grid_params.grid_render_params.font_weight);
            let output_focused_range =
                find_render_data
                    .as_ref()
                    .and_then(|data: &BlockFindRenderData<'_>| {
                        data.focused_range_for_grid(GridType::Output)
                    });
            block.output_grid().draw(
                *grid_origin,
                viewport_origin,
                glyphs,
                255,
                highlighted_url
                    .filter(|url| url.is_output_grid() && url.block_index == block_index)
                    .map(|url| &url.inner),
                link_tool_tip
                    .filter(|url| !url.is_in_command_content() && url.block_index == block_index)
                    .map(|url| &url.inner),
                hovered_secret,
                // Render find matches in output grid.
                find_render_data
                    .as_ref()
                    .and_then(|data: &BlockFindRenderData<'_>| data.output_grid_matches()),
                output_focused_range.as_ref(),
                output_grid_properties,
                block_grid_params,
                cursor_visible.then(|| block.output_grid().cursor_style().shape),
                image_metadata,
                ctx,
                app,
            );

            if block.is_active_and_long_running()
            // Check if the "hide cursor" escape sequence is present.
            && block.is_mode_set(TermMode::SHOW_CURSOR)
            // Don't draw the Warp cursor when rich input is hiding
            // the CLI agent's cursor cell — agents like OpenCode and Codex
            // rely on Warp's cursor, so we suppress it here too.
            && !block_grid_params.grid_render_params.hide_cursor_cell
            {
                block.output_grid().draw_cursor(
                    *grid_origin,
                    &block_grid_params.grid_render_params,
                    ctx,
                    terminal_view_id,
                    cursor_hint_text,
                    if block.is_agent_blocked() {
                        AnsiColorIdentifier::Yellow
                            .to_ansi_color(
                                &block_grid_params
                                    .grid_render_params
                                    .warp_theme
                                    .terminal_colors()
                                    .normal,
                            )
                            .into()
                    } else if block.is_agent_in_control() {
                        ai_brand_color(&block_grid_params.grid_render_params.warp_theme)
                    } else {
                        block_grid_params
                            .grid_render_params
                            .warp_theme
                            .cursor()
                            .into()
                    },
                    app,
                );
            }

            // Offset the origin by the height of the output grid.
            *grid_origin += vec2f(
                0.,
                cell_size_height * (block.output_grid().len_displayed() as f32),
            );

            if let Some(footer_element) = footer_element {
                let remaining_padding_lines = block.footer_top_padding() + block.padding_bottom();
                let remaining_padding_px =
                    remaining_padding_lines.as_f64() as f32 * cell_size_height;

                let label_origin = *grid_origin
                    + vec2f(
                        block_grid_params
                            .grid_render_params
                            .size_info
                            .padding_x_px()
                            .as_f32(),
                        remaining_padding_px / 2.,
                    );
                footer_element.paint(label_origin, ctx, app);

                // Offset the origin by the footer padding + height.
                let footer_offset_px = cell_size_height
                    * (block.footer_top_padding() + block.footer_height()).as_f64() as f32;
                *grid_origin += vec2f(0., footer_offset_px);
            }

            // Add padding to the grid origin before returning
            *grid_origin += vec2f(
                0.,
                cell_size_height * block.padding_bottom().as_f64() as f32,
            );
        }

        if snackbar_header.map(|h| h.header_rect()).flatten().is_some() {
            // End the clipping of the area under the snackbar header rect
            ctx.scene.stop_layer();
        }
    }

    /// Returns the location where the snackbar toggle button should be drawn, or None if it should
    /// not be drawn.
    fn compute_snackbar_toggle_button_draw_location(
        &self,
        block_grid_params: &BlockGridParams,
    ) -> Option<Vector2F> {
        let button_size = self
            .snackbar_toggle_button
            .as_ref()
            .and_then(|button| button.size())?;

        if self.snackbar_header_state.show_snackbar {
            self.compute_snackbar_collapse_button_draw_location(button_size)
        } else {
            self.compute_snackbar_expand_button_draw_location(button_size, block_grid_params)
        }
    }

    fn compute_snackbar_collapse_button_draw_location(
        &self,
        button_size: Vector2F,
    ) -> Option<Vector2F> {
        // Show the collapse button if it's rendered and hovered.
        let header = self.snackbar_header_state();
        if !header.hovered {
            return None;
        }

        header.header_position.map(|header_position| {
            Vector2F::new(
                header_position.rect.center().x() - button_size.x() / 2.,
                header_position.rect.lower_left().y() - button_size.y(),
            )
        })
    }

    fn compute_snackbar_expand_button_draw_location(
        &self,
        button_size: Vector2F,
        block_grid_params: &BlockGridParams,
    ) -> Option<Vector2F> {
        // Show the expand button if the snackbar is toggled off (but would otherwise be shown) and
        // the cursor is near the top of the screen.
        let header = self.snackbar_header_state();
        if !(header.hidden_by_toggle && self.snackbar_header_state.hover_near_snackbar_area) {
            return None;
        }

        Some(Vector2F::new(
            block_grid_params.bounds.center().x() - button_size.x() / 2.,
            block_grid_params.bounds.upper_right().y(),
        ))
    }

    /// Translate the given (mouse) position into the terminal view's coordinate space.
    /// Because the bounds are affected by the horizontal scroll position, we need to account for horizontal scroll.
    fn position_in_terminal_view(&self, position: Vector2F) -> Vector2F {
        let bounds_offset = self
            .bounds
            .expect("Bounds should be defined before mouse clicks")
            .origin();
        let horizontal_scroll_offset = self.horizontal_clipped_scroll_state.scroll_start().as_f32();
        position - bounds_offset - vec2f(horizontal_scroll_offset, 0.)
    }

    /// Return whether the mouse position is in the visible bounds of the block list element.
    /// The block list element's self.bounds go beyond what is actually visible in the pane if there is a horizontal scroll bar causing clipping -
    /// use this function to check whether the position is actually within the visible bounds.
    fn is_mouse_position_within_bounds(&self, position: Vector2F) -> bool {
        let bounds = self
            .bounds
            .expect("Bounds should be set before event dispatching");
        // The block list element's bounds go beyond what is actually visible in the pane if there is a horizontal scroll bar.
        // Check if the mouse position is within the actually visible bounds.
        let visible_bounds_min_x =
            bounds.min_x() + self.horizontal_clipped_scroll_state.scroll_start().as_f32();
        let visible_bounds_max_x = visible_bounds_min_x + self.size_info.pane_width_px().as_f32();
        if position.x() < visible_bounds_min_x || position.x() > visible_bounds_max_x {
            return false;
        }
        bounds.contains_point(position)
    }

    /// Return whether the mouse position is within the visible bounds of a text selection.
    /// Note that this function does not handle text selections within rich content views.
    fn is_mouse_position_within_selection(&self, position: Vector2F) -> bool {
        let Some(range) = &self.selection_ranges else {
            return false;
        };

        let model = self.model.lock();
        let block_list = model.block_list();
        let viewport = self.viewport_state_after_layout(block_list);

        // To avoid highlighting over rich blocks, we split the original selection range into multiple
        // sub-ranges, none of which include a rich block.
        let mut selection_ranges = range
            .iter()
            .flat_map(|selection| self.segment_blocklist_selection(selection, block_list));

        // The is_within_snackbar_bounds check is necessary to ensure that the right-click is registered at the correct layer.
        // For example, clicking on the part of a selection that's clipped off shouldn't dispatch a TextRightClick action.
        let is_within_snackbar_bounds = self.snackbar_header_state().contains_point(position);

        let position_in_terminal_view = self.position_in_terminal_view(position);

        selection_ranges.any(|range| {
            let (start, end) = viewport.selection_as_viewport_points(&range);

            let adjusted_scroll_top =
                if is_within_snackbar_bounds {
                    self.snackbar_header_state()
                    .calculate_scroll_top_for_selection(
                        &self.size_info,
                        viewport.scroll_top_in_lines(),
                        self.origin.expect("Origin should be defined before mouse clicks").xy(),
                        self.bounds
                            .expect("Bounds should be set before selection render"),
                    )
                    .expect(
                        "Snackbar header should be exist if the mouse is within snackbar bounds",
                    )
                } else {
                    viewport.scroll_top_in_lines()
                };

            let selection_bounds = grid_renderer::calculate_selection_bounds(
                &start,
                &end,
                &self.size_info,
                adjusted_scroll_top,
                Vector2F::zero(),
            );

            selection_bounds
                .iter()
                .any(|bounds| bounds.contains_point(position_in_terminal_view))
        })
    }

    /// Splits a SelectionRange into a list of non-overlapping sub-ranges, none of which span rich blocks.
    fn segment_blocklist_selection(
        &self,
        range: &SelectionRange,
        block_list: &BlockList,
    ) -> Vec<SelectionRange> {
        let blocklist_inverted = self.input_mode.is_inverted_blocklist();

        // Initialize block cursor. current_block_cursor points to the block that contains line range.start.row.0
        let mut current_block_cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        current_block_cursor.seek(
            &BlockHeight::from(range.start.row),
            sum_tree::SeekBias::Right,
        );

        // Initialize the first selection range. Because of the clamping logic executed in clamp_to_grid_points,
        // a completed blocklist selection's start and end points are always clamped to the nearest command block(s) it spans.
        // As a result, the start is guaranteed to be in a command block.
        let mut selections: Vec<SelectionRange> = vec![];
        let mut current_selection_start = Some(range.start);
        let mut current_selection_end = None;
        let max_blocklist_column = block_list.size().columns().saturating_sub(1);

        // This loop finishes the block BEFORE the last block
        while if blocklist_inverted {
            range.end.row <= current_block_cursor.end().height
        } else {
            range.end.row >= current_block_cursor.end().height
        } {
            let Some(current_item) = current_block_cursor.item() else {
                break;
            };

            match current_item {
                BlockHeightItem::Block { .. } => {
                    let current_block_index = current_block_cursor.start().block_count.into();
                    if let Some(current_block) = block_list.block_at(current_block_index) {
                        // If current_selection_start is none, it means this is the first command block after a sequence
                        // of rich blocks. In this case, we set the start of a new selection range to be rendered.
                        if current_selection_start.is_none() {
                            let current_block_top_offset = current_block.padding_top();
                            let current_selection_start_row =
                                current_block_cursor.start().height + current_block_top_offset;
                            current_selection_start =
                                Some(BlockListPoint::new(current_selection_start_row, 0));
                        }

                        // Here, we're extending the end of the current selection to the end of the current command
                        // block. If a rich block follows this block, the current selection correctly ends at the end
                        // of the last command block. If a command block that's not the final block follows this block,
                        // we continue extending the selection. If a command block that is the final block follows this block,
                        // the loop exits and we end the current selection at the original selection range's end.
                        let current_block_bottom_offset =
                            current_block.padding_bottom() + 1.into_lines();
                        let current_selection_end_row =
                            current_block_cursor.end().height - current_block_bottom_offset;
                        current_selection_end = Some(BlockListPoint::new(
                            current_selection_end_row,
                            max_blocklist_column,
                        ));
                    }
                }
                BlockHeightItem::RichContent { .. } => {
                    // If we reach a rich block and there's an ongoing selection, we've already encountered a
                    // command block and should add its selection to the list of selections to render. Then,
                    // we should reset the current selection range in preparation for the next command block.
                    if let (Some(current_start), Some(current_end)) =
                        (current_selection_start, current_selection_end)
                    {
                        selections.push(SelectionRange {
                            start: current_start,
                            end: current_end,
                        });

                        current_selection_start = None;
                        current_selection_end = None;
                    }
                }
                _ => {}
            }

            if blocklist_inverted {
                current_block_cursor.prev();
            } else {
                current_block_cursor.next();
            }
        }

        // Add the final range, which includes the original range's end. If there were no rich blocks in the range,
        // this ends up being the original selection range.
        if let Some(current_start) = current_selection_start {
            selections.push(SelectionRange {
                start: current_start,
                end: range.end,
            });
        } else {
            let current_block_index = current_block_cursor.start().block_count.into();
            if let Some(current_block) = block_list.block_at(current_block_index) {
                let current_block_top_offset = current_block.padding_top();

                let current_selection_start_row =
                    current_block_cursor.start().height + current_block_top_offset;
                selections.push(SelectionRange {
                    start: BlockListPoint::new(current_selection_start_row, 0),
                    end: range.end,
                });
            }
        }

        selections
    }

    /// Returns all rich text view ids that are currently visible in the viewport.
    ///
    /// This function examines the currently visible items in the blocklist,
    /// filters for RichContent items only, and returns the corresponding
    /// view id for those that are visible.
    fn visible_rich_content_views(&self) -> Vec<EntityId> {
        let mut result = Vec::new();

        // If there are no visible items, return an empty vector
        let Some(visible_items) = &self.visible_items else {
            return result;
        };

        // Filter visible items for RichContent items and collect their view_ids
        for item in visible_items.iter() {
            if let VisibleItem::RichContent { view_id, .. } = item {
                result.push(*view_id);
            }
        }

        result
    }

    #[cfg(feature = "voice_input")]
    fn maybe_handle_voice_toggle(
        &self,
        key_code: &KeyCode,
        state: &KeyState,
        ctx: &mut EventContext,
    ) -> bool {
        use crate::terminal::view::TerminalAction;

        if let Some(voice_input_toggle_key_code) = self.voice_input_toggle_key_code {
            if *key_code == voice_input_toggle_key_code {
                ctx.dispatch_typed_action(TerminalAction::ToggleCLIAgentVoiceInput(
                    voice_input::VoiceInputToggledFrom::Key { state: *state },
                ));
                return true;
            }
        }
        false
    }

    #[cfg(not(feature = "voice_input"))]
    fn maybe_handle_voice_toggle(
        &self,
        _key_code: &KeyCode,
        _state: &KeyState,
        _ctx: &mut EventContext,
    ) -> bool {
        false
    }
}

/// With a `WithinBlock<IndexPoint>`, the point will count rows with 0 starting with the beginning
/// of the block grid. This function adjusts the row so that 0 starts at the first row visible in
/// the viewport.
fn point_from_first_visible_row(
    viewport: &ViewportState<'_>,
    within_block: WithinBlock<IndexPoint>,
) -> IndexPoint {
    // Get the first visible output row to adjust for scrolled blocks
    let first_visible_row = if within_block.grid == GridType::Output {
        viewport
            .get_first_visible_output_row(within_block.block_index)
            .unwrap_or(0)
    } else {
        0
    };
    // Adjust row to be relative to the visible viewport, not the entire block grid
    let visible_row = within_block.inner.row.saturating_sub(first_visible_row);
    IndexPoint::new(visible_row, within_block.inner.col)
}

impl Element for BlockListElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        if let Some(overflow_menu_button) = &mut self.overflow_menu_button {
            overflow_menu_button.layout(
                SizeConstraint::strict(vec2f(BLOCK_HOVER_BUTTON_HEIGHT, BLOCK_HOVER_BUTTON_HEIGHT)),
                ctx,
                app,
            );
        }
        if let Some(snackbar_toggle_button) = &mut self.snackbar_toggle_button {
            snackbar_toggle_button.layout(
                SizeConstraint::strict(vec2f(
                    SNACKBAR_TOGGLE_BUTTON_WIDTH,
                    SNACKBAR_TOGGLE_BUTTON_HEIGHT,
                )),
                ctx,
                app,
            );
        }
        if let Some(ask_ai_assistant_button) = &mut self.ask_ai_assistant_button {
            ask_ai_assistant_button.layout(
                SizeConstraint::new(
                    vec2f(BLOCK_HOVER_BUTTON_HEIGHT, BLOCK_HOVER_BUTTON_HEIGHT),
                    vec2f(150., 150.),
                ),
                ctx,
                app,
            );
        }
        if let Some(save_as_workflow_button) = &mut self.save_as_workflow_button {
            save_as_workflow_button.layout(
                // The size constraint needs to be big enough to cover the total rect when tooltip is rendered.
                SizeConstraint::new(
                    vec2f(BLOCK_HOVER_BUTTON_HEIGHT, BLOCK_HOVER_BUTTON_HEIGHT),
                    vec2f(240., 64.),
                ),
                ctx,
                app,
            );
        }
        if let Some(cursor_hint_text) = &mut self.cursor_hint_text_element {
            cursor_hint_text.layout(constraint, ctx, app);
        }

        let mut model = self.model.lock();
        let cell_size = Vector2F::new(
            self.size_info.cell_width_px().as_f32(),
            self.size_info.cell_height_px().as_f32(),
        );
        let mut visible_items = vec![];
        let mut block_indices_with_label_elements = vec![];
        let mut visible_block_indices = vec![];
        let mut updated_rich_content_heights = HashMap::new();

        // First, remeasure any rich content items whose heights may be out of date.
        // We track these in the BlockList via a dirty set keyed by view ID to
        // avoid laying out every rich content block on every frame.
        let dirty_rich_content_items = model.block_list_mut().take_dirty_rich_content_items();
        if !dirty_rich_content_items.is_empty() {
            for view_id in &dirty_rich_content_items {
                if let Some(rich_content) = self.rich_content_elements.get_mut(view_id) {
                    // Lay out the rich content with an infinite vertical constraint,
                    // allowing it to take up as much size as it needs.
                    let current_size = rich_content.layout(
                        SizeConstraint::tight_on_cross_axis(Axis::Vertical, constraint),
                        ctx,
                        app,
                    );
                    let height_px = current_size.y();

                    updated_rich_content_heights.insert(*view_id, height_px as f64);
                }
            }
        }
        model
            .block_list_mut()
            .update_rich_content_heights(&updated_rich_content_heights);

        // Use a macro for creating a viewport, to ensure that callers use consistent parameters
        macro_rules! create_viewport {
            ($block_list:expr) => {
                ViewportState::new(
                    $block_list,
                    self.snackbar_header_state.clone(),
                    self.input_mode,
                    self.size_info,
                    self.scroll_position,
                    self.visible_items.clone(),
                    self.horizontal_clipped_scroll_state.clone(),
                    constraint.max,
                    self.input_size_at_last_frame,
                    if self.ai_render_context.borrow().has_active_conversation() {
                        AutoscrollBehavior::WhenScrolledToEnd
                    } else {
                        AutoscrollBehavior::Always
                    },
                    self.inline_menu_positioner.clone(),
                )
            };
        }

        // Manually construct the viewport with the max size constraint because
        // size isn't set on the element until layout is done, but we need the
        // viewport iterator for layout.
        let viewport = create_viewport!(model.block_list());

        // Collect all the necessary subshell flags here. Usually there will only be one in the
        // viewport, but it's possible the user might start a subshell, exit, and start another
        // one within the same viewport. They might also start a nested subshell.
        let mut subshell_flags = HashMap::new();

        // Keep track of whether the previous block in this loop was part of a subshell, and if so
        // what was the session_id. We need this to determine if the current block needs to have a
        // subshell flag on it.
        let mut prev_block_subshell_session_id: Option<SessionId> = None;

        if let Some(banner) = &mut self.block_banner {
            banner.layout(constraint, ctx, app);
        }

        // Do a first pass to calculate the updated heights of all RichContent blocks by laying
        // them out and then measuring their size.
        //
        // A separate first pass is necessary so that the viewport iterator uses an updated list
        // of visible blocks for the final layout, which includes blocks that may not have been
        // visible before a RichContent block's height was updated.
        //
        // To ensure we don't miss any blocks that change heights just offscreen, we pad the size
        // of the viewport iterator by 20 lines.
        //
        // NOTE: This approach assumes that we don't have other RichContent blocks also collapsing
        // offscreen. If we allow that behavior, this first pass won't account for the updated
        // heights of those blocks not currently visible.
        let mut viewport_iter = viewport.iter_with_bottom_overhang(Lines::new(20.));
        for viewport_item in viewport_iter.by_ref() {
            if let BlockHeightItem::RichContent(RichContentItem {
                view_id,
                last_laid_out_height: height,
                ..
            }) = viewport_item.block_height_item
            {
                // Skip any items which were already updated because they were explicitly
                // dirtied.
                if updated_rich_content_heights.contains_key(&view_id) {
                    continue;
                }

                let Some(rich_content) = self.rich_content_elements.get_mut(&view_id) else {
                    log::warn!("Missing rich content element for ID: {view_id:?}");
                    continue;
                };

                let prev_height_px = height.as_f64() * cell_size.y() as f64;
                // Lay out the rich content with an infinite vertical constraint, allowing it
                // to take up as much size as it needs
                let current_size = rich_content.layout(
                    SizeConstraint::tight_on_cross_axis(Axis::Vertical, constraint),
                    ctx,
                    app,
                );
                let height_px = current_size.y() as f64;

                // If the new laid out height is different from the value currently in the
                // BlockList, then we flag the block for updating once we're done laying out.
                // To avoid unnecessary churn with rounding errors, we require any changes be
                // at least 1 pixel
                if f64::abs(height_px - prev_height_px) > 1. {
                    updated_rich_content_heights.insert(view_id, height_px);
                }
            }
        }

        // Drop the iterator and viewport so we can safely update the model with new RichContent heights.
        drop(viewport_iter);
        drop(viewport);

        model
            .block_list_mut()
            .update_rich_content_heights(&updated_rich_content_heights);

        // Rebuild the viewport to account for updated block heights, then compute visible items.
        let viewport = create_viewport!(model.block_list());

        let mut viewport_iter = viewport.iter();
        let mut visible_height_px: f64 = 0.;
        for viewport_item in viewport_iter.by_ref() {
            match viewport_item.block_height_item {
                BlockHeightItem::Block(height) => {
                    if height.as_f64() > 0. {
                        let block_index = viewport_item.block_index.expect("block index defined");
                        let mut subshell_session_id = None;

                        if let Some(block) = model.block_list().block_at(block_index) {
                            if !(block.honor_ps1() || block.is_background() || block.is_static()) {
                                block_indices_with_label_elements.push(block_index);
                            }

                            // Check if the current block belongs to a subshell session. We'll add
                            // a stripe during paint if it does.
                            if let Some(session_id) = block.session_id() {
                                if let Some(command) = self.subshell_sessions.get(&session_id) {
                                    subshell_session_id = Some(session_id);

                                    // Check if this block is the first in this viewport to belong
                                    // to this subshell, and lay out a flag Element for it. Don't
                                    // do this in compact mode though (in which case
                                    // subshell_separator_height will be > 0), or
                                    // if this is a background block.
                                    if (prev_block_subshell_session_id.is_none()
                                        || prev_block_subshell_session_id != Some(session_id))
                                        && self.subshell_separator_height == 0.
                                        && !block.is_background()
                                    {
                                        let command = if let SubshellSource::Command(cmd) = command
                                        {
                                            cmd.split_whitespace()
                                                .next()
                                                .map(|exec| {
                                                    SubshellSource::Command(exec.to_owned())
                                                })
                                                .unwrap_or_else(|| command.clone())
                                        } else {
                                            command.clone()
                                        };

                                        let mut flag_element = render_subshell_flag(
                                            command,
                                            self.font_family,
                                            self.font_size,
                                            &self.warp_theme,
                                        );
                                        flag_element.layout(constraint, ctx, app);
                                        subshell_flags.insert(block_index, flag_element);
                                    }
                                    prev_block_subshell_session_id = Some(session_id);
                                } else {
                                    prev_block_subshell_session_id = None;
                                }
                            }

                            if let Some(cli_subagent_view) =
                                self.cli_subagent_views.get_mut(block.id())
                            {
                                let block_height = (height.as_f64() as f32) * cell_size.y();
                                cli_subagent_view.layout(
                                    SizeConstraint {
                                        min: vec2f(0., 0.),
                                        max: vec2f(
                                            constraint.max.x() * 0.4
                                                - CLI_SUBAGENT_HORIZONTAL_MARGIN,
                                            block_height - CLI_SUBAGENT_VERTICAL_MARGIN * 2.,
                                        ),
                                    },
                                    ctx,
                                    app,
                                );
                            }
                        }

                        visible_items.push(VisibleItem::Block {
                            block_index,
                            index: viewport_item.entry_index,
                            subshell_session_id,
                        });
                        visible_block_indices.push(block_index);
                        visible_height_px += height.as_f64() * cell_size.y() as f64;
                    }
                }
                BlockHeightItem::Gap(height) => {
                    let height_px = height.as_f64() * cell_size.y() as f64;
                    visible_items.push(VisibleItem::Gap {
                        height_px: height_px as f32,
                        index: viewport_item.entry_index,
                    });
                    visible_height_px += height_px;
                }
                BlockHeightItem::RestoredBlockSeparator {
                    is_historical_conversation_restoration,
                    ..
                } => {
                    let item_height = viewport_item.block_height_item.height();
                    let height_px = item_height.as_f64() * cell_size.y() as f64;
                    visible_items.push(VisibleItem::RestoredBlockSeparator {
                        index: viewport_item.entry_index,
                        height_px: height_px as f32,
                    });
                    visible_height_px += height_px;

                    // we want to show different text in the separator if this is an individual conversation
                    // restored from the command palette
                    let banner_intro_text = if is_historical_conversation_restoration {
                        "Conversation restored".to_string()
                    } else {
                        "Previous session".to_string()
                    };

                    let separator_text =
                        if let Some(ts) = (*model).block_list().restored_session_ts() {
                            format!(
                                "{banner_intro_text} from {}",
                                ts.format("%a %b %-d at %-I:%M %p")
                            )
                        } else {
                            banner_intro_text
                        };
                    self.restored_session_separator = Some(
                        Text::new_inline(
                            separator_text,
                            self.ui_font_family,
                            self.font_size * SEPARATOR_TO_MONOSPACE_FONT_SIZE_RATIO,
                        )
                        .with_style(Properties::default().weight(self.font_weight))
                        .with_color(
                            self.warp_theme
                                .main_text_color(self.warp_theme.background())
                                .into_solid(),
                        )
                        .finish(),
                    );
                }
                BlockHeightItem::InlineBanner { banner, .. } => {
                    let banner_id = banner.id;
                    if let Some(banner_element) = self.inline_banners.get_mut(&banner_id) {
                        // Use the item's height() method which accounts for is_hidden
                        let item_height = viewport_item.block_height_item.height();
                        let height_px = item_height.as_f64() * cell_size.y() as f64;
                        banner_element.layout(
                            SizeConstraint::strict(vec2f(constraint.max.x(), height_px as f32)),
                            ctx,
                            app,
                        );
                        visible_items.push(VisibleItem::Banner {
                            index: viewport_item.entry_index,
                            height_px: height_px as f32,
                            banner_id,
                        });
                        visible_height_px += height_px;
                    } else {
                        log::warn!("Missing banner element for ID: {banner_id}");
                    }
                }
                BlockHeightItem::SubshellSeparator { separator_id, .. } => {
                    let Some(separator) = self.subshell_separators.get_mut(&separator_id) else {
                        log::warn!("Missing separator element for ID: {separator_id}");
                        continue;
                    };
                    // Use the item's height() method which accounts for is_hidden
                    let item_height = viewport_item.block_height_item.height();
                    let height_px = item_height.as_f64() * cell_size.y() as f64;
                    separator.layout(
                        SizeConstraint::strict(vec2f(constraint.max.x(), height_px as f32)),
                        ctx,
                        app,
                    );
                    visible_height_px += height_px;
                    visible_items.push(VisibleItem::SubshellSeparator {
                        index: viewport_item.entry_index,
                        height_px: height_px as f32,
                        separator_id,
                    });
                }
                BlockHeightItem::RichContent(RichContentItem {
                    view_id,
                    last_laid_out_height: height,
                    ..
                }) => {
                    // Use updated height if present. Otherwise, lay out the RichContent block and
                    // use its existing blocklist height.
                    //
                    // TODO(vorporeal): figure out if we should be using the height returned by layout
                    // here or if we should be using the height from the BlockList.
                    let height_px = if let Some(h) = updated_rich_content_heights.get(&view_id) {
                        *h
                    } else {
                        if let Some(rich_content) = self.rich_content_elements.get_mut(&view_id) {
                            let _ = rich_content.layout(
                                SizeConstraint::tight_on_cross_axis(Axis::Vertical, constraint),
                                ctx,
                                app,
                            );
                        }
                        height.as_f64() * cell_size.y() as f64
                    };

                    visible_height_px += height_px;
                    visible_items.push(VisibleItem::RichContent {
                        view_id,
                        height_px: height_px as f32,
                        index: viewport_item.entry_index,
                    });
                }
            };
        }
        if let Some(restored_session_separator) = &mut self.restored_session_separator {
            restored_session_separator.layout(
                SizeConstraint::strict(vec2f(
                    self.size_info.pane_width_px().as_f32(),
                    BLOCK_HOVER_BUTTON_HEIGHT,
                )),
                ctx,
                app,
            );
        }

        for avatar_element in self.presence_avatars.values_mut() {
            avatar_element.layout(
                SizeConstraint::new(
                    vec2f(constraint.min.x(), BLOCK_HOVER_BUTTON_HEIGHT),
                    vec2f(constraint.max.x(), BLOCK_HOVER_BUTTON_HEIGHT),
                ),
                ctx,
                app,
            );
        }

        self.visible_blocks = Some(viewport_iter.visible_block_range());
        self.visible_items = Some(Rc::new(visible_items));
        self.subshell_flags = subshell_flags;
        self.label_elements.clear();
        let elements = (self.label_elements_builder)(
            block_indices_with_label_elements.clone(),
            &self.mouse_states.label_mouse_states,
            &model,
            app,
        );

        // Explicitly drop `viewport_iter` so that we're not longer holding on to any immutable
        // references to the terminal model
        drop(viewport_iter);

        if DebugSettings::as_ref(app).should_show_memory_stats() {
            for block_index in &visible_block_indices {
                if let Some(block) = model.block_list().block_at(*block_index) {
                    if !block.has_footer() {
                        continue;
                    }

                    fn adjusted_bytes(bytes: usize) -> byte_unit::AdjustedByte {
                        let unit = if bytes >= 1_000_000 {
                            byte_unit::Unit::MB
                        } else {
                            byte_unit::Unit::KB
                        };
                        byte_unit::Byte::from(bytes).get_adjusted_unit(unit)
                    }

                    let grid_storage_lines = block.grid_storage_lines();
                    let grid_storage_bytes = block.grid_storage_bytes();
                    let flat_storage_lines = block.flat_storage_lines();
                    let flat_storage_bytes = block.flat_storage_bytes();

                    let total_lines = grid_storage_lines + flat_storage_lines;
                    let total_bytes = grid_storage_bytes + flat_storage_bytes;
                    let text = format!("\
                            Lines: {total_lines} (grid: {grid_storage_lines}, flat: {flat_storage_lines}); \
                            Size: {:#.1} (grid: {:#.1}, flat: {:#.1})\
                        ",
                        adjusted_bytes(total_bytes),
                        adjusted_bytes(grid_storage_bytes),
                        adjusted_bytes(flat_storage_bytes),
                    );

                    let mut element = Text::new_inline(text, self.ui_font_family, self.font_size)
                        .with_style(Properties::default().weight(self.font_weight))
                        .with_color(
                            self.warp_theme
                                .sub_text_color(self.warp_theme.background())
                                .into(),
                        )
                        .finish();

                    element.layout(constraint, ctx, app);
                    self.block_footer_elements.insert(*block_index, element);
                }
            }
        }

        // Explicitly drop the terminal model mutex guard so that it can be freed up for other
        // threads
        drop(model);

        for (idx, element) in block_indices_with_label_elements.iter().zip(elements) {
            self.label_elements.insert(*idx, element);
        }
        for label in self.label_elements.values_mut() {
            label.layout(constraint, ctx, app);
        }

        self.bookmark_elements.clear();
        let elements = (self.bookmark_element_builder)(
            visible_block_indices.clone(),
            self.hovered_block_index,
            &self.mouse_states.bookmark_mouse_states,
            app,
        );
        for (idx, maybe_element) in visible_block_indices.iter().zip(elements) {
            if let Some(mut element) = maybe_element {
                element.layout(
                    SizeConstraint::new(
                        vec2f(constraint.min.x(), BLOCK_HOVER_BUTTON_HEIGHT),
                        vec2f(constraint.max.x(), BLOCK_HOVER_BUTTON_HEIGHT),
                    ),
                    ctx,
                    app,
                );
                self.bookmark_elements.insert(*idx, element);
            }
        }

        self.filter_elements.clear();
        let elements = (self.filter_elements_builder)(
            visible_block_indices.clone(),
            self.hovered_block_index,
            self.active_filter_editor_block_index,
            self.filtered_blocks.as_ref(),
            &self.mouse_states.filter_mouse_states,
            app,
        );
        for (idx, maybe_element) in visible_block_indices.iter().zip(elements) {
            if let Some(mut element) = maybe_element {
                element.layout(
                    SizeConstraint::new(
                        vec2f(constraint.min.x(), BLOCK_HOVER_BUTTON_HEIGHT),
                        vec2f(constraint.max.x(), BLOCK_HOVER_BUTTON_HEIGHT),
                    ),
                    ctx,
                    app,
                );
                self.filter_elements.insert(*idx, element);
            }
        }

        let size = match self.input_mode {
            InputMode::PinnedToBottom | InputMode::PinnedToTop => constraint.max,

            // In waterfall mode we limit the size to the visible_height so that the
            // it leaves space for the gap, if there is one.  Without this limit, the size
            // of the blocklist would expand, pushing the input to the bottom of the screen.
            InputMode::Waterfall => vec2f(
                constraint.max.x(),
                constraint.max.y().min(visible_height_px as f32),
            ),
        };
        self.size = Some(size);
        size
    }

    /// BlockListElement's implementation of `after_layout` trait method.
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        // Rich Content is arbitrary, so we want to make sure to call after_layout on each of
        // those elements that were actually laid out
        for rich_content in self
            .rich_content_elements
            .values_mut()
            .filter(|e| e.size().is_some())
        {
            rich_content.after_layout(ctx, app);
        }

        for cli_subagent_view in self
            .cli_subagent_views
            .values_mut()
            .filter(|e| e.size().is_some())
        {
            cli_subagent_view.after_layout(ctx, app);
        }

        let model = self.model.lock();
        let viewport = self.viewport_state_after_layout(model.block_list());

        self.line_height = Some(self.size_info.cell_height_px());
        self.scroll_top = Some(viewport.scroll_top_in_lines());
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(RectF::new(
                origin,
                self.size().unwrap(),
            )));
        let element_bounds =
            RectF::new(origin, self.size.expect("Size should be set before paint"));
        self.bounds = Some(element_bounds);
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let model = self.model.lock();
        let viewport = self.viewport_state_after_layout(model.block_list());
        let size = self.size_info;
        let cell_size = Vector2F::new(
            size.cell_width_px().as_f32(),
            size.cell_height_px().as_f32(),
        );

        let mut glyphs = CellGlyphCache::default();
        let mut grid_origin = origin;

        let block_list = model.block_list();
        let total_block_height = block_list.block_heights().summary().height;

        // If the block list is empty, immediately stop rendering as there is nothing to render on
        // the screen.
        if total_block_height == Lines::zero() {
            ctx.scene.stop_layer();
            self.child_max_z_index = Some(ctx.scene.max_active_z_index());
            return;
        }

        // Align the grid origin to start exactly at the top of the first visible block
        grid_origin += vec2f(0., viewport.offset_to_top_of_first_block(app).as_f32());

        let obfuscate_secrets = get_secret_obfuscation_mode(app);

        let grid_render_params = GridRenderParams {
            warp_theme: self.warp_theme.clone(),
            font_family: self.font_family,
            font_size: self.font_size,
            font_weight: self.font_weight,
            line_height_ratio: self.line_height_ratio,
            enforce_minimum_contrast: self.enforce_minimum_contrast,
            obfuscate_secrets,
            size_info: self.size_info,
            cell_size,
            use_ligature_rendering: self.use_ligature_rendering,
            hide_cursor_cell: self.hide_cursor_cell,
        };
        let block_grid_params = BlockGridParams {
            grid_render_params,
            colors: model.colors(),
            override_colors: model.override_colors(),
            bounds: self.bounds.expect("Must be known at paint time"),
        };

        // Determine which of the visible blocks are selected.
        // Note that we have to be careful with the range boundaries since
        // self.visible_blocks is a Range whereas self.selected_blocks.ranges
        // are RangeInclusives.
        let mut visible_selected_blocks = HashSet::new();
        let mut start_of_continuous_selected_blocks = HashSet::new();
        let mut end_of_continuous_selected_blocks = HashSet::new();
        if let Some(visible_blocks) = &self.visible_blocks {
            if !visible_blocks.is_empty() {
                let visible_blocks_inclusive_range =
                    visible_blocks.start..=(visible_blocks.end - 1.into());
                for range in self.selected_blocks.ranges() {
                    visible_selected_blocks
                        .extend(range.intersection(&visible_blocks_inclusive_range));
                    start_of_continuous_selected_blocks.insert(range.start());
                    end_of_continuous_selected_blocks.insert(range.end());
                }
            }
        }

        // Used to determine border styling of selected blocks.
        let is_singleton = self.selected_blocks.is_singleton();
        let tail_index = self.selected_blocks.tail();

        // We use this variable to determine if we should draw a border above
        // the next block to be drawn.
        let mut draw_border_above_block = true;

        struct CLISubagentRenderParams {
            block_id: BlockId,
            view_origin: Option<Vector2F>,
            should_clip_view: bool,
        }

        let mut cli_subagent_views_to_paint = vec![];
        let agent_view_state = model.block_list().agent_view_state();

        let items = self
            .visible_items
            .as_ref()
            .expect("visible items should not be None");
        let mut items_iter = items.iter().enumerate().peekable();
        while let Some((visible_idx, block_entry)) = items_iter.next() {
            let mut snackbar_header = None;
            if visible_idx == 0 {
                if let VisibleItem::Block { block_index, .. } = block_entry {
                    if let Some(block) = model.block_list().block_at(*block_index) {
                        snackbar_header = self.snackbar_header_state().update(
                            self.snackbar_header_state.snackbar_enabled,
                            self.snackbar_header_state.show_snackbar,
                            block,
                            grid_origin,
                            element_bounds,
                            &block_grid_params,
                            self.scroll_position,
                        );
                    };
                } else {
                    // If the first visible item is not a command block, there should be no snackbar header.
                    self.snackbar_header_state().clear_state();
                }
            }
            match block_entry {
                VisibleItem::Block {
                    block_index,
                    subshell_session_id,
                    ..
                } => {
                    let Some(block) = model.block_list().block_at(*block_index) else {
                        continue;
                    };

                    let header_origin = snackbar_header
                        .and_then(|header| header.header_rect())
                        .map_or(grid_origin, |r| r.origin());

                    let mut header_grid_origin = header_origin;

                    if let (Some(_), Some(banner)) = (block.block_banner(), &mut self.block_banner)
                    {
                        banner.paint(header_origin, ctx, app);
                        header_grid_origin += vec2f(
                            0.,
                            banner.size().map_or(BLOCK_BANNER_HEIGHT, |size| size.y()),
                        );
                    }

                    // TODO(vorporeal): should probably use `Pixels` here
                    let block_pixel_height =
                        block.height(agent_view_state).as_f64() as f32 * cell_size.y();

                    let block_bottom_y = grid_origin.y() + block_pixel_height;
                    let selection_bottom_y = snackbar_header
                        .and_then(|header| header.header_rect())
                        .map_or(block_bottom_y, |r| r.max_y().max(block_bottom_y));
                    let selection_height = selection_bottom_y - header_origin.y();

                    let is_current_block_selected = visible_selected_blocks.contains(block_index);
                    let is_top_of_continuous_selection =
                        start_of_continuous_selected_blocks.contains(block_index);
                    let is_bottom_of_continuous_selection =
                        end_of_continuous_selected_blocks.contains(block_index);
                    if is_current_block_selected {
                        let border_info = compute_border_info(
                            is_singleton,
                            tail_index,
                            *block_index,
                            is_top_of_continuous_selection,
                            is_bottom_of_continuous_selection,
                        );

                        let can_be_ai_context = self.ai_render_context.borrow().is_ai_input_enabled
                            && block.can_be_ai_context(agent_view_state);

                        ctx.scene
                            .draw_rect_with_hit_recording(RectF::new(
                                header_origin,
                                Vector2F::new(
                                    self.bounds
                                        .expect("bounds must be set at paint time")
                                        .width(),
                                    selection_height,
                                ),
                            ))
                            .with_background(if can_be_ai_context {
                                self.warp_theme
                                    .block_selection_as_context_background_color()
                            } else {
                                self.warp_theme.block_selection_color()
                            })
                            .with_border(
                                Border::new(border_info.border_width)
                                    .with_sides(
                                        border_info.has_top_border,
                                        true,
                                        border_info.has_bottom_border,
                                        true,
                                    )
                                    .with_border_fill(if can_be_ai_context {
                                        self.warp_theme.block_selection_as_context_border_color()
                                    } else {
                                        self.warp_theme.accent()
                                    }),
                            );
                    }

                    // If this is the top of a continuous selection, there's a top border, so we don't want to draw
                    // the gray border at the top of the block.
                    if is_top_of_continuous_selection {
                        draw_border_above_block = false;
                    }

                    // Current block is selected by ourselves or by another shared session participant
                    let mut is_current_block_selected_by_anyone = is_current_block_selected;
                    let mut participant_ids_for_avatar_render = vec![];
                    if let Some(presence_manager) = &self.presence_manager {
                        let is_self_reconnecting = presence_manager.as_ref(app).is_reconnecting();
                        // Sort participants by reverse participant ID so we construct participant_ids_for_avatar_render in an ordering that's consistent with the pane header (avatars get rendered from right to left below).
                        // Sorting here before rendering borders also ensures that the border color that gets rendered is always the color of participant on the furthest left,
                        // since that is the color that gets rendered last.
                        for participant in presence_manager
                            .as_ref(app)
                            .get_participants_at_selected_block(*block_index, block_list)
                            .into_iter()
                            .sorted_by(|a, b| b.participant.info.id.cmp(&a.participant.info.id))
                        {
                            if participant.should_show_avatar {
                                participant_ids_for_avatar_render
                                    .push(participant.participant.info.id.clone());
                            }
                            // Don't render any shared session participant background or border if we're rendering our own selection background and border.
                            if is_current_block_selected {
                                continue;
                            }
                            let color: Fill = if is_self_reconnecting {
                                MUTED_PARTICIPANT_COLOR
                            } else {
                                participant.participant.color
                            }
                            .into();
                            ctx.scene
                                .draw_rect_with_hit_recording(RectF::new(
                                    header_origin,
                                    Vector2F::new(
                                        self.bounds
                                            .expect("bounds must be set at paint time")
                                            .width(),
                                        selection_height,
                                    ),
                                ))
                                .with_background(color.with_opacity(10))
                                .with_border(
                                    Border::new(SHARED_SESSION_PARTICIPANT_SELECTION_BORDER_WIDTH)
                                        .with_sides(
                                            participant.is_top_of_continuous_selection,
                                            true,
                                            participant.is_bottom_of_continuous_selection,
                                            true,
                                        )
                                        .with_border_fill(color),
                                );
                            // If we drew a colored top border here due to a participant's selection, don't also draw a gray border at the top.
                            if participant.is_top_of_continuous_selection {
                                draw_border_above_block = false;
                            }
                            is_current_block_selected_by_anyone = true;
                        }
                    }
                    // Render their avatar in the top right of the block.
                    let mut avatar_origin = vec2f(
                        header_origin.x()
                            + self.size_info.pane_width_px().as_f32()
                            + self.horizontal_clipped_scroll_state.scroll_start().as_f32()
                            - SELECTED_BLOCK_AVATAR_EDGE_OFFSET
                            - SHARED_SESSION_AVATAR_DIAMETER,
                        header_origin.y() - SHARED_SESSION_AVATAR_DIAMETER / 2.,
                    );
                    // participant_ids_for_avatar_render is already sorted in reverse order,
                    // so the ordering will be consistent with the pane header as we go right to left.
                    for participant_id in participant_ids_for_avatar_render {
                        if let Some(avatar_element) = self.presence_avatars.get_mut(&participant_id)
                        {
                            avatar_element.paint(avatar_origin, ctx, app);
                            avatar_origin.set_x(
                                avatar_origin.x()
                                    - SHARED_SESSION_AVATAR_DIAMETER
                                    - SPACE_BETWEEN_SELECTED_BLOCK_AVATARS,
                            );
                        } else {
                            log::warn!("Should show avatar for shared session participant at selected block but avatar element was not found")
                        }
                    }

                    // Check if this block is in a subshell. If it is, draw a gray stripe on the
                    // left-hand side.
                    if subshell_session_id.is_some() {
                        draw_flag_pole(
                            header_origin.min(grid_origin),
                            block_pixel_height,
                            self.warp_theme.subshell_background(),
                            ctx,
                        );
                    }

                    // This section draws the subshell flag at the start of the subshell
                    if let Some(flag_element) = self.subshell_flags.get_mut(block_index) {
                        // Drawing the flag at the header_origin will draw it at the top of the
                        // block. However, we want the flag to be sticky once the top of the block
                        // scrolls out of the viewport, hence we do `.max(origin)` to stick it.
                        let mut flag_origin = header_origin.max(origin);

                        // Once the flag scrolls to the final consecutive block in this subshell,
                        // we want to stick the flag to the bottom of that block instead of
                        // overlapping into the next block/banner/visible_item. This peeks the next
                        // item to see if it is in the subshell too.
                        if let Some((_, next_item)) = items_iter.peek() {
                            match next_item {
                                VisibleItem::Block {
                                    subshell_session_id: next_id,
                                    ..
                                } if subshell_session_id == next_id => {}
                                _ => {
                                    // Adjust the flag origin to align with the bottom of this block
                                    let max_y = grid_origin.y() + block_pixel_height
                                        - flag_element
                                            .size()
                                            .expect("block must be laid out before paint")
                                            .y();
                                    if flag_origin.y() > max_y {
                                        flag_origin.set_y(max_y);
                                    }
                                }
                            }
                        }

                        flag_element.paint(flag_origin, ctx, app)
                    }

                    if let Some(banner) = block.block_banner() {
                        grid_origin += vec2f(0., banner.banner_height());
                    }

                    Self::draw_block(
                        block,
                        &mut grid_origin,
                        origin,
                        self.find_model
                            .as_ref(app)
                            .is_find_bar_open()
                            .then(|| {
                                self.find_model.as_ref(app).find_render_data_for_block(
                                    *block_index,
                                    Some(block.prompt_and_command_grid().grid_handler()),
                                    Some(block.output_grid().grid_handler()),
                                )
                            })
                            .flatten(),
                        self.highlighted_url.as_ref(),
                        self.link_tool_tip.as_ref(),
                        self.hovered_secret,
                        &mut glyphs,
                        self.label_elements.get_mut(block_index),
                        self.block_footer_elements.get_mut(block_index),
                        *block_index,
                        self.block_borders_enabled,
                        is_current_block_selected_by_anyone,
                        &block_grid_params,
                        &snackbar_header,
                        self.terminal_view_id,
                        draw_border_above_block,
                        self.ai_render_context.borrow().deref(),
                        self.cursor_hint_text_element.as_mut(),
                        &model.image_id_to_metadata,
                        agent_view_state,
                        ctx,
                        app,
                    );

                    ctx.scene.start_layer(ClipBounds::ActiveLayer);
                    let block_is_bookmarked = self.bookmark_elements.contains_key(block_index);
                    let offset = 136.; // 4 icons of 26px width + 4px padding between icons x3 + 4px left padding + 4 px right padding + 4px for selected block border + 8px scrollbar

                    let block_menu_items_start_origin = header_grid_origin
                        + vec2f(
                            self.size_info.pane_width_px().as_f32() - offset
                                + self.horizontal_clipped_scroll_state.scroll_start().as_f32(),
                            self.overflow_offset,
                        );

                    let block_menu_rect_origin = block_menu_items_start_origin - vec2f(4., 4.);
                    let block_menu_rect_size = vec2f(
                        124., // 4 icons of 26px width + 4px padding between icons x3 + 4px left padding + 4px right padding
                        34.,  // 26px height icons + 4px top padding + 4px bottom padding
                    );

                    // We add in increments of 30 as each icon is 26px wide + 4px gap between icons
                    let ask_ai_assistant_button_origin = block_menu_items_start_origin;
                    let bookmark_button_origin = block_menu_items_start_origin + vec2f(30., 0.);
                    let overflow_menu_button_origin =
                        block_menu_items_start_origin + vec2f(90., 0.);
                    let filter_button_origin = block_menu_items_start_origin + vec2f(60., 0.);

                    if let Some(snackbar_toggle_button_origin) =
                        self.compute_snackbar_toggle_button_draw_location(&block_grid_params)
                    {
                        if let Some(snackbar_toggle_button) = self.snackbar_toggle_button.as_mut() {
                            snackbar_toggle_button.paint(snackbar_toggle_button_origin, ctx, app);
                        }
                    }

                    // The block buttons might overlap with the prompt. If that's the case,
                    // we want to detect that it will overlap and draw a background behind
                    // the buttons to occlude the prompt text behind it.
                    let is_block_hovered = self.hovered_block_index == Some(*block_index);

                    let block_has_active_filter_icon = self
                        .filtered_blocks
                        .as_ref()
                        .is_some_and(|filtered_blocks| filtered_blocks.contains(block_index))
                        || self.active_filter_editor_block_index == Some(*block_index);
                    let show_toolbelt_background =
                        is_block_hovered || block_has_active_filter_icon || block_is_bookmarked;
                    if show_toolbelt_background {
                        let prompt_max_x = match self.label_elements.get_mut(block_index) {
                            // If using the default prompt, use the "label" element's
                            // width.
                            Some(element) => match &element.bounds() {
                                Some(rect) => rect.max_x(),
                                None => 0.,
                            },
                            // Otherwise, we need to measure the prompt grid(s). Grids
                            // aren't warpui::Elements, and hence their width isn't
                            // straightforward to measure. We'll use the column index of the
                            // right-most non-empty cell as a proxy for width.
                            None => {
                                header_grid_origin.x()
                                    + self.size_info.padding_x_px().as_f32()
                                    + block
                                        .prompt_rightmost_visible_nonempty_cell()
                                        .map(|col| (col as f32 + 1.) * cell_size.x())
                                        .unwrap_or(0.)
                            }
                        };
                        // Draw the background if the left prompt is wide enough, there is any right prompt at all, or
                        // it is a background block (output grid could contain long strings overlapping with the toolbelt area).
                        let display_rprompt = block.should_display_rprompt(&size)
                            && !self.label_elements.contains_key(block_index);
                        if is_block_hovered
                            && (prompt_max_x > block_menu_items_start_origin.x()
                                || display_rprompt
                                || block.is_background())
                        {
                            // We render the background around the entire toolbelt.
                            ctx.scene
                                .draw_rect_with_hit_recording(RectF::new(
                                    block_menu_rect_origin,
                                    block_menu_rect_size,
                                ))
                                .with_background(self.warp_theme.surface_1())
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
                        } else if prompt_max_x > bookmark_button_origin.x()
                            || prompt_max_x > filter_button_origin.x()
                            || display_rprompt
                        {
                            // We need to render the background around the active bookmark and/or the active filter.
                            let active_bookmark = self.bookmark_elements.contains_key(block_index);

                            let background_origin = if active_bookmark {
                                bookmark_button_origin - vec2f(4., 4.) // We need to account for padding when drawing the background.
                            } else {
                                filter_button_origin - vec2f(4., 4.) // We need to account for padding when drawing the background.
                            };

                            let background_rect_size =
                                if active_bookmark && block_has_active_filter_icon {
                                    vec2f(64., 34.) // 2 icons of 26px width + 4px padding between icons + 4px left padding + 4px right padding
                                } else {
                                    // Only one of the bookmark or filter icons is active.
                                    vec2f(34., 34.) // 1 icons of 26px width + 4px left padding + 4px right padding
                                };
                            ctx.scene
                                .draw_rect_with_hit_recording(RectF::new(
                                    background_origin,
                                    background_rect_size,
                                ))
                                .with_background(self.warp_theme.surface_1())
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
                        }
                    }

                    if is_block_hovered {
                        if let Some(overflow_icon) = self.overflow_menu_button.as_mut() {
                            overflow_icon.paint(overflow_menu_button_origin, ctx, app);
                        }

                        if let Some(ask_ai_assistant_button) = self.ask_ai_assistant_button.as_mut()
                        {
                            ask_ai_assistant_button.paint(ask_ai_assistant_button_origin, ctx, app);
                        }

                        if let Some(save_as_workflow_button) = self.save_as_workflow_button.as_mut()
                        {
                            save_as_workflow_button.paint(bookmark_button_origin, ctx, app);
                        }
                    }

                    // When a block has an active filter on it, we want the filter icon to show even when the block is not hovered over.
                    if let Some(filter_element) = self.filter_elements.get_mut(block_index) {
                        filter_element.paint(filter_button_origin, ctx, app);
                    }

                    // Paint the CLI subagent view on top of everything else for this block
                    let mut render_params = CLISubagentRenderParams {
                        block_id: block.id().clone(),
                        view_origin: None,
                        should_clip_view: !block.is_agent_blocked(),
                    };

                    if let Some(cli_subagent_view) = self.cli_subagent_views.get_mut(block.id()) {
                        // Only paint if the element was laid out; the business logic that decides to render this element is done at layout time.
                        if let Some(cli_subagent_view_size) = cli_subagent_view.size() {
                            render_params.view_origin = Some(
                                vec2f(
                                    grid_origin.x() + block_grid_params.bounds.width(),
                                    grid_origin.y(),
                                ) - vec2f(
                                    CLI_SUBAGENT_HORIZONTAL_MARGIN,
                                    CLI_SUBAGENT_VERTICAL_MARGIN,
                                ) - cli_subagent_view_size,
                            );
                        }
                    }

                    if render_params.view_origin.is_some() {
                        cli_subagent_views_to_paint.push(render_params);
                    }

                    draw_border_above_block = true;
                    ctx.scene.stop_layer();

                    model.block_list().record_block_painted(*block_index);
                }
                VisibleItem::Gap {
                    height_px: height, ..
                } => {
                    let rect = ctx.scene.draw_rect_with_hit_recording(RectF::new(
                        grid_origin,
                        vec2f(self.bounds.unwrap().width(), 1.),
                    ));
                    if self.block_borders_enabled {
                        rect.with_background(self.warp_theme.outline());
                    }

                    draw_border_above_block = true;

                    grid_origin += vec2f(0., *height);
                }
                VisibleItem::RestoredBlockSeparator {
                    height_px: height, ..
                } => {
                    let bounds = self.bounds.expect("Bound should exist");

                    // Paint the border between blocks and the separator.
                    let border = ctx.scene.draw_rect_with_hit_recording(RectF::new(
                        grid_origin,
                        vec2f(bounds.width(), 1.),
                    ));
                    border.with_background(self.warp_theme.outline());

                    let rect = ctx.scene.draw_rect_with_hit_recording(RectF::new(
                        grid_origin,
                        vec2f(bounds.width(), *height),
                    ));
                    rect.with_background(self.warp_theme.restored_blocks_overlay());

                    // Offset the text by the half of the remaining space between the separator height
                    // and the font line height to center it vertically.
                    if let Some(restored_session_separator) = &mut self.restored_session_separator {
                        let alignment_offset = vec2f(
                            SEPARATOR_LEFT_OFFSET,
                            (height - cell_size.y() * SEPARATOR_TO_MONOSPACE_FONT_SIZE_RATIO) / 2.,
                        );
                        restored_session_separator.paint(grid_origin + alignment_offset, ctx, app);
                    }

                    draw_border_above_block = true;

                    grid_origin += vec2f(0., *height);
                }
                VisibleItem::Banner {
                    height_px: height,
                    banner_id,
                    ..
                } => {
                    if let Some(banner) = self.inline_banners.get_mut(banner_id) {
                        banner.paint(grid_origin, ctx, app);
                    }

                    // Since the shared session banner gives a border effect,
                    // we want to avoid drawing a border between the banner and the next block.
                    // Specifically, if there is a banner, we want to draw the border
                    // iff it's not a shared session banner.
                    draw_border_above_block = match self.shared_session_banner_state {
                        SharedSessionBanners::None => true,
                        SharedSessionBanners::ActiveShare {
                            started_banner_id, ..
                        } => *banner_id != started_banner_id,
                        SharedSessionBanners::LastShared {
                            started_banner_id,
                            ended_banner_id,
                            ..
                        } => *banner_id != started_banner_id && *banner_id != ended_banner_id,
                    };

                    grid_origin += vec2f(0., *height);
                }
                VisibleItem::SubshellSeparator {
                    height_px: height,
                    separator_id,
                    ..
                } => {
                    if let Some(separator) = self.subshell_separators.get_mut(separator_id) {
                        separator.paint(grid_origin, ctx, app);
                    };
                    draw_border_above_block = true;

                    grid_origin += vec2f(0., *height);
                }
                VisibleItem::RichContent {
                    view_id, height_px, ..
                } => {
                    let block_origin = grid_origin;
                    if let Some(rich_content) = self.rich_content_elements.get_mut(view_id) {
                        rich_content.paint(grid_origin, ctx, app);
                    }

                    if !FeatureFlag::AgentView.is_enabled() {
                        let ai_render_context = self.ai_render_context.borrow();
                        if let Some(ai_context_color) = self
                            .rich_content_metadata
                            .get(view_id)
                            .and_then(|metadata| {
                                ai_render_context
                                    .context_color_for_rich_content(metadata, &self.warp_theme)
                            })
                        {
                            ctx.scene.start_layer(ClipBounds::ActiveLayer);
                            draw_flag_pole(block_origin, *height_px, ai_context_color, ctx);
                            ctx.scene.stop_layer();
                        }
                    }

                    // Don't draw a border below session headers (i.e. above the next block).
                    draw_border_above_block = !matches!(
                        self.rich_content_metadata.get(view_id),
                        Some(RichContentMetadata::HarnessSessionHeader)
                    );

                    grid_origin += vec2f(0., *height_px);
                }
            }
        }

        let block_list = model.block_list();
        // Recompute the selection range from the current model state to avoid using stale
        // data that was captured during render (blocks may have changed between render and paint).
        let semantic_selection = SemanticSelection::as_ref(app);
        let fresh_selection_ranges = block_list
            .renderable_selection(semantic_selection, self.input_mode.is_inverted_blocklist());
        if let Some(range) = &fresh_selection_ranges {
            // To avoid highlighting over rich blocks, we split the original selection range into multiple
            // sub-ranges, none of which include a rich block.
            let selection_ranges = range
                .iter()
                .flat_map(|selection| self.segment_blocklist_selection(selection, block_list));

            let text_selection_color = if self
                .ai_render_context
                .borrow()
                .has_pending_context_selected_text
            {
                self.warp_theme
                    .text_selection_as_context_color()
                    .into_solid()
            } else {
                self.warp_theme.text_selection_color().into_solid()
            };

            for current_range in selection_ranges {
                self.render_selection(
                    &current_range,
                    origin,
                    block_list,
                    text_selection_color,
                    SelectionCursorRenderLocation::None,
                    ctx,
                );
            }
        };
        self.render_shared_session_participants_selections(origin, block_list, app, ctx);

        if !cli_subagent_views_to_paint.is_empty() {
            for CLISubagentRenderParams {
                block_id,
                view_origin,
                should_clip_view,
            } in cli_subagent_views_to_paint.into_iter()
            {
                if let (Some(cli_subagent_view), Some(view_origin)) =
                    (self.cli_subagent_views.get_mut(&block_id), view_origin)
                {
                    ctx.scene.start_layer(if should_clip_view {
                        ClipBounds::BoundedBy(
                            self.bounds
                                .expect("Bounds were set at beginning of paint()"),
                        )
                    } else {
                        ClipBounds::None
                    });
                    cli_subagent_view.paint(view_origin, ctx, app);
                    ctx.scene.stop_layer();
                }
            }
        }

        ctx.scene.stop_layer();
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let z_index = self.child_max_z_index.expect("Z-index should exist.");

        // During an active text selection, bypass the z-index coverage check for
        // drag events. The input/footer area below the block list is painted at a
        // higher z-index, so `at_z_index` would filter out drags that cross into
        // that region — breaking selection auto-scroll when dragging downward.
        // This matches the pattern used by SelectableArea, Draggable, Resizable,
        // and both Scrollable variants, which all use `raw_event()` for drags.
        let event_at_z_index = if self.is_terminal_selecting
            && matches!(
                event.raw_event(),
                Event::LeftMouseDragged { .. } | Event::LeftMouseUp { .. }
            ) {
            event.raw_event()
        } else {
            let Some(e) = event.at_z_index(z_index, ctx) else {
                // The event is behind an overlay. Still dispatch interactive
                // events to rich content views so overlay children (e.g.
                // ask-user-question speedbump dropdowns) can handle them.
                if matches!(
                    event.raw_event(),
                    Event::ScrollWheel { .. }
                        | Event::LeftMouseDown { .. }
                        | Event::LeftMouseUp { .. }
                        | Event::LeftMouseDragged { .. }
                        | Event::MiddleMouseDown { .. }
                        | Event::RightMouseDown { .. }
                        | Event::BackMouseDown { .. }
                        | Event::ForwardMouseDown { .. }
                ) && self.pane_state.is_focused()
                {
                    let mut handled = false;
                    for view_id in self.visible_rich_content_views() {
                        if let Some(rich_content) = self.rich_content_elements.get_mut(&view_id) {
                            handled |= rich_content.dispatch_event(event, ctx, app);
                        }
                    }
                    return handled;
                }
                return false;
            };
            e
        };

        let mut handled = false;
        let events_to_propagate_on = matches!(
            event_at_z_index,
            Event::MouseMoved { .. }
                | Event::LeftMouseDragged { .. }
                | Event::LeftMouseDown { .. }
                | Event::LeftMouseUp { .. }
                | Event::RightMouseDown { .. }
                | Event::KeyDown { .. }
                | Event::TypedCharacters { .. }
                | Event::ScrollWheel { .. }
        );

        if events_to_propagate_on {
            for cli_subagent_view in self.cli_subagent_views.values_mut() {
                // If the event is handled by the CLI subagent view, do not propagate it down to the blocklist.
                if cli_subagent_view.dispatch_event(event, ctx, app) {
                    return true;
                }
            }

            // The floating buttons are a group of buttons that are on top of the blocklist elements.
            // If the event is handled by any of them, we do not propagate it down to the blocklist.
            // Note that this is in violation of the dispatch_event contract: we are not
            // unconditionally passing the event to all our children. (See the comment on
            // Element.dispatch_event for more information.)
            let mut handled_by_floating_button = false;

            if let Some(overflow_menu_button) = &mut self.overflow_menu_button {
                handled_by_floating_button |= overflow_menu_button.dispatch_event(event, ctx, app);
            }

            if let Some(ask_ai_assistant_button) = &mut self.ask_ai_assistant_button {
                handled_by_floating_button |=
                    ask_ai_assistant_button.dispatch_event(event, ctx, app);
            }

            if let Some(save_as_workflow_button) = &mut self.save_as_workflow_button {
                handled_by_floating_button |=
                    save_as_workflow_button.dispatch_event(event, ctx, app);
            }

            for bookmark_element in self.bookmark_elements.values_mut() {
                handled_by_floating_button |= bookmark_element.dispatch_event(event, ctx, app);
            }

            for filter_element in self.filter_elements.values_mut() {
                handled_by_floating_button |= filter_element.dispatch_event(event, ctx, app);
            }

            if let Some(snackbar_toggle_button) = &mut self.snackbar_toggle_button {
                handled_by_floating_button |=
                    snackbar_toggle_button.dispatch_event(event, ctx, app);
            }

            for avatar_element in self.presence_avatars.values_mut() {
                handled_by_floating_button |= avatar_element.dispatch_event(event, ctx, app);
            }

            if handled_by_floating_button {
                return true;
            }

            // These elements are not floating, so keep the contract of dispatch_event and pass the
            // event to all of them.
            for label_element in self.label_elements.values_mut() {
                handled |= label_element.dispatch_event(event, ctx, app);
            }

            if let Some(banner) = &mut self.block_banner {
                handled |= banner.dispatch_event(event, ctx, app);
            }

            for banner in self.inline_banners.values_mut() {
                handled |= banner.dispatch_event(event, ctx, app);
            }

            // Only dispatch events to rich content elements if the pane is focused.
            //
            // In general, we might consider not handling any events in the pane is focused -- this
            // is already the case for all the blocklist-level handlers below.
            //
            // Its unclear if this should be the case for the hoverable toolbelt elements above.
            // That's an open product question.
            if self.pane_state.is_focused() {
                for view_id in self.visible_rich_content_views() {
                    if let Some(rich_content) = self.rich_content_elements.get_mut(&view_id) {
                        handled |= rich_content.dispatch_event(event, ctx, app);
                    }
                }
            }
        }

        handled |= match event_at_z_index {
            Event::KeyDown {
                keystroke,
                chars,
                details,
                is_composing,
            } => {
                // If this isn't the currently focused session, it shouldn't receive the keydown
                // event.
                if !self.is_terminal_focused {
                    return false;
                }

                // We need to handle ctrl-d as a one-off here as it has special behavior.
                // It should really be a binding, but we don't want this behavior
                // to leak into the rest of the terminal - and we don't have an
                // intermediate view yet.
                if keystroke.normalized() == "ctrl-d" {
                    return self.ctrl_d(ctx);
                }

                let in_long_running_command =
                    self.model.lock().block_list().active_block().is_executing();
                if !*is_composing
                    || (handle_keystroke_despite_composing(keystroke) && in_long_running_command)
                {
                    if let Some(escape_sequence) = (KeystrokeWithDetails {
                        keystroke,
                        key_without_modifiers: details.key_without_modifiers.as_deref(),
                        chars: Some(chars.as_str()),
                    })
                    .to_escape_sequence(self.model.lock().deref())
                    {
                        ctx.dispatch_typed_action(TerminalAction::ControlSequence(escape_sequence));
                        return true;
                    }
                    return self.key_down(chars.as_str(), ctx);
                } else {
                    return false;
                }
            }
            Event::ScrollWheel {
                position,
                delta,
                precise,
                modifiers: ModifiersState { ctrl: false, .. },
            } if !handled => self.scroll_internal(*position, *delta, *precise, ctx, app),
            Event::LeftMouseDown {
                position,
                click_count,
                is_first_mouse,
                modifiers,
                ..
            } => self.mouse_down(
                *position,
                *click_count,
                *is_first_mouse,
                modifiers,
                ctx,
                app,
            ),
            Event::RightMouseDown { position, .. } if !handled => {
                self.right_mouse_down(*position, ctx)
            }
            Event::LeftMouseUp {
                position,
                modifiers,
            } => self.mouse_up(*position, modifiers, ctx, app),
            Event::LeftMouseDragged {
                position,
                modifiers,
                ..
            } => {
                let is_selecting_blocks = if FeatureFlag::RectSelection.is_enabled() {
                    // If cmd and alt are both active, this should be treated as a rect selection.
                    !(modifiers.cmd && modifiers.alt) && (modifiers.cmd || modifiers.shift)
                } else {
                    modifiers.cmd || modifiers.shift
                };
                self.mouse_dragged(*position, is_selecting_blocks, modifiers, ctx, app)
            }
            Event::MouseMoved { position, .. } => self.mouse_moved(*position, app, ctx),
            Event::TypedCharacters { chars } => self.typed_characters(chars, ctx),
            Event::MiddleMouseDown { position, .. } => self.middle_mouse_down(*position, ctx),
            Event::SetMarkedText {
                marked_text,
                selected_range,
            } => self.set_marked_text(marked_text, selected_range, ctx),
            Event::ClearMarkedText => self.clear_marked_text(ctx),
            Event::ModifierKeyChanged { key_code, state } => {
                if self.is_terminal_focused {
                    let is_press = matches!(state, KeyState::Pressed);
                    if let Some(escape_sequence) = maybe_kitty_keyboard_escape_sequence(
                        self.model.lock().deref(),
                        key_code,
                        is_press,
                    ) {
                        ctx.dispatch_typed_action(TerminalAction::ControlSequence(escape_sequence));
                        return true;
                    }
                    self.maybe_handle_voice_toggle(key_code, state, ctx)
                } else {
                    false
                }
            }
            _ => false,
        };

        handled
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

#[derive(Debug)]
struct BorderInfo {
    border_width: f32,
    has_top_border: bool,
    has_bottom_border: bool,
}

/// We need to avoid double borders when selecting multiple blocks:
/// - the tail block will have all four (thick) borders,
/// - any other selected block will only have
///   - a top border if: the prev block didn't have a bottom border
///   - a bottom border if: it isn't the block before the tail block
///     (which would already contribute a top border) OR if this block
///     precedes a non-block (e.g. restored block separators, gaps)
///
/// If RenderContinuousBlockSelectionsWithSingleBorder is enabled:
/// - the tail block will have all four (thick) borders,
/// - any other selected block will only have
///   - a top border if it's the top of a continuous selection
///   - a bottom border if it's the bottom of a continuous selection
/// Note a single selected block is both the top and bottom of a continuous selection.
#[allow(clippy::too_many_arguments)]
fn compute_border_info(
    is_singleton: bool,
    tail_index: Option<BlockIndex>,
    block_index: BlockIndex,
    is_top_of_continuous_selection: bool,
    is_bottom_of_continuous_selection: bool,
) -> BorderInfo {
    let is_tail = tail_index == Some(block_index);

    let border_widths = SelectionBorderWidth::default();
    let border_width = if is_singleton {
        border_widths.single
    } else if is_tail {
        border_widths.tail_multi
    } else {
        border_widths.reg_multi
    };
    let has_top_border = is_top_of_continuous_selection || is_tail;
    let has_bottom_border = is_bottom_of_continuous_selection || is_tail;

    BorderInfo {
        border_width,
        has_top_border,
        has_bottom_border,
    }
}

/// Much like [`f64::round()`] except that this special-cases values in the range (-0.5, 0.5) to
/// round to -1.0/1.0 instead of 0.
fn round_nonzero(n: f64) -> i32 {
    if 0. < n && n < 0.5 {
        return 1;
    } else if -0.5 < n && n < 0. {
        return -1;
    }
    n.round() as i32
}

impl NewScrollableElement for BlockListElement {
    fn axis(&self) -> ScrollableAxis {
        ScrollableAxis::Vertical
    }

    fn scroll_data(&self, _axis: Axis, app: &AppContext) -> Option<ScrollData> {
        ScrollableElement::scroll_data(self, app)
    }

    fn scroll(&mut self, delta: Pixels, _axis: Axis, ctx: &mut EventContext) {
        ScrollableElement::scroll(self, delta, ctx)
    }

    fn axis_should_handle_scroll_wheel(&self, axis: Axis) -> bool {
        matches!(axis, Axis::Horizontal)
    }
}

impl ScrollableElement for BlockListElement {
    fn scroll_data(&self, _app: &AppContext) -> Option<ScrollData> {
        let line_height = self.line_height?;
        let total_size = self
            .model
            .lock()
            .block_list()
            .block_heights()
            .summary()
            .height
            .to_pixels(line_height);
        let mut visible_px = self.size?.y().into_pixels();

        // If the number of visible_lines is within a rounding error of total
        // lines, just set them to be exactly equal so the scrollable element
        // knows not to render a scrollbar in that case.  Otherwise, we risk
        // seeing spurious scrollbars because of our issues with f32 rounding
        // errors in the block list.
        if heights_approx_eq(
            visible_px.to_lines(line_height),
            total_size.to_lines(line_height),
        ) {
            visible_px = total_size;
        }
        Some(ScrollData {
            scroll_start: self.scroll_top?.to_pixels(line_height),
            visible_px,
            total_size,
        })
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        ctx.dispatch_typed_action(TerminalAction::Scroll {
            delta: delta.to_lines(self.line_height.unwrap()),
        });
    }
}

pub struct ToolbeltButtonTooltip {
    pub label: String,
    pub tool_tip_below_button: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn render_hoverable_block_button<F>(
    icon: Container,
    tooltip_info: Option<ToolbeltButtonTooltip>,
    should_ignore_mouse_events: bool,
    should_allow_action: bool,
    mouse_state: MouseStateHandle,
    theme: &WarpTheme,
    ui_builder: &UiBuilder,
    on_click: F,
) -> Box<dyn Element>
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let mut button = Hoverable::new(mouse_state, |state| {
        let mut container = icon.with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)));

        container = if state.is_clicked() || state.is_hovered() {
            container.with_background(theme.surface_2())
        } else {
            container
        };
        let mut stack = Stack::new().with_child(container.finish());

        if let Some(tooltip_info) = tooltip_info {
            if state.is_hovered() {
                let tool_tip = ui_builder.tool_tip(tooltip_info.label).build().finish();
                // Adjust the position of the tooltip depending on whether it is showing on the snackbar header
                let (parent_anchor, child_anchor, offset) = if tooltip_info.tool_tip_below_button {
                    (
                        ParentAnchor::BottomRight,
                        ChildAnchor::TopRight,
                        vec2f(0., 5.),
                    )
                } else {
                    (
                        ParentAnchor::TopRight,
                        ChildAnchor::BottomRight,
                        vec2f(0., -5.),
                    )
                };
                stack.add_positioned_overlay_child(
                    tool_tip,
                    OffsetPositioning::offset_from_parent(
                        offset,
                        ParentOffsetBounds::Unbounded,
                        parent_anchor,
                        child_anchor,
                    ),
                );
            }
        }

        stack.finish()
    });

    if should_allow_action {
        button = button.with_cursor(Cursor::PointingHand).on_click(on_click);
    } else {
        button = button.with_cursor(Cursor::NotAllowed)
    }

    if should_ignore_mouse_events {
        button = button.disable();
    }

    button.finish()
}
