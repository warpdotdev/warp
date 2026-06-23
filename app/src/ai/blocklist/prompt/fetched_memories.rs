use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use serde_json::{json, Value};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warp_multi_agent_api as api;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Dismiss, DropShadow, Empty, Flex, Hoverable,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
    PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition, ScrollbarWidth,
    Stack, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, EntityId, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::ui_components::blended_colors;

const FETCHED_MEMORIES_BUTTON_SAVE_POSITION_ID: &str = "fetched_memories::chip_button";

const POPUP_WIDTH: f32 = 360.;
const POPUP_MAX_HEIGHT: f32 = 200.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        FetchedMemoriesPopupAction::ClosePopup,
        id!(FetchedMemoriesPopupView::ui_name()),
    )]);
}

fn fetched_memories_for_terminal_view(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Vec<api::message::FetchedMemory> {
    BlocklistAIHistoryModel::as_ref(app)
        .active_conversation(terminal_view_id)
        .map(|conversation| conversation.fetched_memories())
        .unwrap_or_default()
}

fn notify_on_conversation_memory_events(
    event: &BlocklistAIHistoryEvent,
    terminal_view_id: EntityId,
    notify: impl FnOnce(),
) {
    if event
        .terminal_view_id()
        .is_some_and(|id| id != terminal_view_id)
    {
        return;
    }
    match event {
        BlocklistAIHistoryEvent::StartedNewConversation { .. }
        | BlocklistAIHistoryEvent::SetActiveConversation { .. }
        | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
        | BlocklistAIHistoryEvent::AppendedExchange { .. }
        | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. } => notify(),
        _ => (),
    }
}

/// A context chip in the agent input footer showing the memories the server
/// fetched for the active conversation.
pub struct FetchedMemoriesView {
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    terminal_view_id: EntityId,
    chip_mouse_state: MouseStateHandle,
    popup: ViewHandle<FetchedMemoriesPopupView>,
    is_popup_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchedMemoriesAction {
    TogglePopup,
}

impl FetchedMemoriesView {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let popup = ctx
            .add_typed_action_view(|ctx| FetchedMemoriesPopupView::new(terminal_view_id, ctx));
        ctx.subscribe_to_view(&popup, |me, _, event, ctx| match event {
            FetchedMemoriesPopupEvent::Close => {
                me.is_popup_open = false;
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                notify_on_conversation_memory_events(event, me.terminal_view_id, || ctx.notify());
            },
        );

        Self {
            menu_positioning_provider,
            terminal_view_id,
            chip_mouse_state: Default::default(),
            popup,
            is_popup_open: false,
        }
    }

    pub fn should_render(&self, app: &AppContext) -> bool {
        FeatureFlag::FetchedMemoriesChip.is_enabled()
            && !fetched_memories_for_terminal_view(self.terminal_view_id, app).is_empty()
    }

    fn fetched_memories(&self, app: &AppContext) -> Vec<api::message::FetchedMemory> {
        fetched_memories_for_terminal_view(self.terminal_view_id, app)
    }
}

impl Entity for FetchedMemoriesView {
    type Event = ();
}

impl View for FetchedMemoriesView {
    fn ui_name() -> &'static str {
        "FetchedMemoriesView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }
        let memory_count = self.fetched_memories(app).len();

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let base_icon_size = app.font_cache().line_height(
            appearance.monospace_font_size(),
            DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4,
        );
        let text_line_height = app.font_cache().line_height(
            appearance.monospace_font_size() - 1.0,
            appearance.line_height_ratio(),
        );
        let icon_size = (base_icon_size * 1.1).min(text_line_height);

        let memory_icon = Container::new(
            ConstrainedBox::new(
                Icon::Cognition
                    .to_warpui_icon(theme.sub_text_color(blended_colors::neutral_1(theme).into()))
                    .finish(),
            )
            .with_height(icon_size)
            .with_width(icon_size)
            .finish(),
        )
        .finish();

        let chip_font_size = appearance.monospace_font_size() - 1.0;
        let count_text = Text::new_inline(
            format!("{memory_count}"),
            appearance.ui_font_family(),
            chip_font_size,
        )
        .with_color(blended_colors::text_main(theme, theme.surface_1()))
        .with_line_height_ratio(appearance.line_height_ratio())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(memory_icon)
            .with_child(Container::new(count_text).with_margin_left(4.).finish())
            .finish();

