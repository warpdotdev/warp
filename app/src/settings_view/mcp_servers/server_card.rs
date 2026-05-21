use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use warp_core::{
    features::FeatureFlag,
    ui::{
        external_product_icon::ExternalProductIcon,
        icons::{Icon, ICON_DIMENSIONS},
        theme::{color::internal_colors, AnsiColorIdentifier},
    },
};
use warpui::{
    accessibility::ActionAccessibilityContent,
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded, Fill, Flex,
        FormattedTextElement, HighlightedHyperlink, Hoverable, MainAxisAlignment, MainAxisSize,
        MouseState, MouseStateHandle, Padding, ParentElement, Radius, Text, Wrap,
    },
    fonts::Weight,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        chip::Chip,
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    ai::mcp::{
        templatable::CloudTemplatableMCPServer, MCPServerState, TemplatableMCPServerManager,
    },
    appearance::Appearance,
    cloud_object::CloudObject,
    settings_view::mcp_servers::{style, ServerCardItemId},
    ui_components::{
        avatar::{Avatar, AvatarContent, StatusElementTypes},
        blended_colors,
        buttons::icon_button,
        red_notification_dot::RedNotificationDot,
    },
};

/// A chip displayed inline with the server card title, optionally with a leading icon.
#[derive(Debug, Clone)]
pub struct TitleChip {
    pub text: String,
    pub leading_icon: Option<Icon>,
}

