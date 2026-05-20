use sharing::SharedPaneContent;
use std::fmt::Debug;

use crate::{
    appearance::Appearance,
    menu::{Menu, MenuItem},
    pane_group::{
        focus_state::{PaneFocusHandle, PaneGroupFocusEvent},
        pane::{
            view::StandardHeader, ActionOrigin, PaneConfiguration, PaneConfigurationEvent,
            PaneStack, PaneStackEvent, ToolbeltButton,
        },
        BackingView, Direction, PaneDragDropLocation, PaneId, TabBarHoverIndex,
    },
    send_telemetry_from_ctx,
    server::telemetry::{SharingDialogSource, TelemetryEvent},
    settings::CodeSettings,
    tab::tab_position_id,
    terminal::view::TerminalAction,
    view_components::{FeaturePopup, NewFeaturePopupEvent, NewFeaturePopupLabel},
    workspace::{TabBarLocation, VerticalTabsPaneDropTargetData},
};

use crate::workspace::TabBarDropTargetData;

use super::header_content::{HeaderContent, HeaderRenderContext, StandardHeaderOptions};

use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use warp_core::{features::FeatureFlag, settings::Setting};
use warpui::{
    elements::{
        AcceptedByDropTarget, Align, Border, ChildAnchor, Clipped, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Dismiss, Draggable, DraggableState, Empty, Flex,
        Hoverable, Icon, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
        ParentAnchor, ParentElement, ParentOffsetBounds, PositionedElementAnchor,
        PositionedElementOffsetBounds, Radius, SavePosition, Shrinkable, Stack, Text,
    },
    presenter::ChildView,
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::PaneDropTargetData;

mod sharing;

pub(crate) mod components;

pub(crate) const PANE_HEADER_HEIGHT: f32 = 34.;
const DRAG_SPLIT_THRESHOLD: f32 = 0.18;

pub trait ActionPayload: Debug + Send + Sync + Clone + 'static {}
impl<T: Debug + Send + Sync + Clone + 'static> ActionPayload for T {}

pub enum Event<A: ActionPayload, B: ActionPayload> {
    /// An item in the header's overflow menu was selected.
    SelectedOverflowMenuAction(A),
    /// Since pane headers are generic, we allow consumers to render custom elements within it. Sometimes,
    /// consumers will need to dispatch actions from such elements. This action (which is publicly exposed)
    /// allows consumers to dispatch custom actions of type B which will eventually be handled by
    /// `BackingView::handle_custom_action`.
    CustomAction(B),
    /// The close button on the header was clicked.
    Close,
    /// This header has been dragged over the pane with target_id at the location
    MovePaneWithinPaneGroup {
        target_id: PaneId,
        direction: Direction,
    },
    /// A pane or file tab was dragged over the workspace tab bar.
    DraggedOverTabBar {
        origin: ActionOrigin,
        tab_hover_index: TabBarHoverIndex,
        hidden_pane_preview_direction: Direction,
    },
    /// The pane header was dragged over some part of the terminal that is not the pane group
    /// or tab bar
    PaneDraggedOutsideTabBarOrPaneGroup,
    /// This header was dropped, signaling the end of the move
    PaneDroppedWithinPaneGroup,
    /// A pane or file tab was dropped on the workspace tab bar.
    DroppedOnTabBar {
        origin: ActionOrigin,
    },
    // This header was dropped on a place outside of the pane group or tab bar
    PaneDroppedOutsideofTabBarOrPaneGroup,
    // This header was clicked and the pane should be focused
    PaneHeaderClicked,
    /// This header's overflow menu was toggled,
    /// bool is passed to indicate if menu is open
    PaneHeaderOverflowMenuToggled(bool),
    /// One of the pane header's overlay elements was closed.
    OverlayClosed,
}

#[derive(Clone, Debug)]
pub enum PaneHeaderAction<A: ActionPayload, B: ActionPayload> {
    OverflowMenuAction(A),
    CustomAction(B),
    OpenOverflowMenu,
    ShareContents,
    Close,
    PaneHeaderDragStarted,
    PaneHeaderDragged {
        origin: ActionOrigin,
        drag_location: PaneDragDropLocation,
        drag_position: RectF,
        /// Precomputed by drop targets that already know the exact hover state,
        /// such as vertical tabs. When absent, the hover index is derived from
        /// the drag geometry and tab bar location.
        precomputed_tab_hover_index: Option<TabBarHoverIndex>,
    },
    PaneHeaderDropped {
        origin: ActionOrigin,
        drop_location: PaneDragDropLocation, // Represents what kind of drop target the pane was dropped over
    },
    PaneHeaderClicked,
}

impl<P: BackingView> Entity for PaneHeader<P> {
    type Event = Event<P::PaneHeaderOverflowMenuAction, P::CustomAction>;
}

pub struct PaneHeader<P: BackingView> {
    /// The pane stack containing the backing views. The header renders the active view.
    pane_stack: ModelHandle<PaneStack<P>>,
    focus_handle: Option<PaneFocusHandle>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    mouse_state_handles: MouseStateHandles,
    overflow_menu:
        ViewHandle<Menu<PaneHeaderAction<P::PaneHeaderOverflowMenuAction, P::CustomAction>>>,
    toolbelt_buttons: Vec<ToolbeltButton>,
    shared_content: SharedPaneContent,
    open_overlay: OpenOverlay,
    is_visible_in_pane_group: bool, // If this pane header is being dragged along the tab bar, then it is not visible in the pane group
    toolbelt_feature_popup: ViewHandle<FeaturePopup>,
}

impl<P: BackingView> PaneHeader<P> {
    pub fn new(
        pane_stack: ModelHandle<PaneStack<P>>,
        pane_configuration: ModelHandle<PaneConfiguration>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let overflow_menu = ctx.add_typed_action_view(|_| Menu::new());
        ctx.subscribe_to_view(&overflow_menu, move |me, _, event, ctx| {
            me.handle_overflow_menu_action(event, ctx);
        });

        let shared_content = SharedPaneContent::new(ctx);

        let toolbelt_feature_popup = ctx.add_view(|_| {
            FeaturePopup::new_feature(NewFeaturePopupLabel::FromString(
                "Open files and review code diffs".to_string(),
            ))
        });
        ctx.subscribe_to_view(&toolbelt_feature_popup, move |me, _, event, ctx| {
            me.handle_toolbelt_feature_popup_event(event, ctx);
        });

        ctx.subscribe_to_model(&pane_configuration, Self::handle_pane_state_event);
        ctx.subscribe_to_model(&pane_stack, Self::handle_pane_stack_event);

        Self {
            pane_stack,
            pane_configuration,
            focus_handle: None,
            mouse_state_handles: Default::default(),
            overflow_menu,
            shared_content,
            open_overlay: Default::default(),
            toolbelt_buttons: Default::default(),
            is_visible_in_pane_group: true,
            toolbelt_feature_popup,
        }
    }

    fn handle_pane_stack_event(
        &mut self,
        _: ModelHandle<PaneStack<P>>,
        event: &PaneStackEvent<P>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Re-render when the active view changes
        if matches!(
            event,
            PaneStackEvent::ViewAdded(_) | PaneStackEvent::ViewRemoved(_)
        ) {
            ctx.notify();
        }
    }

    pub(super) fn set_focus_handle(
        &mut self,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_model(
            focus_handle.focus_state_handle(),
            |_me, _, event, ctx| match event {
                PaneGroupFocusEvent::InSplitPaneChanged
                | PaneGroupFocusEvent::FocusedPaneMaximizedChanged
                | PaneGroupFocusEvent::FocusChanged { .. } => ctx.notify(),
                PaneGroupFocusEvent::ActiveSessionChanged { .. } => {}
            },
        );
        self.focus_handle = Some(focus_handle);
        ctx.notify();
    }

    fn handle_pane_state_event(
        &mut self,
        _: ModelHandle<PaneConfiguration>,
        event: &PaneConfigurationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Only re-render on HeaderContentChanged. All PaneConfiguration methods that affect
        // header rendering now emit HeaderContentChanged in addition to their specific events.
        if matches!(event, PaneConfigurationEvent::HeaderContentChanged) {
            ctx.notify();
        }
    }

    fn handle_overflow_menu_action(
        &mut self,
        event: &crate::menu::Event,
        ctx: &mut ViewContext<Self>,
    ) {
        if let crate::menu::Event::Close { via_select_item: _ } = event {
            self.close_overlay(ctx);
            self.overflow_menu.update(ctx, |menu, ctx| {
                menu.reset_selection(ctx);
            });
            ctx.emit(Event::PaneHeaderOverflowMenuToggled(false));
            ctx.notify();
        }
    }

    fn handle_toolbelt_feature_popup_event(
        &mut self,
        event: &NewFeaturePopupEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            NewFeaturePopupEvent::Dismissed => {
                // Update the setting to mark the popup as dismissed
                CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .dismissed_code_toolbelt_new_feature_popup
                        .set_value(true, ctx);
                });
                ctx.notify();
            }
        }
    }

    /// Close the open overlay menu.
    fn close_overlay(&mut self, ctx: &mut ViewContext<Self>) {
        self.open_overlay = OpenOverlay::None;
        ctx.emit(Event::OverlayClosed);
        ctx.notify();
    }

    #[cfg(feature = "integration_tests")]
    pub fn is_overlay_open(&self) -> bool {
        !matches!(self.open_overlay, OpenOverlay::None)
    }

    pub fn set_overflow_menu_items(
        &mut self,
        items: impl IntoIterator<Item = MenuItem<P::PaneHeaderOverflowMenuAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Convert the menu items to be [`PaneHeaderAction`]s.
        self.overflow_menu.update(ctx, |menu, ctx| {
            menu.set_items(
                items.into_iter().map(|item| match item {
                    MenuItem::Separator => MenuItem::Separator,
                    MenuItem::Item(item) => {
                        let on_select_action = item
                            .on_select_action()
                            .map(|a| PaneHeaderAction::OverflowMenuAction(a.to_owned()));
                        MenuItem::Item(item.with_different_on_select_action_type(on_select_action))
                    }
                    MenuItem::ItemsRow { items } => {
                        let items = items
                            .into_iter()
                            .map(|item| {
                                let on_select_action = item
                                    .on_select_action()
                                    .map(|a| PaneHeaderAction::OverflowMenuAction(a.to_owned()));
                                item.with_different_on_select_action_type(on_select_action)
                            })
                            .collect();
                        MenuItem::ItemsRow { items }
                    }
                    MenuItem::Submenu { fields, .. } => {
                        panic!(
                            "Submenus are not supported in the pane header overflow menu: {fields:?}"
                        );
                    }
                    MenuItem::Header { fields, .. } => {
                        panic!(
                            "Headers are not supported in the pane header overflow menu: {fields:?}"
                        );
                    }
                }),
                ctx,
            );
        });
        ctx.notify();
    }

    pub fn set_toolbelt_buttons(
        &mut self,
        buttons: Vec<ToolbeltButton>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.toolbelt_buttons = buttons;
        ctx.notify();
    }

    pub fn is_visible_in_pane_group(&self) -> bool {
        self.is_visible_in_pane_group
    }

    /// Based on the drag position and tab bar location, returns whether or not the given drag
    /// is over a tab, or between two tabs. This is done by splitting the tabs into quadrants and seeing
    /// what quadrant the center of the dragged element lives.
    fn calculate_tab_focus_hover_index(
        drag_position: &RectF,
        tab_bar_location: &TabBarLocation,
        ctx: &ViewContext<Self>,
    ) -> TabBarHoverIndex {
        match tab_bar_location {
            TabBarLocation::TabIndex(idx) => {
                if let Some(tab_rect) = ctx.element_position_by_id(tab_position_id(*idx)) {
                    let tab_center_x = tab_rect.center().x();
                    let tab_quarter_x = (tab_center_x + tab_rect.lower_left().x()) / 2.;
                    let tab_three_quarters_x = (tab_center_x + tab_rect.lower_right().x()) / 2.;
                    if drag_position.center().x() < tab_quarter_x {
                        TabBarHoverIndex::BeforeTab(*idx)
                    } else if drag_position.center().x() < tab_three_quarters_x {
                        TabBarHoverIndex::OverTab(*idx)
                    } else {
                        TabBarHoverIndex::BeforeTab(*idx + 1)
                    }
                } else {
                    // If for some reason we can't retrieve the tab position, just default to the index
                    TabBarHoverIndex::OverTab(*idx)
                }
            }
            TabBarLocation::AfterTabIndex(tab_count) => TabBarHoverIndex::BeforeTab(*tab_count),
        }
    }
}