        let tooltip_text = format!("{memory_count} memories fetched for this conversation");
        let chip_button = Hoverable::new(self.chip_mouse_state.clone(), move |state| {
            let background = if state.is_hovered() {
                internal_colors::fg_overlay_2(appearance.theme())
            } else {
                internal_colors::fg_overlay_1(appearance.theme())
            };

            let container = Container::new(content)
                .with_background(background)
                .with_padding_left(6.)
                .with_padding_right(6.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(
                    Border::all(1.0)
                        .with_border_fill(internal_colors::neutral_3(appearance.theme())),
                )
                .with_padding_top(2.)
                .with_padding_bottom(2.)
                .finish();

            if state.is_hovered() {
                let mut stack = Stack::new().with_child(container);

                let tooltip_element = appearance
                    .ui_builder()
                    .tool_tip(tooltip_text)
                    .build()
                    .finish();

                stack.add_positioned_overlay_child(
                    tooltip_element,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -8.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
                stack.finish()
            } else {
                container
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(FetchedMemoriesAction::TogglePopup);
        })
        .finish();

        let chip_button =
            SavePosition::new(chip_button, FETCHED_MEMORIES_BUTTON_SAVE_POSITION_ID).finish();

        let mut chip_button = Stack::new().with_child(chip_button);
        if self.is_popup_open {
            let positioning = match self.menu_positioning_provider.menu_position(app) {
                MenuPositioning::BelowInputBox => {
                    OffsetPositioning::offset_from_save_position_element(
                        FETCHED_MEMORIES_BUTTON_SAVE_POSITION_ID,
                        vec2f(0., 4.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::BottomLeft,
                        ChildAnchor::TopLeft,
                    )
                }
                MenuPositioning::AboveInputBox => {
                    OffsetPositioning::offset_from_save_position_element(
                        FETCHED_MEMORIES_BUTTON_SAVE_POSITION_ID,
                        vec2f(0., -4.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    )
                }
            };
            chip_button
                .add_positioned_overlay_child(ChildView::new(&self.popup).finish(), positioning);
        }

        chip_button.finish()
    }
}

impl TypedActionView for FetchedMemoriesView {
    type Action = FetchedMemoriesAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FetchedMemoriesAction::TogglePopup => {
                self.is_popup_open = !self.is_popup_open;
                if self.is_popup_open {
                    let memory_count = self.fetched_memories(ctx).len();
                    send_telemetry_from_ctx!(
                        FetchedMemoriesTelemetryEvent::PopupOpened { memory_count },
                        ctx
                    );
                    ctx.focus(&self.popup);
                }
                ctx.notify();
            }
        }
    }
}

/// Anchored popup listing the fetched memories. Each row links to the memory
/// in the Oz web app.
pub struct FetchedMemoriesPopupView {
    terminal_view_id: EntityId,
    scroll_state: ClippedScrollStateHandle,
    /// Hover state per memory row, persisted across renders. `RefCell` so
    /// `render` can lazily insert handles for newly fetched memories.
    row_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
}

#[derive(Debug, Clone)]
pub enum FetchedMemoriesPopupAction {
    ClosePopup,
    OpenMemory {
        memory_store_id: String,
        memory_id: String,
    },
}

pub enum FetchedMemoriesPopupEvent {
    Close,
}

impl FetchedMemoriesPopupView {
    pub fn new(terminal_view_id: EntityId, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                notify_on_conversation_memory_events(event, me.terminal_view_id, || ctx.notify());
            },
        );
        Self {
            terminal_view_id,
            scroll_state: Default::default(),
            row_mouse_states: RefCell::new(HashMap::new()),
        }
    }

