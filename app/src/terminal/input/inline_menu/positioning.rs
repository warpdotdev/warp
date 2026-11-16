use std::collections::HashMap;

use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::units::{IntoPixels, Pixels};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, WindowId};

use super::styles::{HEADER_BORDER, HEADER_ROW_HEIGHT};
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::appearance::Appearance;
use crate::settings::{InputModeSettings, InputSettings};
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::input::inline_menu::InlineMenuType;
use crate::terminal::input::inline_menu::message_bar::INLINE_MENU_BORDER_WIDTH;
use crate::terminal::input::inline_menu::styles::{CONTENT_BORDER_WIDTH, CONTENT_VERTICAL_PADDING};
use crate::terminal::input::inline_menu::view::QUERY_RESULT_RENDERER_STYLES;
use crate::terminal::input::message_bar::common::standard_message_bar_height;
use crate::terminal::input::suggestions_mode_model::InputSuggestionsModeModel;
use crate::terminal::{SizeInfo, element_size_at_last_frame};

const DEFAULT_VISIBLE_RESULT_COUNT: f32 = 9.;
const MIN_VISIBLE_RESULT_COUNT: f32 = 3.;
const MAX_VISIBLE_RESULT_COUNT: f32 = 20.;

/// Returns the pixel height of the content panel for a given number of visible result rows,
/// including vertical padding and border widths.
fn content_height_for_row_count(count: f32, appearance: &Appearance) -> f32 {
    (QUERY_RESULT_RENDERER_STYLES.result_item_height_fn)(appearance) * count
        + (CONTENT_VERTICAL_PADDING * 2.)
        + (CONTENT_BORDER_WIDTH * 2.)
}

/// Owns the positioning and sizing state for all inline menus.
///
/// Computes whether the menu renders above or below the input — which depends on the user's
/// `InputMode` setting and the available viewport space — and acts as the sole source of truth
/// for per-menu content heights. The view never stores height; it reads from and writes to
/// this positioner exclusively.
pub struct InlineMenuPositioner {
    size_info: SizeInfo,
    terminal_content_position_id: String,
    /// The save position ID for the input box (prompt, editor, message bar, padding)
    /// WITHOUT the inline menu. Used to measure the input height from the previous frame
    /// for pane-aware height capping.
    input_save_position_id: String,
    window_id: WindowId,
    should_render_below_input: bool,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    agent_view_controller: ModelHandle<AgentViewController>,
    /// Per-menu custom content heights, set by resize.
    custom_content_heights: HashMap<InlineMenuType, f32>,
}

impl InlineMenuPositioner {
    pub fn new(
        suggestions_mode_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: &ModelHandle<AgentViewController>,
        terminal_content_position_id: String,
        input_save_position_id: String,
        size_info: SizeInfo,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let persisted_heights = InputSettings::as_ref(ctx)
            .inline_menu_custom_content_heights
            .value()
            .clone();
        ctx.subscribe_to_model(suggestions_mode_model, |me, _, _, ctx| {
            let suggestions_mode_model = me.suggestions_mode_model.as_ref(ctx);
            if suggestions_mode_model.is_inline_menu_open() {
                if me.agent_view_controller.as_ref(ctx).is_active() {
                    me.should_render_below_input = false;
                } else {
                    match *InputModeSettings::as_ref(ctx).input_mode {
                        InputMode::PinnedToBottom => {
                            me.should_render_below_input = false;
                        }
                        InputMode::PinnedToTop => me.should_render_below_input = true,
                        InputMode::Waterfall => {
                            let terminal_view_height = element_size_at_last_frame(
                                &me.terminal_content_position_id,
                                me.window_id,
                                ctx,
                            )
                            .map(|size| size.y())
                            .unwrap_or(me.size_info.pane_height_px().as_f32());

                            me.should_render_below_input = terminal_view_height
                                < me.inline_menu_height(ctx) + me.menu_frame_height(ctx);
                        }
                    }
                }
                ctx.emit(Updated::Repositioned);
            }
        });

        Self {
            size_info,
            terminal_content_position_id,
            input_save_position_id,
            window_id,
            suggestions_mode_model: suggestions_mode_model.clone(),
            agent_view_controller: agent_view_controller.clone(),
            should_render_below_input: false,
            custom_content_heights: persisted_heights,
        }
    }