#[derive(Default)]
struct MouseStateHandles {
    close_button_handle: MouseStateHandle,
    overflow_button_handle: MouseStateHandle,
    draggable_state: DraggableState,
    header_click_handle: MouseStateHandle,
    header_hover_handle: MouseStateHandle,
}

#[derive(Default, Debug, PartialEq, Eq)]
enum OpenOverlay {
    OverflowMenu,
    SharingDialog,
    #[default]
    None,
}

impl<P: BackingView> PaneHeader<P> {
    fn overflow_button_position_id(&self) -> String {
        format!(
            "pane_header_overflow_button:{}",
            self.pane_configuration.id()
        )
    }

    fn render_toolbelt_buttons(&self, app: &AppContext) -> Box<dyn Element> {
        let mut flex = Flex::row();
        for toolbelt_button in &self.toolbelt_buttons {
            flex.add_child(
                SavePosition::new(
                    ChildView::new(&toolbelt_button.action_button).finish(),
                    &toolbelt_button_position_id(
                        &self.pane_configuration,
                        toolbelt_button.action_button.id(),
                    ),
                )
                .finish(),
            );
        }
        let container = Container::new(flex.finish()).with_margin_left(2.).finish();

        // Create Stack with the container as the first child
        let mut stack = Stack::new().with_child(container);

        // Check if tooltip has been dismissed already.
        // We should only trigger this if we are in a git repository,
        // but the pane header will only render if we are already in one.
        let auth_state = crate::auth::AuthStateProvider::as_ref(app).get();
        let should_show_tooltip = FeatureFlag::CodeLaunchModal.is_enabled()
            && !auth_state.is_onboarded().unwrap_or_default() // We only want to show the tooltip for new users.
            && !*CodeSettings::as_ref(app)
                .dismissed_code_toolbelt_new_feature_popup
                .value()
                // We should not render the tooltip if no code toolbelt buttons are present.
                && !self.toolbelt_buttons.is_empty();

        if should_show_tooltip {
            // Position the FeaturePopup tooltip below the header
            stack.add_positioned_overlay_child(
                Dismiss::new(ChildView::new(&self.toolbelt_feature_popup).finish())
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(
                            PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                                TerminalAction::DismissCodeToolbeltTooltip,
                            ),
                        );
                        ctx.notify();
                    })
                    .finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        stack.finish()
    }
}

