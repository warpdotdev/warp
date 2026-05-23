use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use regex_automata::hybrid::BuildError;
use warp_editor::editor::NavigationKey;
use warpui::accessibility::{AccessibilityContent, WarpA11yRole};
use warpui::elements::{
    Align, Border, ChildAnchor, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dash, Dismiss, DropShadow, Empty, Flex, Hoverable, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Rect, Shrinkable,
    Stack, Text,
};
use warpui::presenter::ChildView;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::model::find::{FindConfig, RegexDFAs};
use crate::appearance::Appearance;
use crate::editor::{
    EditOrigin, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions, ValidInputType,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::themes::theme::Fill;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

const FILTER_BLOCK_PLACEHOLDER_TEXT: &str = "Filter block output";

const BLOCK_FILTER_BAR_WIDTH: f32 = 380.;
const BLOCK_FILTER_BAR_PADDING: f32 = 4.;
const BLOCK_FILTER_EDITOR_PADDING: f32 = 6.;
const BLOCK_FILTER_EDITOR_BORDER_RADIUS: f32 = 4.;
const BLOCK_FILTER_BAR_MARGIN_BETWEEN_EDITORS: f32 = 4.;
const BLOCK_FILTER_EDITOR_BORDER_WIDTH: f32 = 1.;
const BLOCK_FILTER_FONT_SIZE: f32 = 12.;
const BLOCK_FILTER_ICON_PADDING: f32 = 2.;
const BLOCK_FILTER_ICON_MARGIN: f32 = 4.;
const BLOCK_FILTER_ICON_CORNER_RADIUS: f32 = 4.;

const MAXIMUM_CONTEXT_LINES: u16 = 99;
/// The maximum buffer length that we allow in the context line editor.
const MAXIMUM_CONTEXT_LINE_EDITOR_BUFFER_LENGTH: usize = 2;
pub type ContextLines = u16;
pub const DEFAULT_CONTEXT_LINES_VALUE: ContextLines = 0;
const CONTEXT_LINE_EDITOR_TOOLTIP_LABEL: &str = "Show context lines around matches";
const REGEX_TOOLTIP_LABEL: &str = "Regex toggle";
const CASE_SENSITIVITY_TOOLTIP_LABEL: &str = "Case sensitive search";
const INVERT_FILTER_TOOLTIP_LABEL: &str = "Invert filter";

pub const BLOCK_FILTER_DOTTED_LINE_DASH: Dash = Dash {
    dash_length: 4.,
    gap_length: 4.,
    force_consistent_gap_length: false,
};
pub const BLOCK_FILTER_DOTTED_LINE_WIDTH: f32 = 1.;

/// View for the block filter editor.
pub struct BlockFilterEditor {
    query_editor: ViewHandle<EditorView>,
    regex_enabled: bool,
    case_sensitivity_enabled: bool,
    invert_filter_enabled: bool,
    context_line_editor: ViewHandle<EditorView>,
    mouse_state_handles: MouseStateHandles,
    /// This keeps track of whether the previous editor event was select_all
    /// before any user edits were made.
    previous_editor_event_was_select_all: bool,
    /// The number of logical lines that match the current filter. Is None if no
    /// filter is currently active.
    num_matched_lines: Option<usize>,
    /// The number of context lines in the previous query.
    prev_num_context_lines: ContextLines,
}

#[derive(Default)]
struct MouseStateHandles {
    regex_mouse_state_handle: MouseStateHandle,
    case_sensitivity_mouse_state_handle: MouseStateHandle,
    context_line_editor_mouse_state_handle: MouseStateHandle,
    clear_filter_mouse_state_handle: MouseStateHandle,
    invert_filter_mouse_state_handle: MouseStateHandle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockFilterQuery {
    pub query: String,
    /// The number of context lines to include above/below each matched line.
    pub num_context_lines: ContextLines,
    pub regex_enabled: bool,
    pub case_sensitivity_enabled: bool,
    pub invert_filter_enabled: bool,
    /// Only active queries will be applied to a block. Inactive queries will not
    /// be applied, but are used to store the previous filter state on a block.
    pub is_active: bool,
}

pub enum OpenedFromClick {
    Yes,
    No,
}

impl BlockFilterQuery {
    pub fn construct_dfas(&self) -> Result<RegexDFAs, Box<BuildError>> {
        RegexDFAs::new_with_config(
            self.query.as_str(),
            FindConfig {
                is_regex_enabled: self.regex_enabled,
                is_case_sensitive: self.case_sensitivity_enabled,
            },
        )
    }

    /// Returns true if this block filter query will apply an active filter to
    /// the block. If false, this query will set the block into a non-filtered
    /// state.
    pub fn is_active_and_nonempty(&self) -> bool {
        !self.query.is_empty() && self.is_active
    }

    #[cfg(test)]
    pub fn new_for_test(query: String) -> Self {
        Self {
            query,
            num_context_lines: 0,
            regex_enabled: false,
            case_sensitivity_enabled: false,
            invert_filter_enabled: false,
            is_active: true,
        }
    }
}

pub enum BlockFilterEditorEvent {
    UpdateFilter(BlockFilterQuery),
    Close,
}

#[derive(Debug, Clone, Copy)]
pub enum BlockFilterEditorAction {
    Close,
    ToggleRegex,
    ToggleCaseSensitivity,
    ToggleInvertFilter,
    ClearQuery,
}

impl Entity for BlockFilterEditor {
    type Event = BlockFilterEditorEvent;
}

impl TypedActionView for BlockFilterEditor {
    type Action = BlockFilterEditorAction;

    fn handle_action(&mut self, action: &BlockFilterEditorAction, ctx: &mut ViewContext<Self>) {
        match action {
            BlockFilterEditorAction::Close => self.close(ctx),
            BlockFilterEditorAction::ToggleRegex => self.toggle_regex(ctx),
            BlockFilterEditorAction::ToggleCaseSensitivity => self.toggle_case_sensitivity(ctx),
            BlockFilterEditorAction::ToggleInvertFilter => self.toggle_invert_filter(ctx),
            BlockFilterEditorAction::ClearQuery => self.clear_query(ctx),
        }
    }
}

pub fn filter_button_position_id(block: BlockIndex) -> String {
    format!("filter_button_for_block_{block}")
}

impl BlockFilterEditor {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let query_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(BLOCK_FILTER_FONT_SIZE), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(FILTER_BLOCK_PLACEHOLDER_TEXT, ctx);
            editor
        });

        ctx.subscribe_to_view(&query_editor, |me, _handle, event, ctx| {
            me.handle_query_editor_event(event, ctx);
        });

        let context_line_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(BLOCK_FILTER_FONT_SIZE), appearance),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    max_buffer_len: Some(MAXIMUM_CONTEXT_LINE_EDITOR_BUFFER_LENGTH),
                    valid_input_type: ValidInputType::PositiveInteger,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_buffer_text(DEFAULT_CONTEXT_LINES_VALUE.to_string().as_str(), ctx);
            editor
        });

        ctx.subscribe_to_view(&context_line_editor, |me, _handle, event, ctx| {
            me.handle_context_line_editor_event(event, ctx);
        });

        Self {
            query_editor,
            regex_enabled: false,
            case_sensitivity_enabled: false,
            invert_filter_enabled: false,
            mouse_state_handles: Default::default(),
            previous_editor_event_was_select_all: false,
            num_matched_lines: None,
            context_line_editor,
            prev_num_context_lines: DEFAULT_CONTEXT_LINES_VALUE,
        }
    }

    pub fn open_and_set_filter(
        &mut self,
        active_filter_query: Option<BlockFilterQuery>,
        num_matched_lines: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(active_filter) = &active_filter_query {
            self.regex_enabled = active_filter.regex_enabled;
            self.case_sensitivity_enabled = active_filter.case_sensitivity_enabled;
            self.invert_filter_enabled = active_filter.invert_filter_enabled
        } else {
            self.reset_toggles();
        }
        self.num_matched_lines = num_matched_lines;

        // If a previous active filter is present, we populate the editor with that number of context lines.
        // Otherwise, we use the default value.
        match active_filter_query {
            Some(active_filter) => {
                self.query_editor.update(ctx, |editor, ctx| {
                    editor.system_reset_buffer_text(active_filter.query.as_str(), ctx);
                });
                self.context_line_editor.update(ctx, |editor, ctx| {
                    editor.system_reset_buffer_text(
                        active_filter.num_context_lines.to_string().as_str(),
                        ctx,
                    );
                });
            }
            None => self.context_line_editor.update(ctx, |editor, ctx| {
                editor
                    .system_reset_buffer_text(DEFAULT_CONTEXT_LINES_VALUE.to_string().as_str(), ctx)
            }),
        }
    }

    fn reset_toggles(&mut self) {
        self.regex_enabled = false;
        self.case_sensitivity_enabled = false;
        self.invert_filter_enabled = false;
    }

    pub fn set_num_matched_lines(&mut self, num_matched_lines: Option<usize>) {
        self.num_matched_lines = num_matched_lines;
    }

    fn query_editor_text(&self, ctx: &AppContext) -> String {
        self.query_editor.as_ref(ctx).buffer_text(ctx)
    }

    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.query_editor.update(ctx, |editor, ctx| {
            editor.system_clear_buffer(true, ctx);
        });
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(BlockFilterEditorEvent::Close);
    }

    fn toggle_regex(&mut self, ctx: &mut ViewContext<Self>) {
        self.regex_enabled = !self.regex_enabled;
        self.update_query(ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleBlockFilterRegex {
                enabled: self.regex_enabled
            },
            ctx
        );
    }

    fn toggle_case_sensitivity(&mut self, ctx: &mut ViewContext<Self>) {
        self.case_sensitivity_enabled = !self.case_sensitivity_enabled;
        self.update_query(ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleBlockFilterCaseSensitivity {
                enabled: self.case_sensitivity_enabled
            },
            ctx
        );
    }

    fn toggle_invert_filter(&mut self, ctx: &mut ViewContext<Self>) {
        self.invert_filter_enabled = !self.invert_filter_enabled;
        self.update_query(ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleBlockFilterInvert {
                enabled: self.invert_filter_enabled
            },
            ctx
        );
    }

    /// Sends a block filter query update.
    fn update_query(&mut self, ctx: &mut ViewContext<Self>) {
        let num_context_lines = self
            .context_line_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .parse()
            .unwrap_or(DEFAULT_CONTEXT_LINES_VALUE);

        ctx.emit(BlockFilterEditorEvent::UpdateFilter(BlockFilterQuery {
            query: self.query_editor_text(ctx),
            num_context_lines,
            regex_enabled: self.regex_enabled,
            case_sensitivity_enabled: self.case_sensitivity_enabled,
            invert_filter_enabled: self.invert_filter_enabled,
            is_active: true,
        }));

        if num_context_lines != self.prev_num_context_lines {
            send_telemetry_from_ctx!(
                TelemetryEvent::UpdateBlockFilterQueryContextLines { num_context_lines },
                ctx
            );
            self.prev_num_context_lines = num_context_lines;
        }
    }

    fn clear_query(&mut self, ctx: &mut ViewContext<Self>) {
        self.query_editor.update(ctx, |editor, ctx| {
            editor.system_clear_buffer(false, ctx);
        });
        self.update_query(ctx);
        ctx.focus(&self.query_editor);
    }

    fn handle_query_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(edit_origin) => {
                if matches!(edit_origin, EditOrigin::SystemEdit) {
                    return;
                }

                self.update_query(ctx);

                // If the previous editor event was selecting all text and
                // the user now types in a non-empty query, then we should count this as an `UpdateBlockFilterQuery` event.
                if self.previous_editor_event_was_select_all
                    && !self.query_editor_text(ctx).is_empty()
                {
                    send_telemetry_from_ctx!(TelemetryEvent::UpdateBlockFilterQuery, ctx);
                }
                self.previous_editor_event_was_select_all = false;
            }
            EditorEvent::Escape => self.close(ctx),
            EditorEvent::SelectionChanged => {
                self.query_editor.read(ctx, |editor, ctx| {
                    let buffer_text = editor.buffer_text(ctx);
                    let selected_text = editor.selected_text(ctx);
                    if !buffer_text.is_empty() && buffer_text == selected_text {
                        self.previous_editor_event_was_select_all = true;
                    }
                });
            }
            EditorEvent::Navigate(NavigationKey::Tab) => self.focus_other_editor(ctx),
            _ => (),
        }
    }

    fn handle_context_line_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) => self.update_query(ctx),
            EditorEvent::Navigate(NavigationKey::Up) => self.increase_context_line_count(ctx),
            EditorEvent::Navigate(NavigationKey::Down) => self.decrease_context_line_count(ctx),
            EditorEvent::Navigate(NavigationKey::Tab) => self.focus_other_editor(ctx),
            EditorEvent::Escape => self.close(ctx),
            _ => (),
        }
    }

    fn focus_other_editor(&mut self, ctx: &mut ViewContext<Self>) {
        if self.context_line_editor.is_focused(ctx) {
            ctx.focus(&self.query_editor);
        } else if self.query_editor.is_focused(ctx) {
            ctx.focus(&self.context_line_editor);
        }
    }

    fn increase_context_line_count(&mut self, ctx: &mut ViewContext<Self>) {
        self.context_line_editor.update(ctx, |editor, ctx| {
            let parsed_number = editor.buffer_text(ctx).parse::<u16>();
            let new_number = match parsed_number {
                Ok(prev_number) => prev_number.saturating_add(1).min(MAXIMUM_CONTEXT_LINES),
                Err(_) => DEFAULT_CONTEXT_LINES_VALUE,
            };
            editor.set_buffer_text(new_number.to_string().as_str(), ctx);
        })
    }

    fn decrease_context_line_count(&mut self, ctx: &mut ViewContext<Self>) {
        self.context_line_editor.update(ctx, |editor, ctx| {
            let parsed_number = editor.buffer_text(ctx).parse::<u16>();
            let new_number = match parsed_number {
                Ok(prev_number) => prev_number.saturating_sub(1),
                Err(_) => DEFAULT_CONTEXT_LINES_VALUE,
            };
            editor.set_buffer_text(new_number.to_string().as_str(), ctx);
        })
    }

    fn render_separator_line(&self, color: Fill, height: f32) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(Rect::new().with_background(color).finish())
                .with_width(1.)
                .with_height(height)
                .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_hoverable_icon(
        &self,
        appearance: &Appearance,
        icon: Icon,
        is_selected: bool,
        mouse_state_handle: MouseStateHandle,
        on_click_action: BlockFilterEditorAction,
        size: f32,
        tooltip_text: Option<&str>,
    ) -> Box<dyn Element> {
        Hoverable::new(mouse_state_handle, |state| {
            let (border, background) = if is_selected {
                (
                    Border::all(1.).with_border_fill(appearance.theme().accent()),
                    appearance.theme().find_bar_button_selection_color(),
                )
            } else if state.is_hovered() {
                let hover_color = appearance.theme().foreground_button_color();
                (Border::all(1.).with_border_fill(hover_color), hover_color)
            } else {
                let transparent = Fill::Solid(ColorU::transparent_black());
                (Border::all(1.).with_border_fill(transparent), transparent)
            };
            let icon = Container::new(
                ConstrainedBox::new(
                    icon.to_warpui_icon(appearance.theme().active_ui_text_color())
                        .finish(),
                )
                .with_height(size)
                .with_width(size)
                .finish(),
            )
            .with_uniform_padding(BLOCK_FILTER_ICON_PADDING)
            .with_margin_right(BLOCK_FILTER_ICON_MARGIN)
            .with_border(border)
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                BLOCK_FILTER_ICON_CORNER_RADIUS,
            )))
            .finish();

            let mut stack = Stack::new().with_child(icon);
            if let (Some(tooltip_text), true) = (tooltip_text, state.is_hovered()) {
                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(tooltip_text.to_string())
                    .build()
                    .finish();

                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -5.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .on_click(move |ctx, _app, _| ctx.dispatch_typed_action(on_click_action))
        .finish()
    }
}

