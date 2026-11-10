//! Stack control (header trigger) and overlay stack map for CODE-1947.
//!
//! See `specs/CODE-1947/PRODUCT.md` "Stack map" / "Reviewing a layer" and
//! `specs/CODE-1947/TECH.md` "5. Add stack selection state and UI". This
//! module owns presentation only: `CodeReviewView` builds a
//! [`StackMapPresentation`] snapshot from `PrStackInfo` and pushes it in;
//! selection emits [`StackControlEvent`] back to the owner.

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    ChildAnchor, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, DispatchEventResult, DropShadow, Element, Empty, EventHandler,
    Flex, Hoverable, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, SavePosition, ScrollTarget, ScrollToPositionMode,
    ScrollbarWidth, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, FocusContext, SingletonEntity as _, TypedActionView, View, ViewContext, id,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

const MAP_WIDTH: f32 = 320.;
const MAP_MAX_LIST_HEIGHT: f32 = 280.;
const MAP_CORNER_RADIUS: f32 = 6.;
const ROW_HORIZONTAL_PADDING: f32 = 12.;
const ROW_VERTICAL_PADDING: f32 = 8.;
const TRUNK_ROW_VERTICAL_PADDING: f32 = 6.;
const TRIGGER_CORNER_RADIUS: f32 = 4.;
const TRIGGER_VERTICAL_PADDING: f32 = 5.;
const TRIGGER_HORIZONTAL_PADDING: f32 = 8.;

/// Display state of a single pull request row. Mirrors PRODUCT.md item 9.
/// Every state is also rendered as text so focus/selection/state never rely
/// on color alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackRowState {
    Draft,
    Open,
    Merged,
    Closed,
}

impl StackRowState {
    pub fn from_pr_state(state: &str, draft: bool, merged_at_is_some: bool) -> Self {
        if merged_at_is_some {
            Self::Merged
        } else if draft {
            Self::Draft
        } else if state.eq_ignore_ascii_case("closed") {
            Self::Closed
        } else {
            Self::Open
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Open => "Open",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
}

/// One selectable pull request row in the stack map, built by `CodeReviewView`
/// from the latest `PrStackInfo` snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct StackMapRow {
    pub pr_number: u64,
    pub title: String,
    pub head_ref: String,
    pub state: StackRowState,
    pub is_current_branch: bool,
    pub is_selected: bool,
}

/// Immutable presentation snapshot for the stack map. `rows` is ordered
/// bottom to top (index 0 = the layer directly on top of `trunk_ref`),
/// matching `PrStackInfo::layers`; the map renders them in reverse (top to
/// bottom, trunk last) per PRODUCT.md item 7.
#[derive(Clone, Debug, PartialEq)]
pub struct StackMapPresentation {
    pub trunk_ref: String,
    pub rows: Vec<StackMapRow>,
    /// 1-indexed position of the current branch's row, for the trigger
    /// label ("Stack · 2 of 4"). `None` when no row matches the current
    /// branch (e.g. a layer was just merged and removed from the stack).
    pub current_position: Option<usize>,
}

impl StackMapPresentation {
    /// Rows in the order the map renders them: top of stack first, trunk last.
    fn visual_rows(&self) -> impl Iterator<Item = &StackMapRow> {
        self.rows.iter().rev()
    }
}

#[derive(Clone, Debug)]
pub enum StackControlEvent {
    SelectLayer(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StackControlAction {
    Toggle,
    ClickRow { pr_number: u64 },
    HoverRow { pr_number: u64 },
    SelectUp,
    SelectDown,
    SelectEnter,
    Close,
}

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            StackControlAction::SelectUp,
            id!(StackControl::ui_name()),
        ),
        FixedBinding::new(
            "down",
            StackControlAction::SelectDown,
            id!(StackControl::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            StackControlAction::SelectEnter,
            id!(StackControl::ui_name()),
        ),
        FixedBinding::new(
            "space",
            StackControlAction::SelectEnter,
            id!(StackControl::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            StackControlAction::Close,
            id!(StackControl::ui_name()),
        ),
    ]);
}

pub struct StackControl {
    presentation: Option<StackMapPresentation>,
    map_open: bool,
    /// Index into the focused row's position among `presentation.rows`
    /// (bottom-to-top indexing, matching `rows`), when the map is open.
    focused_row_index: Option<usize>,
    trigger_mouse_state: MouseStateHandle,
    /// Scroll state for the row list, so a stack with more layers than fit
    /// in `MAP_MAX_LIST_HEIGHT` remains fully navigable (PRODUCT.md item 7
    /// requires every layer to be reachable, not just the ones that fit).
    list_scroll_state: ClippedScrollStateHandle,
    /// Persisted hover state for the trunk row's tooltip. Pull request rows
    /// reuse keyboard/mouse focus (`focused_row_index`) for their tooltip
    /// instead, since trunk isn't a selectable row and has no such state.
    trunk_mouse_state: MouseStateHandle,
    /// Stable prefix for this control's row `SavePosition` ids, so
    /// `scroll_to_position` can reveal a specific row regardless of how many
    /// `StackControl` instances exist (e.g. split panes).
    position_id_prefix: String,
}

impl StackControl {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self {
            presentation: None,
            map_open: false,
            focused_row_index: None,
            trigger_mouse_state: MouseStateHandle::default(),
            list_scroll_state: ClippedScrollStateHandle::new(),
            trunk_mouse_state: MouseStateHandle::default(),
            position_id_prefix: format!("stack_map_row:{}", ctx.view_id()),
        }
    }