impl<P: BackingView> PaneHeader<P> {
    /// Creates the right justified row with controls for header rendering.
    #[allow(clippy::too_many_arguments)]
    fn render_right_justified_row(
        &self,
        should_show_on_header: bool,
        should_display_overflow_menu_button: bool,
        close_button: Box<dyn Element>,
        overflow_menu_button: Box<dyn Element>,
        left_of_overflow: Option<Box<dyn Element>>,
        options: &StandardHeaderOptions,
        app: &AppContext,
    ) -> (Flex, f32) {
        let can_show_close = !options.hide_close_button
            && self
                .focus_handle
                .as_ref()
                .is_some_and(|h| h.is_in_split_pane(app));
        let can_show_overflow = should_display_overflow_menu_button;
        let should_show_close = should_show_on_header && can_show_close;
        let should_show_overflow = should_show_on_header && can_show_overflow;

        let mut required_controls = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        // Add left_of_overflow elements (restore button, sync indicator, etc.) first
        if let Some(left_element) = left_of_overflow {
            required_controls.add_child(left_element);
        }

        if should_show_overflow {
            required_controls.add_child(overflow_menu_button);
        }
        if should_show_close {
            required_controls.add_child(close_button);
        }

        let mut optional_controls = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        if should_show_on_header {
            let appearance = Appearance::as_ref(app);
            self.render_sharing_controls(&mut optional_controls, appearance, None, None, app);
        }

        let optional_controls =
            Shrinkable::new(1., Clipped::new(optional_controls.finish()).finish()).finish();

        let mut right_justified_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        right_justified_row.add_child(optional_controls);
        right_justified_row.add_child(required_controls.finish());

        let required_icons_count = can_show_overflow as u32 + can_show_close as u32;
        let required_width = components::header_edge_min_width(required_icons_count);

        (right_justified_row, required_width)
    }

