use std::path::PathBuf;

use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Shrinkable, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::profiles::{
    AIExecutionProfilesModel, AIExecutionProfilesModelEvent, ClientProfileId,
};
use crate::ai::execution_profiles::{
    ActionPermission, AskUserQuestionPermission, RunAgentsPermission, WriteToPtyPermission,
};
use crate::ai::llms::LLMPreferences;
use crate::appearance::Appearance;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};
use crate::TemplatableMCPServerManager;

#[derive(Debug, Clone)]
pub enum ExecutionProfileViewAction {
    EditProfile,
}

pub enum ExecutionProfileViewEvent {
    EditProfile,
}

pub struct ExecutionProfileView {
    profile_id: ClientProfileId,
    edit_button: ViewHandle<ActionButton>,
}

impl ExecutionProfileView {
    pub fn new(profile_id: ClientProfileId, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&AIExecutionProfilesModel::handle(ctx), |me, _, event, ctx| {
            if matches!(event, AIExecutionProfilesModelEvent::ProfileUpdated(profile_id) if *profile_id == me.profile_id) {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |_me, _, _, ctx| {
            ctx.notify();
        });

        let edit_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Edit", SecondaryTheme)
                .with_icon(Icon::Pencil)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ExecutionProfileViewAction::EditProfile);
                })
        });

        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

        edit_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_any_ai_enabled, ctx);
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, _, ctx| {
            let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
            me.edit_button.update(ctx, |button, ctx| {
                button.set_disabled(!is_any_ai_enabled, ctx);
            });
            ctx.notify();
        });

        Self {
            profile_id,
            edit_button,
        }
    }
}

impl Entity for ExecutionProfileView {
    type Event = ExecutionProfileViewEvent;
}

impl View for ExecutionProfileView {
    fn ui_name() -> &'static str {
        "ExecutionProfileView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        let permissions = BlocklistAIPermissions::as_ref(app);
        let profile = permissions.permissions_profile_for_id(app, self.profile_id);

        let llm_preferences = LLMPreferences::as_ref(app);

        let base_model = profile
            .base_model
            .as_ref()
            .and_then(|id| llm_preferences.get_llm_info(id))
            .map(|info| info.display_name.clone())
            .unwrap_or_else(|| {
                llm_preferences
                    .get_default_base_model(app)
                    .display_name
                    .clone()
            });

        let cli_agent_model = profile
            .cli_agent_model
            .as_ref()
            .and_then(|id| llm_preferences.get_llm_info(id))
            .map(|info| info.display_name.clone())
            .unwrap_or_else(|| {
                llm_preferences
                    .get_default_cli_agent_model(app)
                    .display_name
                    .clone()
            });

        let computer_use_model = profile
            .computer_use_model
            .as_ref()
            .and_then(|id| llm_preferences.get_llm_info(id))
            .map(|info| info.display_name.clone())
            .unwrap_or_else(|| {
                llm_preferences
                    .get_default_computer_use_model(app)
                    .display_name
                    .clone()
            });

