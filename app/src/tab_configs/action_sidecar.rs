use warp_util::path::user_friendly_path;
use warpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, Text,
    },
    platform::Cursor,
    ui_components::{
        button::{ButtonTooltipPosition, ButtonVariant},
        components::{UiComponent, UiComponentStyles},
    },
    AppContext, Element, SingletonEntity,
};

use crate::{
    appearance::Appearance, settings::ai::DefaultSessionMode, tab_configs::TabConfig,
    terminal::available_shells::AvailableShell, workspace::WorkspaceAction,
};

pub(crate) const SIDECAR_WIDTH: f32 = 260.;
const SIDECAR_PADDING: f32 = 12.;

/// Describes what the sidecar is showing, which determines which buttons appear.
#[derive(Clone, Debug)]
pub(crate) enum SidecarItemKind {
    /// A built-in item (Terminal, a specific shell, Agent, Cloud Oz).
    BuiltIn {
        name: String,
        default_mode: DefaultSessionMode,
        shell: Option<AvailableShell>,
    },
    /// A user-created tab config loaded from disk.
    UserTabConfig { config: TabConfig },
}

#[derive(Default)]
pub(crate) struct SidecarMouseStates {
    pub(crate) make_default: MouseStateHandle,
    pub(crate) edit_config: MouseStateHandle,
    pub(crate) remove_config: MouseStateHandle,
}

/// Renders the action sidecar panel as a raw element tree.
/// Called directly from the Workspace render method (not via ChildView).
pub(crate) fn render_action_sidecar(
    item: &SidecarItemKind,
    mouse_states: &SidecarMouseStates,
    is_already_default: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size();

    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Min);

    // Title
    let title = match item {
        SidecarItemKind::BuiltIn { name, .. } => name.clone(),
        SidecarItemKind::UserTabConfig { config } => config.name.clone(),
    };
    column.add_child(
        Container::new(
            Text::new_inline(title, font_family, font_size + 1.)
                .with_color(theme.main_text_color(theme.surface_2()).into())
                .finish(),
        )
        .with_margin_bottom(4.)
        .finish(),
    );

    // Subtitle (file path for user configs)
    if let SidecarItemKind::UserTabConfig { config } = item {
        if let Some(path) = &config.source_path {
            let raw_path = path.to_string_lossy().into_owned();
            let home_dir = dirs::home_dir();
            let path_str =
                user_friendly_path(&raw_path, home_dir.as_ref().and_then(|h| h.to_str()))
                    .into_owned();
            column.add_child(
                Container::new(
                    Text::new_inline(path_str, font_family, font_size - 1.)
                        .with_color(theme.sub_text_color(theme.surface_2()).into())
                        .finish(),
                )
                .with_margin_bottom(8.)
                .finish(),
            );
        }
    }

    let primary_text_color = theme.main_text_color(theme.surface_2());
    let button_style = UiComponentStyles {
        font_size: Some(12.),
        font_weight: Some(warpui::fonts::Weight::Bold),
        font_color: Some(primary_text_color.into()),
        padding: Some(warpui::ui_components::components::Coords {
            top: 4.,
            bottom: 4.,
            left: 8.,
            right: 8.,
        }),
        ..Default::default()
    };

    let make_default_action = match item {
        SidecarItemKind::BuiltIn {
            default_mode,
            shell,
            ..
        } => WorkspaceAction::TabConfigSidecarMakeDefault {
            mode: *default_mode,
            tab_config_path: None,
            shell: shell.clone(),
        },
        SidecarItemKind::UserTabConfig { config } => WorkspaceAction::TabConfigSidecarMakeDefault {
            mode: DefaultSessionMode::TabConfig,
            tab_config_path: config.source_path.clone(),
            shell: None,
        },
    };

    // "Make default" button (always shown; visually disabled with tooltip when already the default)
    let make_default_button = if is_already_default {
        let disabled_style = UiComponentStyles {
            font_color: Some(theme.disabled_text_color(theme.surface_2()).into()),
            border_color: Some(theme.outline().into()),
            ..button_style
        };
        appearance
            .ui_builder()
            .button(ButtonVariant::Outlined, mouse_states.make_default.clone())
            .with_centered_text_label(crate::i18n::tr_static(app, "Make default").into())
            .with_style(disabled_style)
            .with_tooltip({
                let ui_builder = appearance.ui_builder().clone();
                let tooltip = crate::i18n::tr_static(app, "Already the default").to_string();
                move || ui_builder.tool_tip(tooltip.clone()).build().finish()
            })
            .with_tooltip_position(ButtonTooltipPosition::Above)
            .set_clicked_styles(None)
            .build()
            .finish()
    } else {
        appearance
            .ui_builder()
            .button(ButtonVariant::Outlined, mouse_states.make_default.clone())
            .with_centered_text_label(crate::i18n::tr_static(app, "Make default").into())
            .with_style(button_style)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx: &mut warpui::elements::EventContext, _, _| {
                ctx.dispatch_typed_action(make_default_action.clone())
            })
            .finish()
    };
    column.add_child(
        ConstrainedBox::new(make_default_button)
            .with_max_width(SIDECAR_WIDTH - SIDECAR_PADDING * 2.)
            .finish(),
    );

    // "Edit config" and "Remove" buttons (only for user tab configs)
    if let SidecarItemKind::UserTabConfig { config } = item {
        if let Some(config_path) = &config.source_path {
            let edit_path = config_path.clone();
            let edit_button = appearance
                .ui_builder()
                .button(ButtonVariant::Outlined, mouse_states.edit_config.clone())
                .with_centered_text_label(crate::i18n::tr_static(app, "Edit config").into())
                .with_style(button_style)
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx: &mut warpui::elements::EventContext, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::TabConfigSidecarEditConfig {
                        path: edit_path.clone(),
                    })
                })
                .finish();
            column.add_child(
                Container::new(
                    ConstrainedBox::new(edit_button)
                        .with_max_width(SIDECAR_WIDTH - SIDECAR_PADDING * 2.)
                        .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );

            let remove_name = config.name.clone();
            let remove_path = config_path.clone();
            let red_color = theme.ansi_fg_red();
            let remove_style = UiComponentStyles {
                font_color: Some(red_color),
                border_color: Some(red_color.into()),
                ..button_style
            };
            let remove_button = appearance
                .ui_builder()
                .button(ButtonVariant::Outlined, mouse_states.remove_config.clone())
                .with_centered_text_label(crate::i18n::tr_static(app, "Remove").into())
                .with_style(remove_style)
                .with_hovered_styles(UiComponentStyles {
                    border_color: Some(theme.accent().into()),
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx: &mut warpui::elements::EventContext, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::TabConfigSidecarRemoveConfig {
                        name: remove_name.clone(),
                        path: remove_path.clone(),
                    })
                })
                .finish();
            column.add_child(
                Container::new(
                    ConstrainedBox::new(remove_button)
                        .with_max_width(SIDECAR_WIDTH - SIDECAR_PADDING * 2.)
                        .finish(),
                )
                .with_margin_top(4.)
                .finish(),
            );
        }
    }

    ConstrainedBox::new(
        Container::new(column.finish())
            .with_padding_left(SIDECAR_PADDING)
            .with_padding_right(SIDECAR_PADDING)
            .with_padding_top(SIDECAR_PADDING)
            .with_padding_bottom(SIDECAR_PADDING)
            .with_background(theme.surface_2())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .finish(),
    )
    .with_width(SIDECAR_WIDTH)
    .finish()
}
