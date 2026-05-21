use std::collections::HashSet;
use std::sync::LazyLock;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{ChildView, MainAxisSize};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity as _, View, ViewContext,
    ViewHandle,
};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::block::cli_controller::{CLISubagentController, CLISubagentEvent};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::llms::{LLMId, LLMPreferences, LLMPreferencesEvent};
use crate::features::FeatureFlag;
use crate::search::data_source::{Query, QueryFilter};
use crate::search::mixer::{SearchMixer, SearchMixerEvent};
use crate::settings_view::SettingsSection;
use crate::terminal::input::buffer_model::InputBufferModel;
use crate::terminal::input::inline_menu::{
    InlineMenuEvent, InlineMenuHeaderConfig, InlineMenuModel, InlineMenuPositioner,
    InlineMenuTabConfig, InlineMenuView,
};
use crate::terminal::input::models::data_source::{AcceptModel, ModelSelectorDataSource};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ActionButtonTheme, ButtonSize};
use crate::view_components::alert::{Alert, AlertConfig};
use crate::workspace::WorkspaceAction;
use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent};

struct ManageDefaultsTheme;

impl ActionButtonTheme for ManageDefaultsTheme {
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
pub enum InlineModelSelectorTab {
    BaseAgent,
    FullTerminalUse,
}

#[derive(Debug, Clone)]
pub enum InlineModelSelectorEvent {
    SelectedModel {
        id: LLMId,
        selected_tab: InlineModelSelectorTab,
        set_as_default: bool,
    },
    Dismissed,
}

static TAB_CONFIGS: LazyLock<Vec<InlineMenuTabConfig<InlineModelSelectorTab>>> =
    LazyLock::new(|| {
        let mut configs = vec![InlineMenuTabConfig {
            id: InlineModelSelectorTab::BaseAgent,
            label: "Base".to_string(),
            filters: HashSet::from([QueryFilter::BaseModels]),
        }];
        if FeatureFlag::InlineMenuHeaders.is_enabled() {
            configs.push(InlineMenuTabConfig {
                id: InlineModelSelectorTab::FullTerminalUse,
                label: "Full Terminal Use".to_string(),
                filters: HashSet::from([QueryFilter::FullTerminalUseModels]),
            });
        }
        configs
    });

struct TabSwitchSelection {
    model_id: Option<LLMId>,
    index: Option<usize>,
}

pub struct InlineModelSelectorView {
    menu_view: ViewHandle<InlineMenuView<AcceptModel, InlineModelSelectorTab>>,
    mixer: ModelHandle<SearchMixer<AcceptModel>>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
    terminal_view_id: EntityId,
    /// Stashed selection state to restore after a tab switch requery.
    selection_before_tab_switch: Option<TabSwitchSelection>,
    /// Controls whether or not we should filter the contents of the menu
    /// based on the contents of the input.
    filter_results_by_input: bool,
}

impl InlineModelSelectorView {
    pub fn new(
        terminal_view_id: EntityId,
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        input_buffer_model: &ModelHandle<InputBufferModel>,
        cli_subagent_controller: ModelHandle<CLISubagentController>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let data_source = ctx.add_model(|_| ModelSelectorDataSource::new(terminal_view_id));

        let tab_configs = TAB_CONFIGS.clone();
        let initial_filters = tab_configs
            .first()
            .map(|config| config.filters.clone())
            .unwrap_or_default();
        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptModel>::new();
            mixer.add_sync_source(
                data_source.clone(),
                [QueryFilter::BaseModels, QueryFilter::FullTerminalUseModels],
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
            let manage_defaults_button = ctx.add_view(|ctx| {
                ActionButton::new(
                    crate::i18n::tr_static(ctx, "Manage defaults"),
                    ManageDefaultsTheme,
                )
                .with_icon(Icon::Settings)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPageWithSearch {
                        search_query: String::new(),
                        section: Some(SettingsSection::WarpAgent),
                    });
                })
            });
            let header_config = InlineMenuHeaderConfig {
                label: "/model".to_string(),
                trailing_element: Some(Box::new(move |_app: &AppContext| {
                    ChildView::new(&manage_defaults_button).finish()
                })),
            };

