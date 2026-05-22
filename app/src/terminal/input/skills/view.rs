use ai::skills::SkillReference;
use warpui::elements::ChildView;
use warpui::{Element, Entity, EntityId, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::Query;
use crate::search::mixer::{SearchMixer, SearchMixerEvent};
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::skills::data_source::{
    AcceptSkill, SkillSelectorDataSource, UpdatedAvailableSkills,
};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::model::session::active_session::ActiveSession;

#[derive(Debug, Clone)]
pub enum InlineSkillSelectorEvent {
    SelectedSkill {
        skill_name: String,
        skill_reference: SkillReference,
    },
}

pub struct InlineSkillSelectorView {
    menu_view: ViewHandle<InlineMenuView<AcceptSkill>>,
    mixer: ModelHandle<SearchMixer<AcceptSkill>>,
    data_source: ModelHandle<SkillSelectorDataSource>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
}

impl InlineSkillSelectorView {
    pub fn new(
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        active_session: ModelHandle<ActiveSession>,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx
            .add_model(|ctx| SkillSelectorDataSource::new(active_session, terminal_view_id, ctx));

        let mixer = ctx.add_model(|_| {
            let mut mixer = SearchMixer::<AcceptSkill>::new();
            mixer.add_sync_source(data_source.clone(), []);
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
                ctx.emit(InlineSkillSelectorEvent::SelectedSkill {
                    skill_name: item.skill_name.clone(),
                    skill_reference: item.skill_reference.clone(),
                });
            }
        });

        ctx.subscribe_to_model(&suggestions_mode_model, |me, model, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if model.as_ref(ctx).is_skill_menu() {
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
            if me.suggestions_mode_model.as_ref(ctx).is_skill_menu() {
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

        ctx.subscribe_to_model(&mixer, |_me, _, event, ctx| {
            let SearchMixerEvent::ResultsChanged = event;
            // No special handling needed for skills - just re-render
            ctx.notify();
        });

        ctx.subscribe_to_model(&data_source, |me, _, _: &UpdatedAvailableSkills, ctx| {
            // Re-run the query when skills change (e.g., pwd changed)
            me.mixer.update(ctx, |mixer, ctx| {
                if let Some(query) = mixer.current_query().cloned() {
                    mixer.run_query(query, ctx);
                }
            });
        });

        Self {
            menu_view,
            mixer,
            data_source,
            suggestions_mode_model,
            input_buffer_model: input_buffer_model.clone(),
        }
    }

    /// Sets whether bundled skills are included in results.
    /// Should be called before opening the menu.
    pub fn set_include_bundled(&self, include_bundled: bool, ctx: &mut ViewContext<Self>) {
        self.data_source.update(ctx, |ds, _| {
            ds.set_include_bundled(include_bundled);
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

impl View for InlineSkillSelectorView {
    fn ui_name() -> &'static str {
        "InlineSkillSelectorView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineSkillSelectorView {
    type Event = InlineSkillSelectorEvent;
}
