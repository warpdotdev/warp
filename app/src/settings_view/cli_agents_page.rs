//! The "Third party CLI agents" settings page, shown under the Agents umbrella.
//!
//! Everything on this page controls third-party coding agents (Claude Code,
//! Codex, Gemini CLI) rather than Warp's own AI, so its settings are always
//! interactive regardless of the global AI toggle.

use std::cell::RefCell;
use std::collections::HashMap;

use enum_iterator::all;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use regex::Regex;
use settings::{Setting, ToggleableSetting};
use warp_core::features::FeatureFlag;
use warp_errors::report_if_error;
use warpui::elements::{
    ChildView, Container, CornerRadius, CrossAxisAlignment, Element, Empty, Flex,
    FormattedTextElement, HighlightedHyperlink, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Shrinkable,
};
use warpui::keymap::ContextPredicate;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    Action, AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle, id,
};

use super::ai_shared::{
    render_ai_feature_switch, render_ai_setting_toggle, render_toolbar_layout_editor, styles,
    update_editor_interaction_state,
};
use super::settings_page::{
    AdditionalInfo, CONTENT_FONT_SIZE, LocalOnlyIconState, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget, ToggleState, build_toggle_element,
    render_body_item_label,
};
use super::{SettingsAction, SettingsSection, ToggleSettingActionPair, flags};
use crate::ai::blocklist::agent_view::agent_input_footer::editor::{
    AgentToolbarEditorMode, AgentToolbarInlineEditor,
};
use crate::appearance::Appearance;
use crate::menu::{MenuItem, MenuItemFields};
use crate::settings::{
    AISettings, AISettingsChangedEvent, AutoDismissRichInputAfterSubmit,
    AutoOpenRichInputOnCLIAgentStart, AutoToggleRichInput, ShouldRenderCLIAgentToolbar,
    SubmitRichInputOnCtrlEnter,
};
use crate::terminal::CLIAgent;
use crate::util::bindings;
use crate::view_components::dropdown::DropdownAction;
use crate::view_components::{Dropdown, SubmittableTextInput, SubmittableTextInputEvent};
use crate::{TelemetryEvent, send_telemetry_from_ctx};

const PAGE_TITLE: &str = "Third party CLI agents";

pub struct CLIAgentsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    cli_agent_footer_command_editor: ViewHandle<SubmittableTextInput>,
    cli_agent_footer_command_mouse_state_handles: Vec<MouseStateHandle>,
    cli_agent_footer_command_agent_dropdowns: Vec<ViewHandle<Dropdown<CLIAgentsPageAction>>>,
    cli_agent_toolbar_inline_editor: ViewHandle<AgentToolbarInlineEditor>,
}

