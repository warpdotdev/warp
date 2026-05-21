use crate::appearance::Appearance;
use crate::pane_group::SplitPaneState;
use crate::settings::EnforceMinimumContrast;
use crate::terminal::blockgrid_renderer::GridRenderParams;
use crate::terminal::find::TerminalFindModel;
use crate::terminal::grid_renderer::CellGlyphCache;
use crate::terminal::meta_shortcuts::handle_keystroke_despite_composing;
use crate::terminal::model::escape_sequences::{
    maybe_kitty_keyboard_escape_sequence, KeystrokeWithDetails, ToEscapeSequence,
};
use crate::terminal::model::grid::grid_handler::{Link, TermMode};
use crate::terminal::model::grid::{Dimensions, RespectDisplayedOutput};
use crate::terminal::model::index::Point;
use crate::terminal::model::mouse::{MouseAction, MouseButton, MouseState};
use crate::terminal::model::selection::{SelectAction, SelectionPoint};
use crate::terminal::model::terminal_model::WithinModel;
use crate::terminal::model::SecretHandle;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::shared_session::presence_manager::{
    text_selection_color, PresenceManager, MUTED_PARTICIPANT_COLOR,
};
use crate::terminal::view::{
    ActiveSessionState, TerminalAction, TerminalEditor, TerminalViewRenderContext,
};
use crate::terminal::{grid_renderer, SizeInfo};
use crate::terminal::{heights_approx_eq, TerminalModel};
use num_traits::Float as _;
use parking_lot::FairMutex;
use pathfinder_geometry::vector::vec2f;
use vec1::Vec1;
use warp_core::features::FeatureFlag;
use warp_util::user_input::UserInput;
use warpui::elements::new_scrollable::{NewScrollableElement, ScrollableAxis};
use warpui::event::{KeyState, ModifiersState};
use warpui::platform::keyboard::KeyCode;
use warpui::text::SelectionType;

use super::{should_intercept_mouse, should_intercept_scroll};
use std::ops::{Deref as _, Range};
use std::sync::Arc;
use warpui::elements::{Axis, Point as UiPoint, ScrollData, ScrollableElement};
use warpui::fonts::Properties;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::units::{IntoLines, IntoPixels, Lines, Pixels};
use warpui::{
    end_trace,
    event::{DispatchedEvent, InBoundsExt},
    record_trace_event, start_trace, AfterLayoutContext, AppContext, Element, Event, EventContext,
    LayoutContext, PaintContext, SizeConstraint,
};
use warpui::{ClipBounds, EntityId, ModelHandle};

const CLI_SUBAGENT_HORIZONTAL_MARGIN: f32 = 8.;
const CLI_SUBAGENT_VERTICAL_MARGIN: f32 = 8.;

pub struct AltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    find_model: ModelHandle<TerminalFindModel>,
    is_terminal_focused: bool,
    is_terminal_selecting: bool,
    size: Option<Vector2F>,
    bounds: Option<RectF>,
    origin: Option<UiPoint>,
    highlighted_url: Option<Link>,
    link_tool_tip: Option<Link>,
    grid_render_params: GridRenderParams,

    /// Optional handle to a hovered secret.
    hovered_secret: Option<SecretHandle>,

    /// Used to save the position of the active cursor.
    terminal_view_id: EntityId,

    pane_state: SplitPaneState,
    active_session_state: ActiveSessionState,
    selection_range: Option<Vec1<Range<Point>>>,

    presence_manager: Option<ModelHandle<PresenceManager>>,

    // Fields needed for vertical scrolling for shared session viewer when window is smaller than sharer's
    scroll_top: Lines,
    max_scroll_top: Option<Lines>,
    visible_lines: Option<Lines>,

    cursor_hint_text: Option<Box<dyn Element>>,

    cli_subagent_view: Option<Box<dyn Element>>,

    /// Voice input toggle key code for CLI agent footer integration.
    #[cfg_attr(not(feature = "voice_input"), allow(unused))]
    voice_input_toggle_key_code: Option<KeyCode>,
}

