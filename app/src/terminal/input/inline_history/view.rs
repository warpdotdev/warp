//! Inline history menu view for up-arrow history with conversations, commands and prompts.
use std::collections::HashSet;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::ChildView;
use warpui::{AppContext, Element, Entity, EntityId, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::features::FeatureFlag;
use crate::search::data_source::{Query, QueryFilter};
use crate::search::mixer::{SearchMixer, SearchMixerEvent};
use crate::settings_view::SettingsSection;
use crate::terminal::history::LinkedWorkflowData;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_history::data_source::{
    AcceptHistoryItem, InlineHistoryMenuDataSource,
};
use crate::terminal::input::inline_menu::{
    InlineMenuEvent, InlineMenuHeaderConfig, InlineMenuModel, InlineMenuPositioner,
    InlineMenuTabConfig, InlineMenuView,
};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};
use crate::workspace::WorkspaceAction;

#[derive(Debug, Clone)]
pub enum InlineHistoryMenuEvent {
    NavigateToConversation {
        conversation_id: AIConversationId,
    },
    AcceptCommand {
        command: String,
        linked_workflow_data: Option<LinkedWorkflowData>,
    },
    AcceptAIPrompt {
        query_text: String,
    },
    SelectCommand {
        command: String,
        linked_workflow_data: Option<LinkedWorkflowData>,
    },
    SelectAIPrompt {
        query_text: String,
    },
    /// Emitted when a conversation row becomes selected so any previously
    /// previewed command or prompt text is cleared from the input buffer.
    SelectConversation,
    NoResults,
    /// Emitted when the inline menu should be closed and additionally restore the
    /// original input buffer contents.
    Close,
}

#[derive(Clone)]
/// Identifies a history item well enough to reselect the same logical item
/// after rerunning the current query.
enum HistoryItemIdentity {
    Conversation(AIConversationId),
    Command(String),
    AIPrompt(String),
}

impl HistoryItemIdentity {
    fn from_item(item: &AcceptHistoryItem) -> Self {
        match item {
            AcceptHistoryItem::Conversation {
                conversation_id, ..
            } => Self::Conversation(*conversation_id),
            AcceptHistoryItem::Command { command, .. } => Self::Command(command.clone()),
            AcceptHistoryItem::AIPrompt { query_text } => Self::AIPrompt(query_text.clone()),
        }
    }

    fn matches(&self, item: &AcceptHistoryItem) -> bool {
        match (self, item) {
            (
                Self::Conversation(expected_id),
                AcceptHistoryItem::Conversation {
                    conversation_id, ..
                },
            ) => *expected_id == *conversation_id,
            (Self::Command(expected_command), AcceptHistoryItem::Command { command, .. }) => {
                expected_command == command
            }
            (Self::AIPrompt(expected_query), AcceptHistoryItem::AIPrompt { query_text }) => {
                expected_query == query_text
            }
            _ => false,
        }
    }
}

struct ConfigureButtonTheme;

impl ActionButtonTheme for ConfigureButtonTheme {
    fn background(&self, _hovered: bool, _appearance: &Appearance) -> Option<Fill> {
        None
    }

