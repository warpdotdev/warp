use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::profiles::{
    AIExecutionProfilesModel, AIExecutionProfilesModelEvent, ClientProfileId,
};
use crate::ai::execution_profiles::{
    ActionPermission, AskUserQuestionPermission, WriteToPtyPermission,
};
use crate::ai::llms::LLMPreferences;
use crate::appearance::Appearance;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::localization::{self, LocalizationUpdater};
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};
use crate::TemplatableMCPServerManager;
use std::path::PathBuf;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::elements::ParentElement;
use warpui::SingletonEntity;
use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
        Shrinkable, Text, Wrap,
    },
    fonts::{Properties, Weight},
    AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle,
};

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

        let edit_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                localization::text_for_app(ctx, "settings.execution_profile.edit"),
                SecondaryTheme,
            )
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

        ctx.subscribe_to_model(&LocalizationUpdater::handle(ctx), |me, _, _, ctx| {
            me.edit_button.update(ctx, |button, ctx| {
                button.set_label(
                    localization::text_for_app(ctx, "settings.execution_profile.edit"),
                    ctx,
                );
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
                    .get_default_base_model()
                    .display_name
                    .clone()
            });

        let cli_agent_model = profile
            .cli_agent_model
            .as_ref()
            .and_then(|id| llm_preferences.get_llm_info(id))
            .map(|info| info.display_name.clone())
            .unwrap_or_else(|| localization::text_for_app(app, "settings.execution_profile.auto"));

        let computer_use_model = profile
            .computer_use_model
            .as_ref()
            .and_then(|id| llm_preferences.get_llm_info(id))
            .map(|info| info.display_name.clone())
            .unwrap_or_else(|| localization::text_for_app(app, "settings.execution_profile.auto"));

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
                            Text::new(
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.models",
                                ),
                                appearance.ui_font_family(),
                                10.,
                            )
                            .with_color(appearance.theme().disabled_ui_text_color().into())
                            .finish(),
                        )
                        .with_margin_bottom(8.)
                        .finish(),
                    );
                    model_flex.add_child(with_standard_vertical_margin(
                        render_model_line_with_icon(
                            Icon::Lightning,
                            localization::text_for_app(
                                app,
                                "settings.execution_profile.base_model",
                            ),
                            base_model,
                            appearance,
                            is_any_ai_enabled,
                        ),
                    ));
                    model_flex.add_child(with_standard_vertical_margin(
                        render_model_line_with_icon(
                            Icon::Terminal,
                            localization::text_for_app(
                                app,
                                "settings.execution_profile.full_terminal_use",
                            ),
                            cli_agent_model,
                            appearance,
                            is_any_ai_enabled,
                        ),
                    ));
                    if FeatureFlag::LocalComputerUse.is_enabled() {
                        model_flex.add_child(with_standard_vertical_margin(
                            render_model_line_with_icon(
                                Icon::Laptop,
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.computer_use",
                                ),
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
                                    Text::new(
                                        localization::text_for_app(
                                            app,
                                            "settings.execution_profile.permissions",
                                        ),
                                        appearance.ui_font_family(),
                                        10.,
                                    )
                                    .with_color(appearance.theme().disabled_ui_text_color().into())
                                    .finish(),
                                )
                                .with_margin_bottom(8.)
                                .finish(),
                            )
                            .with_child(with_standard_vertical_margin(
                                render_action_permission_line_with_icon(
                                    Icon::Code2,
                                    localization::text_for_app(
                                        app,
                                        "settings.execution_profile.apply_code_diffs",
                                    ),
                                    &profile.apply_code_diffs,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ))
                            .with_child(with_standard_vertical_margin(
                                render_action_permission_line_with_icon(
                                    Icon::Notebook,
                                    localization::text_for_app(
                                        app,
                                        "settings.execution_profile.read_files",
                                    ),
                                    &profile.read_files,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ));

                        if profile.read_files == ActionPermission::AlwaysAsk
                            || profile.read_files == ActionPermission::AgentDecides
                        {
                            permissions_column.add_child(render_directory_allowlist(
                                &profile,
                                app,
                                appearance,
                                is_any_ai_enabled,
                            ));
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_action_permission_line_with_icon(
                                Icon::Terminal,
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.execute_commands",
                                ),
                                &profile.execute_commands,
                                app,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        match profile.execute_commands {
                            ActionPermission::AlwaysAllow => {
                                permissions_column.add_child(render_command_denylist(
                                    &profile,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                            }
                            ActionPermission::AlwaysAsk => {
                                permissions_column.add_child(render_command_allowlist(
                                    &profile,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                            }
                            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                                permissions_column.add_child(render_command_allowlist(
                                    &profile,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                                permissions_column.add_child(render_command_denylist(
                                    &profile,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ));
                            }
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_write_to_pty_permission_line_with_icon(
                                Icon::Workflow,
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.interact_with_running_commands",
                                ),
                                &profile.write_to_pty,
                                app,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        if FeatureFlag::LocalComputerUse.is_enabled() {
                            permissions_column.add_child(with_standard_vertical_margin(
                                render_computer_use_permission_line_with_icon(
                                    Icon::Laptop,
                                    localization::text_for_app(
                                        app,
                                        "settings.execution_profile.computer_use",
                                    ),
                                    &profile.computer_use,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ));
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_ask_user_question_permission_line_with_icon(
                                Icon::MessageText,
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.ask_questions",
                                ),
                                &profile.ask_user_question,
                                app,
                                appearance,
                                is_any_ai_enabled,
                            ),
                        ));

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_action_permission_line_with_icon(
                                Icon::Dataflow,
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.call_mcp_servers",
                                ),
                                &profile.mcp_permissions,
                                app,
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
                                    localization::text_for_app(
                                        app,
                                        "settings.execution_profile.call_web_tools",
                                    ),
                                    profile.web_search_enabled,
                                    app,
                                    appearance,
                                    is_any_ai_enabled,
                                ),
                            ));
                        }

                        permissions_column.add_child(with_standard_vertical_margin(
                            render_bool_permission_line_with_icon(
                                Icon::Compass,
                                localization::text_for_app(
                                    app,
                                    "settings.execution_profile.auto_sync_plans",
                                ),
                                profile.autosync_plans_to_warp_drive,
                                app,
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
    app: &AppContext,
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
            Text::new(
                localization::text_for_app(app, "settings.execution_profile.none"),
                appearance.ui_font_family(),
                12.,
            )
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
    app: &AppContext,
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
                Shrinkable::new(1., render_chips_row(items, app, appearance, is_ai_enabled))
                    .finish(),
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
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let items_str: Vec<String> = items.iter().map(|p| p.display().to_string()).collect();
    render_allowlist_denylist_row(icon, label, &items_str, app, appearance, is_ai_enabled)
}

fn render_command_predicate_row(
    icon: Icon,
    label: String,
    items: &[crate::settings::AgentModeCommandExecutionPredicate],
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let items_str: Vec<String> = items.iter().map(|c| c.to_string()).collect();
    render_allowlist_denylist_row(icon, label, &items_str, app, appearance, is_ai_enabled)
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
    render_allowlist_denylist_row(icon, label, &items_str, app, appearance, is_ai_enabled)
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
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let key = match permission {
        ActionPermission::AgentDecides => "settings.execution_profile.permission.agent_decides",
        ActionPermission::AlwaysAllow => "settings.execution_profile.permission.always_allow",
        ActionPermission::AlwaysAsk => "settings.execution_profile.permission.always_ask",
        ActionPermission::Unknown => "settings.execution_profile.permission.unknown",
    };
    let permission_text = localization::text_for_app(app, key);
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_write_to_pty_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &WriteToPtyPermission,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let key = match permission {
        WriteToPtyPermission::AlwaysAllow => "settings.execution_profile.permission.always_allow",
        WriteToPtyPermission::AlwaysAsk => "settings.execution_profile.permission.always_ask",
        WriteToPtyPermission::AskOnFirstWrite => {
            "settings.execution_profile.permission.ask_on_first_write"
        }
        WriteToPtyPermission::Unknown => "settings.execution_profile.permission.unknown",
    };
    let permission_text = localization::text_for_app(app, key);
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_computer_use_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &crate::ai::execution_profiles::ComputerUsePermission,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let key = match permission {
        crate::ai::execution_profiles::ComputerUsePermission::Never
        | crate::ai::execution_profiles::ComputerUsePermission::Unknown => {
            "settings.execution_profile.permission.never"
        }
        crate::ai::execution_profiles::ComputerUsePermission::AlwaysAsk => {
            "settings.execution_profile.permission.always_ask"
        }
        crate::ai::execution_profiles::ComputerUsePermission::AlwaysAllow => {
            "settings.execution_profile.permission.always_allow"
        }
    };
    let permission_text = localization::text_for_app(app, key);
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_ask_user_question_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    permission: &AskUserQuestionPermission,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let key = match permission {
        AskUserQuestionPermission::Never => "settings.execution_profile.permission.never_ask",
        AskUserQuestionPermission::AskExceptInAutoApprove | AskUserQuestionPermission::Unknown => {
            "settings.execution_profile.permission.ask_unless_auto_approve"
        }
        AskUserQuestionPermission::AlwaysAsk => "settings.execution_profile.permission.always_ask",
    };
    let permission_text = localization::text_for_app(app, key);
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_bool_permission_line_with_icon(
    icon: Icon,
    label: impl Into<String>,
    enabled: bool,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    let permission_text = if enabled {
        localization::text_for_app(app, "settings.execution_profile.permission.on")
    } else {
        localization::text_for_app(app, "settings.execution_profile.permission.off")
    };
    render_permission_line_with_icon(icon, label, permission_text, appearance, is_ai_enabled)
}

fn render_directory_allowlist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_pathbuf_allowlist_row(
        Icon::Check,
        localization::text_for_app(app, "settings.execution_profile.directory_allowlist"),
        &profile.directory_allowlist,
        app,
        appearance,
        is_ai_enabled,
    ))
}

fn render_command_allowlist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_command_predicate_row(
        Icon::Check,
        localization::text_for_app(app, "settings.execution_profile.command_allowlist"),
        &profile.command_allowlist,
        app,
        appearance,
        is_ai_enabled,
    ))
}

fn render_command_denylist(
    profile: &crate::ai::execution_profiles::AIExecutionProfile,
    app: &AppContext,
    appearance: &Appearance,
    is_ai_enabled: bool,
) -> Box<dyn Element> {
    with_standard_vertical_margin(render_command_predicate_row(
        Icon::SlashCircle,
        localization::text_for_app(app, "settings.execution_profile.command_denylist"),
        &profile.command_denylist,
        app,
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
        localization::text_for_app(app, "settings.execution_profile.mcp_allowlist"),
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
        localization::text_for_app(app, "settings.execution_profile.mcp_denylist"),
        &profile.mcp_denylist,
        appearance,
        app,
        is_ai_enabled,
    ))
}
