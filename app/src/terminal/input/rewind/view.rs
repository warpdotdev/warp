//! View for the rewind menu.

use warpui::elements::ChildView;
use warpui::{Element, Entity, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::Query;
use crate::search::mixer::SearchMixer;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::rewind::data_source::{RewindDataSource, SelectRewindPoint};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};

/// Events emitted by RewindMenuView.
#[derive(Debug, Clone)]
pub enum RewindMenuEvent {
    /// User accepted a rewind point (hit enter).
    /// If exchange_id is None, user selected "Current" (dismiss without rewinding).
    AcceptedRewindPoint {
        exchange_id: Option<AIAgentExchangeId>,
    },
    /// User dismissed the menu (escape or click).
    Dismissed,
}

pub struct RewindMenuView {
    menu_view: ViewHandle<InlineMenuView<SelectRewindPoint>>,
    data_source: ModelHandle<RewindDataSource>,
    mixer: ModelHandle<SearchMixer<SelectRewindPoint>>,
    input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
}

impl RewindMenuView {
    pub fn new(
        conversation_id: AIConversationId,
        input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx.add_model(|_| RewindDataSource::new(conversation_id));

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<SelectRewindPoint>::new();
            mixer.add_sync_source(data_source.clone(), []);
            mixer.run_query(Query::default(), ctx);
            mixer
        });

        let menu_view = ctx.add_typed_action_view(|ctx| {
            InlineMenuView::new(
                mixer.clone(),
                positioner.clone(),
                &input_suggestions_model,
                agent_view_controller,
                ctx,
            )
        });

        ctx.subscribe_to_view(&menu_view, |_, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem { item, .. } => {
                ctx.emit(RewindMenuEvent::AcceptedRewindPoint {
                    exchange_id: item.exchange_id,
                });
            }
            InlineMenuEvent::SelectedItem { .. } => {
                // Could emit a preview event if needed
            }
            InlineMenuEvent::NoResults | InlineMenuEvent::TabChanged => {}
            InlineMenuEvent::Dismissed => {
                ctx.emit(RewindMenuEvent::Dismissed);
            }
        });

        ctx.subscribe_to_model(
            &input_suggestions_model,
            |me, input_suggestions_model, event, ctx| {
                let InputSuggestionsModeEvent::ModeChanged { .. } = event;
                if let Some(conversation_id) =
                    input_suggestions_model.as_ref(ctx).rewind_conversation_id()
                {
                    me.data_source.update(ctx, |ds, _| {
                        ds.set_conversation_id(conversation_id);
                    });
                    me.refresh_results("", ctx);
                }
            },
        );

        ctx.subscribe_to_model(input_buffer_model, |me, _, event, ctx| {
            if me.input_suggestions_model.as_ref(ctx).is_rewind_menu() {
                let InputBufferUpdateEvent { new_content, .. } = event;
                me.refresh_results(new_content, ctx);
            }
        });

        Self {
            menu_view,
            data_source,
            mixer,
            input_suggestions_model,
        }
    }

    fn refresh_results(&self, search_query: &str, ctx: &mut ViewContext<Self>) {
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: search_query.to_owned(),
                    ..Default::default()
                },
                ctx,
            );
        });
    }

    pub fn select_up(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_up(ctx));
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_down(ctx));
    }

    pub fn accept_selected_item(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |v, ctx| v.accept_selected_item(false, ctx));
    }
}

impl View for RewindMenuView {
    fn ui_name() -> &'static str {
        "RewindMenuView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for RewindMenuView {
    type Event = RewindMenuEvent;
}