impl CLIAgentsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cli_agent_footer_command_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text("command (supports regex)", ctx);
            input
        });
        update_editor_interaction_state(
            cli_agent_footer_command_editor.as_ref(ctx).editor().clone(),
            true,
            ctx,
        );
        ctx.subscribe_to_view(
            &cli_agent_footer_command_editor,
            |_, _, event, ctx| match event {
                SubmittableTextInputEvent::Submit(command) => {
                    AISettings::handle(ctx).update(ctx, |settings, ctx| {
                        settings.add_cli_agent_footer_enabled_command(command, ctx);
                    });
                }
                SubmittableTextInputEvent::Escape => ctx.emit(CLIAgentsPageEvent::FocusModal),
            },
        );

        let cli_agent_footer_command_mouse_state_handles = AISettings::as_ref(ctx)
            .cli_agent_footer_enabled_commands
            .value()
            .keys()
            .map(|_| Default::default())
            .collect();

        let cli_agent_toolbar_inline_editor = ctx.add_typed_action_view(|ctx| {
            AgentToolbarInlineEditor::new(AgentToolbarEditorMode::CLIAgent, ctx)
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            // Adding or removing a command changes the length of the command
            // list, so both the per-row mouse states and the per-row agent
            // dropdowns have to be rebuilt to stay index-aligned with it.
            if let AISettingsChangedEvent::CLIAgentToolbarEnabledCommands { .. } = event {
                me.cli_agent_footer_command_mouse_state_handles = AISettings::as_ref(ctx)
                    .cli_agent_footer_enabled_commands
                    .value()
                    .keys()
                    .map(|_| Default::default())
                    .collect();
                me.cli_agent_footer_command_agent_dropdowns = Self::create_cli_agent_dropdowns(ctx);
            }
            ctx.notify();
        });

        Self {
            page: Self::build_page(),
            local_only_icon_tooltip_states: Default::default(),
            cli_agent_footer_command_editor,
            cli_agent_footer_command_mouse_state_handles,
            cli_agent_footer_command_agent_dropdowns: Self::create_cli_agent_dropdowns(ctx),
            cli_agent_toolbar_inline_editor,
        }
    }

    fn build_page() -> PageType<Self> {
        let widgets: Vec<Box<dyn SettingsWidget<View = Self>>> = vec![
            Box::new(CLIAgentWidget::default()),
            Box::new(CLIAgentAutoToggleRichInputWidget::default()),
            Box::new(CLIAgentAutoOpenRichInputWidget::default()),
            Box::new(CLIAgentAutoDismissRichInputWidget::default()),
            Box::new(CLIAgentSubmitRichInputWidget::default()),
            Box::new(CLIAgentCommandsWidget),
            Box::new(CLIAgentToolbarLayoutWidget),
        ];
        PageType::new_uncategorized(widgets, Some(PAGE_TITLE))
    }

    fn create_cli_agent_dropdowns(
        ctx: &mut ViewContext<Self>,
    ) -> Vec<ViewHandle<Dropdown<CLIAgentsPageAction>>> {
        let entries: Vec<(String, CLIAgent)> = AISettings::as_ref(ctx)
            .cli_agent_footer_enabled_commands
            .value()
            .iter()
            .map(|(pattern, agent_value)| {
                (pattern.clone(), CLIAgent::from_serialized_name(agent_value))
            })
            .collect();

        entries
            .into_iter()
            .map(|(pattern_clone, current_agent)| {
                ctx.add_typed_action_view(move |ctx| {
                    let mut dropdown = Dropdown::new(ctx);
                    dropdown.set_top_bar_max_width(160.);
                    dropdown.set_menu_width(180., ctx);
                    dropdown.set_main_axis_size(MainAxisSize::Min, ctx);

                    let mut items: Vec<MenuItem<DropdownAction>> = Vec::new();

                    for agent in all::<CLIAgent>() {
                        if matches!(agent, CLIAgent::Unknown) {
                            continue;
                        }
                        let icon = agent.icon();
                        let mut fields = MenuItemFields::new(agent.display_name())
                            .with_on_select_action(DropdownAction::select_action_and_close(
                                CLIAgentsPageAction::SetCLIAgentForCommand {
                                    pattern: pattern_clone.clone(),
                                    agent: Some(agent),
                                },
                            ));
                        if let Some(icon) = icon {
                            fields = fields.with_icon(icon);
                        }
                        items.push(fields.into_item());
                    }

                    items.push(
                        MenuItemFields::new("Other")
                            .with_on_select_action(DropdownAction::select_action_and_close(
                                CLIAgentsPageAction::SetCLIAgentForCommand {
                                    pattern: pattern_clone.clone(),
                                    agent: None,
                                },
                            ))
                            .into_item(),
                    );

                    dropdown.set_rich_items(items, ctx);

                    dropdown.set_menu_header_text_override(|label| {
                        if label == "Other" {
                            "Select coding agent".to_string()
                        } else {
                            label.to_string()
                        }
                    });

                    let selected_name = if matches!(current_agent, CLIAgent::Unknown) {
                        "Other"
                    } else {
                        current_agent.display_name()
                    };
                    dropdown.set_selected_by_name(selected_name, ctx);

                    dropdown
                })
            })
            .collect()
    }
}

impl Entity for CLIAgentsPageView {
    type Event = CLIAgentsPageEvent;
}