impl View for BlockFilterEditor {
    fn ui_name() -> &'static str {
        "BlockFilterEditor"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let editor_height = self
            .query_editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance);
        let mut query_editor_row =
            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let regex_icon = self.render_hoverable_icon(
            appearance,
            Icon::Regex,
            self.regex_enabled,
            self.mouse_state_handles.regex_mouse_state_handle.clone(),
            BlockFilterEditorAction::ToggleRegex,
            editor_height,
            Some(REGEX_TOOLTIP_LABEL),
        );
        let case_sensitive_icon = self.render_hoverable_icon(
            appearance,
            Icon::CaseSensitivity,
            self.case_sensitivity_enabled,
            self.mouse_state_handles
                .case_sensitivity_mouse_state_handle
                .clone(),
            BlockFilterEditorAction::ToggleCaseSensitivity,
            editor_height,
            Some(CASE_SENSITIVITY_TOOLTIP_LABEL),
        );
        let invert_filter_icon = self.render_hoverable_icon(
            appearance,
            Icon::Repeat,
            self.invert_filter_enabled,
            self.mouse_state_handles
                .invert_filter_mouse_state_handle
                .clone(),
            BlockFilterEditorAction::ToggleInvertFilter,
            editor_height,
            Some(INVERT_FILTER_TOOLTIP_LABEL),
        );

        let query_editor = Shrinkable::new(
            1.,
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.query_editor).finish()).finish())
                .with_height(editor_height)
                .finish(),
        )
        .finish();
        query_editor_row.add_child(query_editor);

        // We only show the matched line count if there's an active, non-inverted
        // filter. In the future, we may want to show this for inverted filters
        // well, but this requires some changes to how we calculate matched lines.
        let matched_line_count = if let (Some(num_matched_lines), false) =
            (self.num_matched_lines, self.invert_filter_enabled)
        {
            Container::new(
                Text::new_inline(
                    num_matched_lines.to_string(),
                    appearance.ui_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(appearance.theme().disabled_ui_text_color().into())
                .finish(),
            )
            .with_margin_right(8.)
            .finish()
        } else {
            Empty::new().finish()
        };
        let clear_filter_icon = self.render_hoverable_icon(
            appearance,
            Icon::XCircle,
            false, // The "clear filter" icon should never be selected.
            self.mouse_state_handles
                .clear_filter_mouse_state_handle
                .clone(),
            BlockFilterEditorAction::ClearQuery,
            editor_height,
            None,
        );

        query_editor_row.add_child(matched_line_count);
        query_editor_row.add_child(clear_filter_icon);
        query_editor_row.add_child(
            self.render_separator_line(appearance.theme().split_pane_border_color(), editor_height),
        );
        query_editor_row.add_child(regex_icon);
        query_editor_row.add_child(case_sensitive_icon);
        query_editor_row.add_child(invert_filter_icon);

        let mut block_filter_row = Flex::row().with_child(
            Shrinkable::new(
                1.,
                Container::new(query_editor_row.finish())
                    .with_padding_left(BLOCK_FILTER_EDITOR_PADDING)
                    .with_vertical_padding(BLOCK_FILTER_EDITOR_PADDING)
                    .with_background(appearance.theme().surface_1())
                    .with_border(
                        Border::all(BLOCK_FILTER_EDITOR_BORDER_WIDTH)
                            .with_border_fill(appearance.theme().surface_3()),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        BLOCK_FILTER_EDITOR_BORDER_RADIUS,
                    )))
                    .finish(),
            )
            .finish(),
        );

        let mut context_line_editor_row =
            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let hoverable = Hoverable::new(
            self.mouse_state_handles
                .context_line_editor_mouse_state_handle
                .clone(),
            |state| {
                let context_line_icon = ConstrainedBox::new(
                    Icon::DistributeSpacingVertical
                        .to_warpui_icon(
                            blended_colors::text_main(
                                appearance.theme(),
                                appearance.theme().background(),
                            )
                            .into(),
                        )
                        .finish(),
                )
                .with_width(editor_height - 4.)
                .with_height(editor_height - 4.);

                let mut stack =
                    Stack::new().with_child(Align::new(context_line_icon.finish()).finish());

                if state.is_hovered() {
                    let tool_tip = appearance
                        .ui_builder()
                        .tool_tip(CONTEXT_LINE_EDITOR_TOOLTIP_LABEL.to_string())
                        .build()
                        .finish();
                    stack.add_positioned_child(
                        tool_tip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -5.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    );
                }
                stack.finish()
            },
        );

        context_line_editor_row.add_child(
            Container::new(hoverable.finish())
                .with_margin_right(4.)
                .finish(),
        );

        let context_line_editor = Container::new(
            ConstrainedBox::new(
                Clipped::new(ChildView::new(&self.context_line_editor).finish()).finish(),
            )
            .with_height(editor_height)
            .with_width(24.)
            .finish(),
        )
        .with_margin_left(8.)
        .finish();
        context_line_editor_row.add_child(context_line_editor);

        block_filter_row.add_child(
            Container::new(context_line_editor_row.finish())
                .with_background(appearance.theme().surface_1())
                .with_padding_left(8.)
                .with_padding_right(4.)
                .with_vertical_padding(BLOCK_FILTER_EDITOR_PADDING)
                .with_border(
                    Border::all(BLOCK_FILTER_EDITOR_BORDER_WIDTH)
                        .with_border_fill(appearance.theme().surface_3()),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    BLOCK_FILTER_EDITOR_BORDER_RADIUS,
                )))
                .with_margin_left(BLOCK_FILTER_BAR_MARGIN_BETWEEN_EDITORS)
                .finish(),
        );

        // On Windows, we need an extra 2 pixels of height to prevent tooltip text
        // from being clipped due to insufficient vertical space.
        let block_filter_bar_height = editor_height
            + (2. * BLOCK_FILTER_EDITOR_PADDING)
            + (2. * BLOCK_FILTER_BAR_PADDING)
            + if cfg!(windows) { 2. } else { 0. };

        let block_filter_bar = Container::new(
            ConstrainedBox::new(
                Container::new(block_filter_row.finish())
                    .with_background(appearance.theme().surface_2())
                    .finish(),
            )
            .with_height(block_filter_bar_height)
            .with_width(BLOCK_FILTER_BAR_WIDTH)
            .finish(),
        )
        .with_uniform_padding(BLOCK_FILTER_BAR_PADDING)
        .with_background(appearance.theme().surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            BLOCK_FILTER_EDITOR_BORDER_RADIUS,
        )))
        .with_drop_shadow(DropShadow::default())
        .finish();

        Dismiss::new(block_filter_bar)
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(BlockFilterEditorAction::Close))
            .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.query_editor.update(ctx, |editor, ctx| {
                editor.select_all(ctx);
            });
            ctx.focus(&self.query_editor);
            ctx.notify();
        }
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "Type searched phrase.",
            "Press escape to quit",
            WarpA11yRole::TextareaRole,
        ))
    }
}