    fn text_color(
        &self,
        hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        let theme = appearance.theme();
        if hovered {
            internal_colors::text_main(theme, theme.background())
        } else {
            internal_colors::text_sub(theme, theme.background())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryTab {
    All,
    Commands,
    Prompts,
}

fn build_tab_configs(is_agent_view: bool) -> Vec<InlineMenuTabConfig<HistoryTab>> {
    if !is_agent_view {
        return vec![InlineMenuTabConfig {
            id: HistoryTab::All,
            label: "All".to_string(),
            filters: HashSet::new(),
        }];
    }

    vec![
        InlineMenuTabConfig {
            id: HistoryTab::All,
            label: "All".to_string(),
            filters: HashSet::new(),
        },
        InlineMenuTabConfig {
            id: HistoryTab::Commands,
            label: "Commands".to_string(),
            filters: HashSet::from([QueryFilter::Commands]),
        },
        InlineMenuTabConfig {
            id: HistoryTab::Prompts,
            label: "Prompts".to_string(),
            filters: HashSet::from([QueryFilter::PromptHistory]),
        },
    ]
}

pub struct InlineHistoryMenuView {
    menu_view: ViewHandle<InlineMenuView<AcceptHistoryItem, HistoryTab>>,
    mixer: ModelHandle<SearchMixer<AcceptHistoryItem>>,
    model: ModelHandle<InlineMenuModel<AcceptHistoryItem, HistoryTab>>,
    buffer_model: ModelHandle<InputBufferModel>,
    pending_tab_switch_selection: Option<HistoryItemIdentity>,
    caller_supplied_tabs: bool,
    pending_initial_buffer_sync: bool,
}

impl InlineHistoryMenuView {
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
        let is_agent_view = agent_view_controller.as_ref(ctx).is_active();
        let tab_configs = build_tab_configs(is_agent_view);
        Self::new_inner(
            terminal_view_id,
            active_session,
            input_suggestions_model,
            agent_view_controller,
            positioner,
            buffer_model,
            tab_configs,
            /* caller_supplied_tabs */ false,
            ctx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_tab_configs(
        terminal_view_id: EntityId,
        active_session: ModelHandle<ActiveSession>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        buffer_model: ModelHandle<InputBufferModel>,
        tab_configs: Vec<InlineMenuTabConfig<HistoryTab>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_inner(
            terminal_view_id,
            active_session,
            input_suggestions_model,
            agent_view_controller,
            positioner,
            buffer_model,
            tab_configs,
            /* caller_supplied_tabs */ true,
            ctx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        terminal_view_id: EntityId,
        active_session: ModelHandle<ActiveSession>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        buffer_model: ModelHandle<InputBufferModel>,
        tab_configs: Vec<InlineMenuTabConfig<HistoryTab>>,
        caller_supplied_tabs: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx.add_model(|_| {
            InlineHistoryMenuDataSource::new(
                terminal_view_id,
                active_session,
                agent_view_controller.clone(),
            )
        });

        let initial_filters = tab_configs
            .first()
            .map(|config| config.filters.clone())
            .unwrap_or_default();

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptHistoryItem>::new();
            mixer.add_sync_source(
                data_source,
                [
                    QueryFilter::Commands,
                    QueryFilter::Conversations,
                    QueryFilter::PromptHistory,
                ],
            );
            mixer.run_query(
                Query {
                    text: String::new(),
                    filters: initial_filters,
                },
                ctx,
            );
            mixer
        });

        let menu_view = if FeatureFlag::InlineMenuHeaders.is_enabled() {
            let configure_button = ctx.add_view(|_| {
                ActionButton::new("Configure", ConfigureButtonTheme)
                    .with_icon(Icon::Settings)
                    .with_size(ButtonSize::Small)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPageWithSearch {
                            search_query: "commands history".into(),
                            section: Some(SettingsSection::WarpAgent),
                        });
                    })
            });
            let header_config = InlineMenuHeaderConfig {
                label: "History".to_string(),
                trailing_element: Some(Box::new(move |_app: &AppContext| {
                    ChildView::new(&configure_button).finish()
                })),
            };
            ctx.add_typed_action_view(|ctx| {
                InlineMenuView::new_with_tabs(
                    mixer.clone(),
                    positioner.clone(),
                    input_suggestions_model,
                    agent_view_controller.clone(),
                    tab_configs,
                    None,
                    ctx,
                )
                .with_header_config(header_config)
            })
        } else {
            ctx.add_typed_action_view(|ctx| {
                InlineMenuView::new_with_tabs(
                    mixer.clone(),
                    positioner.clone(),
                    input_suggestions_model,
                    agent_view_controller.clone(),
                    tab_configs,
                    None,
                    ctx,
                )
            })
        };
        let model = menu_view.as_ref(ctx).model().clone();

        ctx.subscribe_to_model(input_suggestions_model, |me, model, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if model.as_ref(ctx).is_inline_history_menu() {
                me.open_with_current_buffer(ctx);
            }
        });

        let suggestions_mode_model_for_buffer = input_suggestions_model.clone();
        ctx.subscribe_to_model(
            &buffer_model,
            move |me, _, _: &InputBufferUpdateEvent, ctx| {
                if !suggestions_mode_model_for_buffer
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
            },
        );