impl View for CLIAgentsPageView {
    fn ui_name() -> &'static str {
        "CLIAgentsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

pub enum CLIAgentsPageEvent {
    FocusModal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CLIAgentsPageAction {
    ToggleCLIAgentToolbar,
    ToggleAutoToggleRichInput,
    ToggleAutoOpenRichInputOnCLIAgentStart,
    ToggleAutoDismissRichInputAfterSubmit,
    ToggleSubmitRichInputOnCtrlEnter,
    RemoveCLIAgentToolbarEnabledCommand(String),
    SetCLIAgentForCommand {
        pattern: String,
        agent: Option<CLIAgent>,
    },
}

impl TypedActionView for CLIAgentsPageView {
    type Action = CLIAgentsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CLIAgentsPageAction::ToggleCLIAgentToolbar => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .should_render_cli_agent_footer
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleCLIAgentToolbarSetting {
                                is_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for CLI Agent Footer setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            CLIAgentsPageAction::ToggleAutoToggleRichInput => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.auto_toggle_rich_input.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            CLIAgentsPageAction::ToggleAutoOpenRichInputOnCLIAgentStart => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .auto_open_rich_input_on_cli_agent_start
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            CLIAgentsPageAction::ToggleAutoDismissRichInputAfterSubmit => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .auto_dismiss_rich_input_after_submit
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            CLIAgentsPageAction::ToggleSubmitRichInputOnCtrlEnter => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.submit_on_ctrl_enter.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            CLIAgentsPageAction::RemoveCLIAgentToolbarEnabledCommand(command) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.remove_cli_agent_footer_enabled_command(command, ctx);
                });
            }
            CLIAgentsPageAction::SetCLIAgentForCommand { pattern, agent } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.set_cli_agent_for_command(pattern, *agent, ctx);
                });
            }
        }
    }
}

impl SettingsPageMeta for CLIAgentsPageView {
    fn section() -> SettingsSection {
        SettingsSection::ThirdPartyCLIAgents
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::AgentMode.is_enabled()
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<CLIAgentsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<CLIAgentsPageView>) -> Self {
        SettingsPageViewHandle::CLIAgents(view_handle)
    }
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "coding agent toolbar",
                builder(SettingsAction::CLIAgents(
                    CLIAgentsPageAction::ToggleCLIAgentToolbar,
                )),
                context,
                flags::CLI_AGENT_FOOTER_ENABLED,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "auto show or hide Rich Input based on agent status",
                builder(SettingsAction::CLIAgents(
                    CLIAgentsPageAction::ToggleAutoToggleRichInput,
                )),
                &(context.clone() & id!(flags::CLI_AGENT_FOOTER_ENABLED)),
                flags::AUTO_TOGGLE_RICH_INPUT_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::CLIAgentRichInput.is_enabled()),
            ToggleSettingActionPair::new(
                "auto open Rich Input when a coding agent session starts",
                builder(SettingsAction::CLIAgents(
                    CLIAgentsPageAction::ToggleAutoOpenRichInputOnCLIAgentStart,
                )),
                &(context.clone() & id!(flags::CLI_AGENT_FOOTER_ENABLED)),
                flags::AUTO_OPEN_RICH_INPUT_ON_CLI_AGENT_START_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::CLIAgentRichInput.is_enabled()),
            ToggleSettingActionPair::new(
                "auto dismiss Rich Input after prompt submission",
                builder(SettingsAction::CLIAgents(
                    CLIAgentsPageAction::ToggleAutoDismissRichInputAfterSubmit,
                )),
                &(context.clone() & id!(flags::CLI_AGENT_FOOTER_ENABLED)),
                flags::AUTO_DISMISS_RICH_INPUT_AFTER_SUBMIT_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::CLIAgentRichInput.is_enabled()),
        ],
        app,
    );
}

/// Widget id backing the `cli_agents` deeplink slug. Lives here alongside the
/// widget itself because the default `widget_id()` is the type's full path,
/// which changes whenever the widget moves modules.
#[cfg(not(target_family = "wasm"))]
pub fn cli_agent_settings_widget_id() -> &'static str {
    CLIAgentWidget::static_widget_id()
}

