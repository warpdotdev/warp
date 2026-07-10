//! Generic inline menu view for rendering search results with selection and navigation.
use std::sync::LazyLock;

use itertools::Itertools;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::color::ColorU;
use warpui::elements::drag_resize::drag_resize_handle;
use warpui::elements::{
    ChildAnchor, Clipped, DispatchEventResult, DragResizeElement, DragResizeHandle, EventHandler,
    Expanded, Hoverable, MainAxisAlignment, MainAxisSize, MouseInBehavior, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, ResizeEndFn,
    ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, SizeConstraintCondition,
    SizeConstraintSwitch, Stack, UniformList, UniformListState,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::prelude::{
    Align, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex, SavePosition,
    Text,
};
use warpui::scene::{Border, CornerRadius, Radius};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    Action, AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle,
};

use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::search::item::IconLocation;
use crate::search::mixer::{SearchMixer, SearchMixerEvent};
use crate::search::result_renderer::{
    ItemHighlightState, QueryResultRenderer, QueryResultRendererStyles,
};
use crate::terminal::input::inline_menu::message_bar::{
    InlineMenuMessageBar, InlineMenuMessageBarArgs,
};
use crate::terminal::input::inline_menu::model::{InlineMenuModel, InlineMenuTabConfig};
use crate::terminal::input::inline_menu::positioning::Updated as PositionerUpdated;
use crate::terminal::input::inline_menu::{
    default_navigation_message_items, styles as inline_styles, InlineMenuMessageArgs,
    InlineMenuPositioner, InlineMenuType,
};
use crate::terminal::input::message_bar::Message;
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::{self, input};

/// Events emitted by InlineMenuView.
#[derive(Debug, Clone)]
pub enum InlineMenuEvent<T: Action + Clone> {
    /// User accepted an item via keyboard, or via click when click behavior is accept-on-click.
    // `cmd_or_ctrl_shift_enter` is true if accepted via Cmd/Ctrl+Enter (vs Enter/click).
    AcceptedItem {
        item: T,
        cmd_or_ctrl_shift_enter: bool,
    },
    /// Selection changed during cycling (up/down navigation).
    SelectedItem { item: T },
    /// No results found for current query.
    NoResults,
    /// User dismissed the menu (via escape or click).
    Dismissed,
    /// Active tab changed.
    TabChanged,
}

type TrailingElementFn = Box<dyn Fn(&AppContext) -> Box<dyn Element>>;
type BannerFn = Box<dyn Fn(&AppContext) -> Option<Box<dyn Element>>>;

/// Configuration for the conditional header bar above the inline menu.
pub struct InlineMenuHeaderConfig {
    /// Command label shown at the left of the header.
    pub label: String,
    /// Optional trailing element rendered on the right side of the header.
    pub trailing_element: Option<TrailingElementFn>,
}

/// Determines which item's details are shown in the details pane:
/// the keyboard-selected item or the mouse-hovered item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum DetailsPaneTarget {
    #[default]
    Selection,
    Hover,
}

#[derive(Default)]
struct StateHandles {
    scroll_state: ScrollStateHandle,
    uniform_list: UniformListState,
}

pub(super) static QUERY_RESULT_RENDERER_STYLES: LazyLock<QueryResultRendererStyles> =
    LazyLock::<QueryResultRendererStyles>::new(|| QueryResultRendererStyles {
        result_item_height_fn: |appearance| appearance.monospace_font_size() + 8.,
        panel_border_fn: |appearance| {
            Border::all(1.0).with_border_fill(appearance.theme().outline())
        },
        panel_corner_radius: CornerRadius::with_all(Radius::Pixels(0.)),
        result_vertical_padding: 2.,
        ..Default::default()
    });

pub trait QueryResultRendererExt {
    fn render_inline(
        &self,
        result_index: usize,
        is_selected: bool,
        app: &AppContext,
    ) -> Box<dyn Element>;

    fn render_inline_with_highlight_state(
        &self,
        highlight_state: ItemHighlightState,
        is_static_separator: bool,
        app: &AppContext,
    ) -> Box<dyn Element>;
}

impl<A: InlineMenuAction> QueryResultRendererExt for QueryResultRenderer<A> {
    fn render_inline(
        &self,
        result_index: usize,
        is_selected: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        use warpui::elements::{DispatchEventResult, EventHandler, Hoverable};
        use warpui::platform::Cursor;

        if self.search_result.is_static_separator() {
            return self.render_inline_with_highlight_state(ItemHighlightState::Default, true, app);
        }

        let tooltip_text = self.search_result.tooltip();
        let hoverable = Hoverable::new(self.mouse_state_handle.clone(), move |mouse_state| {
            let content = self.render_inline_with_highlight_state(
                ItemHighlightState::new(is_selected, mouse_state),
                false,
                app,
            );

            match tooltip_text {
                Some(ref text) if mouse_state.is_hovered() => {
                    let tooltip_element = Appearance::as_ref(app)
                        .ui_builder()
                        .tool_tip(text.clone())
                        .build()
                        .finish();
                    let positioning = OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    );
                    Stack::new()
                        .with_child(content)
                        .with_positioned_overlay_child(tooltip_element, positioning)
                        .finish()
                }
                _ => content,
            }
        });