impl TitleChip {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            leading_icon: None,
        }
    }

    pub fn with_icon(text: impl Into<String>, icon: Icon) -> Self {
        Self {
            text: text.into(),
            leading_icon: Some(icon),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ServerCardAction {
    ToggleToolsExpanded,
    ToggleRunningSwitch,
    Edit(ServerCardItemId),
    Share(ServerCardItemId),
    Install(ServerCardItemId),
    InstallServerUpdate(ServerCardItemId),
    ViewLogs(ServerCardItemId),
    LogOut(ServerCardItemId),
    FullCardClick,
}

#[derive(Debug, Clone)]
pub enum ServerCardEvent {
    Edit(ServerCardItemId),
    Share(ServerCardItemId),
    ToggleRunningSwitch(ServerCardItemId, bool),
    Install(ServerCardItemId),
    InstallServerUpdate(ServerCardItemId),
    ViewLogs(ServerCardItemId),
    LogOut(ServerCardItemId),
}

#[derive(Default)]
pub struct ServerCardMouseHandles {
    show_logs_icon_button: MouseStateHandle,
    logout_icon_button: MouseStateHandle,
    share_icon_button: MouseStateHandle,
    edit_icon_button: MouseStateHandle,
    update_icon_button: MouseStateHandle,

    view_logs_button: MouseStateHandle,
    edit_config_button: MouseStateHandle,
    setup_button: MouseStateHandle,

    tools_expandable_hover: MouseStateHandle,
    card_hover: MouseStateHandle,
}

pub enum StatusColor {
    Red,
    Yellow,
    Green,
    Neutral,
}

impl StatusColor {
    fn to_color(&self, appearance: &Appearance) -> ColorU {
        match self {
            StatusColor::Red => appearance.theme().ui_error_color(),
            StatusColor::Yellow => appearance.theme().ansi_fg_yellow(),
            StatusColor::Green => appearance.theme().ansi_fg_green(),
            StatusColor::Neutral => {
                blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1())
            }
        }
    }
}

pub struct StatusElement {
    indicator_type: StatusElementTypes,
    color: StatusColor,
}

#[derive(Default)]
pub enum Background {
    #[default]
    Transparent,
    Filled,
}

#[derive(Default)]
pub struct ServerCardOptions {
    pub show_view_logs_icon_button: bool,
    pub show_log_out_icon_button: bool,
    pub show_share_icon_button: bool,
    pub show_edit_config_icon_button: bool,
    pub show_update_available_icon_button: bool,
    pub show_view_logs_text_button: bool,
    pub show_edit_config_text_button: bool,
    pub show_setup_text_button: bool,
    pub show_add_icon: bool,

    pub full_card_clickable: bool,
    pub server_running_switch_state: Option<bool>,
    pub status_indicator: Option<StatusElement>,
    pub status_line: Option<String>,
    pub background: Background,
}

#[derive(PartialEq, Clone, Copy)]
pub enum ServerCardStatus {
    AvailableToSave,
    SavedToDrive,
    Installed,
    StartingServer,
    Authenticating,
    Running,
    ShuttingDown,
    Error,
}

// TODO(aeybel): We'll want to get rid of this `from` function and the ServerCardStatus enum
impl From<MCPServerState> for ServerCardStatus {
    fn from(state: MCPServerState) -> ServerCardStatus {
        match state {
            MCPServerState::NotRunning => ServerCardStatus::Installed,
            MCPServerState::Starting => ServerCardStatus::StartingServer,
            MCPServerState::Authenticating => ServerCardStatus::Authenticating,
            MCPServerState::Running => ServerCardStatus::Running,
            MCPServerState::ShuttingDown => ServerCardStatus::ShuttingDown,
            MCPServerState::FailedToStart => ServerCardStatus::Error,
        }
    }
}

impl From<ServerCardStatus> for ServerCardOptions {
    fn from(status: ServerCardStatus) -> ServerCardOptions {
        match status {
            ServerCardStatus::AvailableToSave => ServerCardOptions {
                show_view_logs_icon_button: false,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: false,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: false,
                show_add_icon: true,

                server_running_switch_state: None,
                status_indicator: None,
                status_line: None,
                background: Background::Transparent,
                full_card_clickable: true,
            },
            ServerCardStatus::SavedToDrive => ServerCardOptions {
                show_view_logs_icon_button: false,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: true,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: true,
                show_add_icon: false,

                server_running_switch_state: None,
                status_indicator: None,
                status_line: None,
                background: Background::Filled,
                full_card_clickable: false,
            },
            ServerCardStatus::Installed => ServerCardOptions {
                show_view_logs_icon_button: true,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: true,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: false,
                show_add_icon: false,

                server_running_switch_state: Some(false),
                status_indicator: Some(StatusElement {
                    indicator_type: StatusElementTypes::Circle,
                    color: StatusColor::Neutral,
                }),
                status_line: Some("Offline".to_string()),
                background: Background::Filled,
                full_card_clickable: false,
            },
            ServerCardStatus::StartingServer => ServerCardOptions {
                show_view_logs_icon_button: true,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: true,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: false,
                show_add_icon: false,

                server_running_switch_state: Some(true),
                status_indicator: Some(StatusElement {
                    indicator_type: StatusElementTypes::Circle,
                    color: StatusColor::Yellow,
                }),
                status_line: Some("Starting server...".to_string()),
                background: Background::Filled,
                full_card_clickable: false,
            },
            ServerCardStatus::Authenticating => ServerCardOptions {
                show_view_logs_icon_button: true,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: true,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: false,
                show_add_icon: false,

                server_running_switch_state: Some(true),
                status_indicator: Some(StatusElement {
                    indicator_type: StatusElementTypes::Circle,
                    color: StatusColor::Yellow,
                }),
                status_line: Some("Authenticating...".to_string()),
                background: Background::Filled,
                full_card_clickable: false,
            },
            ServerCardStatus::Running => ServerCardOptions {
                show_view_logs_icon_button: true,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: true,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: false,
                show_add_icon: false,

                server_running_switch_state: Some(true),
                status_indicator: Some(StatusElement {
                    indicator_type: StatusElementTypes::Circle,
                    color: StatusColor::Green,
                }),
                status_line: None,
                background: Background::Filled,
                full_card_clickable: false,
            },
            ServerCardStatus::ShuttingDown => ServerCardOptions {
                show_view_logs_icon_button: true,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: true,
                show_update_available_icon_button: false,
                show_view_logs_text_button: false,
                show_edit_config_text_button: false,
                show_setup_text_button: false,
                show_add_icon: false,

                server_running_switch_state: Some(false),
                status_indicator: Some(StatusElement {
                    indicator_type: StatusElementTypes::Circle,
                    color: StatusColor::Neutral,
                }),
                status_line: Some("Shutting down...".to_string()),
                background: Background::Filled,
                full_card_clickable: false,
            },
            ServerCardStatus::Error => ServerCardOptions {
                show_view_logs_icon_button: false,
                show_log_out_icon_button: false,
                show_share_icon_button: false,
                show_edit_config_icon_button: false,
                show_update_available_icon_button: false,
                show_view_logs_text_button: true,
                show_edit_config_text_button: true,
                show_setup_text_button: false,
                show_add_icon: false,

                server_running_switch_state: Some(false),
                status_indicator: Some(StatusElement {
                    indicator_type: StatusElementTypes::Icon(Icon::AlertTriangle),
                    color: StatusColor::Red,
                }),
                status_line: None,
                background: Background::Filled,
                full_card_clickable: false,
            },
        }
    }
}

pub struct ServerCardView {
    pub item_id: ServerCardItemId,

    title: String,
    description: Option<String>,
    tools: Option<Vec<String>>,
    error_text: Option<String>,
    title_chips: Vec<TitleChip>,

    render_options: ServerCardOptions,

    switch_state_handle: SwitchStateHandle,
    mouse_handles: ServerCardMouseHandles,

    is_tools_expanded: bool,
}

impl ServerCardView {
    pub fn new(
        item_id: ServerCardItemId,
        title: String,
        description: Option<String>,
        tools: Option<Vec<String>>,
        error_text: Option<String>,
        title_chips: Vec<TitleChip>,
        render_options: ServerCardOptions,
    ) -> Self {
        Self {
            item_id,
            title,
            description,
            tools,
            error_text,
            title_chips,
            render_options,

            switch_state_handle: Default::default(),
            mouse_handles: Default::default(),

            is_tools_expanded: false,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn render_options(&self) -> &ServerCardOptions {
        &self.render_options
    }

    fn render_server_icon_and_status(&self, appearance: &Appearance) -> Box<dyn Element> {
        // TODO(aeybel) will want to use gallery ids instead of title in the future
        // pending data model for the gallery items
        let product_icon = ExternalProductIcon::from_string(self.title.as_str());
        let avatar_content = if let Some(icon) = product_icon {
            AvatarContent::ExternalProductIcon(icon)
        } else {
            AvatarContent::DisplayName(self.title.clone())
        };

        let mut avatar = Avatar::new(
            avatar_content,
            UiComponentStyles {
                width: Some(32.),
                height: Some(32.),
                border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                font_family_id: Some(appearance.ui_font_family()),
                font_weight: Some(Weight::Bold),
                background: Some(appearance.theme().background().into()),
                font_size: Some(20.),
                font_color: Some(blended_colors::text_main(
                    appearance.theme(),
                    appearance.theme().background(),
                )),
                ..Default::default()
            },
        );

        if let Some(status_indicator) = &self.render_options.status_indicator {
            let status_indicator_circle_diameter: f32 = 8.;
            let status_indicator_icon_diameter: f32 = 10.;
            let status_style = match status_indicator.indicator_type {
                StatusElementTypes::Circle => UiComponentStyles {
                    width: Some(status_indicator_circle_diameter),
                    height: Some(status_indicator_circle_diameter),
                    border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                    background: Some(Fill::Solid(status_indicator.color.to_color(appearance))),
                    ..Default::default()
                },
                StatusElementTypes::Icon(_) => UiComponentStyles {
                    width: Some(status_indicator_icon_diameter),
                    height: Some(status_indicator_icon_diameter),
                    font_color: Some(status_indicator.color.to_color(appearance)),
                    ..Default::default()
                },
            };

            avatar = avatar.with_status_element_with_offset(
                status_indicator.indicator_type.clone(),
                status_style,
                -5.,
                5.,
            );
        }

        avatar.build().finish()
    }

    fn render_tool_chips(tools: &[String], appearance: &Appearance) -> Vec<Box<dyn Element>> {
        tools
            .iter()
            .map(|tool| {
                Chip::new(
                    tool.to_string(),
                    UiComponentStyles {
                        margin: Some(Coords {
                            top: 0.,
                            bottom: 0.,
                            left: 0.,
                            right: 6.,
                        }),
                        font_family_id: Some(appearance.ui_font_family()),
                        font_size: Some(style::TOOL_CHIP_TEXT_SIZE),
                        font_color: Some(blended_colors::text_main(
                            appearance.theme(),
                            internal_colors::neutral_4(appearance.theme()),
                        )),
                        background: Some(internal_colors::neutral_4(appearance.theme()).into()),
                        border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
                        ..Default::default()
                    },
                )
                .build()
                .finish()
            })
            .collect()
    }

    fn render_tools_expandable(
        &self,
        tools: &[String],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text_color =
            blended_colors::text_sub(appearance.theme(), appearance.theme().background());

        if tools.is_empty() {
            return Text::new(
                "No tools available".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(text_color)
            .with_selectable(false)
            .finish();
        }

        let chevron_icon = if self.is_tools_expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let toggle_mouse_state = self.mouse_handles.tools_expandable_hover.clone();
        let chevron_dimensions = 16.;

        Hoverable::new(toggle_mouse_state, move |_is_hovered| {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        format!("{} tools available", tools.len()),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(text_color)
                    .with_selectable(false)
                    .finish(),
                )
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            chevron_icon.to_warpui_icon(text_color.into()).finish(),
                        )
                        .with_width(chevron_dimensions)
                        .with_height(chevron_dimensions)
                        .finish(),
                    )
                    .with_margin_right(4.)
                    .finish(),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ServerCardAction::ToggleToolsExpanded);
        })
        .finish()
    }

    fn render_title_chip(chip: &TitleChip, appearance: &Appearance) -> Box<dyn Element> {
        let chip_color = appearance
            .theme()
            .sub_text_color(appearance.theme().surface_3())
            .into_solid();

        let text_element = Text::new(
            chip.text.clone(),
            appearance.ui_font_family(),
            style::TITLE_CHIP_FONT_SIZE,
        )
        .with_color(chip_color)
        .finish();

        let inner: Box<dyn Element> = if let Some(icon) = chip.leading_icon {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(2.)
                .with_child(
                    ConstrainedBox::new(icon.to_warpui_icon(chip_color.into()).finish())
                        .with_width(style::TITLE_CHIP_FONT_SIZE)
                        .with_height(style::TITLE_CHIP_FONT_SIZE)
                        .finish(),
                )
                .with_child(text_element)
                .finish()
        } else {
            text_element
        };

        Container::new(inner)
            .with_background(appearance.theme().surface_3())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
            .with_horizontal_padding(3.)
            .with_vertical_padding(1.)
            .finish()
    }

    fn render_title_and_title_chip(
        title: String,
        title_chips: &[TitleChip],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let title = Text::new(
            title.clone(),
            appearance.ui_font_family(),
            appearance.ui_builder().ui_font_size(),
        )
        .with_color(blended_colors::text_main(
            appearance.theme(),
            appearance.theme().surface_1(),
        ))
        .finish();

        if title_chips.is_empty() {
            return title;
        }

        let mut wrap = Wrap::row()
            .with_spacing(style::SERVER_CARD_INTERIOR_SPACING)
            .with_run_spacing(style::SERVER_CARD_INTERIOR_SPACING)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(title);

        for chip in title_chips {
            wrap = wrap.with_child(Self::render_title_chip(chip, appearance));
        }

        wrap.finish()
    }

    fn render_debug_lines(&self, app: &AppContext, appearance: &Appearance) -> Box<dyn Element> {
        let mut lines = vec![format!("{}", self.item_id)];

        match self.item_id {
            ServerCardItemId::TemplatableMCP(template_uuid) => {
                let cloud_server = CloudTemplatableMCPServer::get_by_uuid(&template_uuid, app);
                if let Some(cloud_server) = cloud_server {
                    lines.push(format!("Template sync id: {}", cloud_server.sync_id()));
                }
            }
            ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                let installation = TemplatableMCPServerManager::as_ref(app)
                    .get_installed_server(&installation_uuid);
                if let Some(installation) = installation {
                    let template_uuid = installation.template_uuid();
                    let gallery_uuid = installation.gallery_uuid();
                    let gallery_uuid_text = match gallery_uuid {
                        Some(uuid) => format!("Gallery Id: {uuid}"),
                        None => "Gallery Id: None".to_string(),
                    };
                    let cloud_server = CloudTemplatableMCPServer::get_by_uuid(&template_uuid, app);
                    let template_sync_id_text = match cloud_server {
                        Some(cloud_server) => {
                            format!("Template sync id: {}", cloud_server.sync_id())
                        }
                        None => "Could not find cloud template".to_string(),
                    };
                    lines.push(format!(
                        "{}",
                        ServerCardItemId::TemplatableMCP(template_uuid)
                    ));
                    lines.push(gallery_uuid_text);
                    lines.push(template_sync_id_text);
                }
            }
            ServerCardItemId::GalleryMCP(_) => {}
            ServerCardItemId::FileBasedMCP(_) => {}
        }

        FormattedTextElement::new(
            FormattedText::new(
                lines
                    .into_iter()
                    .map(|line| {
                        FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(line)])
                    })
                    .collect::<Vec<_>>(),
            ),
            appearance.ui_builder().ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            HighlightedHyperlink::default(),
        )
        .finish()
    }

    fn add_subtitle_lines(
        &self,
        mut info_column: Flex,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Flex {
        if let Some(description) = &self.description {
            info_column = info_column.with_child(
                FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(vec![
                        FormattedTextFragment::plain_text(description.clone()),
                    ])]),
                    appearance.ui_builder().ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    blended_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_1(),
                    ),
                    HighlightedHyperlink::default(),
                )
                .finish(),
            );
        }

        if let Some(status_line) = &self.render_options.status_line {
            info_column = info_column.with_child(
                FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(vec![
                        FormattedTextFragment::plain_text(status_line.clone()),
                    ])]),
                    appearance.ui_builder().ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
                    HighlightedHyperlink::default(),
                )
                .finish(),
            );
        }

        if let Some(error_text) = &self.error_text {
            info_column = info_column.with_child(
                FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(vec![
                        FormattedTextFragment::plain_text(error_text.clone()),
                    ])]),
                    appearance.ui_builder().ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    appearance.theme().ui_error_color(),
                    HighlightedHyperlink::default(),
                )
                .finish(),
            );
        }

        if FeatureFlag::McpDebuggingIds.is_enabled() {
            info_column = info_column.with_child(self.render_debug_lines(app, appearance));
        }

        info_column
    }

    fn build_icon_button(
        &self,
        appearance: &Appearance,
        icon: Icon,
        tooltip_text: String,
        mouse_handle: MouseStateHandle,
    ) -> Hoverable {
        let ui_builder = appearance.ui_builder().clone();

        icon_button(appearance, icon, false, mouse_handle)
            .with_tooltip(move || ui_builder.tool_tip(tooltip_text.clone()).build().finish())
            .build()
    }

    fn render_actions_row(
        &self,
        state: &MouseState,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let item_id = self.item_id;
        let mut actions_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(style::SERVER_CARD_INTERIOR_SPACING);

        if state.is_hovered() {
            if self.render_options.show_view_logs_icon_button {
                actions_row = actions_row.with_child(
                    self.build_icon_button(
                        appearance,
                        Icon::Code1,
                        "Show logs".to_string(),
                        self.mouse_handles.show_logs_icon_button.clone(),
                    )
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ServerCardAction::ViewLogs(item_id))
                    })
                    .finish(),
                );
            }

            if self.render_options.show_log_out_icon_button {
                actions_row = actions_row.with_child(
                    self.build_icon_button(
                        appearance,
                        Icon::LogOut,
                        "Log out".to_string(),
                        self.mouse_handles.logout_icon_button.clone(),
                    )
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ServerCardAction::LogOut(item_id));
                    })
                    .finish(),
                );
            }

            if self.render_options.show_share_icon_button {
                actions_row = actions_row.with_child(
                    self.build_icon_button(
                        appearance,
                        Icon::Share,
                        "Share server".to_string(),
                        self.mouse_handles.share_icon_button.clone(),
                    )
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ServerCardAction::Share(item_id));
                    })
                    .finish(),
                );
            }

            if self.render_options.show_edit_config_icon_button {
                actions_row = actions_row.with_child(
                    self.build_icon_button(
                        appearance,
                        Icon::Pencil,
                        "Edit".to_string(),
                        self.mouse_handles.edit_icon_button.clone(),
                    )
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(ServerCardAction::Edit(item_id));
                    })
                    .finish(),
                );
            }
        }

        if self.render_options.show_update_available_icon_button {
            actions_row = actions_row.with_child(self.render_update_available_icon(appearance));
        }

        if self.render_options.show_view_logs_text_button {
            let view_logs_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Secondary,
                    self.mouse_handles.view_logs_button.clone(),
                )
                .with_centered_text_label(crate::i18n::tr_static(app, "View logs").to_string())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ServerCardAction::ViewLogs(item_id))
                })
                .finish();
            actions_row = actions_row.with_child(view_logs_button);
        }

        if self.render_options.show_edit_config_text_button {
            let edit_config_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Accent,
                    self.mouse_handles.edit_config_button.clone(),
                )
                .with_centered_text_label(crate::i18n::tr_static(app, "Edit config").to_string())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ServerCardAction::Edit(item_id));
                })
                .finish();
            actions_row = actions_row.with_child(edit_config_button);
        }

        if self.render_options.show_setup_text_button {
            let setup_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Accent,
                    self.mouse_handles.setup_button.clone(),
                )
                .with_centered_text_label(crate::i18n::tr_static(app, "Set up").to_string())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ServerCardAction::Install(item_id));
                })
                .finish();
            actions_row = actions_row.with_child(setup_button);
        }

        if self.render_options.show_add_icon {
            let add_icon = warpui::elements::Icon::new(
                Icon::Plus.into(),
                blended_colors::text_main(appearance.theme(), appearance.theme().background()),
            );
            actions_row = actions_row.with_child(
                ConstrainedBox::new(add_icon.finish())
                    .with_width(ICON_DIMENSIONS)
                    .with_height(ICON_DIMENSIONS)
                    .finish(),
            );
        }

        if let Some(switch_state) = self.render_options.server_running_switch_state {
            let switch = appearance
                .ui_builder()
                .switch(self.switch_state_handle.clone())
                .check(switch_state)
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ServerCardAction::ToggleRunningSwitch)
                })
                .finish();

            actions_row = actions_row.with_child(switch);
        }

        ConstrainedBox::new(actions_row.finish())
            .with_min_height(ICON_DIMENSIONS)
            .with_width(self.get_actions_row_width())
            .finish()
    }

    fn render_update_available_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        let item_id = self.item_id;
        let update_available_button = self
            .build_icon_button(
                appearance,
                Icon::Refresh,
                "Server update available".to_string(),
                self.mouse_handles.update_icon_button.clone(),
            )
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ServerCardAction::InstallServerUpdate(item_id));
            })
            .finish();

        RedNotificationDot::render_with_offset(
            update_available_button,
            &UiComponentStyles {
                width: Some(style::UPDATE_AVAILABLE_DOT_WIDTH),
                height: Some(style::UPDATE_AVAILABLE_DOT_WIDTH),
                background: Some(Fill::Solid(
                    AnsiColorIdentifier::Blue
                        .to_ansi_color(&appearance.theme().terminal_colors().normal)
                        .into(),
                )),
                ..RedNotificationDot::default_styles(appearance)
            },
            (-4., 4.),
        )
    }

    fn get_actions_row_width(&self) -> f32 {
        let mut number_of_buttons = 0;
        if self.render_options.show_add_icon {
            number_of_buttons += 1;
        }
        if self.render_options.show_view_logs_text_button {
            number_of_buttons += 1;
        }
        if self.render_options.show_edit_config_text_button {
            number_of_buttons += 1;
        }

        if number_of_buttons > 1 {
            style::SERVER_CARD_ACTIONS_WIDE_WIDTH
        } else {
            style::SERVER_CARD_ACTIONS_STANDARD_WIDTH
        }
    }
}

