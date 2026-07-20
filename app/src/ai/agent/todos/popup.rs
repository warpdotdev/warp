use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, DropShadow, Empty, Expanded, Flex, MainAxisSize, ParentElement,
    Radius, SavePosition, ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Shrinkable, Text,
};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext,
};

use crate::ai::agent::icons::{in_progress_icon, pending_icon, succeeded_icon};
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::blocklist::{
    BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel,
};
use crate::ui_components::blended_colors;

pub struct AgentTodosPopupView {
    terminal_view_id: EntityId,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    scroll_state: ClippedScrollStateHandle,
}

const IN_PROGRESS_POSITION_ID: &str = "AgentTodosPopup-in-progress";

#[derive(Debug, Clone, Copy)]
pub enum AgentTodosPopupAction {
    ClosePopup,
}

pub enum AgentTodosPopupEvent {
    Close,
}

struct Styles {
    ui_font_family: FamilyId,
    background: Fill,
    main_text_color: ColorU,
    sub_text_color: ColorU,
    detail_font_size: f32,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AgentTodosPopupAction::ClosePopup,
        id!(AgentTodosPopupView::ui_name()),
    )]);
}

impl AgentTodosPopupView {
    pub fn new(
        terminal_view_id: EntityId,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let blocklist_history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&blocklist_history_model, move |me, _, event, ctx| {
            me.handle_blocklist_history_event(event, ctx);
        });
        ctx.subscribe_to_model(&ai_context_model, move |_, _, event, ctx| {
            if let BlocklistAIContextEvent::PendingQueryStateUpdated = event {
                ctx.notify();
            }
        });
        Self {
            terminal_view_id,
            ai_context_model,
            scroll_state: Default::default(),
        }
    }

    fn handle_blocklist_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let BlocklistAIHistoryEvent::UpdatedTodoList {
            terminal_surface_id,
        } = event
            && *terminal_surface_id == self.terminal_view_id
        {
            ctx.notify();
        }
    }

    /// Scroll to the in-progress item, if not currently visible.
    pub fn scroll_to_in_progress_item(&self) {
        self.scroll_state.scroll_to_position(ScrollTarget {
            position_id: IN_PROGRESS_POSITION_ID.to_string(),
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(AgentTodosPopupEvent::Close);
    }

    fn styles(&self, appearance: &Appearance) -> Styles {
        let theme = appearance.theme();
        let background = theme.surface_1();
        let main_text_color = blended_colors::text_main(theme, background);
        let sub_text_color = blended_colors::text_sub(theme, background);
        let detail_font_size = appearance.ui_font_size();
        let ui_font_family = appearance.ui_font_family();

        Styles {
            ui_font_family,
            background,
            main_text_color,
            sub_text_color,
            detail_font_size,
        }
    }

    fn render_header(
        &self,
        app: &warpui::AppContext,
        todo_list: &AIAgentTodoList,
    ) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);
        let theme = appearance.theme();

        let completed_count = todo_list.completed_items().len();
        let total_count = todo_list.pending_items().len() + completed_count;

        let mut header_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let mut header = Text::new(
            "Tasks".to_string(),
            appearance.header_font_family(),
            styles.detail_font_size + 2.,
        )
        .with_color(styles.main_text_color)
        .with_style(Properties::default().weight(Weight::Semibold));

        header.add_text_with_highlights(
            format!(" {completed_count}/{total_count}"),
            theme.sub_text_color(theme.surface_1()).into(),
            Properties::default().weight(Weight::Semibold),
        );

        header_row.add_child(header.finish());
        header_row.finish()
    }
}

impl View for AgentTodosPopupView {
    fn ui_name() -> &'static str {
        "AgentTodosPopup"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let Some(todo_list) = self
            .ai_context_model
            .as_ref(app)
            .selected_conversation_todolist(app)
        else {
            // We don't have an empty state.
            // Assume the popup will only be shown if there are todos.
            return Empty::new().finish();
        };

        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);
        let theme = appearance.theme();

        let background = styles.background;
        let main_text_color = styles.main_text_color;
        let sub_text_color = styles.sub_text_color;
        let detail_font_size = styles.detail_font_size;
        let ui_font_family = styles.ui_font_family;

        let mut list_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(12.);

        let items_with_icons = todo_list
            .completed_items()
            .iter()
            .map(|item| (item, succeeded_icon(appearance)))
            .chain(
                todo_list
                    .pending_items()
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        (
                            item,
                            if i == 0 {
                                in_progress_icon(appearance)
                            } else {
                                pending_icon(appearance)
                            },
                        )
                    }),
            );

        for (item, status_icon) in items_with_icons {
            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max);

            // Status icon
            row.add_child(
                Container::new(
                    ConstrainedBox::new(status_icon.finish())
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            );

            let is_in_progress = todo_list
                .in_progress_item()
                .map(|t| t.id.clone())
                .as_ref()
                .map(|id| &item.id == id)
                .unwrap_or(false);

            // Title and status
            let mut text_col =
                Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

            let text_color = if is_in_progress {
                main_text_color
            } else {
                sub_text_color
            };

            text_col.add_child(
                Text::new(item.title.clone(), ui_font_family, detail_font_size)
                    .with_color(text_color)
                    .finish(),
            );

            row.add_child(Expanded::new(1.0, text_col.finish()).finish());

            let row = if is_in_progress {
                SavePosition::new(row.finish(), IN_PROGRESS_POSITION_ID).finish()
            } else {
                row.finish()
            };

            list_col.add_child(row);
        }

        let header = Container::new(self.render_header(app, todo_list))
            .with_padding_top(16.)
            .with_horizontal_padding(16.)
            .with_padding_bottom(8.)
            .finish();

        let scrollable_body = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            Container::new(list_col.finish())
                .with_horizontal_padding(16.)
                .with_padding_bottom(16.)
                .finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let panel_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Shrinkable::new(1.0, scrollable_body).finish());

        Dismiss::new(
            ConstrainedBox::new(
                Container::new(panel_col.finish())
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .with_drop_shadow(DropShadow::default())
                    .finish(),
            )
            .with_width(300.)
            .with_max_height(420.)
            .finish(),
        )
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(AgentTodosPopupAction::ClosePopup);
        })
        .finish()
    }
}

impl Entity for AgentTodosPopupView {
    type Event = AgentTodosPopupEvent;
}

impl TypedActionView for AgentTodosPopupView {
    type Action = AgentTodosPopupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentTodosPopupAction::ClosePopup => {
                self.close(ctx);
            }
        }
    }
}
