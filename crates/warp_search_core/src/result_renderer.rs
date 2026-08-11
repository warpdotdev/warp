use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui_core::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    DropShadow, Empty, EventHandler, Fill as ElementFill, Flex, Hoverable, MainAxisSize,
    MouseState, MouseStateHandle, ParentElement, Radius, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch,
};
use warpui_core::platform::Cursor;
use warpui_core::{Action, AppContext, Element, EventContext, SingletonEntity};

use super::data_source::QueryResult;
use crate::item::IconLocation;

const DETAILS_MIN_WIDTH: f32 = 180.;
const DETAILS_MAX_WIDTH: f32 = 480.;

const CORNER_RADIUS: f32 = 8.;
const DEFAULT_RESULT_HORIZONTAL_PADDING: f32 = 8.;

/// Index of the [`QueryResult`] that was clicked.
pub type QueryResultIndex = usize;

/// Function that is executed when a [`QueryResult`] is clicked.
pub type OnQueryResultClickedFn<T> = Arc<dyn Fn(QueryResultIndex, T, &mut EventContext)>;

/// Struct to generate styles for a [`QueryResultRenderer`].
#[derive(Copy, Clone)]
pub struct QueryResultRendererStyles {
    /// Function that computes the height of the [`QueryResult`].
    pub result_item_height_fn: fn(&Appearance) -> f32,
    /// Function that computes the background [`Fill`] for a [`QueryResult`] that has a details
    /// panel.
    pub panel_background_fill_fn: fn(&Appearance) -> Fill,
    /// Function that computes the background [`Border`] for a [`QueryResult`] that has a details
    /// panel.
    pub panel_border_fn: fn(&Appearance) -> Border,
    /// The [`DropShadow`] of the details panel [`QueryResult`].
    pub panel_drop_shadow: DropShadow,
    /// The [`CornerRadius`] of the details panel [`QueryResult`].
    pub panel_corner_radius: CornerRadius,
    /// Horizontal padding that should be applied to the inner query result.
    pub result_horizontal_padding: f32,
    /// Vertical padding that should be applied to the inner query result.
    pub result_vertical_padding: f32,
    /// Vertical padding that should be applied to the inner query result for multiline items.
    pub result_multiline_vertical_padding: f32,
    /// Horizontal padding applied to each result row (e.g. to prevent a hovered/selected background
    /// from appearing full-bleed against the panel edge).
    pub result_outer_horizontal_padding_fn: fn(&Appearance) -> f32,
    /// Corner radius to apply to a hovered/selected row background.
    pub item_highlight_corner_radius: CornerRadius,
}

impl Default for QueryResultRendererStyles {
    fn default() -> Self {
        Self {
            result_item_height_fn: |appearance| {
                appearance.monospace_font_size() * (1.0 + appearance.line_height_ratio())
            },
            panel_background_fill_fn: |appearance| appearance.theme().surface_2(),
            panel_border_fn: |_| Border::new(0.),
            panel_drop_shadow: DropShadow::default(),
            panel_corner_radius: CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)),
            result_horizontal_padding: DEFAULT_RESULT_HORIZONTAL_PADDING,
            result_vertical_padding: 0.,
            result_multiline_vertical_padding: 0.,
            result_outer_horizontal_padding_fn: |_| 0.,
            item_highlight_corner_radius: CornerRadius::with_all(Radius::Pixels(0.)),
        }
    }
}

impl QueryResultRendererStyles {
    fn result_item_height(&self, appearance: &Appearance) -> f32 {
        (self.result_item_height_fn)(appearance)
    }

    fn panel_background_fill(&self, appearance: &Appearance) -> Fill {
        (self.panel_background_fill_fn)(appearance)
    }

    fn panel_border(&self, appearance: &Appearance) -> Border {
        (self.panel_border_fn)(appearance)
    }
}

/// Struct wrapping a [`QueryResult`], used to render a single query result in the search
/// panel.
///
/// This contains common rendering logic and state required by all search result types. An example
/// of common rendering logic is the common layout of result details (icon, text). An example of
/// commonly required state is the mouse_state_handle used for the [`Hoverable`] element that wraps
/// each result.
pub struct QueryResultRenderer<T: Action + Clone> {
    pub mouse_state_handle: MouseStateHandle,
    pub search_result: QueryResult<T>,
    pub position_id: String,
    pub on_result_click_fn: OnQueryResultClickedFn<T>,
    renderer_styles: QueryResultRendererStyles,
}