        Container::new(
            Flex::column()
                .with_child(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Text::new(profile.display_name(), appearance.ui_font_family(), 14.)
                                .with_style(Properties::default().weight(Weight::Medium))
                                .with_color(if is_any_ai_enabled {
                                    appearance.theme().active_ui_text_color().into()
                                } else {
                                    appearance.theme().disabled_ui_text_color().into()
                                })
                                .finish(),
                        )
                        .with_child(self.edit_button.as_ref(app).render(app))
                        .finish(),
                )
                .with_child({
                    let mut model_flex = Flex::column();
                    model_flex.add_child(
                        Container::new(
                            Text::new("MODELS", appearance.ui_font_family(), 10.)
                                .with_color(appearance.theme().disabled_ui_text_color().into())
                                .finish(),
                        )
                        .with_margin_bottom(8.)
                        .finish(),
                    );
                    model_flex.add_child(with_standard_vertical_margin(
                        render_model_line_with_icon(
                            Icon::Lightning,
                            "Base model:",
                            base_model,
                            appearance,
                            is_any_ai_enabled,
                        ),
                    ));
                    model_flex.add_child(with_standard_vertical_margin(
                        render_model_line_with_icon(
                            Icon::Terminal,
                            "Full terminal use:",
                            cli_agent_model,
                            appearance,
                            is_any_ai_enabled,
                        ),
                    ));
                    if FeatureFlag::LocalComputerUse.is_enabled() {
                        model_flex.add_child(with_standard_vertical_margin(
                            render_model_line_with_icon(
                                Icon::Laptop,
                                "Computer use:",
                                computer_use_model,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));
                    }
                    Container::new(model_flex.finish())
                        .with_margin_top(16.)
                        .with_margin_bottom(8.)
                        .finish()
                })
                .with_child(
                    Container::new({
                        let mut permissions_column = Flex::column()
                            .with_child(
                                Container::new(
                                    Text::new("PERMISSIONS", appearance.ui_font_family(), 10.)
                                        .with_color(
                                            appearance.theme().disabled_ui_text_color().into(),
                                        )
                                        .finish(),
                                )
                                .with_margin_bottom(8.)
                                .finish(),
                            )
                            .with_child(with_standard_vertical_margin(
                                render_action_permission_line_with_icon(
                                    Icon::Code2,
                                    "Apply code diffs:",
                                    &profile.apply_code_diffs,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ))
                            .with_child(with_standard_vertical_margin(
                                render_action_permission_line_with_icon(
                                    Icon::Notebook,
                                    "Read files:",
                                    &profile.read_files,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ));

                        if profile.read_files == ActionPermission::AlwaysAsk
                            || profile.read_files == ActionPermission::AgentDecides
                        {
                            permissions_column.add_child(render_directory_allowlist(
                                &profile,
                                appearance,
                                is_any_ai_enabled,
                            ));
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_action_permission_line_with_icon(
                                Icon::Terminal,
                                "Execute commands:",
                                &profile.execute_commands,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        match profile.execute_commands {
                            ActionPermission::AlwaysAllow => {
                                permissions_column.add_child(render_command_denylist(
                                    &profile,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                            }
                            ActionPermission::AlwaysAsk => {
                                permissions_column.add_child(render_command_allowlist(
                                    &profile,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                            }
                            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                                permissions_column.add_child(render_command_allowlist(
                                    &profile,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                                permissions_column.add_child(render_command_denylist(
                                    &profile,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                            }
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_write_to_pty_permission_line_with_icon(
                                Icon::Workflow,
                                "Interact with running commands:",
                                &profile.write_to_pty,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        if FeatureFlag::LocalComputerUse.is_enabled() {
                            permissions_column.add_child(with_standard_vertical_margin(
                                render_computer_use_permission_line_with_icon(
                                    Icon::Laptop,
                                    "Computer use:",
                                    &profile.computer_use,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ));
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_ask_user_question_permission_line_with_icon(
                                Icon::MessageText,
                                "Ask questions:",
                                &profile.ask_user_question,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));
                        permissions_column.add_child(with_standard_vertical_margin(
                            render_run_agents_permission_line_with_icon(
                                Icon::Workflow,
                                "Run agents:",
                                &profile.run_agents,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_action_permission_line_with_icon(
                                Icon::Dataflow,
                                "Call MCP servers:",
                                &profile.mcp_permissions,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        match profile.mcp_permissions {
                            ActionPermission::AlwaysAllow => {
                                permissions_column.add_child(render_mcp_denylist(
                                    &profile,
                                    appearance,
                                    app,
                                    is_any_ai_enabled,
                                ));
                            }
                            ActionPermission::AlwaysAsk => {
                                permissions_column.add_child(render_mcp_allowlist(
                                    &profile,
                                    appearance,
                                    app,
                                    is_any_ai_enabled,
                                ));
                            }
                            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                                permissions_column.add_child(render_mcp_allowlist(
                                    &profile,
                                    appearance,
                                    app,
                                    is_any_ai_enabled,
                                ));
                                permissions_column.add_child(render_mcp_denylist(
                                    &profile,
                                    appearance,
                                    app,
                                    is_any_ai_enabled,
                                ));
                            }
                        }

                        if FeatureFlag::WebSearchUI.is_enabled() {
                            permissions_column.add_child(with_standard_vertical_margin(
                                render_bool_permission_line_with_icon(
                                    Icon::Globe,
                                    "Call web tools:",
                                    profile.web_search_enabled,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ));
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_bool_permission_line_with_icon(
                                Icon::Compass,
                                "Auto-sync plans to Warp Drive:",
                                profile.autosync_plans_to_warp_drive,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        permissions_column.finish()
                    })
                    .with_margin_top(16.)
                    .with_margin_bottom(8.)
                    .finish(),
                )
                .finish(),
        )
        .with_background(appearance.theme().surface_2())
        .with_border(
            warpui::elements::Border::new(1.).with_border_fill(appearance.theme().outline()),
        )
        .with_corner_radius(warpui::elements::CornerRadius::with_all(
            warpui::elements::Radius::Pixels(4.),
        ))
        .with_horizontal_padding(16.)
        .with_vertical_padding(12.)
        .finish()
    }
}

impl TypedActionView for ExecutionProfileView {
    type Action = ExecutionProfileViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ExecutionProfileViewAction::EditProfile => {
                ctx.emit(ExecutionProfileViewEvent::EditProfile);
            }
        }
    }
}

fn render_chips_row<I, S>(
    items: I,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element>
where
    I: IntoIterator<Item = S>,
    S: ToString,
{
    let items_vec: Vec<String> = items.into_iter().map(|item| item.to_string()).collect();
    if items_vec.is_empty() {
        return Container::new(
            Text::new("None", appearance.ui_font_family(), 12.)
                .with_color(appearance.theme().disabled_ui_text_color().into())
                .finish(),
        )
        .finish();
    }
    Wrap::row()
        .with_run_spacing(4.)
        .with_children(
            items_vec
                .into_iter()
                .map(|item| {
                    Container::new(
                        Container::new(
                            Text::new(item, appearance.ui_font_family(), 11.)
                                .with_color(if is_ai_enabled {
                                    appearance.theme().active_ui_text_color().into()
                                } else {
                                    appearance.theme().disabled_ui_text_color().into()
                                })
                                .finish(),
                        )
                        .with_background(appearance.theme().surface_2())
                        .with_border(
                            warpui::elements::Border::all(1.)
                                .with_border_fill(appearance.theme().outline()),
                        )
                        .with_corner_radius(warpui::elements::CornerRadius::with_all(
                            warpui::elements::Radius::Pixels(3.),
                        ))
                        .with_horizontal_padding(6.)
                        .with_vertical_padding(2.)
                        .finish(),
                    )
                    .with_margin_right(4.)
                    .finish()
                })
                .collect::<Vec<_>>(),
        )
        .finish()
}

fn render_allowlist_denylist_row(
    icon: Icon,
    label: String,
    items: &[String],
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        icon.to_warpui_icon(if is_ai_enabled {
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().surface_1())
                        } else {
                            appearance.theme().disabled_ui_text_color()
                        })
                        .finish(),
                    )
                    .with_width(12.)
                    .with_height(12.)
                    .finish(),
                )
                .with_margin_right(6.)
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(label, appearance.ui_font_family(), 12.)
                        .with_color(if is_ai_enabled {
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().surface_1())
                                .into()
                        } else {
                            appearance.theme().disabled_ui_text_color().into()
                        })
                        .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                Shrinkable::new(1., render_chips_row(items, appearance, is_ai_enabled)).finish(),
            )
            .finish(),
    )
    .with_margin_left(8.)
    .with_border(warpui::elements::Border::left(1.).with_border_fill(appearance.theme().outline()))
    .with_padding_left(8.)
    .finish()
}

fn render_pathbuf_allowlist_row(
    icon: Icon,
    label: String,
    items: &[PathBuf],
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let items_str: Vec<String> = items.iter().map(|p| p.display().to_string()).collect();
    render_allowlist_denylist_row(icon, label, &items_str, appearance, is_ai_enabled)
}

fn render_command_predicate_row(
    icon: Icon,
    label: String,
    items: &[crate::settings::AgentModeCommandExecutionPredicate],
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let items_str: Vec<String> = items.iter().map(|c| c.to_string()).collect();
    render_allowlist_denylist_row(icon, label, &items_str, appearance, is_ai_enabled)
}

fn render_mcp_uuid_row(
    icon: Icon,
    label: String,
    uuids: &[Uuid],
    appearance: &Appearance,
    app: &AppContext,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let items_str: Vec<String> = uuids
        .iter()
        .filter_map(|uuid| TemplatableMCPServerManager::get_mcp_name(uuid, app))
        .collect();
    render_allowlist_denylist_row(icon, label, &items_str, appearance, is_ai_enabled)
}

fn with_standard_vertical_margin(element: Box<dyn Element>) -> Box<dyn Element> {
    Container::new(element)
        .with_margin_top(4.)
        .with_margin_bottom(4.)
        .finish()
}

fn render_model_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    model_name: impl Into<String>,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let label = label.into();
    let model_name = model_name.into();

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        icon.to_warpui_icon(if is_ai_enabled {
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().surface_1())
                        } else {
                            appearance.theme().disabled_ui_text_color()
                        })
                        .finish(),
                    )
                    .with_width(12.)
                    .with_height(12.)
                    .finish(),
                )
                .with_margin_right(6.)
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(label, appearance.ui_font_family(), 12.)
                        .with_color(if is_ai_enabled {
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().surface_1())
                                .into()
                        } else {
                            appearance.theme().disabled_ui_text_color().into()
                        })
                        .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                Text::new(model_name, appearance.ui_font_family(), 12.)
                    .with_color(if is_ai_enabled {
                        appearance.theme().active_ui_text_color().into()
                    } else {
                        appearance.theme().disabled_ui_text_color().into()
                    })
                    .finish(),
            )
            .finish(),
    )
    .finish()
}

fn render_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission_text: impl Into<String>,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let label = label.into();
    let permission_text = permission_text.into();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                ConstrainedBox::new(
                    icon.to_warpui_icon(if is_ai_enabled {
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_1())
                    } else {
                        appearance.theme().disabled_ui_text_color()
                    })
                    .finish(),
                )
                .with_width(12.)
                .with_height(12.)
                .finish(),
            )
            .with_margin_right(6.)
            .finish(),
        )
        .with_child(
            Container::new(
                Text::new(label, appearance.ui_font_family(), 12.)
                    .with_color(if is_ai_enabled {
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_1())
                            .into()
                    } else {
                        appearance.theme().disabled_ui_text_color().into()
                    })
                    .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .with_child(
            Text::new(permission_text, appearance.ui_font_family(), 12.)
                .with_color(if is_ai_enabled {
                    appearance.theme().active_ui_text_color().into()
                } else {
                    appearance.theme().disabled_ui_text_color().into()
                })
                .finish(),
        )
        .finish()
}

fn render_action_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &ActionPermission,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = match permission {
        ActionPermission::AgentDecides => "Agent decides",
        ActionPermission::AlwaysAllow => "Always allow",
        ActionPermission::AlwaysAsk => "Always ask",
        ActionPermission::Unknown => "Unknown",
    };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_write_to_pty_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &WriteToPtyPermission,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = match permission {
        WriteToPtyPermission::AlwaysAllow => "Always allow",
        WriteToPtyPermission::AlwaysAsk => "Always ask",
        WriteToPtyPermission::AskOnFirstWrite => "Ask on first write",
        WriteToPtyPermission::Unknown => "Unknown",
    };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_computer_use_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &crate::ai::execution_profiles::ComputerUsePermission,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = match permission {
        crate::ai::execution_profiles::ComputerUsePermission::Never
        | crate::ai::execution_profiles::ComputerUsePermission::Unknown => "Never",
        crate::ai::execution_profiles::ComputerUsePermission::AlwaysAsk => "Always ask",
        crate::ai::execution_profiles::ComputerUsePermission::AlwaysAllow => "Always allow",
    };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_ask_user_question_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &AskUserQuestionPermission,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = match permission {
        AskUserQuestionPermission::Never => "Never ask",
        AskUserQuestionPermission::AskExceptInAutoApprove | AskUserQuestionPermission::Unknown => {
            "Ask unless auto-approve"
        }
        AskUserQuestionPermission::AlwaysAsk => "Always ask",
    };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_run_agents_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &RunAgentsPermission,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = match permission {
        RunAgentsPermission::NeverAllow | RunAgentsPermission::Unknown => "Never",
        RunAgentsPermission::AlwaysAllow => "Always allow",
        RunAgentsPermission::AlwaysAsk => "Always ask",
    };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_bool_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    enabled: bool,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = if enabled { "On" } else { "Off" };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_directory_allowlist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_pathbuf_allowlist_row(
        Icon::Check,
        "Directory allowlist:".to_string(),
        &profile.directory_allowlist,
        appearance,
        is_ai_enabled,
    ))
}

fn render_command_allowlist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_command_predicate_row(
        Icon::Check,
        "Command allowlist:".to_string(),
        &profile.command_allowlist,
        appearance,
        is_ai_enabled,
    ))
}

fn render_command_denylist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_command_predicate_row(
        Icon::SlashCircle,
        "Command denylist:".to_string(),
        &profile.command_denylist,
        appearance,
        is_ai_enabled,
    ))
}

fn render_mcp_allowlist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    appearance: &Appearance,
    app: &AppContext,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_mcp_uuid_row(
        Icon::Check,
        "MCP allowlist:".to_string(),
        &profile.mcp_allowlist,
        appearance,
        app,
        is_ai_enabled,
    ))
}

fn render_mcp_denylist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    appearance: &Appearance,
    app: &AppContext,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_mcp_uuid_row(
        Icon::SlashCircle,
        "MCP denylist:".to_string(),
        &profile.mcp_denylist,
        appearance,
        app,
        is_ai_enabled,
    ))
}
