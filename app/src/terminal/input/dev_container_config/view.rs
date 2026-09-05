//! Inline selector view for choosing among multiple discovered Dev Container configs.
use std::path::PathBuf;

use warpui::elements::ChildView;
use warpui::{Element, Entity, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::Query;
use crate::search::mixer::SearchMixer;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::dev_container_config::SelectDevContainerConfig;
use crate::terminal::input::dev_container_config::data_source::DevContainerConfigSelectorDataSource;
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};

/// Events emitted by [`InlineDevContainerConfigSelectorView`].
#[derive(Debug, Clone)]
pub enum InlineDevContainerConfigSelectorEvent {
    /// User selected a config to bring up the Dev Container with.
    Selected { config_path: PathBuf },
    /// User dismissed the menu.
    Dismissed,
}

pub struct InlineDevContainerConfigSelectorView {
    menu_view: ViewHandle<InlineMenuView<SelectDevContainerConfig>>,
    data_source: ModelHandle<DevContainerConfigSelectorDataSource>,
    mixer: ModelHandle<SearchMixer<SelectDevContainerConfig>>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
}

impl InlineDevContainerConfigSelectorView {
    pub fn new(
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx.add_model(|_| DevContainerConfigSelectorDataSource::new(Vec::new()));

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<SelectDevContainerConfig>::new();
            mixer.add_sync_source(data_source.clone(), []);
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

        ctx.subscribe_to_view(&menu_view, |_, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem { item, .. } => {
                ctx.emit(InlineDevContainerConfigSelectorEvent::Selected {
                    config_path: item.config_path.clone(),
                });
            }
            InlineMenuEvent::Dismissed => {
                ctx.emit(InlineDevContainerConfigSelectorEvent::Dismissed);
            }
            InlineMenuEvent::SelectedItem { .. }
            | InlineMenuEvent::NoResults
            | InlineMenuEvent::TabChanged => {}
        });

        ctx.subscribe_to_model(&suggestions_mode_model, |me, model, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if let Some(configs) = model.as_ref(ctx).dev_container_config_selector_configs() {
                me.data_source.update(ctx, |ds, _| {
                    ds.set_configs(configs);
                });
                me.refresh_results("", ctx);
            }
        });

        ctx.subscribe_to_model(input_buffer_model, |me, _, event, ctx| {
            if !me
                .suggestions_mode_model
                .as_ref(ctx)
                .is_dev_container_config_selector()
            {
                return;
            }
            let InputBufferUpdateEvent { new_content, .. } = event;
            me.refresh_results(new_content, ctx);
        });

        Self {
            menu_view,
            data_source,
            mixer,
            suggestions_mode_model,
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
        self.menu_view.update(ctx, |view, ctx| view.select_up(ctx));
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |view, ctx| view.select_down(ctx));
    }

    pub fn accept_selected_item(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |view, ctx| view.accept_selected_item(false, ctx));
    }
}

impl View for InlineDevContainerConfigSelectorView {
    fn ui_name() -> &'static str {
        "InlineDevContainerConfigSelectorView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineDevContainerConfigSelectorView {
    type Event = InlineDevContainerConfigSelectorEvent;
}