impl Entity for ServerCardView {
    type Event = ServerCardEvent;
}

impl TypedActionView for ServerCardView {
    type Action = ServerCardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ServerCardAction::ToggleToolsExpanded => {
                self.is_tools_expanded = !self.is_tools_expanded;
                ctx.notify();
            }
            ServerCardAction::ToggleRunningSwitch => {
                if let Some(server_running_state) = self.render_options.server_running_switch_state
                {
                    let new_state = !server_running_state;
                    self.render_options.server_running_switch_state = Some(new_state);
                    ctx.emit(ServerCardEvent::ToggleRunningSwitch(
                        self.item_id,
                        new_state,
                    ));
                } else {
                    log::error!("Server card: Tried to toggle a switch that does not exist.")
                }
                ctx.notify();
            }
            ServerCardAction::Share(item_id) => {
                ctx.emit(ServerCardEvent::Share(*item_id));
                ctx.notify();
            }
            ServerCardAction::Edit(item_id) => {
                ctx.emit(ServerCardEvent::Edit(*item_id));
                ctx.notify();
            }
            ServerCardAction::Install(item_id) => {
                ctx.emit(ServerCardEvent::Install(*item_id));
                ctx.notify();
            }
            ServerCardAction::InstallServerUpdate(item_id) => {
                ctx.emit(ServerCardEvent::InstallServerUpdate(*item_id));
                ctx.notify();
            }
            ServerCardAction::ViewLogs(item_id) => {
                ctx.emit(ServerCardEvent::ViewLogs(*item_id));
                ctx.notify();
            }
            ServerCardAction::LogOut(item_id) => {
                ctx.emit(ServerCardEvent::LogOut(*item_id));
                ctx.notify();
            }
            ServerCardAction::FullCardClick => ctx.emit(ServerCardEvent::Install(self.item_id)),
        }
    }

    fn action_accessibility_contents(
        &mut self,
        _action: &Self::Action,
        _ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        ActionAccessibilityContent::default()
    }
}