impl<T: Action + Clone> QueryResultRenderer<T> {
    /// Creates a new QueryResultRenderer, taking ownership of the given search_result.
    ///
    /// # Arguments
    ///
    /// - `search_result`: The query result that should be rendered.
    /// - `position_id`: Unique string denoting the position id (via a [`SavePosition`]) for the
    ///   element.
    /// - `on_result_click_fn`: Function executed when this item is clicked.
    pub fn new(
        search_result: QueryResult<T>,
        position_id: String,
        on_item_click_fn: impl Fn(QueryResultIndex, T, &mut EventContext) + 'static,
        renderer_styles: QueryResultRendererStyles,
    ) -> Self {
        Self {
            mouse_state_handle: Default::default(),
            search_result,
            position_id,
            renderer_styles,
            on_result_click_fn: Arc::new(on_item_click_fn),
        }
    }

    /// Renders a single result in the search panel, delegating to more granular
    /// [`QueryResult`] render methods to render the internals. This method ensures that
    /// result contents are rendered in a uniform way with respect to layout, spacing, and
    /// coloring.
    pub fn render(
        &self,
        result_index: usize,
        is_selected: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // For static separators, render without hover effects or click handling
        if self.search_result.is_static_separator() {
            return self.render_with_highlight_state(ItemHighlightState::Default, true, app);
        }

        let accept_result = self.search_result.accept_result();
        let on_item_click_fn = self.on_result_click_fn.clone();
        EventHandler::new(
            Hoverable::new(self.mouse_state_handle.clone(), move |mouse_state| {
                self.render_with_highlight_state(
                    ItemHighlightState::new(is_selected, mouse_state),
                    false,
                    app,
                )
            })
            .with_cursor(Cursor::PointingHand)
            .finish(),
        )
        // Reimplement an `on_click` handler using mouse events
        // and prevent propagation to the modal close handler on mouse down.
        .on_left_mouse_down(move |_, _, _| DispatchEventResult::StopPropagation)
        .on_left_mouse_up(move |event_ctx, _, _| {
            on_item_click_fn(result_index, accept_result.clone(), event_ctx);
            DispatchEventResult::StopPropagation
        })
        .finish()
    }

    fn render_with_highlight_state(
        &self,
        highlight_state: ItemHighlightState,
        is_static_separator: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let icon = self.search_result.render_icon(highlight_state, appearance);
        let item = self.search_result.render_item(highlight_state, app);

        let row_height = self.renderer_styles.result_item_height(appearance);
        let (cross_axis_alignment, margin_top) = match self.search_result.icon_location(appearance)
        {
            IconLocation::Centered => (CrossAxisAlignment::Center, 0.),
            IconLocation::Top {
                margin_top: padding_top,
            } => (CrossAxisAlignment::Start, padding_top),
        };

        let is_multiline = self.search_result.is_multiline();
        let row_vertical_padding = if is_multiline {
            self.renderer_styles.result_multiline_vertical_padding
        } else {
            self.renderer_styles.result_vertical_padding
        };

        let row_contents = Container::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(cross_axis_alignment)
                    .with_child(Container::new(icon).with_margin_top(margin_top).finish())
                    .with_child(Shrinkable::new(1., Container::new(item).finish()).finish())
                    .finish(),
            )
            .finish(),
        )
        .with_horizontal_padding(self.renderer_styles.result_horizontal_padding)
        .with_padding_top(row_vertical_padding)
        .with_padding_bottom(if is_static_separator {
            // Static separators should hug their proceeding element more closely,
            // so we don't give them any bottom padding.
            0.
        } else {
            row_vertical_padding
        })
        .finish();

        let row = ConstrainedBox::new(row_contents)
            .with_height(row_height)
            .finish();

        let row = if let Some(background_fill) = self
            .search_result
            .item_background(highlight_state, appearance)
        {
            // Never use gradient backgrounds for hovered/selected states in the command palette.
            // If a gradient is provided, convert it to a solid using the gradient's start (left/top) color.
            let highlighted = Container::new(row)
                .with_corner_radius(self.renderer_styles.item_highlight_corner_radius);
            let solid_bg: pathfinder_color::ColorU =
                ElementFill::from(background_fill).start_color();
            highlighted.with_background_color(solid_bg).finish()
        } else {
            row
        };

        let outer_padding = (self.renderer_styles.result_outer_horizontal_padding_fn)(appearance);
        if outer_padding > 0. {
            Container::new(row)
                .with_padding_left(outer_padding)
                .with_padding_right(outer_padding)
                .finish()
        } else {
            row
        }
    }

    /// Returns a "details" [`Element`] to be rendered in the details pane or `None` if the details
    /// pane should not be shown for this result.
    pub fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        self.search_result.render_details(ctx).map(|details| {
            SizeConstraintSwitch::new(
                ConstrainedBox::new(
                    Container::new(details)
                        .with_corner_radius(self.renderer_styles.panel_corner_radius)
                        .with_background(self.renderer_styles.panel_background_fill(appearance))
                        .with_border(self.renderer_styles.panel_border(appearance))
                        .with_drop_shadow(self.renderer_styles.panel_drop_shadow)
                        .with_uniform_padding(12.)
                        .finish(),
                )
                .with_max_width(DETAILS_MAX_WIDTH)
                .finish(),
                vec![(
                    SizeConstraintCondition::WidthLessThan(DETAILS_MIN_WIDTH),
                    Empty::new().finish(),
                )],
            )
            .finish()
        })
    }
}