            ctx.add_typed_action_view(|ctx| {
                let menu = InlineMenuView::new_with_tabs(
                    mixer.clone(),
                    positioner.clone(),
                    &suggestions_mode_model,
                    agent_view_controller,
                    tab_configs,
                    None,
                    ctx,
                )
                .with_header_config(header_config);

                let menu_model = menu.model().clone();
                let cli_ctrl = cli_subagent_controller.clone();
                menu.with_banner_fn(move |app| {
                    let active_tab = menu_model
                        .as_ref(app)
                        .active_tab_id()
                        .unwrap_or(InlineModelSelectorTab::BaseAgent);
                    let history = BlocklistAIHistoryModel::as_ref(app);

                    let main_agent_in_progress = history
                        .active_conversation(terminal_view_id)
                        .is_some_and(|c| !c.is_empty() && c.status().is_in_progress());
                    let is_cli_agent_in_control_or_tagged_in =
                        cli_ctrl.as_ref(app).is_agent_in_control_or_tagged_in();
                    let message = match active_tab {
                        InlineModelSelectorTab::FullTerminalUse if main_agent_in_progress && !is_cli_agent_in_control_or_tagged_in => {
                            Some("You're using the base agent. Full terminal use models only apply to the full terminal use agent.")
                        }
                        InlineModelSelectorTab::BaseAgent if is_cli_agent_in_control_or_tagged_in => {
                            Some("You're using the full terminal use agent. Base models only apply to the base agent.")
                        }
                        _ => None,
                    };

                    message.map(|msg| {
                        let appearance = Appearance::as_ref(app);
                        Alert::new().render(
                            AlertConfig::warning(msg.to_string())
                                .with_main_axis_size(MainAxisSize::Max),
                            appearance,
                        )
                    })
                })
            })
        } else {
            ctx.add_typed_action_view(|ctx| {
                InlineMenuView::new_with_tabs(
                    mixer.clone(),
                    positioner.clone(),
                    &suggestions_mode_model,
                    agent_view_controller,
                    tab_configs,
                    None,
                    ctx,
                )
            })
        };

        ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem {
                item,
                cmd_or_ctrl_shift_enter,
            } => {
                ctx.emit(InlineModelSelectorEvent::SelectedModel {
                    id: item.id.clone(),
                    selected_tab: me.active_tab(ctx),
                    set_as_default: *cmd_or_ctrl_shift_enter,
                });
            }
            InlineMenuEvent::Dismissed => {
                ctx.emit(InlineModelSelectorEvent::Dismissed);
            }
            InlineMenuEvent::TabChanged => {
                me.selection_before_tab_switch = Some(TabSwitchSelection {
                    model_id: me
                        .menu_model(ctx)
                        .selected_item()
                        .map(|item| item.id.clone()),
                    index: me.menu_view.as_ref(ctx).selected_idx(),
                });
                me.rerun_query(ctx);
            }
            InlineMenuEvent::SelectedItem { .. } | InlineMenuEvent::NoResults => {}
        });

        ctx.subscribe_to_model(&suggestions_mode_model, |me, model, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if model.as_ref(ctx).is_inline_model_selector() {
                me.rerun_query(ctx);
            } else if model.as_ref(ctx).is_closed() {
                me.filter_results_by_input = true;
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.reset_results(ctx);
                });
            }
        });

        ctx.subscribe_to_model(input_buffer_model, |me, _, _, ctx| {
            if !me
                .suggestions_mode_model
                .as_ref(ctx)
                .is_inline_model_selector()
            {
                return;
            }

            if !me.filter_results_by_input {
                // If the user clears the buffer, we should re-enable filtering so that
                // any fresh typing acts as a search query.
                if me.input_buffer_model.as_ref(ctx).current_value().is_empty() {
                    me.filter_results_by_input = true;
                    me.rerun_query(ctx);
                }
                return;
            }

            me.rerun_query(ctx);
        });

        ctx.subscribe_to_model(
            &LLMPreferences::handle(ctx),
            |me, _, event, ctx| match event {
                LLMPreferencesEvent::UpdatedAvailableLLMs
                | LLMPreferencesEvent::UpdatedActiveAgentModeLLM
                    if me
                        .suggestions_mode_model
                        .as_ref(ctx)
                        .is_inline_model_selector() =>
                {
                    me.mixer.update(ctx, |mixer, ctx| {
                        if let Some(query) = mixer.current_query().cloned() {
                            mixer.run_query(query, ctx);
                        }
                    });
                }
                _ => (),
            },
        );
        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), |me, _, event, ctx| {
            if !matches!(event, ApiKeyManagerEvent::KeysUpdated) {
                return;
            }
            if me
                .suggestions_mode_model
                .as_ref(ctx)
                .is_inline_model_selector()
            {
                me.mixer.update(ctx, |mixer, ctx| {
                    if let Some(query) = mixer.current_query().cloned() {
                        mixer.run_query(query, ctx);
                    }
                });
            }
        });

        ctx.subscribe_to_model(&cli_subagent_controller, |me, _, event, ctx| match event {
            CLISubagentEvent::SpawnedSubagent { .. }
            | CLISubagentEvent::FinishedSubagent { .. }
            | CLISubagentEvent::UpdatedControl { .. } => {
                me.menu_view.update(ctx, |_, ctx| ctx.notify());
            }
            CLISubagentEvent::UpdatedLastSnapshot
            | CLISubagentEvent::ToggledHideResponses
            | CLISubagentEvent::ControlHandedBackAfterTransfer => {}
        });

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |me, _, event, ctx| {
                if let BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    terminal_view_id: event_terminal_view_id,
                    ..
                } = event
                {
                    if *event_terminal_view_id == terminal_view_id {
                        me.menu_view.update(ctx, |_, ctx| ctx.notify());
                    }
                }
            },
        );

        ctx.subscribe_to_model(&mixer, |me, _, event, ctx| {
            let SearchMixerEvent::ResultsChanged = event;

            if !me
                .suggestions_mode_model
                .as_ref(ctx)
                .is_inline_model_selector()
            {
                return;
            }

            // On tab switch, try to preserve the previously selected model.
            // If the model isn't present in the new tab's results, fall back
            // to the same index position.
            if let Some(selection) = me.selection_before_tab_switch.take() {
                let found_by_id = selection.model_id.is_some_and(|id| {
                    me.menu_view.update(ctx, |menu, ctx| {
                        menu.select_first_where(|item| item.id == id, ctx)
                    })
                });
                if !found_by_id {
                    if let Some(idx) = selection.index {
                        let count = me.menu_view.as_ref(ctx).result_count();
                        if count > 0 {
                            me.menu_view.update(ctx, |menu, ctx| {
                                menu.select_idx(idx.min(count - 1), ctx);
                            });
                        }
                    }
                }
                return;
            }

            // If the user is actively filtering, don't override their selection.
            if me.filter_results_by_input
                && !me.input_buffer_model.as_ref(ctx).current_value().is_empty()
            {
                return;
            }

            let active_id = me.active_model_id_for_current_tab(ctx);

            me.menu_view.update(ctx, |menu, ctx| {
                menu.select_first_where(|item| item.id == active_id, ctx);
            });
        });

        Self {
            menu_view,
            mixer,
            suggestions_mode_model,
            input_buffer_model: input_buffer_model.clone(),
            terminal_view_id,
            selection_before_tab_switch: None,
            filter_results_by_input: true,
        }
    }

    fn menu_model<'a>(
        &self,
        ctx: &'a ViewContext<Self>,
    ) -> &'a InlineMenuModel<AcceptModel, InlineModelSelectorTab> {
        self.menu_view.as_ref(ctx).model().as_ref(ctx)
    }

    fn active_tab(&self, ctx: &ViewContext<Self>) -> InlineModelSelectorTab {
        self.menu_model(ctx)
            .active_tab_id()
            .unwrap_or(InlineModelSelectorTab::BaseAgent)
    }

    fn active_model_id_for_current_tab(&self, ctx: &ViewContext<Self>) -> LLMId {
        let llm_preferences = LLMPreferences::as_ref(ctx);
        match self.active_tab(ctx) {
            InlineModelSelectorTab::BaseAgent => llm_preferences
                .get_active_base_model(ctx, Some(self.terminal_view_id))
                .id
                .clone(),
            InlineModelSelectorTab::FullTerminalUse => llm_preferences
                .get_active_cli_agent_model(ctx, Some(self.terminal_view_id))
                .id
                .clone(),
        }
    }

    fn rerun_query(&self, ctx: &mut ViewContext<Self>) {
        let filters = self.menu_model(ctx).active_tab_filters();
        let text = if self.filter_results_by_input {
            self.input_buffer_model
                .as_ref(ctx)
                .current_value()
                .to_owned()
        } else {
            String::new()
        };
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(Query { text, filters }, ctx);
        });
    }

    pub fn filter_results_by_input(&self) -> bool {
        self.filter_results_by_input
    }

    pub fn set_filter_results_by_input(&mut self, filter: bool) {
        self.filter_results_by_input = filter;
    }

    pub fn set_active_tab(&self, tab: InlineModelSelectorTab, ctx: &mut ViewContext<Self>) {
        let index = self
            .menu_view
            .as_ref(ctx)
            .model()
            .as_ref(ctx)
            .tab_configs()
            .iter()
            .position(|config| config.id == tab);
        if let Some(index) = index {
            self.menu_view
                .update(ctx, |v, ctx| v.set_active_tab(index, ctx));
        }
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

    pub fn accept_selected_item(&self, cmd_or_ctrl_enter: bool, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |v, ctx| v.accept_selected_item(cmd_or_ctrl_enter, ctx));
    }
}

impl View for InlineModelSelectorView {
    fn ui_name() -> &'static str {
        "InlineModelSelectorView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineModelSelectorView {
    type Event = InlineModelSelectorEvent;
}