#[derive(Default)]
struct CLIAgentWidget {
    cli_agent_footer_toggle: SwitchStateHandle,
}

impl SettingsWidget for CLIAgentWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent claude codex gemini toolbar footer quick actions show"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);

        let cli_agent_footer_toggle = render_ai_setting_toggle::<ShouldRenderCLIAgentToolbar>(
            "Show coding agent toolbar",
            CLIAgentsPageAction::ToggleCLIAgentToolbar,
            *ai_settings.should_render_cli_agent_footer,
            true,
            self.cli_agent_footer_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description_fragments = vec![
            FormattedTextFragment::plain_text(
                "Show a toolbar with quick actions when running coding agents like ",
            ),
            FormattedTextFragment::inline_code("claude"),
            FormattedTextFragment::plain_text(", "),
            FormattedTextFragment::inline_code("codex"),
            FormattedTextFragment::plain_text(", or "),
            FormattedTextFragment::inline_code("gemini"),
            FormattedTextFragment::plain_text("."),
        ];

        let description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(description_fragments)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            styles::description_font_color(true, app).into(),
            HighlightedHyperlink::default(),
        );

        Flex::column()
            .with_child(cli_agent_footer_toggle)
            .with_child(
                Container::new(description.finish())
                    .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                    .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                    .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                    .finish(),
            )
            .finish()
    }
}

fn should_render_cli_agent_detail(app: &AppContext) -> bool {
    *AISettings::as_ref(app).should_render_cli_agent_footer
}

fn should_render_cli_agent_rich_input(app: &AppContext) -> bool {
    should_render_cli_agent_detail(app) && FeatureFlag::CLIAgentRichInput.is_enabled()
}

#[derive(Default)]
struct CLIAgentAutoToggleRichInputWidget {
    toggle: SwitchStateHandle,
    info_tooltip: MouseStateHandle,
}

impl SettingsWidget for CLIAgentAutoToggleRichInputWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent rich input auto show hide status plugin"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        should_render_cli_agent_rich_input(app)
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        let label = render_body_item_label::<CLIAgentsPageAction>(
            "Auto show/hide Rich Input based on agent status".into(),
            Some(styles::header_font_color(true, app)),
            Some(AdditionalInfo {
                mouse_state: self.info_tooltip.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    "Requires the Warp plugin for your coding agent".to_owned(),
                ),
            }),
            LocalOnlyIconState::for_setting(
                AutoToggleRichInput::storage_key(),
                AutoToggleRichInput::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
        );

        build_toggle_element(
            label,
            render_ai_feature_switch(
                self.toggle.clone(),
                *AISettings::as_ref(app).auto_toggle_rich_input,
                true,
                CLIAgentsPageAction::ToggleAutoToggleRichInput,
                app,
            ),
            appearance,
            None,
        )
    }
}

#[derive(Default)]
struct CLIAgentAutoOpenRichInputWidget {
    toggle: SwitchStateHandle,
}

impl SettingsWidget for CLIAgentAutoOpenRichInputWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent rich input auto open session start"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        should_render_cli_agent_rich_input(app)
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        render_ai_setting_toggle::<AutoOpenRichInputOnCLIAgentStart>(
            "Auto open Rich Input when a coding agent session starts",
            CLIAgentsPageAction::ToggleAutoOpenRichInputOnCLIAgentStart,
            *AISettings::as_ref(app).auto_open_rich_input_on_cli_agent_start,
            true,
            self.toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        )
    }
}

#[derive(Default)]
struct CLIAgentAutoDismissRichInputWidget {
    toggle: SwitchStateHandle,
}

impl SettingsWidget for CLIAgentAutoDismissRichInputWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent rich input auto dismiss prompt submission"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        should_render_cli_agent_rich_input(app)
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        render_ai_setting_toggle::<AutoDismissRichInputAfterSubmit>(
            "Auto dismiss Rich Input after prompt submission",
            CLIAgentsPageAction::ToggleAutoDismissRichInputAfterSubmit,
            *AISettings::as_ref(app).auto_dismiss_rich_input_after_submit,
            true,
            self.toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        )
    }
}