impl AltScreenElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Arc<FairMutex<TerminalModel>>,
        terminal_view_render_context: TerminalViewRenderContext,
        find_model: ModelHandle<TerminalFindModel>,
        enforce_minimum_contrast: EnforceMinimumContrast,
        selection_range: Option<Vec1<Range<Point>>>,
        appearance: &Appearance,
        scroll_top: Lines,
        cursor_hint_text: Option<Box<dyn Element>>,
        cli_subagent_view: Option<Box<dyn Element>>,
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
            is_terminal_focused: terminal_view_render_context.is_terminal_focused,
            is_terminal_selecting: terminal_view_render_context.is_terminal_selecting,
            terminal_view_id: terminal_view_render_context.terminal_view_id,
            selection_range,
            size: None,
            bounds: None,
            origin: None,
            hovered_secret: terminal_view_render_context.hovered_secret,
            highlighted_url,
            link_tool_tip,
            pane_state: terminal_view_render_context.pane_state,
            active_session_state: terminal_view_render_context.active_session_state,
            grid_render_params: GridRenderParams {
                warp_theme: appearance.theme().clone(),
                font_family: appearance.monospace_font_family(),
                font_size: appearance.monospace_font_size(),
                font_weight: appearance.monospace_font_weight(),
                line_height_ratio: appearance.ui_builder().line_height_ratio(),
                enforce_minimum_contrast,
                obfuscate_secrets: terminal_view_render_context.obfuscate_secrets,
                size_info: terminal_view_render_context.size_info,
                cell_size: Vector2F::new(
                    terminal_view_render_context
                        .size_info
                        .cell_width_px()
                        .as_f32(),
                    terminal_view_render_context
                        .size_info
                        .cell_height_px()
                        .as_f32(),
                ),
                use_ligature_rendering: false,
                hide_cursor_cell: false,
            },
            presence_manager: None,
            scroll_top,
            visible_lines: None,
            max_scroll_top: None,
            cursor_hint_text,
            cli_subagent_view,
            voice_input_toggle_key_code: None,
        }
    }

    pub fn with_ligature_rendering(mut self) -> Self {
        self.grid_render_params.use_ligature_rendering = true;
        self
    }

    pub fn with_hide_cursor_cell(mut self) -> Self {
        self.grid_render_params.hide_cursor_cell = true;
        self
    }

    pub fn with_shared_session_presence(
        mut self,
        presence_manager: Option<ModelHandle<PresenceManager>>,
    ) -> Self {
        self.presence_manager = presence_manager;
        self
    }

    /// Sets the voice input toggle key code for CLI agent footer integration.
    #[cfg(feature = "voice_input")]
    pub fn with_voice_input_toggle_key(mut self, key_code: Option<KeyCode>) -> Self {
        self.voice_input_toggle_key_code = key_code;
        self
    }

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

    fn drag_and_drop_file(&mut self, paths: &[String], ctx: &mut EventContext) -> bool {
        if self.is_terminal_focused && !paths.is_empty() {
            let paths = paths.iter().map(ToOwned::to_owned).collect();
            ctx.dispatch_typed_action(TerminalAction::DragAndDropFiles(paths));
            return true;
        }
        false
    }

    fn middle_mouse_down(&self, local_position: Vector2F, ctx: &mut EventContext) -> bool {
        let point = self.coord_to_point(local_position);
        ctx.dispatch_typed_action(TerminalAction::MiddleClickOnGrid {
            position: Some(WithinModel::AltScreen(Point {
                col: point.col,
                row: point.row,
            })),
        });
        true
    }

    fn left_mouse_down(
        &self,
        mouse_state: MouseState,
        local_position: Vector2F,
        click_count: u32,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if !self.pane_state.is_focused() {
            return false;
        }

        ctx.dispatch_typed_action(TerminalAction::Focus);

        // On mobile, request soft keyboard so users can input.
        if warpui::platform::is_mobile_device() {
            ctx.request_soft_keyboard();
        }

        let point = self.coord_to_point(local_position);
        let side = self
            .grid_render_params
            .size_info
            .get_mouse_side(local_position);
        let selection_type = if FeatureFlag::RectSelection.is_enabled() {
            SelectionType::from_mouse_event(*mouse_state.modifiers(), click_count)
        } else {
            SelectionType::from_click_count(click_count)
        };

        if should_intercept_mouse(&self.model.lock(), mouse_state.modifiers().shift, app) {
            ctx.dispatch_typed_action(TerminalAction::AltSelect(SelectAction::Begin {
                point,
                side,
                selection_type,
                position: local_position,
            }));
        } else {
            ctx.dispatch_typed_action(TerminalAction::MaybeClearAltSelect);
            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(mouse_state.set_point(point)));
        }
        true
    }

    fn right_mouse_down(
        &self,
        mouse_state: MouseState,
        local_position: Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.active_session_state != ActiveSessionState::Active {
            return false;
        }

        let point = self.coord_to_point(local_position);

        if should_intercept_mouse(&self.model.lock(), mouse_state.modifiers().shift, app) {
            ctx.dispatch_typed_action(TerminalAction::AltScreenContextMenu {
                position: local_position,
            });
        } else {
            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(mouse_state.set_point(point)));
        }
        true
    }

    fn mouse_moved(
        &self,
        local_position: Vector2F,
        is_synthetic: bool,
        app: &AppContext,
        ctx: &mut EventContext,
    ) -> bool {
        if self.active_session_state != ActiveSessionState::Active {
            return false;
        }

        let point = self.coord_to_point(local_position);

        // If SGR_MOUSE is set -- we consider user to be in an editor like vim or nano.
        let from_editor = match self.model.lock().is_term_mode_set(TermMode::SGR_MOUSE) {
            true => TerminalEditor::Yes,
            false => TerminalEditor::No,
        };

        let grid_point = WithinModel::AltScreen(Point {
            col: point.col,
            row: point.row,
        });
        if get_secret_obfuscation_mode(app).is_visually_obfuscated() {
            let secret_handle = self
                .model
                .lock()
                .secret_at_point(&grid_point)
                .map(|(handle, _)| handle);
            ctx.dispatch_typed_action(TerminalAction::MaybeHoverSecret { secret_handle });
        }

        ctx.dispatch_typed_action(TerminalAction::MaybeLinkHover {
            position: Some(grid_point),
            from_editor,
        });

        // For alt-screen PTY purposes, we ignore synthetic mouse events!
        // This is especially relevant for mouse drags - we do not want mouse moved
        // events being handled at the same time as mouse dragged events.
        if !is_synthetic && self.model.lock().is_term_mode_set(TermMode::MOUSE_MOTION) {
            // Note: it is counter-intuitive to use "pressed" as a state here for mouse hover motions, however,
            // we're largely just following standards from Alacritty/other terminals and the original terminal specs.
            // We intend to combine the mouse button and mouse action enums to avoid "weird" or "impossible" combinations,
            // see Linear issue at https://linear.app/warpdotdev/issue/CORE-1039/combine-the-mousebutton-and-mouseaction-enums-to-avoid-impossible.
            let mouse_state =
                MouseState::new(MouseButton::Move, MouseAction::Pressed, Default::default());
            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(mouse_state.set_point(point)));
        }

        // Allow the event to continue propagating.
        false
    }

    /// Called when the mouse is moved outside of the element.
    fn mouse_out(&self, ctx: &mut EventContext) -> bool {
        ctx.dispatch_typed_action(TerminalAction::MaybeLinkHover {
            position: None,
            from_editor: TerminalEditor::No,
        });
        true
    }

    fn mouse_up(
        &self,
        mouse_state: MouseState,
        local_position: Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.active_session_state != ActiveSessionState::Active {
            return false;
        }

        let point = self.coord_to_point(local_position);

        ctx.dispatch_typed_action(TerminalAction::ClickOnGrid {
            position: WithinModel::AltScreen(Point {
                col: point.col,
                row: point.row,
            }),
            modifiers: *mouse_state.modifiers(),
        });

        if self.is_terminal_selecting {
            ctx.dispatch_typed_action(TerminalAction::AltSelect(SelectAction::End));
        }

        if !should_intercept_mouse(&self.model.lock(), mouse_state.modifiers().shift, app) {
            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(mouse_state.set_point(point)));
        }

        true
    }

    fn mouse_dragged(
        &self,
        mouse_state: MouseState,
        local_position: Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.active_session_state != ActiveSessionState::Active {
            return false;
        }

        let mut is_mouse_dragged = false;
        let point = self.coord_to_point(local_position);

        if self.is_terminal_selecting && self.bounds.is_some() {
            let side = self
                .grid_render_params
                .size_info
                .get_mouse_side(local_position);
            ctx.dispatch_typed_action(TerminalAction::AltSelect(SelectAction::Update {
                point,
                // Scrolling is not yet implemented in AltScreen so leave as 0 for now
                delta: Lines::zero(),
                side,
                position: local_position,
            }));
            is_mouse_dragged = true;
        }
        if !should_intercept_mouse(&self.model.lock(), mouse_state.modifiers().shift, app) {
            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(mouse_state.set_point(point)));
        }
        is_mouse_dragged
    }

    fn on_scroll(
        &mut self,
        local_position: Vector2F,
        delta: Vector2F,
        precise: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let cell_height = self.grid_render_params.size_info.cell_height_px;
        let delta = if precise {
            // Handle Trackpad Scroll by converting pixel height into lines.
            delta.y().into_pixels().to_lines(cell_height)
        } else {
            // Handle Mouse Scroll, whose delta is already in terms of lines.
            delta.y().into_lines()
        };

        // The alt screen can be vertically scrollable iff we're a shared session reader
        // and our window is smaller than the sharer's.
        if self.model.lock().shared_session_status().is_reader() {
            ScrollableElement::scroll(self, delta.to_pixels(cell_height), ctx);
        }

        ctx.dispatch_typed_action(TerminalAction::MaybeDismissToolTip {
            from_keybinding: false,
        });

        let delta = self
            .model
            .lock()
            .alt_screen_mut()
            .accumulate_lines_to_scroll(delta);

        if should_intercept_scroll(&self.model.lock(), app) {
            ctx.dispatch_typed_action(TerminalAction::AltScroll { delta });
        } else {
            let point = self.coord_to_point(local_position);

            ctx.dispatch_typed_action(TerminalAction::AltMouseAction(
                MouseState::new(
                    MouseButton::Wheel,
                    MouseAction::Scrolled { delta },
                    Default::default(),
                )
                .set_point(point),
            ));
        }
        true
    }

    /// Return a Vector2F that can be used to adjust a position to account for vertical scrolling.
    fn vertical_scroll_pixels(&self) -> Vector2F {
        vec2f(
            0.,
            self.scroll_top.as_f64() as f32 * self.line_height().as_f32(),
        )
    }

    /// Converts a pixel coordinate to a point in the `AltScreen` coordinate space.
    fn coord_to_point(&self, coord: Vector2F) -> Point {
        let model = self.model.lock();
        let grid = model.alt_screen().grid_handler();
        let total_height = grid.total_rows();
        let size = self.grid_render_params.size_info;

        let column = ((coord.x() - size.padding_x_px.as_f32()) / size.cell_width_px().as_f32())
            .max(0.)
            .min(grid.columns() as f32 - 1.) as usize;

        let row = (coord.y() / size.cell_height_px().as_f32())
            .max(0.)
            .min(total_height as f32 - 1.) as usize;
        Point::new(row, column)
    }

    /// Renders our own selection.
    fn render_selections(&self, size_info: &SizeInfo, origin: Vector2F, ctx: &mut PaintContext) {
        let text_selection_color = self
            .grid_render_params
            .warp_theme
            .text_selection_color()
            .into_solid();

        if let Some(ranges) = &self.selection_range {
            for range in ranges {
                let start = SelectionPoint {
                    row: range.start.row.into_lines(),
                    col: range.start.col,
                };
                let end = SelectionPoint {
                    row: range.end.row.into_lines(),
                    col: range.end.col,
                };
                grid_renderer::render_selection(
                    &start,
                    &end,
                    size_info,
                    Lines::zero(),
                    origin,
                    text_selection_color,
                    ctx,
                );
            }
        };
    }

    /// Renders any shared session participants' selections.
    fn render_participant_selections(
        &self,
        size_info: &SizeInfo,
        origin: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        if let Some(presence_manager) = &self.presence_manager {
            let is_self_reconnecting = presence_manager.as_ref(app).is_reconnecting();
            for participant in presence_manager.as_ref(app).all_present_participants() {
                let session_sharing_protocol::common::Selection::AltScreenText {
                    start,
                    end,
                    is_reversed,
                } = &participant.info.selection
                else {
                    continue;
                };
                let start = SelectionPoint {
                    row: start.row.into_lines(),
                    col: start.col,
                };
                let end = SelectionPoint {
                    row: end.row.into_lines(),
                    col: end.col,
                };
                let participant_color = if is_self_reconnecting {
                    MUTED_PARTICIPANT_COLOR
                } else {
                    participant.color
                };
                grid_renderer::render_selection(
                    &start,
                    &end,
                    size_info,
                    Lines::zero(),
                    origin,
                    text_selection_color(participant_color),
                    ctx,
                );
                let cursor_point = if *is_reversed { &start } else { &end };
                grid_renderer::render_selection_cursor(
                    cursor_point,
                    size_info,
                    Lines::zero(),
                    origin,
                    participant_color,
                    !*is_reversed,
                    ctx,
                );
            }
        }
    }

    fn total_lines(&self) -> Lines {
        self.grid_render_params.size_info.rows().into_lines()
    }

    fn line_height(&self) -> Pixels {
        self.grid_render_params.size_info.cell_height_px()
    }

    #[cfg(feature = "voice_input")]
    fn maybe_handle_voice_toggle(
        &self,
        key_code: &KeyCode,
        state: &KeyState,
        ctx: &mut EventContext,
    ) -> bool {
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

impl Element for AltScreenElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.size = Some(constraint.max);

        if let Some(cursor_hint_text) = &mut self.cursor_hint_text {
            cursor_hint_text.layout(constraint, ctx, app);
        }

        if let Some(cli_subagent_view) = &mut self.cli_subagent_view {
            cli_subagent_view.layout(
                SizeConstraint {
                    min: vec2f(0., 0.),
                    max: vec2f(
                        constraint.max.x() * 0.3 - CLI_SUBAGENT_HORIZONTAL_MARGIN,
                        constraint.max.y() - CLI_SUBAGENT_VERTICAL_MARGIN * 3.,
                    ),
                },
                ctx,
                app,
            );
        }

        constraint.max
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        let size = self.size.expect("Size should be set in `layout()`");
        self.visible_lines = Some(size.y().into_pixels().to_lines(self.line_height()).floor());
        self.max_scroll_top = Some(self.total_lines() - self.visible_lines.unwrap());
        // After resizing the window to be larger, the max_scroll_top could have decreased,
        // so we need to make sure scroll_top is in bounds.
        self.scroll_top = self.scroll_top.min(self.max_scroll_top.unwrap());

        // We want to make sure to call after_layout on each of the elements that were actually laid out.
        if let Some(cli_subagent_view) = &mut self.cli_subagent_view {
            if cli_subagent_view.size().is_some() {
                cli_subagent_view.after_layout(ctx, app);
            }
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        start_trace!("alt_screen_element:paint");
        self.bounds = Some(RectF::new(
            origin,
            self.size.expect("Size should be set before paint"),
        ));
        self.origin = Some(UiPoint::from_vec2f(origin, ctx.scene.z_index()));

        let model = self.model.lock();

        let override_colors = model.override_colors();

        let grid = model.alt_screen().grid_handler();

        let cell_size = Vector2F::new(
            self.grid_render_params.size_info.cell_width_px().as_f32(),
            self.grid_render_params.size_info.cell_height_px().as_f32(),
        );

        let mut glyphs = CellGlyphCache::default();

        let properties = Properties::default();

        let padding_x = self.grid_render_params.size_info.padding_x_px();

        let find_model = self.find_model.as_ref(app);
        let find_run = find_model
            .is_find_bar_open()
            .then(|| find_model.alt_screen_find_run())
            .flatten();
        let alt_screen_matches = find_run.map(|run| run.matches().iter().rev());
        let focused_match_range = find_run.and_then(|run| run.focused_match_range());

        let obfuscate_secrets =
            get_secret_obfuscation_mode(app).and(&grid.get_secret_obfuscation());

        let mut sampler = model.alt_screen().bg_color_sampler.lock();
        sampler.reset();

        // Render grid cells. Since the alt screen has no scrollback we can always start at index 0.
        record_trace_event!("alt_screen_element:paint:preparing_to_render_grid");
        let start_row = self.scroll_top.as_f64();
        let end_row = (start_row
            + self
                .visible_lines
                .expect("should be set after layout")
                .as_f64())
        .min(grid.visible_rows() as f64);
        let adjusted_grid_origin = origin - self.vertical_scroll_pixels();
        let cursor_visible = model.alt_screen().is_mode_set(TermMode::SHOW_CURSOR);
        grid_renderer::render_grid(
            grid,
            start_row.floor() as usize,
            end_row.ceil() as usize,
            &model.colors(),
            &override_colors,
            &self.grid_render_params.warp_theme,
            properties,
            self.grid_render_params.font_family,
            self.grid_render_params.font_size,
            self.grid_render_params.line_height_ratio,
            cell_size,
            padding_x,
            adjusted_grid_origin,
            &mut glyphs,
            255, /* alpha */
            self.highlighted_url.as_ref(),
            self.link_tool_tip.as_ref(),
            alt_screen_matches,
            focused_match_range,
            self.grid_render_params.enforce_minimum_contrast,
            obfuscate_secrets,
            self.hovered_secret,
            self.grid_render_params.use_ligature_rendering,
            cursor_visible.then(|| model.alt_screen().cursor_style().shape),
            RespectDisplayedOutput::Yes,
            &model.image_id_to_metadata,
            Some(&mut sampler),
            self.grid_render_params.hide_cursor_cell,
            ctx,
            app,
        );
        record_trace_event!("alt_screen_element:paint:grid_rendered");

        // Render cursor if the escape sequence is set.
        // Also suppress the cursor when hide_cursor_cell is active (CLI agent rich input is open).
        if cursor_visible && !self.grid_render_params.hide_cursor_cell {
            grid_renderer::render_cursor(
                &self.grid_render_params,
                grid.cursor_render_point(),
                grid.is_cursor_on_wide_char(),
                model.alt_screen().cursor_style(),
                padding_x,
                adjusted_grid_origin,
                self.grid_render_params.warp_theme.cursor().into(),
                ctx,
                self.terminal_view_id,
                self.cursor_hint_text.as_mut(),
                app,
            );
        }

        record_trace_event!("alt_screen_element:paint:cursor_rendered");

        self.render_selections(
            &self.grid_render_params.size_info,
            adjusted_grid_origin,
            ctx,
        );
        self.render_participant_selections(
            &self.grid_render_params.size_info,
            adjusted_grid_origin,
            ctx,
            app,
        );

        if let Some(cli_subagent_view) = &mut self.cli_subagent_view {
            ctx.scene.start_layer(ClipBounds::ActiveLayer);
            let size = cli_subagent_view
                .size()
                .expect("Subagent output was laid out already.");
            cli_subagent_view.paint(
                vec2f(
                    self.bounds.expect("bounds set during paint.").max_x()
                        - CLI_SUBAGENT_HORIZONTAL_MARGIN
                        - size.x(),
                    self.bounds.expect("bounds set during paint.").max_y()
                        - CLI_SUBAGENT_VERTICAL_MARGIN
                        - size.y(),
                ),
                ctx,
                app,
            );
            ctx.scene.stop_layer();
        }

        record_trace_event!("alt_screen_element:paint:selection_rendered");
        end_trace!();
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
        if let Some(cli_subagent_view) = &mut self.cli_subagent_view {
            if cli_subagent_view.dispatch_event(event, ctx, app) {
                return true;
            }
        }

        let bounds = self
            .bounds
            .expect("Bounds should be set before event dispatching");
        let in_bounds = event.raw_event().in_bounds(bounds);
        let vertical_scroll_pixels = self.vertical_scroll_pixels();
        // Helper function to convert a global (window-space) position to a
        // local (element-space) one.
        let to_local = |position| position - bounds.origin() + vertical_scroll_pixels;

        let z_index = self.z_index().expect("Z-index should exist.");
        let Some(event_at_z_index) = event.at_z_index(z_index, ctx) else {
            // Only proceed if there's a relevant event at this z-index.
            return false;
        };

        match event_at_z_index {
            Event::KeyDown {
                keystroke,
                chars,
                details,
                is_composing,
            } => {
                if !self.is_terminal_focused
                    || (*is_composing && !handle_keystroke_despite_composing(keystroke))
                {
                    return false;
                }
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
                self.key_down(chars, ctx)
            }
            Event::ScrollWheel {
                position,
                delta,
                precise,
                modifiers: ModifiersState { ctrl: false, .. },
            } if in_bounds => self.on_scroll(to_local(*position), *delta, *precise, ctx, app),
            Event::LeftMouseDown {
                position,
                click_count,
                modifiers,
                ..
            } if in_bounds => self.left_mouse_down(
                MouseState::new(MouseButton::Left, MouseAction::Pressed, *modifiers),
                to_local(*position),
                *click_count,
                ctx,
                app,
            ),
            Event::RightMouseDown {
                position,
                cmd,
                shift,
                ..
            } if in_bounds => self.right_mouse_down(
                MouseState::new(
                    MouseButton::Right,
                    MouseAction::Pressed,
                    ModifiersState {
                        alt: false,
                        cmd: *cmd,
                        shift: *shift,
                        ctrl: false,
                        func: false,
                    },
                ),
                to_local(*position),
                ctx,
                app,
            ),
            Event::LeftMouseUp {
                position,
                modifiers,
                ..
            } if in_bounds => self.mouse_up(
                MouseState::new(MouseButton::Left, MouseAction::Released, *modifiers),
                to_local(*position),
                ctx,
                app,
            ),
            Event::LeftMouseDragged {
                position,
                modifiers,
                ..
            } if in_bounds => self.mouse_dragged(
                MouseState::new(MouseButton::LeftDrag, MouseAction::Pressed, *modifiers),
                to_local(*position),
                ctx,
                app,
            ),
            Event::MouseMoved {
                position,
                is_synthetic,
                ..
            } => {
                if in_bounds {
                    self.mouse_moved(to_local(*position), *is_synthetic, app, ctx)
                } else {
                    self.mouse_out(ctx)
                }
            }
            Event::MiddleMouseDown { position, .. } if in_bounds => {
                self.middle_mouse_down(to_local(*position), ctx)
            }
            Event::TypedCharacters { chars } => self.typed_characters(chars, ctx),
            Event::DragAndDropFiles { paths, .. } if in_bounds => {
                self.drag_and_drop_file(paths, ctx)
            }
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
        }
    }

    fn origin(&self) -> Option<UiPoint> {
        self.origin
    }
}

impl NewScrollableElement for AltScreenElement {
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

impl ScrollableElement for AltScreenElement {
    fn scroll_data(&self, _app: &AppContext) -> Option<ScrollData> {
        let line_height = self.line_height();
        let visible_lines = self.visible_lines.expect("should be set after layout");
        let total_lines = self.total_lines();
        // If the number of visible_lines is within a rounding error of total
        // lines, just set them to be exactly equal so the scrollable element
        // knows not to render a scrollbar in that case.  Otherwise, we risk
        // seeing spurious scrollbars because of our issues with f32 rounding
        // errors.
        let visible_px = if heights_approx_eq(visible_lines, total_lines) {
            total_lines.to_pixels(line_height)
        } else {
            visible_lines.to_pixels(line_height)
        };
        Some(ScrollData {
            scroll_start: self.scroll_top.to_pixels(line_height),
            visible_px,
            total_size: total_lines.to_pixels(line_height),
        })
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        self.scroll_top = (self.scroll_top - delta.to_lines(self.line_height()))
            .max(Lines::zero())
            .min(self.max_scroll_top.unwrap());
        ctx.dispatch_typed_action(TerminalAction::SharedSessionViewerAltScroll {
            new_scroll_top: self.scroll_top,
        });
    }
}