    /// Returns `true` if the inline menu should be rendered below the input, factoring in the
    /// current input position in the viewport and the available space above/below it.
    ///
    /// The preference is always to render the menu above the input, if there is available space.
    pub fn should_render_inline_menu_below_input(&self) -> bool {
        self.should_render_below_input
    }

    /// Returns `true` if the inline menu should render results in reverse.
    ///
    /// If `true`, the 0-index item in the menu is rendered at the bottom of the menu instead of the
    /// top.
    pub(super) fn should_render_results_in_reverse(&self, app: &AppContext) -> bool {
        // History menu should not reverse results - up-arrow history is always cycled
        // using the up arrow key, so the ordering should be consistent.
        if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu()
        {
            return false;
        }
        self.should_render_below_input
    }

    /// Returns an 'inset' to be applied to the y position of the top of the blocklist element when:
    /// * in waterfall mode, which is the only mode in which the input may be rendered in a floating
    ///   position between the top and bottom of the viewport.
    /// * the inline menu is visible
    /// * the menu is rendered above the input
    ///
    /// Applying this inset to the blocklist element's position simulates 'scrolling' the blocklist
    /// downwards by the height of the inline menu. Effectively, this "slides" blocklist content
    /// upwards when the menu is open.
    ///
    /// This inset application is done in one of two ways in waterfall mode:
    /// * If there is no gap, we subtract the 'inset', equivalent to the inline menu
    ///   height from the max height constraint of the blocklist element
    /// * If there _is_ a gap, we subtract the inset from painted visible blocklist element originj
    ///   at paint time.
    ///
    /// When there is no gap, that's pretty much all we need to do.
    ///
    /// When there is a gap, there is further accounting to be done: this blocklist translation is
    /// applied at paint-time to reduce the surface area of logic that needs to be aware of the
    /// inline menu visiblity/height (for example, the sumtree doesn't need to be aware of this at
    /// all). This means, however, we do need to account for inset value in logic that translates
    /// artifacts of rendering (layout/paint) back to the data model -- main in places where the
    /// sumtree heights are dependent on the laid out input size (the inline menu is part of the
    /// input element subtree). The main areas considered are:
    ///
    ///   * Gap height calculation (which is one of the items in the sumtree)
    ///   * Scroll position
    ///
    /// Both of these calculations depend on the laid out input size, which must account for the
    /// visible inline menu height.
    ///
    /// Here there be dragons - the key 'contract' of this method is that it returns the exact
    /// height of the inline menu when it is open and rendered above the input.
    ///
    /// If and how the blocklist element logic uses this value depends on whether or not it's
    /// in waterfall mode and whether or not there is an active gap.
    pub(in crate::terminal) fn blocklist_top_inset_when_in_waterfall_mode(
        &self,
        app: &AppContext,
    ) -> Option<Pixels> {
        if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_menu_open()
        {
            (!self.should_render_below_input)
                .then(|| (self.inline_menu_height(app) + self.menu_frame_height(app)).into_pixels())
        } else {
            None
        }
    }

    /// Returns `true` when we should skip rendering the menu entirely because the pane is too short
    /// (otherwise we run into weird clipping and overflow bugs).
    pub fn should_hide_inline_menu_for_pane_size(&self, app: &AppContext) -> bool {
        let one_row_height = content_height_for_row_count(0., Appearance::as_ref(app));
        self.max_content_height_for_pane(app) < one_row_height
    }

    /// Returns the content panel height for the currently active menu, or the default
    /// height if no custom height has been set via resize.
    ///
    /// The returned height is capped so the total menu container plus the non-menu input
    /// elements do not exceed the pane height.
    pub fn inline_menu_height(&self, app: &AppContext) -> f32 {
        self.base_content_height(app)
            .min(self.max_content_height_for_pane(app))
    }