        if self.search_result.is_disabled() {
            hoverable.finish()
        } else {
            let accept_result = self.search_result.accept_result();
            let on_item_click_fn = self.on_result_click_fn.clone();
            EventHandler::new(hoverable.with_cursor(Cursor::PointingHand).finish())
                .on_left_mouse_down(move |_, _, _| DispatchEventResult::StopPropagation)
                .on_left_mouse_up(move |event_ctx, _, _| {
                    on_item_click_fn(result_index, accept_result.clone(), event_ctx);
                    DispatchEventResult::StopPropagation
                })
                .finish()
        }
    }

    fn render_inline_with_highlight_state(
        &self,
        highlight_state: ItemHighlightState,
        is_static_separator: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        use warpui::elements::{MainAxisSize, Shrinkable};

        let appearance = Appearance::as_ref(app);
        let icon = self.search_result.render_icon(highlight_state, appearance);
        let item = self.search_result.render_item(highlight_state, app);

        let (cross_axis_alignment, margin_top) = match self.search_result.icon_location(appearance)
        {
            IconLocation::Centered => (CrossAxisAlignment::Center, 0.),
            IconLocation::Top {
                margin_top: padding_top,
            } => (CrossAxisAlignment::Start, padding_top),
        };

        let inner_content = Container::new(
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
        .with_horizontal_padding(QUERY_RESULT_RENDERER_STYLES.result_horizontal_padding)
        .with_padding_top(QUERY_RESULT_RENDERER_STYLES.result_vertical_padding)
        .with_padding_bottom(if is_static_separator {
            0.
        } else {
            QUERY_RESULT_RENDERER_STYLES.result_vertical_padding
        })
        .finish();

        let result_container = ConstrainedBox::new(inner_content)
            .with_height((QUERY_RESULT_RENDERER_STYLES.result_item_height_fn)(
                appearance,
            ))
            .finish();

        if let Some(background_fill) = self
            .search_result
            .item_background(highlight_state, appearance)
        {
            Container::new(result_container)
                .with_background(background_fill)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    inline_styles::ITEM_CORNER_RADIUS,
                )))
                .finish()
        } else {
            result_container
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DetailsRenderConfig {
    /// The minimum amount of width required to render the detail.
    ///
    /// If `Some()` and the available space to render the details is less than this, no details
    /// are rendered.
    pub min_required_details_width: Option<f32>,

    // If not provided, the details pane will be rendered with half the width of the menu.
    //
    // Else, this is used to apply a max constraint on the result items, and the details pane will
    // take up the rest of the space.
    pub max_result_width: Option<f32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InlineMenuClickBehavior {
    #[default]
    AcceptOnClick,
    SelectOnClick,
}

/// The `Action` type that is dispatched when an item in the menu is 'accepted' (via enter keypress
/// or click).
///
/// This trait includes one additional method that must be implemented to drive the content of
/// menu 'message' bar, which contains contextual hints for navigation and item selection.
pub trait InlineMenuAction: Action + Clone {
    /// The menu type that this action corresponds to.
    const MENU_TYPE: InlineMenuType;

    /// Default implementation that just returns navigation/dismiss hints.
    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        Some(Message::new(default_navigation_message_items(&args)))
    }

    /// If `Some()`, and the selected item has details, will render details on the right side of
    /// the menu.
    fn details_render_config(_app: &AppContext) -> Option<DetailsRenderConfig> {
        None
    }

    /// Determines whether clicking this item should accept it (like pressing Enter) or
    /// select it (like pressing an arrow key). Defaults to accept-on-click.
    fn click_behavior(&self) -> InlineMenuClickBehavior {
        InlineMenuClickBehavior::AcceptOnClick
    }
}

/// A generic inline menu view that renders search results with selection and navigation.
///
/// This view is generic over the action type `A` that is emitted when an item is selected.
/// It handles:
/// - Rendering results from a `SearchMixer<A>`
/// - Selection state management
/// - Keyboard navigation (up/down)
/// - Scrolling
///
/// Domain-specific views (e.g., `InlineSlashCommandView`) should wrap this and:
/// - Create and configure the mixer with appropriate data sources
/// - Subscribe to `InlineMenuEvent` and map to domain-specific events
pub struct InlineMenuView<A: InlineMenuAction, T: 'static + Send + Sync = ()> {
    mixer: ModelHandle<SearchMixer<A>>,
    model: ModelHandle<InlineMenuModel<A, T>>,
    state_handles: StateHandles,
    selected_idx: Option<usize>,
    hovered_idx: Option<usize>,
    details_pane_target: DetailsPaneTarget,
    // This is ordered by increasing score (the top match is the last item), per the `SearchMixer`
    // API.
    result_renderers: Vec<QueryResultRenderer<A>>,
    weak_handle: WeakViewHandle<Self>,
    positioner: ModelHandle<InlineMenuPositioner>,
    message_bar: ViewHandle<InlineMenuMessageBar<A, T>>,
    agent_view_controller: ModelHandle<AgentViewController>,
    header_config: InlineMenuHeaderConfig,
    banner_fn: Option<BannerFn>,
    resize_handle: DragResizeHandle,
    drag_indicator_mouse_state: MouseStateHandle,
    compact_layout: bool,
    dismiss_on_row_click: bool,
}

