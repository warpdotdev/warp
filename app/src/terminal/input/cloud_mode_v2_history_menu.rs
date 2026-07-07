use std::collections::HashSet;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, DropShadow, Radius,
};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, View, ViewContext,
    ViewHandle,
};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::{Query, QueryFilter};
use crate::search::mixer::SearchMixer;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_history::{
    AcceptHistoryItem, InlineHistoryMenuDataSource, InlineHistoryMenuEvent,
};
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::input::InputSuggestionsMode;
use crate::terminal::model::session::active_session::ActiveSession;

const MENU_MAX_HEIGHT: f32 = 168.;

const MENU_VERTICAL_PADDING: f32 = 4.;

const MENU_CORNER_RADIUS: f32 = 6.;

const DROP_SHADOW_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 77,
};

pub struct CloudModeV2HistoryMenuView {
    menu_view: ViewHandle<InlineMenuView<AcceptHistoryItem>>,
    mixer: ModelHandle<SearchMixer<AcceptHistoryItem>>,
    buffer_model: ModelHandle<InputBufferModel>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    pending_initial_buffer_sync: bool,
}

impl CloudModeV2HistoryMenuView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        terminal_view_id: EntityId,
        active_session: ModelHandle<ActiveSession>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        buffer_model: ModelHandle<InputBufferModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx.add_model(|_| {
            InlineHistoryMenuDataSource::new(
                terminal_view_id,
                active_session,
                agent_view_controller.clone(),
            )
        });

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptHistoryItem>::new();
            mixer.add_sync_source(data_source, [QueryFilter::PromptHistory]);
            mixer.run_query(prompts_query(""), ctx);
            mixer
        });

        let menu_view = ctx.add_typed_action_view(|ctx| {
            InlineMenuView::new(
                mixer.clone(),
                positioner.clone(),
                input_suggestions_model,
                agent_view_controller,
                ctx,
            )
            .with_compact_layout()
            .with_dismiss_on_row_click()
        });

        ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem {
                item: AcceptHistoryItem::AIPrompt { query_text },
                ..
            } => {
                ctx.emit(InlineHistoryMenuEvent::AcceptAIPrompt {
                    query_text: query_text.clone(),
                });
            }
            InlineMenuEvent::SelectedItem {
                item: AcceptHistoryItem::AIPrompt { query_text },
            } => {
                ctx.emit(InlineHistoryMenuEvent::SelectAIPrompt {
                    query_text: query_text.clone(),
                });
            }
            InlineMenuEvent::Dismissed => {
                me.suggestions_mode_model.update(ctx, |model, ctx| {
                    model.set_mode(InputSuggestionsMode::Closed, ctx);
                });
            }
            InlineMenuEvent::NoResults => {
                ctx.emit(InlineHistoryMenuEvent::NoResults);
            }
            InlineMenuEvent::AcceptedItem { .. }
            | InlineMenuEvent::SelectedItem { .. }
            | InlineMenuEvent::TabChanged => {}
        });

        ctx.subscribe_to_model(input_suggestions_model, |me, model, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if model.as_ref(ctx).is_inline_history_menu() {
                me.open_with_current_buffer(ctx);
            }
        });

        ctx.subscribe_to_model(&buffer_model, |me, _, _: &InputBufferUpdateEvent, ctx| {
            if !me
                .suggestions_mode_model
                .as_ref(ctx)
                .is_inline_history_menu()
            {
                return;
            }
            if !me.pending_initial_buffer_sync {
                return;
            }
            me.pending_initial_buffer_sync = false;
            me.open_with_current_buffer(ctx);
        });

        Self {
            menu_view,
            mixer,
            buffer_model,
            suggestions_mode_model: input_suggestions_model.clone(),
            pending_initial_buffer_sync: false,
        }
    }

    pub fn select_up(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_up(ctx));
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
        // Mirror the legacy `InlineHistoryMenuView::select_down` behavior:
        // pressing Down past the last item (or with no results) closes the
        // history menu rather than wrapping back to the first item.
        let should_close = self.menu_view.read(ctx, |v, _| {
            let result_count = v.result_count();
            let is_last_item_selected =
                result_count > 0 && v.selected_idx().is_some_and(|idx| idx == result_count - 1);
            is_last_item_selected || result_count == 0
        });
        if should_close {
            ctx.emit(InlineHistoryMenuEvent::Close);
        } else {
            self.menu_view.update(ctx, |v, ctx| v.select_down(ctx));
        }
    }

    pub fn accept_selected(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |v, ctx| v.accept_selected_item(false, ctx));
    }

    pub fn arm_initial_buffer_sync(&mut self, _ctx: &mut ViewContext<Self>) {
        self.pending_initial_buffer_sync = true;
    }

    pub fn has_selection(&self, app: &AppContext) -> bool {
        self.menu_view
            .as_ref(app)
            .model()
            .as_ref(app)
            .selected_item()
            .is_some()
    }

    /// Returns the currently selected AI prompt's query text, if any.
    ///
    /// The cloud-mode v2 menu is restricted to `AcceptHistoryItem::AIPrompt`
    /// items via its data source filter, so we only ever expect prompt
    /// selections; the other arms are unreachable but matched defensively.
    pub fn selected_query_text(&self, app: &AppContext) -> Option<String> {
        match self
            .menu_view
            .as_ref(app)
            .model()
            .as_ref(app)
            .selected_item()?
        {
            AcceptHistoryItem::AIPrompt { query_text } => Some(query_text.clone()),
            AcceptHistoryItem::Command { .. } | AcceptHistoryItem::Conversation { .. } => None,
        }
    }

    fn open_with_current_buffer(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.buffer_model.as_ref(ctx).current_value().to_owned();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(prompts_query(&text), ctx);
        });
    }
}

fn prompts_query(text: &str) -> Query {
    Query {
        text: text.to_owned(),
        filters: HashSet::from([QueryFilter::PromptHistory]),
    }
}

impl Entity for CloudModeV2HistoryMenuView {
    type Event = InlineHistoryMenuEvent;
}

impl View for CloudModeV2HistoryMenuView {
    fn ui_name() -> &'static str {
        "CloudModeV2HistoryMenuView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let border_color = internal_colors::neutral_4(theme);
        let background = internal_colors::neutral_1(theme);

        let menu_with_height = ConstrainedBox::new(ChildView::new(&self.menu_view).finish())
            .with_max_height(MENU_MAX_HEIGHT)
            .finish();

        let padded = Container::new(menu_with_height)
            .with_padding_top(MENU_VERTICAL_PADDING)
            .with_padding_bottom(MENU_VERTICAL_PADDING)
            .finish();

        Container::new(padded)
            .with_background(background)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MENU_CORNER_RADIUS)))
            .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                DROP_SHADOW_COLOR,
            ))
            .finish()
    }
}