impl View for ServerCardView {
    fn ui_name() -> &'static str {
        "ServerCardView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut hoverable = Hoverable::new(self.mouse_handles.card_hover.clone(), |state| {
            let server_icon = self.render_server_icon_and_status(appearance);

            let title_and_title_chip = ServerCardView::render_title_and_title_chip(
                self.title.clone(),
                &self.title_chips,
                appearance,
            );

            let mut info_column = Flex::column().with_child(title_and_title_chip);

            info_column = self.add_subtitle_lines(info_column, appearance, app);

            if let Some(tools) = &self.tools {
                let tools_info_row = self.render_tools_expandable(tools, appearance);
                info_column = info_column.with_child(tools_info_row)
            }

            let actions_row = self.render_actions_row(state, appearance, app);

            let mut card_body = Flex::column()
                .with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Expanded::new(1., info_column.finish()).finish())
                        .with_child(actions_row)
                        .finish(),
                )
                .with_spacing(style::SERVER_CARD_INTERIOR_SPACING);

            if self.is_tools_expanded {
                if let Some(tools) = &self.tools {
                    let tool_chips = ServerCardView::render_tool_chips(tools, appearance);
                    let tool_chips_row = Wrap::row()
                        .with_run_spacing(6.)
                        .with_children(tool_chips)
                        .finish();
                    card_body = card_body.with_child(tool_chips_row);
                }
            }

            let mut card = Container::new(
                Flex::row()
                    .with_child(server_icon)
                    .with_child(Expanded::new(1., card_body.finish()).finish())
                    .with_spacing(style::SERVER_CARD_INTERIOR_SPACING)
                    .finish(),
            )
            .with_padding(Padding::uniform(12.))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(style::CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()));

            if self.render_options.full_card_clickable && (state.is_hovered() || state.is_clicked())
            {
                card = card.with_background(theme.surface_3());
            } else if matches!(self.render_options.background, Background::Filled) {
                card = card.with_background(theme.surface_1());
            }

            card.finish()
        });

        if self.render_options.full_card_clickable {
            hoverable = hoverable
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(ServerCardAction::FullCardClick));
        }

        hoverable.finish()
    }
}