        let suggestions_mode_model = input_suggestions_model.clone();
        ctx.subscribe_to_model(
            &agent_view_controller,
            move |me, controller, event, ctx| match event {
                AgentViewControllerEvent::EnteredAgentView { .. }
                | AgentViewControllerEvent::ExitedAgentView { .. } => {
                    // Only auto-rebuild tabs from `is_agent_view` when the
                    // caller did not supply tabs explicitly. Callers that
                    // pinned tabs (e.g. the cloud-mode V2 wrapper) want their
                    // tab set preserved across agent-view enter/exit.
                    if !me.caller_supplied_tabs {
                        let is_agent_view = controller.as_ref(ctx).is_active();
                        let new_configs = build_tab_configs(is_agent_view);
                        me.model.update(ctx, |model, _| {
                            model.set_tab_configs(new_configs);
                        });
                        if suggestions_mode_model.as_ref(ctx).is_inline_history_menu() {
                            me.pending_tab_switch_selection = me
                                .model
                                .as_ref(ctx)
                                .selected_item()
                                .map(HistoryItemIdentity::from_item);
                            me.rerun_query(ctx);
                        }
                    }
                    me.menu_view.update(ctx, |_, ctx| ctx.notify());
                }
                AgentViewControllerEvent::ExitConfirmed { .. } => {}
            },
        );

        ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem { item, .. } => match item {
                AcceptHistoryItem::Conversation {
                    conversation_id, ..
                } => {
                    ctx.emit(InlineHistoryMenuEvent::NavigateToConversation {
                        conversation_id: *conversation_id,
                    });
                }
                AcceptHistoryItem::Command {
                    command,
                    linked_workflow_data,
                } => {
                    ctx.emit(InlineHistoryMenuEvent::AcceptCommand {
                        command: command.clone(),
                        linked_workflow_data: linked_workflow_data.clone(),
                    });
                }
                AcceptHistoryItem::AIPrompt { query_text } => {
                    ctx.emit(InlineHistoryMenuEvent::AcceptAIPrompt {
                        query_text: query_text.clone(),
                    });
                }
            },
            InlineMenuEvent::SelectedItem { item } => match item {
                AcceptHistoryItem::Command {
                    command,
                    linked_workflow_data,
                } => {
                    ctx.emit(InlineHistoryMenuEvent::SelectCommand {
                        command: command.clone(),
                        linked_workflow_data: linked_workflow_data.clone(),
                    });
                }
                AcceptHistoryItem::AIPrompt { query_text } => {
                    ctx.emit(InlineHistoryMenuEvent::SelectAIPrompt {
                        query_text: query_text.clone(),
                    });
                }
                AcceptHistoryItem::Conversation { .. } => {
                    ctx.emit(InlineHistoryMenuEvent::SelectConversation);
                }
            },
            InlineMenuEvent::Dismissed => {
                ctx.emit(InlineHistoryMenuEvent::Close);
            }
            InlineMenuEvent::NoResults => {
                ctx.emit(InlineHistoryMenuEvent::NoResults);
            }
            InlineMenuEvent::TabChanged => {
                me.pending_tab_switch_selection = me
                    .model
                    .as_ref(ctx)
                    .selected_item()
                    .map(HistoryItemIdentity::from_item);
                me.rerun_query(ctx);
            }
        });

        let suggestions_mode_model = input_suggestions_model.clone();
        ctx.subscribe_to_model(&mixer, move |me, _, event, ctx| {
            let SearchMixerEvent::ResultsChanged = event;
            if !suggestions_mode_model.as_ref(ctx).is_inline_history_menu() {
                return;
            }
            // Only tab switches stash a pending selection to restore after the
            // query reruns. For all other result updates, keep the inline menu's
            // default selection behavior.
            let Some(selection) = me.pending_tab_switch_selection.take() else {
                return;
            };
            me.menu_view.update(ctx, |menu, ctx| {
                menu.select_last_where(|item| selection.matches(item), ctx);
            });
        });

        Self {
            menu_view,
            mixer,
            model,
            buffer_model,
            pending_tab_switch_selection: None,
            caller_supplied_tabs,
            pending_initial_buffer_sync: false,
        }
    }

    /// Returns the model handle for external use (e.g., by message bars).
    pub fn model(&self) -> &ModelHandle<InlineMenuModel<AcceptHistoryItem, HistoryTab>> {
        &self.model
    }

    pub fn render_results_only(&self, app: &AppContext) -> Box<dyn Element> {
        self.menu_view.as_ref(app).render_results_only(
            /* should_render_results_in_reverse */ false, /* horizontal_padding */ 0.,
            app,
        )
    }

    pub fn result_count(&self, app: &AppContext) -> usize {
        self.menu_view.as_ref(app).result_count()
    }

    pub fn select_up(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_up(ctx));
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
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

    pub fn select_next_tab(&self, ctx: &mut ViewContext<Self>) -> bool {
        self.menu_view.update(ctx, |v, ctx| v.select_next_tab(ctx))
    }

    pub fn accept_selected_item(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |v, ctx| v.accept_selected_item(false, ctx));
    }

    pub fn arm_initial_buffer_sync(&mut self) {
        self.pending_initial_buffer_sync = true;
    }

    fn open_with_current_buffer(&mut self, ctx: &mut ViewContext<Self>) {
        let query_text = self.buffer_model.as_ref(ctx).current_value().to_owned();
        let filters = self.model.as_ref(ctx).active_tab_filters();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: query_text,
                    filters,
                },
                ctx,
            );
        });
    }

    fn rerun_query(&self, ctx: &mut ViewContext<Self>) {
        let filters = self.model.as_ref(ctx).active_tab_filters();
        let query_text = self.current_query_text(ctx);
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: query_text,
                    filters,
                },
                ctx,
            );
        });
    }

    fn current_query_text(&self, ctx: &ViewContext<Self>) -> String {
        // Read the query text from the mixer rather than the editor buffer. While the
        // inline history menu is open, moving the highlighted row can preview that
        // item in the editor buffer. When we rerun the search after a tab change, we
        // want to preserve the user's typed query, not the temporary preview text.
        self.mixer
            .as_ref(ctx)
            .current_query()
            .map(|q| q.text.clone())
            .unwrap_or_default()
    }
}

impl View for InlineHistoryMenuView {
    fn ui_name() -> &'static str {
        "InlineHistoryMenuView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineHistoryMenuView {
    type Event = InlineHistoryMenuEvent;
}
