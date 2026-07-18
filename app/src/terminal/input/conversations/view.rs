//! Inline conversation menu view for selecting AI conversations.

use std::collections::HashSet;
use std::sync::LazyLock;

use warpui::elements::ChildView;
use warpui::{Element, Entity, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent_conversations_model::AgentConversationEntryId;
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::conversation_selection::ConversationSelectionHandle;
use crate::features::FeatureFlag;
use crate::search::data_source::{Query, QueryFilter};
use crate::search::mixer::SearchMixer;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::conversations::data_source::ConversationMenuDataSource;
use crate::terminal::input::conversations::{AcceptConversation, InlineConversationMenuTab};
use crate::terminal::input::inline_menu::{
    InlineMenuEvent, InlineMenuPositioner, InlineMenuTabConfig, InlineMenuView,
};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::model::session::active_session::ActiveSession;

/// Events emitted by InlineConversationMenuView.
#[derive(Debug, Clone)]
pub enum InlineConversationMenuEvent {
    /// User 'accepted' a conversation (hit enter).
    NavigateToConversation { item_id: AgentConversationEntryId },
    /// User dismissed the menu (escape or click).
    Dismissed,
}

static TAB_CONFIGS: LazyLock<Vec<InlineMenuTabConfig<InlineConversationMenuTab>>> =
    LazyLock::new(|| {
        let mut configs = vec![InlineMenuTabConfig {
            id: InlineConversationMenuTab::All,
            label: "All".to_string(),
            filters: HashSet::new(),
        }];
        if FeatureFlag::InlineMenuHeaders.is_enabled() {
            configs.push(InlineMenuTabConfig {
                id: InlineConversationMenuTab::CurrentDirectory,
                label: "Current Directory".to_string(),
                filters: HashSet::from([QueryFilter::CurrentDirectoryConversations]),
            });
        }
        configs
    });

pub struct InlineConversationMenuView {
    menu_view: ViewHandle<InlineMenuView<AcceptConversation, InlineConversationMenuTab>>,
    mixer: ModelHandle<SearchMixer<AcceptConversation>>,
    input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
}

impl InlineConversationMenuView {
    pub fn new(
        input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        conversation_selection: ConversationSelectionHandle,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        active_session: ModelHandle<ActiveSession>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx
            .add_model(|_| ConversationMenuDataSource::new(conversation_selection, active_session));

        let tab_configs = TAB_CONFIGS.clone();
        let initial_filters = tab_configs
            .first()
            .map(|config| config.filters.clone())
            .unwrap_or_default();

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptConversation>::new();
            mixer.add_sync_source(data_source, [QueryFilter::CurrentDirectoryConversations]);
            mixer.run_query(
                Query {
                    text: String::new(),
                    filters: initial_filters,
                },
                ctx,
            );
            mixer
        });

        let menu_view = ctx.add_typed_action_view(|ctx| {
            InlineMenuView::new_with_tabs(
                mixer.clone(),
                positioner.clone(),
                &input_suggestions_model,
                agent_view_controller,
                tab_configs,
                None,
                ctx,
            )
        });
        ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem { item, .. } => {
                ctx.emit(InlineConversationMenuEvent::NavigateToConversation {
                    item_id: item.item_id,
                });
            }
            InlineMenuEvent::SelectedItem { .. } | InlineMenuEvent::NoResults => (),
            InlineMenuEvent::TabChanged => {
                me.rerun_query(ctx);
            }
            InlineMenuEvent::Dismissed => {
                ctx.emit(InlineConversationMenuEvent::Dismissed);
            }
        });

        ctx.subscribe_to_model(
            &input_suggestions_model,
            |me, input_suggestions_model, event, ctx| {
                let InputSuggestionsModeEvent::ModeChanged { .. } = event;
                if input_suggestions_model.as_ref(ctx).is_conversation_menu() {
                    me.rerun_query(ctx);
                }
            },
        );
        ctx.subscribe_to_model(input_buffer_model, |me, _, event, ctx| {
            if me
                .input_suggestions_model
                .as_ref(ctx)
                .is_conversation_menu()
            {
                let InputBufferUpdateEvent { new_content: _, .. } = event;
                me.rerun_query(ctx);
            }
        });

        let active_agent_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_agent_views_model, |me, _, _, ctx| {
            if me
                .input_suggestions_model
                .as_ref(ctx)
                .is_conversation_menu()
            {
                me.menu_view.update(ctx, |_, ctx| ctx.notify());
            }
        });

        Self {
            menu_view,
            mixer,
            input_suggestions_model,
            input_buffer_model: input_buffer_model.clone(),
        }
    }

    fn rerun_query(&self, ctx: &mut ViewContext<Self>) {
        let filters = self
            .menu_view
            .as_ref(ctx)
            .model()
            .as_ref(ctx)
            .active_tab_filters();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: self
                        .input_buffer_model
                        .as_ref(ctx)
                        .current_value()
                        .to_owned(),
                    filters,
                },
                ctx,
            );
        });
    }

    pub fn select_next_tab(&self, ctx: &mut ViewContext<Self>) -> bool {
        self.menu_view.update(ctx, |v, ctx| v.select_next_tab(ctx))
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

impl View for InlineConversationMenuView {
    fn ui_name() -> &'static str {
        "InlineConversationMenuView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineConversationMenuView {
    type Event = InlineConversationMenuEvent;
}