/// Represents the UI state of a search result in the panel.  If a result is selected via
/// navigation keys _and_ hovered by the mouse, its state should be `Selected`.
#[derive(Copy, Clone, Debug)]
pub enum ItemHighlightState {
    /// A selected item can still be hovered, so we want to keep track of that within the selected state.
    Selected {
        is_hovered: bool,
    },
    Hovered,
    Default,
}

impl ItemHighlightState {
    pub fn new(is_selected: bool, mouse_state: &MouseState) -> Self {
        if is_selected {
            ItemHighlightState::Selected {
                is_hovered: mouse_state.is_hovered(),
            }
        } else if mouse_state.is_hovered() {
            ItemHighlightState::Hovered
        } else {
            ItemHighlightState::Default
        }
    }

    /// Returns the fill to be used for the search result icon.
    pub fn icon_fill(&self, appearance: &Appearance) -> Fill {
        let theme = appearance.theme();
        match self {
            ItemHighlightState::Selected { .. } => theme.main_text_color(theme.accent()),
            ItemHighlightState::Hovered | ItemHighlightState::Default => {
                theme.main_text_color(theme.surface_2()).with_opacity(80)
            }
        }
    }

    /// Returns the fill to be used for the search result's main text.
    pub fn main_text_fill(&self, appearance: &Appearance) -> Fill {
        let theme = appearance.theme();
        match self {
            ItemHighlightState::Selected { .. } => {
                theme.main_text_color(theme.accent().with_opacity(80))
            }
            ItemHighlightState::Hovered | ItemHighlightState::Default => {
                theme.main_text_color(theme.surface_2())
            }
        }
    }

    /// Returns the fill to be used for the search result's sub text.
    pub fn sub_text_fill(&self, appearance: &Appearance) -> Fill {
        let theme = appearance.theme();
        match self {
            ItemHighlightState::Selected { .. } => {
                theme.sub_text_color(theme.accent().with_opacity(80))
            }
            ItemHighlightState::Hovered | ItemHighlightState::Default => {
                theme.sub_text_color(theme.surface_2())
            }
        }
    }

    /// Returns the fill to be used as the background of the search result.
    pub fn container_background_fill(&self, appearance: &Appearance) -> Option<Fill> {
        match self {
            ItemHighlightState::Selected { .. } | ItemHighlightState::Hovered => Some(
                appearance
                    .theme()
                    .accent()
                    .with_opacity(self.container_background_opacity()),
            ),
            ItemHighlightState::Default => None,
        }
    }

    pub fn container_background_opacity(&self) -> u8 {
        match self {
            ItemHighlightState::Selected { .. } => 90,
            ItemHighlightState::Hovered => 20,
            ItemHighlightState::Default => 0,
        }
    }

    pub fn is_hovered(&self) -> bool {
        match self {
            ItemHighlightState::Selected { is_hovered, .. } => *is_hovered,
            ItemHighlightState::Hovered => true,
            ItemHighlightState::Default => false,
        }
    }

    pub fn is_selected(&self) -> bool {
        matches!(self, ItemHighlightState::Selected { .. })
    }
}
