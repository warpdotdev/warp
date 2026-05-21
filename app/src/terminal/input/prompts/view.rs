use warpui::elements::ChildView;
use warpui::{Element, Entity, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::Query;
use crate::search::mixer::SearchMixer;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::prompts::AcceptPrompt;
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use warp_server_client::ids::SyncId;

#[derive(Debug, Clone)]
pub enum InlinePromptsMenuEvent {
    SelectedPrompt { id: SyncId },
}

pub struct InlinePromptsMenuView {
    menu_view: ViewHandle<InlineMenuView<AcceptPrompt>>,
    mixer: ModelHandle<SearchMixer<AcceptPrompt>>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
}

impl InlinePromptsMenuView {
    pub fn new(
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptPrompt>::new();
            mixer.run_query(Query::default(), ctx);
            mixer
        });

        let menu_view = ctx.add_typed_action_view(|ctx| {
            InlineMenuView::new(
                mixer.clone(),
                positioner.clone(),
                &suggestions_mode_model,
                agent_view_controller,
                ctx,
            )
        });

        ctx.subscribe_to_view(&menu_view, |_, _, event, ctx| {
            if let InlineMenuEvent::AcceptedItem { item, .. } = event {
                ctx.emit(InlinePromptsMenuEvent::SelectedPrompt {
                    id: item.id.clone(),
                });
            }
        });

        ctx.subscribe_to_model(&suggestions_mode_model, |me, model, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if model.as_ref(ctx).is_prompts_menu() {
                let query_text = me.input_buffer_model.as_ref(ctx).current_value().to_owned();
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.run_query(
                        Query {
                            text: query_text,
                            ..Default::default()
                        },
                        ctx,
                    );
                });
            } else if model.as_ref(ctx).is_closed() {
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.reset_results(ctx);
                });
            }
        });

        ctx.subscribe_to_model(input_buffer_model, |me, _, event, ctx| {
            if me.suggestions_mode_model.as_ref(ctx).is_prompts_menu() {
                let InputBufferUpdateEvent { new_content, .. } = event;
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.run_query(
                        Query {
                            text: new_content.clone(),
                            ..Default::default()
                        },
                        ctx,
                    );
                });
            }
        });

        Self {
            menu_view,
            mixer,
            suggestions_mode_model,
            input_buffer_model: input_buffer_model.clone(),
        }
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

impl View for InlinePromptsMenuView {
    fn ui_name() -> &'static str {
        "InlinePromptsMenuView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlinePromptsMenuView {
    type Event = InlinePromptsMenuEvent;
}