    fn row_mouse_state(&self, memory_id: &str) -> MouseStateHandle {
        self.row_mouse_states
            .borrow_mut()
            .entry(memory_id.to_string())
            .or_default()
            .clone()
    }
}

impl Entity for FetchedMemoriesPopupView {
    type Event = FetchedMemoriesPopupEvent;
}

impl View for FetchedMemoriesPopupView {
    fn ui_name() -> &'static str {
        "FetchedMemoriesPopup"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let memories = fetched_memories_for_terminal_view(self.terminal_view_id, app);
        if memories.is_empty() {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let background = theme.surface_2();
        let main_text_color = blended_colors::text_main(theme, background);
        let sub_text_color = blended_colors::text_sub(theme, background);
        let font_size = appearance.ui_font_size();
        let line_height_ratio = appearance.line_height_ratio();
        let content_line_height = app.font_cache().line_height(font_size, line_height_ratio);

        let mut list_col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for memory in &memories {
            let content_text = ConstrainedBox::new(
                Text::new(
                    memory.content.clone(),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(main_text_color)
                .with_line_height_ratio(line_height_ratio)
                .with_selectable(false)
                .finish(),
            )
            .with_max_height(content_line_height * 2.0)
            .finish();

            let mut row_col =
                Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            row_col.add_child(content_text);

            let annotation = match &memory.source {
                Some(api::message::fetched_memory::Source::Conversation(_)) => {
                    Some("From conversation")
                }
                Some(api::message::fetched_memory::Source::Manual(_)) => Some("Manual"),
                None => None,
            };
            if let Some(annotation) = annotation {
                row_col.add_child(
                    Container::new(
                        Text::new_inline(annotation, appearance.ui_font_family(), font_size - 2.)
                            .with_color(sub_text_color)
                            .with_selectable(false)
                            .finish(),
                    )
                    .with_margin_top(2.)
                    .finish(),
                );
            }

            let row_content = row_col.finish();
            let memory_store_id = memory.memory_store_id.clone();
            let memory_id = memory.memory_id.clone();
            let row = Hoverable::new(self.row_mouse_state(&memory.memory_id), move |state| {
                let mut container = Container::new(row_content)
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(6.);
                if state.is_hovered() {
                    container = container.with_background(internal_colors::fg_overlay_2(theme));
                }
                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(FetchedMemoriesPopupAction::OpenMemory {
                    memory_store_id: memory_store_id.clone(),
                    memory_id: memory_id.clone(),
                });
            })
            .finish();

            list_col.add_child(row);
        }

        let scrollable_body = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            Container::new(list_col.finish())
                .with_vertical_padding(6.)
                .finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        Dismiss::new(
            ConstrainedBox::new(
                Container::new(scrollable_body)
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .with_drop_shadow(DropShadow::default())
                    .finish(),
            )
            .with_width(POPUP_WIDTH)
            .with_max_height(POPUP_MAX_HEIGHT)
            .finish(),
        )
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(FetchedMemoriesPopupAction::ClosePopup);
        })
        .finish()
    }
}

impl TypedActionView for FetchedMemoriesPopupView {
    type Action = FetchedMemoriesPopupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FetchedMemoriesPopupAction::ClosePopup => {
                ctx.emit(FetchedMemoriesPopupEvent::Close);
            }
            FetchedMemoriesPopupAction::OpenMemory {
                memory_store_id,
                memory_id,
            } => {
                let memory_count =
                    fetched_memories_for_terminal_view(self.terminal_view_id, ctx).len();
                send_telemetry_from_ctx!(
                    FetchedMemoriesTelemetryEvent::MemoryLinkClicked { memory_count },
                    ctx
                );
                let oz_root_url = ChannelState::oz_root_url();
                let url = format!(
                    "{oz_root_url}/memory/{}/memories/{}",
                    urlencoding::encode(memory_store_id),
                    urlencoding::encode(memory_id)
                );
                ctx.open_url(&url);
                ctx.emit(FetchedMemoriesPopupEvent::Close);
            }
        }
    }
}

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum FetchedMemoriesTelemetryEvent {
    PopupOpened { memory_count: usize },
    MemoryLinkClicked { memory_count: usize },
}

impl TelemetryEvent for FetchedMemoriesTelemetryEvent {
    fn name(&self) -> &'static str {
        FetchedMemoriesTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::PopupOpened { memory_count } | Self::MemoryLinkClicked { memory_count } => {
                Some(json!({
                    "memory_count": memory_count,
                }))
            }
        }
    }

    fn description(&self) -> &'static str {
        FetchedMemoriesTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        FetchedMemoriesTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::PopupOpened { .. } | Self::MemoryLinkClicked { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for FetchedMemoriesTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::PopupOpened => "AgentMode.FetchedMemories.PopupOpened",
            Self::MemoryLinkClicked => "AgentMode.FetchedMemories.MemoryLinkClicked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::PopupOpened => "User opened the fetched memories popup from the footer chip",
            Self::MemoryLinkClicked => {
                "User clicked a fetched memory row to open it in the Oz web app"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::PopupOpened | Self::MemoryLinkClicked => {
                EnablementState::Flag(FeatureFlag::FetchedMemoriesChip)
            }
        }
    }
}

warp_core::register_telemetry_event!(FetchedMemoriesTelemetryEvent);