    fn row_position_id(&self, pr_number: u64) -> String {
        format!("{}:{pr_number}", self.position_id_prefix)
    }

    /// Scrolls the currently focused row fully into view. Called after every
    /// change to `focused_row_index` (keyboard navigation, hover-follow, and
    /// reopening the map on the previously selected row) so a stack larger
    /// than the visible list area never leaves the focused row unreachable.
    fn reveal_focused_row(&self) {
        let Some(presentation) = &self.presentation else {
            return;
        };
        let Some(row) = self
            .focused_row_index
            .and_then(|index| presentation.rows.get(index))
        else {
            return;
        };
        self.list_scroll_state.scroll_to_position(ScrollTarget {
            position_id: self.row_position_id(row.pr_number),
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    /// Replaces the presentation snapshot. Closes the map when the stack no
    /// longer has two or more layers (e.g. discovery cleared or the layer
    /// count dropped below the stack threshold).
    pub fn set_presentation(
        &mut self,
        presentation: Option<StackMapPresentation>,
        ctx: &mut ViewContext<Self>,
    ) {
        if presentation.as_ref().is_none_or(|p| p.rows.len() < 2) {
            self.presentation = None;
            self.close(ctx);
            return;
        }
        self.presentation = presentation;
        if self.map_open {
            self.reset_focus_to_selected();
            self.reveal_focused_row();
        }
        ctx.notify();
    }

    /// Whether the trigger should render at all.
    pub fn is_visible(&self) -> bool {
        self.presentation.is_some()
    }

    fn reset_focus_to_selected(&mut self) {
        let Some(presentation) = &self.presentation else {
            self.focused_row_index = None;
            return;
        };
        self.focused_row_index = presentation
            .rows
            .iter()
            .position(|row| row.is_selected)
            .or(Some(presentation.rows.len().saturating_sub(1)));
    }

    fn toggle(&mut self, ctx: &mut ViewContext<Self>) {
        if self.map_open {
            self.close(ctx);
        } else if self.presentation.is_some() {
            self.map_open = true;
            self.reset_focus_to_selected();
            self.reveal_focused_row();
            ctx.notify();
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        if self.map_open {
            self.map_open = false;
            ctx.notify();
        }
    }

    /// Moves focus by `delta` rows in visual order (top to bottom), skipping
    /// the non-selectable trunk row entirely since it isn't part of `rows`.
    fn move_focus(&mut self, delta: i32, ctx: &mut ViewContext<Self>) {
        let Some(presentation) = &self.presentation else {
            return;
        };
        if presentation.rows.is_empty() {
            return;
        }
        // `rows` is bottom-to-top; visual order is top-to-bottom, so moving
        // "down" visually means moving toward index 0.
        let visual_len = presentation.rows.len() as i32;
        let current_visual = self
            .focused_row_index
            .map(|i| (presentation.rows.len() - 1 - i) as i32)
            .unwrap_or(0);
        let next_visual = (current_visual + delta).clamp(0, visual_len - 1);
        self.focused_row_index = Some((presentation.rows.len() as i32 - 1 - next_visual) as usize);
        self.reveal_focused_row();
        ctx.notify();
    }

    fn select_focused(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(presentation) = &self.presentation else {
            return;
        };
        let Some(index) = self.focused_row_index else {
            return;
        };
        let Some(row) = presentation.rows.get(index) else {
            return;
        };
        let pr_number = row.pr_number;
        self.close(ctx);
        ctx.emit(StackControlEvent::SelectLayer(pr_number));
    }

    fn trigger_label(&self) -> String {
        match &self.presentation {
            Some(presentation) => match presentation.current_position {
                Some(position) => format!("Stack · {position} of {}", presentation.rows.len()),
                None => format!("Stack · {} layers", presentation.rows.len()),
            },
            None => String::new(),
        }
    }

    fn render_trigger(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.background()).into_solid();

        let icon = warpui::elements::ConstrainedBox::new(
            Icon::LayersThree01
                .to_warpui_icon(Fill::Solid(text_color))
                .finish(),
        )
        .with_width(14.)
        .with_height(14.)
        .finish();

        let label = Text::new_inline(
            self.trigger_label(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(text_color)
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_clip(ClipConfig::ellipsis())
        .finish();

        let custom_label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon)
            .with_child(Container::new(label).with_margin_left(6.).finish())
            .finish();

        let hover_or_active_styles = UiComponentStyles {
            background: Some(theme.surface_2().into()),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(
                TRIGGER_CORNER_RADIUS,
            ))),
            ..Default::default()
        };

        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.trigger_mouse_state.clone())
            .with_custom_label(custom_label)
            .with_style(UiComponentStyles {
                padding: Some(Coords {
                    top: TRIGGER_VERTICAL_PADDING,
                    bottom: TRIGGER_VERTICAL_PADDING,
                    left: TRIGGER_HORIZONTAL_PADDING,
                    right: TRIGGER_HORIZONTAL_PADDING,
                }),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(
                    TRIGGER_CORNER_RADIUS,
                ))),
                ..Default::default()
            })
            .with_hovered_styles(hover_or_active_styles)
            .with_active_styles(hover_or_active_styles);