impl<A: InlineMenuAction> InlineMenuView<A> {
    /// Create a new InlineMenuView with the given mixer, positioner, and styles.
    ///
    /// The view subscribes to mixer events and updates result_renderers automatically.
    /// It also manages its own `InlineMenuModel<A>`, updating it when selection changes
    /// and clearing it when the menu is closed.
    pub fn new(
        mixer: ModelHandle<SearchMixer<A>>,
        positioner: ModelHandle<InlineMenuPositioner>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let inline_menu_model = ctx.add_model(|_| InlineMenuModel::new());
        Self::new_inner(
            mixer,
            positioner,
            input_suggestions_model,
            agent_view_controller,
            inline_menu_model,
            ctx,
        )
    }
}

impl<A: InlineMenuAction, T: 'static + Send + Sync + Clone + PartialEq> InlineMenuView<A, T> {
    pub fn new_with_tabs(
        mixer: ModelHandle<SearchMixer<A>>,
        positioner: ModelHandle<InlineMenuPositioner>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        tab_configs: Vec<InlineMenuTabConfig<T>>,
        initial_tab: Option<T>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let inline_menu_model =
            ctx.add_model(|_| InlineMenuModel::new_with_tabs(tab_configs, initial_tab));
        Self::new_inner(
            mixer,
            positioner,
            input_suggestions_model,
            agent_view_controller,
            inline_menu_model,
            ctx,
        )
    }
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> InlineMenuView<A, T> {
    fn new_inner(
        mixer: ModelHandle<SearchMixer<A>>,
        positioner: ModelHandle<InlineMenuPositioner>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        inline_menu_model: ModelHandle<InlineMenuModel<A, T>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let menu_bar_args = InlineMenuMessageBarArgs {
            inline_menu_model: inline_menu_model.clone(),
            agent_view_controller: agent_view_controller.clone(),
            positioner: positioner.clone(),
        };
        let message_bar = ctx.add_view(|ctx| InlineMenuMessageBar::new(menu_bar_args, ctx));

        ctx.subscribe_to_model(&agent_view_controller, |_, _, event, ctx| match event {
            AgentViewControllerEvent::EnteredAgentView { .. }
            | AgentViewControllerEvent::ExitedAgentView { .. } => ctx.notify(),
            _ => (),
        });

        ctx.subscribe_to_model(
            input_suggestions_model,
            |me, suggestions_model, event, ctx| {
                let InputSuggestionsModeEvent::ModeChanged { .. } = event;
                if suggestions_model.as_ref(ctx).is_closed() {
                    me.selected_idx = None;
                    me.hovered_idx = None;
                    me.details_pane_target = DetailsPaneTarget::default();
                    me.model.update(ctx, |model, ctx| {
                        model.clear_selected_item(ctx);
                    });
                }
            },
        );

        ctx.subscribe_to_model(&mixer, |me, _, event, ctx| match event {
            SearchMixerEvent::ResultsChanged => {
                if me.mixer.as_ref(ctx).is_loading() {
                    // Keep stale results visible while async sources are pending to avoid flicker.
                    return;
                }

                if me.mixer.as_ref(ctx).are_results_empty() {
                    me.result_renderers.clear();
                    me.selected_idx = None;
                    me.hovered_idx = None;
                    me.details_pane_target = DetailsPaneTarget::default();
                    me.model.update(ctx, |model, ctx| {
                        model.clear_selected_item(ctx);
                    });

                    if !me.mixer.as_ref(ctx).is_loading() {
                        ctx.emit(InlineMenuEvent::NoResults);
                    }

                    ctx.notify();
                    return;
                }

                let results = me.mixer.as_ref(ctx).results();

                let dismiss_on_row_click = me.dismiss_on_row_click;
                me.result_renderers = results
                    .clone()
                    .into_iter()
                    .enumerate()
                    .map(|(idx, result)| {
                        QueryResultRenderer::new(
                            result,
                            format!("inline_menu_view:{idx}"),
                            move |result_index, item, ctx| {
                                let action = match item.click_behavior() {
                                    InlineMenuClickBehavior::AcceptOnClick => {
                                        InlineMenuRowAction::Accept {
                                            item,
                                            cmd_or_ctrl_enter: false,
                                        }
                                    }
                                    InlineMenuClickBehavior::SelectOnClick => {
                                        InlineMenuRowAction::Select { result_index, item }
                                    }
                                };
                                ctx.dispatch_typed_action(action);
                                if dismiss_on_row_click {
                                    ctx.dispatch_typed_action(InlineMenuRowAction::<A>::Dismiss);
                                }
                            },
                            *QUERY_RESULT_RENDERER_STYLES,
                        )
                    })
                    .collect();

                me.hovered_idx = None;
                me.details_pane_target = DetailsPaneTarget::default();
                if me.mixer.as_ref(ctx).are_results_empty() {
                    me.selected_idx = None;
                } else {
                    // Select the last non-disabled item by default.
                    let default_selected_idx = (0..me.result_renderers.len())
                        .rev()
                        .find(|&idx| !me.result_renderers[idx].search_result.is_disabled())
                        .unwrap_or(me.result_renderers.len() - 1);
                    me.selected_idx = Some(default_selected_idx);
                    if let Some(result_renderer) = me.result_renderers.get(default_selected_idx) {
                        let item = result_renderer.search_result.accept_result();
                        me.model.update(ctx, |model, ctx| {
                            model.update_selected_item(item.clone(), ctx);
                        });
                        ctx.emit(InlineMenuEvent::SelectedItem { item });
                    }
                };
                me.scroll_to_selected_idx(ctx);
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&positioner, |me, _, event, ctx| {
            if matches!(event, PositionerUpdated::Repositioned) {
                me.scroll_to_selected_idx(ctx);
            }
            ctx.notify()
        });
        Self {
            mixer,
            model: inline_menu_model,
            positioner,
            message_bar,
            agent_view_controller,
            selected_idx: None,
            hovered_idx: None,
            details_pane_target: DetailsPaneTarget::default(),
            result_renderers: vec![],
            state_handles: Default::default(),
            weak_handle: ctx.handle(),
            header_config: InlineMenuHeaderConfig {
                label: A::MENU_TYPE.display_label().to_string(),
                trailing_element: None,
            },
            banner_fn: None,
            resize_handle: drag_resize_handle(),
            drag_indicator_mouse_state: MouseStateHandle::default(),
            compact_layout: false,
            dismiss_on_row_click: false,
        }
    }

    pub fn with_header_config(mut self, config: InlineMenuHeaderConfig) -> Self {
        self.header_config = config;
        self
    }

    pub fn with_compact_layout(mut self) -> Self {
        self.compact_layout = true;
        self
    }

    pub fn with_dismiss_on_row_click(mut self) -> Self {
        self.dismiss_on_row_click = true;
        self
    }

    pub fn with_banner_fn(
        mut self,
        banner_fn: impl Fn(&AppContext) -> Option<Box<dyn Element>> + 'static,
    ) -> Self {
        self.banner_fn = Some(Box::new(banner_fn));
        self
    }

    pub fn model(&self) -> &ModelHandle<InlineMenuModel<A, T>> {
        &self.model
    }

    pub fn accept_selected_item(&mut self, cmd_or_ctrl_enter: bool, ctx: &mut ViewContext<Self>) {
        let Some(selected_idx) = self.selected_idx else {
            return;
        };
        let Some(selected_result) = self
            .result_renderers
            .get(selected_idx)
            .map(|renderer| &renderer.search_result)
        else {
            return;
        };
        if selected_result.is_disabled() {
            return;
        }
        ctx.emit(InlineMenuEvent::AcceptedItem {
            item: selected_result.accept_result(),
            cmd_or_ctrl_shift_enter: cmd_or_ctrl_enter,
        });
    }

    pub fn select_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .positioner
            .as_ref(ctx)
            .should_render_results_in_reverse(ctx)
        {
            self.select_next(ctx)
        } else {
            self.select_prev(ctx)
        }
    }

    pub fn select_down(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .positioner
            .as_ref(ctx)
            .should_render_results_in_reverse(ctx)
        {
            self.select_prev(ctx)
        } else {
            self.select_next(ctx)
        }
    }

    pub fn result_count(&self) -> usize {
        self.result_renderers.len()
    }

    pub fn selected_idx(&self) -> Option<usize> {
        self.selected_idx
    }

    pub fn select_idx(&mut self, idx: usize, ctx: &mut ViewContext<Self>) {
        if idx >= self.result_renderers.len() {
            return;
        }

        self.selected_idx = Some(idx);
        self.hovered_idx = None;
        self.details_pane_target = DetailsPaneTarget::Selection;
        self.scroll_to_selected_idx(ctx);
        if let Some(result_renderer) = self.result_renderers.get(idx) {
            let item = result_renderer.search_result.accept_result();
            self.model.update(ctx, |model, ctx| {
                model.update_selected_item(item.clone(), ctx);
            });
            ctx.emit(InlineMenuEvent::SelectedItem { item });
        }
        ctx.notify();
    }

    pub fn select_first_where(
        &mut self,
        predicate: impl Fn(&A) -> bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        for (idx, renderer) in self.result_renderers.iter().enumerate() {
            let action = renderer.search_result.accept_result();
            if predicate(&action) {
                self.select_idx(idx, ctx);
                return true;
            }
        }
        false
    }

    pub fn select_last_where(
        &mut self,
        predicate: impl Fn(&A) -> bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        for idx in (0..self.result_renderers.len()).rev() {
            let action = self.result_renderers[idx].search_result.accept_result();
            if predicate(&action) {
                self.select_idx(idx, ctx);
                return true;
            }
        }
        false
    }

    fn scroll_to_selected_idx(&self, app: &AppContext) {
        let result_count = self.result_renderers.len();
        if result_count == 0 {
            return;
        }
        let Some(selected_idx) = self.selected_idx else {
            return;
        };

        self.state_handles.uniform_list.scroll_to(
            if self
                .positioner
                .as_ref(app)
                .should_render_results_in_reverse(app)
            {
                reverse_index(selected_idx, result_count)
            } else {
                selected_idx
            },
        );
    }

    fn select_next(&mut self, ctx: &mut ViewContext<Self>) {
        let result_count = self.result_renderers.len();
        if result_count == 0 {
            return;
        }
        let start = match self.selected_idx {
            Some(idx) if idx < result_count.saturating_sub(1) => idx + 1,
            Some(_) | None => 0,
        };
        if let Some(idx) = self.next_enabled_idx(start, ScanDirection::Forward) {
            self.select_idx(idx, ctx);
        }
    }

    fn select_prev(&mut self, ctx: &mut ViewContext<Self>) {
        let result_count = self.result_renderers.len();
        if result_count == 0 {
            return;
        }
        let start = match self.selected_idx {
            Some(idx) if idx > 0 => idx - 1,
            Some(_) | None => result_count - 1,
        };
        if let Some(idx) = self.next_enabled_idx(start, ScanDirection::Backward) {
            self.select_idx(idx, ctx);
        }
    }

    /// Starting from `start`, scan for the nearest non-disabled item in the given direction.
    /// Wraps around at most once. Returns `None` if every item is disabled.
    fn next_enabled_idx(&self, start: usize, direction: ScanDirection) -> Option<usize> {
        let count = self.result_renderers.len();
        for offset in 0..count {
            let candidate = match direction {
                ScanDirection::Forward => (start + offset) % count,
                ScanDirection::Backward => (start + count - offset) % count,
            };
            if !self.result_renderers[candidate].search_result.is_disabled() {
                return Some(candidate);
            }
        }
        None
    }

    pub fn set_active_tab(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.model
            .update(ctx, |m, ctx| m.set_active_tab_index(index, ctx));
        ctx.emit(InlineMenuEvent::TabChanged);
        ctx.notify();
    }

    pub fn select_next_tab(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let num_tabs = self.model.as_ref(ctx).tab_configs().len();
        if num_tabs <= 1 {
            return false;
        }

        let current = self.model.as_ref(ctx).active_tab_index();
        let next = (current + 1) % num_tabs;
        self.set_active_tab(next, ctx);
        true
    }

    fn render_header(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !FeatureFlag::InlineMenuHeaders.is_enabled() {
            return None;
        }

        let model = self.model.as_ref(app);
        let tab_configs = model.tab_configs();

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let header_bg: ColorU = Fill::from(inline_styles::menu_background_color(app))
            .blend(&theme.surface_overlay_1())
            .into();

        let has_tabs = tab_configs.len() > 1;
        let label_margin_right = if has_tabs { 16. } else { 0. };

        let mut left_section = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    Text::new_inline(
                        self.header_config.label.to_uppercase(),
                        appearance.monospace_font_family(),
                        12.,
                    )
                    .with_color(
                        inline_styles::primary_text_color(theme, header_bg.into()).into_solid(),
                    )
                    .finish(),
                )
                .with_margin_right(label_margin_right)
                .finish(),
            );

        if has_tabs {
            let tab_button_styles = UiComponentStyles {
                font_size: Some(12.),
                font_weight: Some(Weight::Semibold),
                padding: Some(Coords {
                    top: 4.,
                    bottom: 4.,
                    left: 8.,
                    right: 8.,
                }),
                ..Default::default()
            };

            let active_tab_index = model.active_tab_index();
            let mut tab_row = Flex::row();
            for (idx, tab_config) in tab_configs.iter().enumerate() {
                let is_active = idx == active_tab_index;
                let Some(mouse_state) = model.tab_mouse_states().get(idx).cloned() else {
                    continue;
                };

                let mut button = appearance
                    .ui_builder()
                    .button(ButtonVariant::Text, mouse_state)
                    .with_text_label(tab_config.label.clone())
                    .with_style(tab_button_styles);

                if is_active {
                    button = button.active().with_active_styles(UiComponentStyles {
                        font_color: Some(theme.main_text_color(theme.background()).into_solid()),
                        ..Default::default()
                    });
                }

                let button_element = button
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(InlineMenuRowAction::<A>::SelectTab {
                            index: idx,
                        });
                    })
                    .finish();

                let is_first_tab = idx == 0;
                let mut tab_container = Container::new(button_element).with_border(
                    Border::new(1.)
                        .with_sides(false, is_first_tab, false, true)
                        .with_border_fill(theme.outline()),
                );
                if is_active {
                    tab_container = tab_container
                        .with_background(theme.background())
                        .with_overdraw_bottom(2.);
                }

                tab_row.add_child(tab_container.finish());
            }

            left_section.add_child(tab_row.finish());
        }

        let left_section = left_section.finish();

        let right_section: Box<dyn Element> = match &self.header_config.trailing_element {
            Some(render_fn) => render_fn(app),
            None => Empty::new().finish(),
        };

        let drag_indicator = Hoverable::new(self.drag_indicator_mouse_state.clone(), |_| {
            ConstrainedBox::new(
                Icon::DragIndicator
                    .to_warpui_icon(Fill::Solid(
                        theme.disabled_text_color(header_bg.into()).into_solid(),
                    ))
                    .finish(),
            )
            .with_height(16.)
            .with_width(16.)
            .finish()
        })
        .with_cursor(Cursor::ResizeUpDown)
        .finish();

        let header_row = ConstrainedBox::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(left_section)
                .with_child(drag_indicator)
                .with_child(right_section)
                .finish(),
        )
        .with_height(inline_styles::HEADER_ROW_HEIGHT)
        .finish();

        let header = Container::new(header_row)
            .with_padding_left(*terminal::view::PADDING_LEFT)
            .with_padding_right(8.)
            .with_background(theme.surface_overlay_1())
            .with_border(
                Border::new(inline_styles::HEADER_BORDER)
                    .with_sides(true, false, true, false)
                    .with_border_fill(theme.outline()),
            )
            .finish();

        // Clip so trailing controls don't paint past the pane in a narrow split pane.
        Some(Clipped::new(header).finish())
    }

    pub fn render_results_only(
        &self,
        should_render_results_in_reverse: bool,
        horizontal_padding: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();

        let selected_idx = self.selected_idx;
        let weak_handle = self.weak_handle.clone();

        let scrollable_items = Scrollable::vertical(
            self.state_handles.scroll_state.clone(),
            UniformList::new(
                self.state_handles.uniform_list.clone(),
                self.result_renderers.len(),
                move |range, app| {
                    let handle = weak_handle.upgrade(app).expect("Handle is upgradeable");
                    let me = handle.as_ref(app);

                    let result_count = me.result_renderers.len();
                    range
                        .filter_map(|idx| {
                            let logical_idx = if should_render_results_in_reverse {
                                reverse_index(idx, result_count)
                            } else {
                                idx
                            };
                            let result_renderer = me.result_renderers.get(logical_idx)?;
                            Some(
                                SavePosition::new(
                                    EventHandler::new(result_renderer.render_inline(
                                        logical_idx,
                                        selected_idx == Some(logical_idx),
                                        app,
                                    ))
                                    .on_mouse_in(
                                        move |ctx, _, _| {
                                            ctx.dispatch_typed_action(
                                                InlineMenuRowAction::<A>::HoverItem {
                                                    result_index: logical_idx,
                                                },
                                            );
                                            DispatchEventResult::PropagateToParent
                                        },
                                        Some(MouseInBehavior {
                                            fire_on_synthetic_events: false,
                                            fire_when_covered: true,
                                        }),
                                    )
                                    .finish(),
                                    result_renderer.position_id.as_str(),
                                )
                                .finish(),
                            )
                        })
                        .collect_vec()
                        .into_iter()
                },
            )
            .finish_scrollable(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        Container::new(scrollable_items)
            .with_vertical_padding(inline_styles::CONTENT_VERTICAL_PADDING)
            .with_horizontal_padding(horizontal_padding)
            .finish()
    }

    fn render_results_list(&self, app: &AppContext) -> Box<dyn Element> {
        let should_reverse = self
            .positioner
            .as_ref(app)
            .should_render_results_in_reverse(app);
        let horizontal_padding = if self.compact_layout {
            0.
        } else {
            *terminal::view::PADDING_LEFT - QUERY_RESULT_RENDERER_STYLES.result_horizontal_padding
        };
        let results = self.render_results_only(should_reverse, horizontal_padding, app);

        if let Some(banner) = self.banner_fn.as_ref().and_then(|f| f(app)) {
            Flex::column()
                .with_child(banner)
                .with_child(Expanded::new(1., results).finish())
                .finish()
        } else {
            results
        }
    }

    fn details_display_idx(&self) -> Option<usize> {
        if self.details_pane_target == DetailsPaneTarget::Hover {
            if let Some(idx) = self.hovered_idx {
                return Some(idx);
            }
        }
        self.selected_idx
    }

    fn render_no_results_state(&self, message: String, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        Align::new(
            Text::new(
                message,
                appearance.ui_font_family(),
                inline_styles::font_size(appearance),
            )
            .with_color(theme.disabled_text_color(theme.background()).into_solid())
            .finish(),
        )
        .finish()
    }
}

fn reverse_index(idx: usize, count: usize) -> usize {
    count.saturating_sub(1).saturating_sub(idx)
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> View for InlineMenuView<A, T> {
    fn ui_name() -> &'static str {
        "InlineMenuView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let is_rendering_below_input = self
            .positioner
            .as_ref(app)
            .should_render_inline_menu_below_input();

        let content: Box<dyn Element>;
        if self.result_renderers.is_empty() {
            content = if self.mixer.as_ref(app).is_loading() {
                self.render_no_results_state("Loading...".into(), app)
            } else {
                self.render_no_results_state("No results".into(), app)
            };
        } else {
            let results_list = self.render_results_list(app);

            if let Some((details_config, rendered_details)) = A::details_render_config(app).zip(
                self.details_display_idx()
                    .and_then(|idx| self.result_renderers.get(idx))
                    .and_then(|renderer| renderer.search_result.render_details(app)),
            ) {
                let mut split_view =
                    Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

                let aligned_results = if is_rendering_below_input {
                    results_list
                } else {
                    Align::new(results_list).bottom_center().finish()
                };
                if let Some(max_result_width) = details_config.max_result_width {
                    split_view.add_child(
                        ConstrainedBox::new(aligned_results)
                            .with_max_width(max_result_width)
                            .finish(),
                    );
                } else {
                    split_view.add_child(Expanded::new(1., aligned_results).finish());
                }

                const DETAILS_MARGIN_LEFT: f32 = 20.;
                let laid_out_details = Container::new(
                    Container::new(Align::new(rendered_details).bottom_left().finish())
                        .with_horizontal_margin(styles::DETAILS_PANE_PADDING)
                        .with_vertical_margin(styles::DETAILS_PANE_PADDING)
                        .with_margin_left(DETAILS_MARGIN_LEFT)
                        .finish(),
                )
                .with_border(Border::left(1.0).with_border_fill(theme.outline()))
                .finish();

                if let Some(min_required_width) = details_config.min_required_details_width {
                    let min_details_total_width = min_required_width
                        + DETAILS_MARGIN_LEFT
                        + styles::DETAILS_PANE_PADDING * 2.;

                    if let Some(max_result_width) = details_config.max_result_width {
                        // When max_result_width is set, the results are a
                        // non-flex child that won't expand when details are
                        // hidden. Use an outer SizeConstraintSwitch to swap in
                        // a full-width results list when the container is too
                        // narrow for the details panel.
                        split_view.add_child(Expanded::new(1., laid_out_details).finish());

                        let narrow_results = self.render_results_list(app);
                        let narrow_aligned = if is_rendering_below_input {
                            narrow_results
                        } else {
                            Align::new(narrow_results).bottom_center().finish()
                        };
                        content = SizeConstraintSwitch::new(
                            split_view.finish(),
                            vec![(
                                SizeConstraintCondition::WidthLessThan(
                                    max_result_width + min_details_total_width,
                                ),
                                narrow_aligned,
                            )],
                        )
                        .finish();
                    } else {
                        // Without max_result_width, results are Expanded and
                        // will naturally reclaim space. Use the inner
                        // SizeConstraintSwitch to hide the details panel when
                        // squeezed.
                        split_view.add_child(
                            Expanded::new(
                                1.,
                                SizeConstraintSwitch::new(
                                    laid_out_details,
                                    vec![(
                                        SizeConstraintCondition::WidthLessThan(
                                            min_details_total_width,
                                        ),
                                        Empty::new().finish(),
                                    )],
                                )
                                .finish(),
                            )
                            .finish(),
                        );
                        content = split_view.finish();
                    }
                } else {
                    split_view.add_child(Expanded::new(1., laid_out_details).finish());
                    content = split_view.finish();
                }
            } else {
                content = results_list;
            }
        }

        if self.compact_layout {
            return Clipped::new(content).finish();
        }

        let aligned_content = if is_rendering_below_input {
            content
        } else {
            Align::new(content).bottom_center().finish()
        };

        let header = self.render_header(app);
        let has_header = header.is_some();

        let height = self.positioner.as_ref(app).inline_menu_height(app);
        let menu = ConstrainedBox::new(
            Container::new(aligned_content)
                .with_border(
                    Border::new(inline_styles::CONTENT_BORDER_WIDTH)
                        .with_sides(
                            is_rendering_below_input || !has_header,
                            false,
                            !is_rendering_below_input || !has_header,
                            false,
                        )
                        .with_border_fill(if self.agent_view_controller.as_ref(app).is_active() {
                            input::agent::styles::default_border_color(theme)
                        } else {
                            input::terminal::styles::default_border_color(theme)
                        }),
                )
                .finish(),
        )
        .with_height(height)
        .finish();

        let mut column = Flex::column();

        if is_rendering_below_input {
            column = column.with_reverse_orientation();
        }

        let mut children: Vec<Box<dyn Element>> = Vec::new();
        if let Some(header) = header {
            let on_resize_end: ResizeEndFn = Box::new(|ctx, _| {
                ctx.dispatch_typed_action(InlineMenuRowAction::<A>::ResizeEnd);
            });
            let header = DragResizeElement::new(
                self.resize_handle.clone(),
                header,
                |delta, ctx, _| {
                    ctx.dispatch_typed_action(InlineMenuRowAction::<A>::ResizeUpdate { delta });
                },
                Some(on_resize_end),
            )
            .finish();
            children.push(header);
        }
        children.push(Clipped::new(menu).finish());
        children.push(ChildView::new(&self.message_bar).finish());

        column.with_children(children).finish()
    }
}

enum ScanDirection {
    Forward,
    Backward,
}

/// Internal action for handling item selection via click.
#[derive(Debug, Clone)]
pub enum InlineMenuRowAction<A: Action + Clone> {
    Accept { item: A, cmd_or_ctrl_enter: bool },
    Select { result_index: usize, item: A },
    HoverItem { result_index: usize },
    Dismiss,
    SelectTab { index: usize },
    ResizeUpdate { delta: f32 },
    ResizeEnd,
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> TypedActionView for InlineMenuView<A, T> {
    type Action = InlineMenuRowAction<A>;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            InlineMenuRowAction::Accept {
                item,
                cmd_or_ctrl_enter,
            } => {
                ctx.emit(InlineMenuEvent::AcceptedItem {
                    item: item.clone(),
                    cmd_or_ctrl_shift_enter: *cmd_or_ctrl_enter,
                });
            }
            InlineMenuRowAction::Select { result_index, item } => {
                if *result_index >= self.result_renderers.len() {
                    return;
                }
                self.selected_idx = Some(*result_index);
                self.scroll_to_selected_idx(ctx);
                self.model.update(ctx, |model, ctx| {
                    model.update_selected_item(item.clone(), ctx);
                });
                ctx.emit(InlineMenuEvent::SelectedItem { item: item.clone() });
                ctx.notify();
            }
            InlineMenuRowAction::HoverItem { result_index } => {
                let idx = *result_index;
                if idx < self.result_renderers.len() && self.hovered_idx != Some(idx) {
                    self.hovered_idx = Some(idx);
                    self.details_pane_target = DetailsPaneTarget::Hover;
                    ctx.notify();
                }
            }
            InlineMenuRowAction::Dismiss => {
                ctx.emit(InlineMenuEvent::Dismissed);
            }
            InlineMenuRowAction::SelectTab { index } => {
                let index = *index;
                let previous_index = self.model.as_ref(ctx).active_tab_index();
                if index != previous_index {
                    self.model
                        .update(ctx, |m, ctx| m.set_active_tab_index(index, ctx));
                    ctx.emit(InlineMenuEvent::TabChanged);
                    ctx.notify();
                }
            }
            InlineMenuRowAction::ResizeUpdate { delta } => {
                let height_change = self.positioner.update(ctx, |p, ctx| {
                    p.apply_resize_delta(A::MENU_TYPE, *delta, ctx)
                });

                if let Some(height_change) = height_change {
                    // When the menu is above the input, content is bottom-aligned and the
                    // container grows/shrinks from the top (where the header is). We must
                    // offset scroll so that items stay visually still.
                    // When the menu is below the input, content is top-aligned and the
                    // container grows/shrinks from the bottom, so items are anchored by default.
                    if !self
                        .positioner
                        .as_ref(ctx)
                        .should_render_inline_menu_below_input()
                    {
                        let item_height = (QUERY_RESULT_RENDERER_STYLES.result_item_height_fn)(
                            Appearance::as_ref(ctx),
                        );
                        let scroll_delta = -(height_change / item_height);
                        self.state_handles.uniform_list.add_scroll_top(scroll_delta);
                    }
                }
                ctx.notify();
            }
            InlineMenuRowAction::ResizeEnd => {
                self.positioner
                    .update(ctx, |p, ctx| p.persist_custom_content_heights(ctx));
            }
        }
    }
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> Entity for InlineMenuView<A, T> {
    type Event = InlineMenuEvent<A>;
}

mod styles {
    pub const DETAILS_PANE_PADDING: f32 = 8.;
}