    /// Adds overlay children to the stack (overflow menu and sharing dialog).
    fn add_overlays_to_stack(
        &self,
        stack: &mut Stack,
        should_display_overflow_menu_button: bool,
        app: &AppContext,
    ) {
        match self.open_overlay {
            OpenOverlay::OverflowMenu => {
                if should_display_overflow_menu_button {
                    stack.add_positioned_overlay_child(
                        ChildView::new(&self.overflow_menu).finish(),
                        OffsetPositioning::offset_from_save_position_element(
                            self.overflow_button_position_id(),
                            vec2f(0., 0.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomRight,
                            ChildAnchor::TopRight,
                        ),
                    );
                }
            }
            OpenOverlay::SharingDialog => {
                if self.is_sharing_dialog_enabled(app) {
                    stack.add_positioned_overlay_child(
                        ChildView::new(self.sharing_dialog()).finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(-8., 0.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::BottomRight,
                            ChildAnchor::TopRight,
                        ),
                    );
                }
            }
            OpenOverlay::None => {}
        }
    }

    fn render_standard_header(&self, header: StandardHeader, app: &AppContext) -> Box<dyn Element> {
        let StandardHeader {
            title,
            title_secondary,
            title_style,
            title_clip_config,
            title_max_width,
            left_of_title,
            right_of_title,
            left_of_overflow,
            options,
        } = header;
        let appearance = Appearance::as_ref(app);
        let header_icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());

        let close_button = components::render_pane_close_button::<
            P::PaneHeaderOverflowMenuAction,
            P::CustomAction,
        >(
            appearance,
            self.mouse_state_handles.close_button_handle.clone(),
            Some(header_icon_color),
            None,
        );

        let overflow_menu_button = components::render_pane_overflow_button::<
            P::PaneHeaderOverflowMenuAction,
            P::CustomAction,
        >(
            appearance,
            self.mouse_state_handles.overflow_button_handle.clone(),
            &self.overflow_button_position_id(),
            Some(header_icon_color),
            None,
        );
        let should_display_overflow_menu_button = !self.overflow_menu.as_ref(app).is_empty();

        let hoverable = Hoverable::new(
            self.mouse_state_handles.header_hover_handle.clone(),
            |hover_state| {
                // Determine if icons should be shown based on hover state and options.
                let should_show_on_header = hover_state.is_hovered()
                    || self.open_overlay != OpenOverlay::None
                    || options.has_open_menu
                    || self.has_shareable_shared_session(app)
                    || options.always_show_icons;

                let (right_justified_row, min_right_width) = self.render_right_justified_row(
                    should_show_on_header,
                    should_display_overflow_menu_button,
                    close_button,
                    overflow_menu_button,
                    left_of_overflow,
                    &options,
                    app,
                );

                // Build the center row with title and optional customization elements.
                let mut center_row = Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max);

                if let Some(left_element) = left_of_title {
                    center_row
                        .add_child(Container::new(left_element).with_margin_right(4.).finish());
                }

                let font_size = appearance.ui_font_size();
                let font_color = appearance
                    .theme()
                    .sub_text_color(appearance.theme().background());

                // Build title row with primary title and optional secondary title.
                let mut title_row = Flex::row();
                let title_text =
                    Text::new_inline(title.clone(), appearance.ui_font_family(), font_size)
                        .with_color(font_color.into())
                        .with_style(title_style.unwrap_or_default())
                        .with_clip(title_clip_config)
                        .finish();
                title_row.add_child(Shrinkable::new(1., title_text).finish());

                if let Some(secondary) = &title_secondary {
                    if !secondary.is_empty() {
                        let secondary_text = Text::new_inline(
                            secondary.clone(),
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(font_color.into())
                        .finish();
                        title_row.add_child(secondary_text);
                    }
                }

                // If a max width is set, constrain the title to that width.
                let title_element = if let Some(max_width) = title_max_width {
                    Shrinkable::new(
                        1.,
                        ConstrainedBox::new(title_row.finish())
                            .with_max_width(max_width)
                            .finish(),
                    )
                    .finish()
                } else {
                    Shrinkable::new(1., title_row.finish()).finish()
                };
                center_row.add_child(title_element);

                if let Some(right_element) = right_of_title {
                    center_row
                        .add_child(Container::new(right_element).with_margin_left(4.).finish());
                }

                // Build the left column with toolbelt buttons.
                let mut left_justified_row = Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min);

                left_justified_row.add_child(self.render_toolbelt_buttons(app));

                let header_left_inset = self.pane_configuration.as_ref(app).header_left_inset;
                let left_justified_container = Container::new(left_justified_row.finish())
                    .with_padding_left(4. + header_left_inset);
                let right_justified_container =
                    Container::new(right_justified_row.finish()).with_padding_right(4.);

                let edge_width = options.control_container_width();
                let left_constrained = ConstrainedBox::new(left_justified_container.finish())
                    .with_min_width(min_right_width)
                    .with_max_width(edge_width)
                    .finish();
                let right_constrained = ConstrainedBox::new(right_justified_container.finish())
                    .with_min_width(min_right_width)
                    .with_max_width(edge_width)
                    .finish();

                // Build the complete 3-column layout.
                let mut row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

                row.add_child(left_constrained);
                // Wrap center_row in Align to vertically center within the stretched space.
                row.add_child(
                    Shrinkable::new(1., Align::new(center_row.finish()).finish()).finish(),
                );
                row.add_child(right_constrained);

                Container::new(
                    Clipped::new(
                        ConstrainedBox::new(row.finish())
                            .with_height(PANE_HEADER_HEIGHT)
                            .finish(),
                    )
                    .finish(),
                )
                .finish()
            },
        )
        .finish();