        if self.map_open {
            button = button.active();
        }

        button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(StackControlAction::Toggle))
            .finish()
    }

    fn render_row(
        &self,
        row: &StackMapRow,
        is_focused: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = if is_focused {
            Some(theme.accent())
        } else if row.is_selected {
            Some(theme.surface_3())
        } else {
            None
        };

        let mut container = Container::new(self.render_row_content(row, is_focused, appearance))
            .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
            .with_vertical_padding(ROW_VERTICAL_PADDING);
        if let Some(bg) = bg {
            container = container.with_background(bg);
        }

        let pr_number = row.pr_number;
        let row_element = EventHandler::new(container.finish())
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(StackControlAction::ClickRow { pr_number });
                DispatchEventResult::StopPropagation
            })
            .on_mouse_in(
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(StackControlAction::HoverRow { pr_number });
                    ctx.notify();
                    DispatchEventResult::StopPropagation
                },
                None,
            )
            .finish();

        let row_element = if is_focused {
            Self::with_tooltip(
                row_element,
                format!("#{} {}", row.pr_number, row.title),
                appearance,
            )
        } else {
            row_element
        };

        SavePosition::new(row_element, &self.row_position_id(pr_number)).finish()
    }

    /// Wraps `element` with a tooltip carrying `full_text` positioned above
    /// it, for labels that may be ellipsized in the row itself (PRODUCT.md
    /// item 10: truncated stack labels must still be inspectable).
    fn with_tooltip(
        element: Box<dyn Element>,
        full_text: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut stack = Stack::new().with_child(element);
        let tooltip = appearance.ui_builder().tool_tip(full_text).build().finish();
        stack.add_positioned_overlay_child(
            tooltip,
            OffsetPositioning::offset_from_parent(
                vec2f(0., -6.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopMiddle,
                ChildAnchor::BottomMiddle,
            ),
        );
        stack.finish()
    }

    fn render_row_content(
        &self,
        row: &StackMapRow,
        is_focused: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = if is_focused {
            theme.main_text_color(theme.accent()).into_solid()
        } else {
            theme.main_text_color(theme.surface_2()).into_solid()
        };
        let sub_color = if is_focused {
            theme.main_text_color(theme.accent()).into_solid()
        } else {
            theme.sub_text_color(theme.surface_2()).into_solid()
        };
        let font_size = appearance.ui_font_size();

        let number_and_title = Text::new_inline(
            format!("#{} {}", row.pr_number, row.title),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_clip(ClipConfig::ellipsis())
        .finish();

        let mut detail_text = format!("{} · {}", row.state.label(), row.head_ref);
        if row.is_current_branch {
            detail_text.push_str(" · Current branch");
        }
        let detail = Text::new_inline(detail_text, appearance.ui_font_family(), font_size - 1.)
            .with_color(sub_color)
            .with_clip(ClipConfig::ellipsis())
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(number_and_title)
            .with_child(Container::new(detail).with_margin_top(2.).finish())
            .finish()
    }

    fn render_trunk_row(&self, trunk_ref: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_color = theme.sub_text_color(theme.surface_2()).into_solid();
        let content = Container::new(
            Text::new_inline(
                format!("{trunk_ref} (trunk)"),
                appearance.ui_font_family(),
                appearance.ui_font_size() - 1.,
            )
            .with_color(sub_color)
            .with_clip(ClipConfig::ellipsis())
            .finish(),
        )
        .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
        .with_vertical_padding(TRUNK_ROW_VERTICAL_PADDING)
        .finish();

        let full_text = format!("{trunk_ref} (trunk)");
        Hoverable::new(self.trunk_mouse_state.clone(), move |mouse_state| {
            if mouse_state.is_hovered() {
                Self::with_tooltip(content, full_text.clone(), appearance)
            } else {
                content
            }
        })
        .finish()
    }

    fn render_map(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(presentation) = &self.presentation else {
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut column = Flex::column().with_main_axis_size(MainAxisSize::Min);
        for row in presentation.visual_rows() {
            let row_index = presentation
                .rows
                .iter()
                .position(|r| r.pr_number == row.pr_number)
                .unwrap_or(0);
            let is_focused = self.focused_row_index == Some(row_index);
            column.add_child(self.render_row(row, is_focused, appearance));
        }

        // Scrollable so a stack with more layers than fit in
        // `MAP_MAX_LIST_HEIGHT` remains fully navigable by pointer and
        // keyboard (PRODUCT.md item 7). The trunk row is pinned below the
        // scroll region since it's non-interactive context, not a layer.
        let scrollable_rows = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.list_scroll_state.clone(),
                child: column.finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        let list = ConstrainedBox::new(scrollable_rows)
            .with_width(MAP_WIDTH)
            .with_max_height(MAP_MAX_LIST_HEIGHT)
            .finish();

        let mut card_column = Flex::column().with_main_axis_size(MainAxisSize::Min);
        card_column.add_child(list);
        card_column.add_child(self.render_trunk_row(&presentation.trunk_ref, appearance));

        let card = ConstrainedBox::new(
            Container::new(card_column.finish())
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MAP_CORNER_RADIUS)))
                .with_drop_shadow(DropShadow::default())
                .finish(),
        )
        .with_width(MAP_WIDTH)
        .finish();

        Dismiss::new(card)
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(StackControlAction::Close);
            })
            .prevent_interaction_with_other_elements()
            .finish()
    }
}

