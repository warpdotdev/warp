use crate::ai::mcp::{Author, MCPServerUpdate};
use crate::appearance::Appearance;
use crate::settings_view::mcp_servers::style::{
    INSTALLATION_MODAL_BUTTON_GAP, INSTALLATION_MODAL_PADDING,
};
use crate::ui_components::avatar::{Avatar, AvatarContent};
use crate::ui_components::blended_colors;
use crate::util::time_format::format_approx_duration_from_now;
use chrono::{Local, TimeZone};
use uuid::Uuid;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::external_product_icon::ExternalProductIcon;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{Align, Empty, Padding, Shrinkable};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::SingletonEntity;
use warpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
        MainAxisAlignment, MouseStateHandle, ParentElement, Radius, Text,
    },
    platform::Cursor,
    AppContext, Element, Entity, TypedActionView, View, ViewContext,
};

pub enum UpdateModalBodyEvent {
    Cancel,
    Update {
        installation_uuid: Option<Uuid>,
        update: MCPServerUpdate,
    },
}

#[derive(Debug)]
pub enum UpdateModalBodyAction {
    Cancel,
    Update,
    SelectOption(usize),
}

#[derive(Default)]
pub struct UpdateModalBody {
    installation_uuid: Option<Uuid>,
    server_name: Option<String>,
    update_options: Vec<MCPServerUpdate>,
    selected_updates: Vec<bool>,
    cancel_mouse_state: MouseStateHandle,
    update_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
    option_mouse_states: Vec<MouseStateHandle>,
}

impl UpdateModalBody {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn set_installation(
        &mut self,
        installation_uuid: Uuid,
        server_name: String,
        update_options: Vec<MCPServerUpdate>,
    ) {
        self.installation_uuid = Some(installation_uuid);
        self.server_name = Some(server_name);
        self.update_options = update_options;
        self.selected_updates = vec![false; self.update_options.len()];
        self.option_mouse_states = (0..self.update_options.len())
            .map(|_| MouseStateHandle::default())
            .collect();
    }

    pub fn clear(&mut self) {
        self.installation_uuid = None;
        self.server_name = None;
        self.update_options = vec![];
        self.selected_updates = vec![];
        self.option_mouse_states = vec![];
    }

    fn render_title(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let name = self.server_name.as_deref().unwrap_or("Server");

        // Renders MCP avatar icon
        let avatar_content = if let Some(icon) = ExternalProductIcon::from_string(name) {
            AvatarContent::ExternalProductIcon(icon)
        } else {
            AvatarContent::DisplayName(name.to_string())
        };
        let avatar = Avatar::new(
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
        )
        .build()
        .finish();

        // Renders MCP title text
        let title = Text::new(
            format!("Update {name}"),
            appearance.ui_font_family(),
            appearance.header_font_size(),
        )
        .with_color(theme.active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        // Renders 'X' icon for closing the modal
        let escape_icon = Shrinkable::new(
            1.,
            Align::new(
                Hoverable::new(self.close_button_mouse_state.clone(), |state| {
                    let mut icon = Container::new(
                        ConstrainedBox::new(
                            Icon::X
                                .to_warpui_icon(theme.active_ui_text_color())
                                .finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_padding(Padding::uniform(2.));
                    if state.is_hovered() {
                        icon = icon.with_background(appearance.theme().surface_2());
                    }
                    icon.finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(UpdateModalBodyAction::Cancel))
                .finish(),
            )
            .right()
            .finish(),
        )
        .finish();

        // Renders 'ESC' text for closing the modal
        let escape_button = Container::new(
            Text::new_inline(
                "ESC".to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size() * 0.8,
            )
            .with_color(theme.active_ui_text_color().into())
            .finish(),
        )
        .with_background_color(theme.surface_2().into())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_padding(Padding::uniform(4.))
        .finish();

        // Renders title row
        let title_row = Flex::row()
            .with_children(vec![avatar, title, escape_icon, escape_button])
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_spacing(8.)
            .finish();

        Container::new(title_row).with_margin_bottom(2.).finish()
    }

    fn render_description(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Modal appears only when multiple updates are available
        let description = format!(
            "This server has {} updates available, which would you like to proceed with?",
            self.update_options.len()
        );

        Text::new(
            description,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(appearance.theme().active_ui_text_color().into())
        .finish()
    }

    fn render_update_option(
        &self,
        index: usize,
        option: &MCPServerUpdate,
        is_selected: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let checkbox = appearance
            .ui_builder()
            .checkbox(MouseStateHandle::default(), None)
            .check(is_selected)
            .build()
            .finish();

        let (title, description) = match option {
            MCPServerUpdate::CloudTemplate {
                publisher,
                new_version_ts,
                ..
            } => {
                let publisher_string = match publisher {
                    Author::CurrentUser => "another device",
                    Author::OtherUser { name } => name,
                    Author::Unknown => "a team member",
                };
                let datetime = Local
                    .timestamp_opt(*new_version_ts, 0)
                    .single()
                    .unwrap_or_else(Local::now);
                let formatted_time = format_approx_duration_from_now(datetime);
                (
                    format!("Update from {publisher_string}"),
                    formatted_time.to_string(),
                )
            }
            MCPServerUpdate::Gallery {
                name, new_version, ..
            } => (
                format!("Update from {name}"),
                format!("Version {new_version}"),
            ),
        };

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new(
                    title.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.active_ui_text_color().into())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
            )
            .with_child(
                Text::new(
                    description.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size() * 0.85,
                )
                .with_color(blended_colors::text_sub(theme, theme.surface_2()))
                .finish(),
            )
            .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(Container::new(checkbox).with_margin_top(-4.).finish())
            .with_child(content)
            .finish();

        let background_color = if is_selected {
            theme.accent().with_opacity(5)
        } else {
            blended_colors::neutral_2(theme).into()
        };

        let border_color = if is_selected {
            theme.accent().into()
        } else {
            internal_colors::neutral_4(theme)
        };

        let option_container = Container::new(row)
            .with_uniform_padding(12.)
            .with_background(background_color)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish();

        Hoverable::new(self.option_mouse_states[index].clone(), |_| {
            option_container
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(UpdateModalBodyAction::SelectOption(index));
        })
        .finish()
    }

    fn render_action_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let cancel_button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.cancel_mouse_state.clone())
            .with_text_label("Cancel".into())
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Bold),
                font_color: Some(theme.active_ui_text_color().into()),
                ..Default::default()
            })
            .with_hovered_styles(UiComponentStyles {
                font_color: Some(theme.disabled_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(UpdateModalBodyAction::Cancel))
            .finish();

        // Disable the update button if no updates are selected
        let has_selection = self.selected_updates.iter().any(|&x| x);
        let label_color = if has_selection {
            theme.font_color(theme.accent())
        } else {
            theme.disabled_text_color(theme.surface_3())
        };

        let corner_down_left_icon = Container::new(
            ConstrainedBox::new(Icon::CornerDownLeft.to_warpui_icon(label_color).finish())
                .with_width(appearance.monospace_font_size())
                .with_height(appearance.monospace_font_size())
                .finish(),
        )
        .with_uniform_padding(2.)
        .with_border(Border::all(1.).with_border_fill(coloru_with_opacity(label_color.into(), 60)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

        let update_button_label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline(
                    "Update",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(label_color.into())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
            )
            .with_child(
                Container::new(corner_down_left_icon)
                    .with_margin_left(8.)
                    .finish(),
            )
            .finish();

        let mut update_button_builder = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.update_mouse_state.clone())
            .with_custom_label(update_button_label)
            .with_style(UiComponentStyles {
                padding: Some(Coords::uniform(5.).left(10.).right(10.)),
                ..Default::default()
            });

        if !has_selection {
            update_button_builder = update_button_builder.disabled();
        }

        let update_button = update_button_builder
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(UpdateModalBodyAction::Update))
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(cancel_button)
                    .with_margin_right(INSTALLATION_MODAL_BUTTON_GAP)
                    .finish(),
            )
            .with_child(Container::new(update_button).finish())
            .finish()
    }