    /// The user-customized or default content height, without any pane-aware capping.
    fn base_content_height(&self, app: &AppContext) -> f32 {
        self.suggestions_mode_model
            .as_ref(app)
            .inline_menu_type()
            .and_then(|mt| self.custom_content_heights.get(&mt).copied())
            .unwrap_or_else(|| {
                content_height_for_row_count(DEFAULT_VISIBLE_RESULT_COUNT, Appearance::as_ref(app))
            })
    }

    /// The non-content height of the menu container (header, message bar, borders).
    fn menu_frame_height(&self, app: &AppContext) -> f32 {
        let header_height = if FeatureFlag::InlineMenuHeaders.is_enabled() {
            HEADER_ROW_HEIGHT + HEADER_BORDER * 2.
        } else {
            0.
        };
        if self.agent_view_controller.as_ref(app).is_active() {
            header_height
        } else {
            header_height + standard_message_bar_height(app) + INLINE_MENU_BORDER_WIDTH
        }
    }

    /// Applies a resize delta to the given menu type's content height, clamping to the
    /// allowed min/max range. Returns the actual height change applied, or `None` if the
    /// delta had no effect.
    pub fn apply_resize_delta(
        &mut self,
        menu_type: InlineMenuType,
        delta: f32,
        ctx: &mut ModelContext<Self>,
    ) -> Option<f32> {
        let current = self.inline_menu_height(ctx);
        let appearance = Appearance::as_ref(ctx);

        let min_height = content_height_for_row_count(MIN_VISIBLE_RESULT_COUNT, appearance);
        let max_height = content_height_for_row_count(MAX_VISIBLE_RESULT_COUNT, appearance)
            .min(self.max_content_height_for_pane(ctx))
            .max(min_height);

        // When the menu renders above input, the header is at the top — dragging
        // down (positive delta) shrinks the menu. When below, the header is at
        // the bottom — dragging down grows the menu.
        let new_height = if self.should_render_below_input {
            current + delta
        } else {
            current - delta
        }
        .clamp(min_height, max_height);
        let height_change = new_height - current;
        if height_change.abs() <= f32::EPSILON {
            return None;
        }

        self.custom_content_heights.insert(menu_type, new_height);

        ctx.emit(Updated::Resized);
        Some(height_change)
    }

    /// Persists the current custom content heights to settings.
    /// Call this when a resize drag finishes rather than on every drag event.
    pub fn persist_custom_content_heights(&self, ctx: &mut ModelContext<Self>) {
        InputSettings::handle(ctx).update(ctx, |settings, ctx| {
            let _ = settings
                .inline_menu_custom_content_heights
                .set_value(self.custom_content_heights.clone(), ctx);
        });
    }

    /// Updates the cached pane size info.
    pub fn set_size_info(&mut self, size_info: SizeInfo, ctx: &mut ModelContext<Self>) {
        let height_changed =
            (self.size_info.pane_height_px().as_f32() - size_info.pane_height_px().as_f32()).abs()
                > f32::EPSILON;
        self.size_info = size_info;
        if height_changed {
            ctx.emit(Updated::Resized);
        }
    }

    /// Returns the maximum content panel height that keeps the entire menu container
    /// plus the non-menu input elements within the pane height.
    fn max_content_height_for_pane(&self, app: &AppContext) -> f32 {
        let pane_height = self.size_info.pane_height_px().as_f32();
        let overhead = self.menu_frame_height(app);

        let input_box_height =
            element_size_at_last_frame(&self.input_save_position_id, self.window_id, app)
                .map(|size| size.y())
                .unwrap_or(0.);

        // We clamp to 0 rather than to MIN_VISIBLE_RESULT_COUNT so that, if the pane is too
        // short to fit even a minimal menu, we still let the menu height shrink to nothing
        // rather than overflowing into an adjacent pane.
        (pane_height - input_box_height - overhead).max(0.)
    }
}

pub enum Updated {
    Repositioned,
    Resized,
}

impl Entity for InlineMenuPositioner {
    type Event = Updated;
}