impl Entity for StackControl {
    type Event = StackControlEvent;
}

impl TypedActionView for StackControl {
    type Action = StackControlAction;

    fn handle_action(&mut self, action: &StackControlAction, ctx: &mut ViewContext<Self>) {
        match action {
            StackControlAction::Toggle => self.toggle(ctx),
            StackControlAction::ClickRow { pr_number } => {
                let Some(presentation) = &self.presentation else {
                    return;
                };
                if let Some(index) = presentation
                    .rows
                    .iter()
                    .position(|row| row.pr_number == *pr_number)
                {
                    self.focused_row_index = Some(index);
                    self.select_focused(ctx);
                }
            }
            StackControlAction::HoverRow { pr_number } => {
                let Some(presentation) = &self.presentation else {
                    return;
                };
                if let Some(index) = presentation
                    .rows
                    .iter()
                    .position(|row| row.pr_number == *pr_number)
                    && self.focused_row_index != Some(index)
                {
                    self.focused_row_index = Some(index);
                    ctx.notify();
                }
            }
            // Visual "down" moves toward the trunk (lower `rows` index counts
            // up in visual-from-top terms); see `move_focus`.
            StackControlAction::SelectDown => self.move_focus(1, ctx),
            StackControlAction::SelectUp => self.move_focus(-1, ctx),
            StackControlAction::SelectEnter => self.select_focused(ctx),
            StackControlAction::Close => self.close(ctx),
        }
    }
}

#[cfg(test)]
#[path = "stack_map_tests.rs"]
mod tests;

impl View for StackControl {
    fn ui_name() -> &'static str {
        "StackControl"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {}

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.is_visible() {
            return warpui::elements::Empty::new().finish();
        }
        let appearance = Appearance::as_ref(app);
        let trigger = self.render_trigger(appearance);

        let mut stack = Stack::new().with_child(trigger);
        if self.map_open {
            stack.add_positioned_overlay_child(
                self.render_map(app),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }
        Container::new(stack.finish()).finish()
    }
}