    fn render_buttons_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let action_buttons = self.render_action_buttons(appearance);

        let spacer = Shrinkable::new(1., Container::new(Empty::new().finish()).finish()).finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_child(spacer)
            .with_child(action_buttons)
            .finish();

        Container::new(row)
            .with_border(Border::top(1.).with_border_fill(appearance.theme().outline()))
            .with_uniform_padding(INSTALLATION_MODAL_PADDING)
            .finish()
    }
}

impl Entity for UpdateModalBody {
    type Event = UpdateModalBodyEvent;
}

impl View for UpdateModalBody {
    fn ui_name() -> &'static str {
        "UpdateModalBody"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);

        let mut content_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(16.);

        content_column.add_child(self.render_title(appearance));
        content_column.add_child(self.render_description(appearance));

        // Add update options
        if self.update_options.is_empty() {
            let no_updates_text = Text::new(
                "No updates available",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .finish();
            content_column.add_child(no_updates_text);
        } else {
            for (index, option) in self.update_options.iter().enumerate() {
                let is_selected = self.selected_updates.get(index).copied().unwrap_or(false);
                content_column.add_child(self.render_update_option(
                    index,
                    option,
                    is_selected,
                    appearance,
                ));
            }
        }

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(content_column.finish())
                    .with_uniform_padding(INSTALLATION_MODAL_PADDING)
                    .finish(),
            )
            .with_child(self.render_buttons_row(appearance))
            .finish()
    }
}

impl TypedActionView for UpdateModalBody {
    type Action = UpdateModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            UpdateModalBodyAction::Cancel => ctx.emit(UpdateModalBodyEvent::Cancel),
            UpdateModalBodyAction::Update => {
                // Collect all selected updates and emit events for each
                for (index, &is_selected) in self.selected_updates.iter().enumerate() {
                    if is_selected {
                        ctx.emit(UpdateModalBodyEvent::Update {
                            installation_uuid: self.installation_uuid,
                            update: self.update_options[index].clone(),
                        });
                    }
                }
            }
            UpdateModalBodyAction::SelectOption(index) => {
                // Toggle the selection at the given index
                if let Some(selected) = self.selected_updates.get_mut(*index) {
                    *selected = !*selected;
                    ctx.notify();
                }
            }
        }
    }
}