        hoverable
    }
}

impl<P: BackingView> View for PaneHeader<P> {
    fn ui_name() -> &'static str {
        "PaneHeader"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let has_overflow_items = !self.overflow_menu.as_ref(app).is_empty();

        let header_left_inset = self.pane_configuration.as_ref(app).header_left_inset;
        let header_ctx = HeaderRenderContext {
            draggable_state: self.mouse_state_handles.draggable_state.clone(),
            close_button_mouse_state: self.mouse_state_handles.close_button_handle.clone(),
            overflow_button_mouse_state: self.mouse_state_handles.overflow_button_handle.clone(),
            overflow_button_position_id: self.overflow_button_position_id(),
            has_overflow_items,
            header_left_inset,
            render_sharing_controls_fn: Box::new(|app, icon_color, button_size| {
                if !self.is_sharing_dialog_enabled(app) {
                    return None;
                }

                let appearance = Appearance::as_ref(app);
                let mut row = Flex::row();
                self.render_sharing_controls(&mut row, appearance, icon_color, button_size, app);
                Some(row.finish())
            }),
        };
        let header_content = self
            .pane_stack
            .as_ref(app)
            .active_view()
            .as_ref(app)
            .render_header_content(&header_ctx, app);

        let show_active_pane_indicator = self
            .pane_configuration
            .as_ref(app)
            .show_active_pane_indicator;