#[derive(Default)]
struct CLIAgentSubmitRichInputWidget {
    toggle: SwitchStateHandle,
}

impl SettingsWidget for CLIAgentSubmitRichInputWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent rich input submit ctrl enter newline"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        should_render_cli_agent_rich_input(app)
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        render_ai_setting_toggle::<SubmitRichInputOnCtrlEnter>(
            "Submit Rich Input with Ctrl+Enter",
            CLIAgentsPageAction::ToggleSubmitRichInputOnCtrlEnter,
            *AISettings::as_ref(app).submit_on_ctrl_enter,
            true,
            self.toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        )
    }
}

struct CLIAgentCommandsWidget;

impl SettingsWidget for CLIAgentCommandsWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent claude codex gemini toolbar commands regex patterns"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        should_render_cli_agent_detail(app)
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        let mut list_column = Flex::column();
        list_column.add_child(
            appearance
                .ui_builder()
                .span("Commands that enable the toolbar".to_string())
                .with_style(UiComponentStyles {
                    font_size: Some(CONTENT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        list_column.add_child(ChildView::new(&view.cli_agent_footer_command_editor).finish());

        let background = appearance.theme().surface_1();
        let font_color = appearance.theme().foreground();
        let items: Vec<_> = AISettings::as_ref(app)
            .cli_agent_footer_enabled_commands
            .value()
            .keys()
            .cloned()
            .collect();
        let len = items.len();
        for (rev_i, pattern) in items.iter().rev().enumerate() {
            let original_i = len - 1 - rev_i;
            let remove_action =
                CLIAgentsPageAction::RemoveCLIAgentToolbarEnabledCommand(pattern.clone());
            let mouse_state = view
                .cli_agent_footer_command_mouse_state_handles
                .get(original_i)
                .cloned()
                .unwrap_or_default();

            let remove_button = appearance
                .ui_builder()
                .close_button(16., mouse_state)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(remove_action.clone());
                })
                .finish();

            let label = appearance
                .ui_builder()
                .wrappable_text(pattern.clone(), true)
                .with_style(UiComponentStyles {
                    font_color: Some(font_color.into_solid()),
                    font_family_id: Some(appearance.monospace_font_family()),
                    font_size: Some(appearance.ui_font_size()),
                    ..Default::default()
                })
                .build()
                .finish();

            let mut right_side = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            if let Some(dropdown_handle) = view
                .cli_agent_footer_command_agent_dropdowns
                .get(original_i)
            {
                right_side.add_child(
                    Container::new(ChildView::new(dropdown_handle).finish())
                        .with_margin_right(8.)
                        .finish(),
                );
            }
            right_side.add_child(remove_button);

            let row = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_children([Shrinkable::new(1., label).finish(), right_side.finish()])
                    .finish(),
            )
            .with_background(background)
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_bottom(4.)
            .finish();

            list_column.add_child(row);
        }

        let description = appearance
            .ui_builder()
            .paragraph("Add regex patterns to show the coding agent toolbar for matching commands.")
            .with_style(UiComponentStyles {
                font_size: Some(appearance.ui_font_size()),
                font_color: Some(styles::description_font_color(true, app).into()),
                margin: Some(
                    Coords::default()
                        .top(4.)
                        .bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                        .right(styles::TOGGLE_WIDTH_MARGIN),
                ),
                ..Default::default()
            })
            .build()
            .finish();

        Flex::column()
            .with_child(list_column.finish())
            .with_child(description)
            .finish()
    }
}

struct CLIAgentToolbarLayoutWidget;

impl SettingsWidget for CLIAgentToolbarLayoutWidget {
    type View = CLIAgentsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent toolbar layout chip chips rearrange re-arrange"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        should_render_cli_agent_detail(app) && FeatureFlag::AgentToolbarEditor.is_enabled()
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        render_toolbar_layout_editor(&view.cli_agent_toolbar_inline_editor, appearance)
    }
}