        let should_wrap_with_draggable = !matches!(
            header_content,
            HeaderContent::Custom {
                has_custom_draggable_behavior: true,
                ..
            }
        );
        let element = match header_content {
            HeaderContent::Standard(mut header) => {
                // On mobile devices, always show icons since hover effects don't work with touch
                if warpui::platform::is_mobile_device() {
                    header.options.always_show_icons = true;
                }
                self.render_standard_header(header, app)
            }
            HeaderContent::Custom { element, .. } => Clipped::new(
                ConstrainedBox::new(element)
                    .with_height(PANE_HEADER_HEIGHT)
                    .finish(),
            )
            .finish(),
        };

        let element = if self.pane_configuration.as_ref(app).has_open_modal {
            Container::new(element)
                .with_foreground_overlay(appearance.theme().inactive_pane_overlay())
                .finish()
        } else {
            element
        };

        let mut stack = Stack::new().with_child(element);

        // Always add overlays — they only render when open_overlay != None,
        // which requires a button click to trigger.
        self.add_overlays_to_stack(&mut stack, has_overflow_items, app);

        if show_active_pane_indicator {
            add_active_pane_indicator_to_stack(&mut stack, appearance);
        }

        let clickable_stack = Hoverable::new(
            self.mouse_state_handles.header_click_handle.clone(),
            move |_| stack.finish(),
        )
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(
                PaneHeaderAction::<P::PaneHeaderOverflowMenuAction, P::CustomAction>::PaneHeaderClicked,
            )
        })
        .finish();

        if should_wrap_with_draggable {
            render_pane_header_draggable::<P>(
                self.pane_configuration.clone(),
                clickable_stack,
                self.mouse_state_handles.draggable_state.clone(),
                app,
            )
        } else {
            clickable_stack
        }
    }
}

/// Based on the drag position and target pane, calculates which direction the pane should move.
///
/// We determine the split by dividing the pane into four quadrants, each referring to a split direction:
/// +--------+
/// |\ up   /|
/// | \    / |
/// |  \  /  |
/// | L \/ R |
/// |   /\   |
/// |  /  \  |
/// | /    \ |
/// |/ down \|
/// +--------+
///
/// We calculate which quadrant the drag lies in by normalizing the drag vector relactive to the pane's
/// center, width, and height, and then comparing those values to determine where the point lies. You can think this
/// by essentially checking is the point "more right or left" or "more up and down" and then splitting based on if the value
/// is positive or negative.
///
/// The only caveat here is that the drag position needs to be greater than a given threshold to trigger a drag,
/// otherwise this will result in a no-op. This ensures that the split is not too sensitive.
fn calculate_pane_move_direction(target_pane: RectF, drag_position: RectF) -> Option<Direction> {
    let moved_drag_center = drag_position.center() - target_pane.center();
    let normalized_drag_center = Vector2F::new(
        moved_drag_center.x() / target_pane.width(),
        moved_drag_center.y() / target_pane.height(),
    );

    if normalized_drag_center
        .y()
        .abs()
        .max(normalized_drag_center.x().abs())
        < DRAG_SPLIT_THRESHOLD
    {
        return None;
    }

    if normalized_drag_center.y().abs() > normalized_drag_center.x().abs() {
        if normalized_drag_center.y() > 0. {
            Some(Direction::Down)
        } else {
            Some(Direction::Up)
        }
    } else if normalized_drag_center.x() > 0. {
        Some(Direction::Right)
    } else {
        Some(Direction::Left)
    }
}

impl<P: BackingView> TypedActionView for PaneHeader<P> {
    type Action = PaneHeaderAction<P::PaneHeaderOverflowMenuAction, P::CustomAction>;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PaneHeaderAction::Close => ctx.emit(Event::Close),
            PaneHeaderAction::OverflowMenuAction(action) => {
                ctx.emit(Event::SelectedOverflowMenuAction(action.to_owned()));
            }
            PaneHeaderAction::CustomAction(action) => {
                ctx.emit(Event::CustomAction(action.to_owned()))
            }
            PaneHeaderAction::OpenOverflowMenu => {
                self.open_overlay = OpenOverlay::OverflowMenu;
                ctx.emit(Event::PaneHeaderOverflowMenuToggled(true));
                ctx.notify();
            }
            PaneHeaderAction::ShareContents => {
                self.share_pane_contents(SharingDialogSource::PaneHeader, ctx)
            }
            PaneHeaderAction::PaneHeaderDragStarted => {
                send_telemetry_from_ctx!(TelemetryEvent::PaneDragInitiated, ctx);
            }
            PaneHeaderAction::PaneHeaderDragged {
                origin,
                drag_location,
                drag_position,
                precomputed_tab_hover_index,
            } => match drag_location {
                PaneDragDropLocation::TabBar(tab_bar_location) => {
                    if matches!(origin, ActionOrigin::Pane) {
                        self.is_visible_in_pane_group = false;
                    }
                    ctx.emit(Event::DraggedOverTabBar {
                        origin: *origin,
                        tab_hover_index: precomputed_tab_hover_index.unwrap_or_else(|| {
                            Self::calculate_tab_focus_hover_index(
                                drag_position,
                                tab_bar_location,
                                ctx,
                            )
                        }),
                        hidden_pane_preview_direction: if precomputed_tab_hover_index.is_some() {
                            Direction::Up
                        } else {
                            Direction::Left
                        },
                    });
                }
                PaneDragDropLocation::PaneGroup(target_id) => {
                    self.is_visible_in_pane_group = true;
                    if let Some(target_pane) = ctx.element_position_by_id(target_id.position_id()) {
                        if let Some(direction) =
                            calculate_pane_move_direction(target_pane, *drag_position)
                        {
                            ctx.emit(Event::MovePaneWithinPaneGroup {
                                target_id: *target_id,
                                direction,
                            });
                        }
                    } else {
                        log::error!(
                            "Attempting to move to pane that does not exist with id: {target_id:?}"
                        );
                    }
                }
                PaneDragDropLocation::Other => {
                    self.is_visible_in_pane_group = true;
                    ctx.emit(Event::PaneDraggedOutsideTabBarOrPaneGroup)
                }
            },
            PaneHeaderAction::PaneHeaderDropped {
                origin,
                drop_location,
            } => {
                match drop_location {
                    PaneDragDropLocation::TabBar(_) => {
                        self.is_visible_in_pane_group = true;
                        ctx.emit(Event::DroppedOnTabBar { origin: *origin })
                    }
                    PaneDragDropLocation::PaneGroup(_) => {
                        ctx.emit(Event::PaneDroppedWithinPaneGroup)
                    }
                    PaneDragDropLocation::Other => {
                        ctx.emit(Event::PaneDroppedOutsideofTabBarOrPaneGroup)
                    }
                }
                send_telemetry_from_ctx!(
                    TelemetryEvent::PaneDropped {
                        drop_location: *drop_location
                    },
                    ctx
                );
            }
            PaneHeaderAction::PaneHeaderClicked => ctx.emit(Event::PaneHeaderClicked),
        }
    }
}

/// Adds the active pane indicator to the stack.
fn add_active_pane_indicator_to_stack(stack: &mut Stack, appearance: &Appearance) {
    let indicator = Icon::new(
        "bundled/svg/upper-left-triangle.svg",
        appearance.theme().accent(),
    )
    .finish();
    let child = ConstrainedBox::new(indicator)
        .with_height(16.)
        .with_width(16.)
        .finish();
    stack.add_positioned_child(
        child,
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::ParentBySize,
            ParentAnchor::TopLeft,
            ChildAnchor::TopLeft,
        ),
    );
}

pub fn toolbelt_button_position_id(
    pane_configuration: &ModelHandle<PaneConfiguration>,
    button_view_id: EntityId,
) -> String {
    format!(
        "pane_header_toolbelt_button:{}:{}",
        pane_configuration.id(),
        button_view_id,
    )
}

/// Wraps the given element in a Draggable that implements drag/drop pane behavior to change pane
/// layout.
pub fn render_pane_header_draggable<P: BackingView>(
    pane_configuration: ModelHandle<PaneConfiguration>,
    element: Box<dyn Element>,
    draggable_state: DraggableState,
    app: &AppContext,
) -> Box<dyn Element> {
    Draggable::new(draggable_state, element)
        .with_drag_bounds_callback(|_, window_size| Some(RectF::new(Vector2F::zero(), window_size)))
        .with_accepted_by_drop_target_fn(move |drop_target_data, _| {
            // Panes can only be dragged into other panes (to rearrange) or onto tab targets
            // (to promote to a new tab or move to an existing tab).
            if drop_target_data.as_any().is::<PaneDropTargetData>()
                || drop_target_data.as_any().is::<TabBarDropTargetData>()
                || drop_target_data
                    .as_any()
                    .is::<VerticalTabsPaneDropTargetData>()
            {
                AcceptedByDropTarget::Yes
            } else {
                AcceptedByDropTarget::No
            }
        })
        .on_drag_start(move |ctx, _, _| {
            ctx.dispatch_typed_action(PaneHeaderAction::<
                P::PaneHeaderOverflowMenuAction,
                P::CustomAction,
            >::PaneHeaderDragStarted);
        })
        .on_drag(move |ctx, _, drag_position, data| {
            if let Some(pane_drop_data) =
                data.and_then(|data| data.as_any().downcast_ref::<PaneDropTargetData>())
            {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDragged {
                    origin: ActionOrigin::Pane,
                    drag_location: PaneDragDropLocation::PaneGroup(pane_drop_data.id),
                    drag_position,
                    precomputed_tab_hover_index: None,
                });
            } else if let Some(data) =
                data.and_then(|data| data.as_any().downcast_ref::<TabBarDropTargetData>())
            {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDragged {
                    origin: ActionOrigin::Pane,
                    drag_location: PaneDragDropLocation::TabBar(data.tab_bar_location),
                    drag_position,
                    precomputed_tab_hover_index: None,
                })
            } else if let Some(data) = data.and_then(|data| {
                data.as_any()
                    .downcast_ref::<VerticalTabsPaneDropTargetData>()
            }) {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDragged {
                    origin: ActionOrigin::Pane,
                    drag_location: PaneDragDropLocation::TabBar(data.tab_bar_location),
                    drag_position,
                    precomputed_tab_hover_index: Some(data.tab_hover_index),
                })
            } else {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDragged {
                    origin: ActionOrigin::Pane,
                    drag_location: PaneDragDropLocation::Other,
                    drag_position,
                    precomputed_tab_hover_index: None,
                })
            }
        })
        .on_drop(move |ctx, _, _, data| {
            if let Some(data) =
                data.and_then(|data| data.as_any().downcast_ref::<TabBarDropTargetData>())
            {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDropped {
                    origin: ActionOrigin::Pane,
                    drop_location: PaneDragDropLocation::TabBar(data.tab_bar_location),
                })
            } else if let Some(data) = data.and_then(|data| {
                data.as_any()
                    .downcast_ref::<VerticalTabsPaneDropTargetData>()
            }) {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDropped {
                    origin: ActionOrigin::Pane,
                    drop_location: PaneDragDropLocation::TabBar(data.tab_bar_location),
                })
            } else if let Some(data) =
                data.and_then(|data| data.as_any().downcast_ref::<PaneDropTargetData>())
            {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDropped {
                    origin: ActionOrigin::Pane,
                    drop_location: PaneDragDropLocation::PaneGroup(data.id),
                })
            } else {
                ctx.dispatch_typed_action(PaneHeaderAction::<
                    P::PaneHeaderOverflowMenuAction,
                    P::CustomAction,
                >::PaneHeaderDropped {
                    origin: ActionOrigin::Pane,
                    drop_location: PaneDragDropLocation::Other,
                })
            }
        })
        .with_alternate_drag_element(render_draggable_placeholder_element(
            pane_configuration,
            app,
        ))
        .finish()
}

fn render_draggable_placeholder_element(
    pane_configuration: ModelHandle<PaneConfiguration>,
    app: &AppContext,
) -> Box<dyn Element> {
    let title = pane_configuration.as_ref(app).title().to_owned();
    let title_secondary = pane_configuration.as_ref(app).title_secondary().to_owned();
    let appearance = Appearance::as_ref(app);

    let font_color = appearance
        .theme()
        .main_text_color(appearance.theme().dark_overlay());
    let font_size = appearance.ui_font_size();

    let mut title_row = Flex::row();

    title_row = title_row.with_child(
        Shrinkable::new(
            1.,
            Text::new_inline(title, appearance.ui_font_family(), font_size)
                .with_color(font_color.into())
                .finish(),
        )
        .finish(),
    );
    if !title_secondary.is_empty() {
        title_row = title_row.with_child(
            Text::new_inline(title_secondary, appearance.ui_font_family(), font_size)
                .with_color(font_color.into())
                .finish(),
        );
    }

    let title = Flex::column()
        .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
        .with_child(
            Align::new(
                Container::new(title_row.finish())
                    .with_horizontal_margin(4.)
                    .finish(),
            )
            .bottom_center()
            .finish(),
        )
        .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .finish();

    Container::new(
        ConstrainedBox::new(title)
            .with_height(28.)
            .with_width(200.)
            .finish(),
    )
    .with_uniform_padding(4.)
    .with_border(Border::all(2.).with_border_color(appearance.theme().surface_2().into()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
    .with_background_color(appearance.theme().dark_overlay().into())
    .finish()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
