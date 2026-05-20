use std::collections::HashMap;

use crate::ai::mcp::templatable_installation::{VariableType, VariableValue};
use crate::appearance::Appearance;
use crate::editor::Event as EditorEvent;
use crate::editor::{EditorView, SingleLineEditorOptions};
use crate::settings_view::mcp_servers::style::{
    INSTALLATION_MODAL_BUTTON_GAP, INSTALLATION_MODAL_BUTTON_PADDING,
    INSTALLATION_MODAL_INPUT_VERTICAL_SPACING, INSTALLATION_MODAL_LABEL_VERTICAL_SPACING,
    INSTALLATION_MODAL_PADDING, INSTALLATION_MODAL_TITLE_VERTICAL_SPACING,
};
use crate::view_components::dropdown::{Dropdown, DropdownItem};
use markdown_parser::parse_markdown;
use warpui::elements::Shrinkable;
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    elements::{
        Align, Border, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex,
        FormattedTextElement, HighlightedHyperlink, Hoverable, MainAxisAlignment, MouseStateHandle,
        ParentElement, Text,
    },
    platform::Cursor,
    AppContext, Element, Entity, FocusContext, TypedActionView, View, ViewHandle,
};
use warpui::{SingletonEntity, ViewContext};

use crate::ai::mcp::{TemplatableMCPServer, TemplatableMCPServerManager, TemplateVariable};

use crate::ui_components::{
    avatar::{Avatar, AvatarContent},
    blended_colors,
};
use warpui::elements::{CornerRadius, Padding, Radius};

use warp_core::ui::{
    color::coloru_with_opacity, external_product_icon::ExternalProductIcon, icons::Icon,
};

pub enum InstallationModalBodyEvent {
    Cancel,
    Install(TemplatableMCPServer, HashMap<String, VariableValue>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropdownValueSelection {
    pub variable_key: String,
    pub selected_value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InstallationModalBodyAction {
    Cancel,
    Install,
    SelectDropdownValue(DropdownValueSelection),
}

/// Represents the input widget for a single template variable.
enum VariableInput {
    /// A freetext editor for variables without predefined values.
    TextInput(ViewHandle<EditorView>),
    /// A dropdown selector for variables with predefined allowed values.
    Dropdown {
        handle: ViewHandle<Dropdown<InstallationModalBodyAction>>,
        selected_value: Option<String>,
    },
}

pub struct InstallationModalBody {
    templatable_mcp_server: Option<TemplatableMCPServer>,
    instructions_in_markdown: Option<String>,
    variable_inputs: HashMap<String, VariableInput>,
    cancel_mouse_state: MouseStateHandle,
    install_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
    is_shared: bool,
}

impl Default for InstallationModalBody {
    fn default() -> Self {
        Self::new()
    }
}

impl InstallationModalBody {
    pub fn new() -> Self {
        Self {
            templatable_mcp_server: None,
            instructions_in_markdown: None,
            variable_inputs: HashMap::new(),
            cancel_mouse_state: Default::default(),
            install_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
            is_shared: false,
        }
    }

    pub fn set_templatable_mcp_server(
        &mut self,
        templatable_mcp_server: Option<TemplatableMCPServer>,
        instructions_in_markdown: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.templatable_mcp_server = templatable_mcp_server.clone();
        self.instructions_in_markdown = instructions_in_markdown;

        if let Some(templatable_mcp_server) = &self.templatable_mcp_server {
            self.is_shared = TemplatableMCPServerManager::as_ref(ctx)
                .is_server_template_shared(templatable_mcp_server.uuid, ctx);

            self.variable_inputs = templatable_mcp_server
                .template
                .variables
                .iter()
                .map(|variable| {
                    let key = variable.key.clone();
                    let allowed_values = variable.allowed_values.clone().unwrap_or_default();

                    let input = if !allowed_values.is_empty() {
                        let variable_key = key.clone();
                        let dropdown_handle = ctx.add_typed_action_view(|ctx| {
                            let mut dropdown = Dropdown::new(ctx);
                            let items: Vec<DropdownItem<InstallationModalBodyAction>> =
                                allowed_values
                                    .iter()
                                    .map(|value| {
                                        DropdownItem::new(
                                            value.clone(),
                                            InstallationModalBodyAction::SelectDropdownValue(
                                                DropdownValueSelection {
                                                    variable_key: variable_key.clone(),
                                                    selected_value: value.clone(),
                                                },
                                            ),
                                        )
                                    })
                                    .collect();
                            dropdown.set_items(items, ctx);
                            dropdown.set_selected_by_index(0, ctx);
                            dropdown
                        });

                        // Initial value can never be None since we know the list is not empty
                        let initial_value = allowed_values.first().cloned();
                        VariableInput::Dropdown {
                            handle: dropdown_handle,
                            selected_value: initial_value,
                        }
                    } else {
                        let editor = ctx.add_view(|ctx| {
                            EditorView::single_line(
                                SingleLineEditorOptions {
                                    soft_wrap: true,
                                    ..Default::default()
                                },
                                ctx,
                            )
                        });
                        ctx.subscribe_to_view(&editor, Self::handle_editor_event);
                        VariableInput::TextInput(editor)
                    };
                    (key, input)
                })
                .collect();
        } else {
            self.variable_inputs = HashMap::new();
            self.is_shared = false;
        }

        ctx.notify();
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Forwards escape key press from an input editor to its parent modal
        if matches!(event, EditorEvent::Escape) {
            ctx.emit(InstallationModalBodyEvent::Cancel);
        }
        // Forwards enter key press from an input editor to trigger installation
        else if matches!(event, EditorEvent::Enter) {
            self.process_installation(ctx);
        }
    }

    fn process_installation(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(templatable_mcp_server) = &self.templatable_mcp_server {
            let variable_values = templatable_mcp_server
                .template
                .variables
                .iter()
                .filter_map(|variable| {
                    let input = self.variable_inputs.get(&variable.key)?;
                    let value = match input {
                        VariableInput::TextInput(editor) => editor.as_ref(ctx).buffer_text(ctx),
                        VariableInput::Dropdown { selected_value, .. } => {
                            selected_value.clone().unwrap_or_default()
                        }
                    };
                    Some((
                        variable.key.clone(),
                        VariableValue {
                            variable_type: VariableType::Text,
                            value,
                        },
                    ))
                })
                .collect();

            ctx.emit(InstallationModalBodyEvent::Install(
                templatable_mcp_server.clone(),
                variable_values,
            ));
        }
    }

    fn render_title(
        name: String,
        appearance: &Appearance,
        close_button_mouse_state: MouseStateHandle,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Renders MCP avatar icon
        let avatar_content = if let Some(icon) = ExternalProductIcon::from_string(name.as_str()) {
            AvatarContent::ExternalProductIcon(icon)
        } else {
            AvatarContent::DisplayName(name.clone())
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
            format!("Install {name}"),
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
                Hoverable::new(close_button_mouse_state, |state| {
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
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(InstallationModalBodyAction::Cancel)
                })
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

        Container::new(title_row)
            .with_margin_bottom(INSTALLATION_MODAL_TITLE_VERTICAL_SPACING)
            .finish()
    }

    fn render_markdown_instructions(
        markdown_instructions: &str,
        appearance: &Appearance,
    ) -> Result<Box<dyn Element>, String> {
        let theme = appearance.theme();
        match parse_markdown(markdown_instructions) {
            Ok(formatted_text) => Ok(Container::new(
                FormattedTextElement::new(
                    formatted_text,
                    appearance.ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    theme.active_ui_text_color().into(),
                    HighlightedHyperlink::default(),
                )
                .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                .register_default_click_handlers(|url, _, ctx| {
                    ctx.open_url(&url.url);
                })
                .finish(),
            )
            .with_margin_bottom(INSTALLATION_MODAL_TITLE_VERTICAL_SPACING)
            .finish()),
            Err(e) => Err(format!("Failed to parse markdown: {e:?}")),
        }
    }

    fn render_input_fields(
        &self,
        mut form_column: Flex,
        variables: Vec<TemplateVariable>,
        appearance: &Appearance,
    ) -> Flex {
        let theme = appearance.theme();
        for template_variable in &variables {
            // Label
            form_column.add_child(
                Container::new(
                    Text::new(
                        template_variable.key.clone(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
                )
                .with_margin_bottom(INSTALLATION_MODAL_LABEL_VERTICAL_SPACING)
                .finish(),
            );

            // Input field: dropdown for allowed_values, text input otherwise
            if let Some(variable_input) = self.variable_inputs.get(&template_variable.key) {
                match variable_input {
                    VariableInput::TextInput(editor) => {
                        form_column.add_child(
                            Container::new(
                                appearance
                                    .ui_builder()
                                    .text_input(editor.clone())
                                    .with_style(UiComponentStyles {
                                        padding: Some(INSTALLATION_MODAL_BUTTON_PADDING),
                                        background: Some(
                                            blended_colors::neutral_2(appearance.theme()).into(),
                                        ),
                                        ..Default::default()
                                    })
                                    .build()
                                    .finish(),
                            )
                            .with_margin_bottom(INSTALLATION_MODAL_INPUT_VERTICAL_SPACING)
                            .finish(),
                        );
                    }
                    VariableInput::Dropdown { handle, .. } => {
                        form_column.add_child(
                            Container::new(ChildView::new(handle).finish())
                                .with_margin_bottom(INSTALLATION_MODAL_INPUT_VERTICAL_SPACING)
                                .finish(),
                        );
                    }
                }
            }
        }
        form_column
    }

    fn render_source_indicator(is_shared: bool, appearance: &Appearance) -> Box<dyn Element> {
        let info_icon = ConstrainedBox::new(
            Icon::Info
                .to_warpui_icon(appearance.theme().disabled_ui_text_color())
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let source_text = if is_shared {
            "Shared from team"
        } else {
            "From another device"
        };

        let label_text = Text::new_inline(
            source_text.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(appearance.theme().disabled_ui_text_color().into())
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(info_icon)
            .with_child(label_text)
            .with_spacing(INSTALLATION_MODAL_BUTTON_GAP)
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
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(InstallationModalBodyAction::Cancel))
            .finish();

        let accent_text_color = theme.font_color(theme.accent());

        let corner_down_left_icon = Container::new(
            ConstrainedBox::new(
                Icon::CornerDownLeft
                    .to_warpui_icon(accent_text_color)
                    .finish(),
            )
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
            .finish(),
        )
        .with_uniform_padding(2.)
        .with_border(
            Border::all(1.).with_border_fill(coloru_with_opacity(accent_text_color.into(), 60)),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

        let install_button_label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline(
                    "Install",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(accent_text_color.into())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
            )
            .with_child(
                Container::new(corner_down_left_icon)
                    .with_margin_left(8.)
                    .finish(),
            )
            .finish();

        let install_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.install_mouse_state.clone())
            .with_custom_label(install_button_label)
            .with_style(UiComponentStyles {
                padding: Some(Coords::uniform(5.).left(10.).right(10.)),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(InstallationModalBodyAction::Install))
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(cancel_button)
                    .with_margin_right(INSTALLATION_MODAL_BUTTON_GAP)
                    .finish(),
            )
            .with_child(Container::new(install_button).finish())
            .finish()
    }

    fn render_buttons_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let source_indicator = Self::render_source_indicator(self.is_shared, appearance);
        let action_buttons = self.render_action_buttons(appearance);

        let spacer = Shrinkable::new(1., Container::new(Empty::new().finish()).finish()).finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(source_indicator)
            .with_child(spacer)
            .with_child(action_buttons)
            .finish();

        Container::new(row)
            .with_border(Border::top(1.).with_border_fill(appearance.theme().outline()))
            .with_uniform_padding(INSTALLATION_MODAL_PADDING)
            .finish()
    }
}

impl Entity for InstallationModalBody {
    type Event = InstallationModalBodyEvent;
}

impl View for InstallationModalBody {
    fn ui_name() -> &'static str {
        "MCPTemplateInstallationModalBody"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            // Focus the first text input editor, if any.
            // Iterate in template variable order to focus the first one.
            if let Some(server) = &self.templatable_mcp_server {
                for variable in &server.template.variables {
                    match self.variable_inputs.get(&variable.key) {
                        Some(VariableInput::TextInput(editor)) => {
                            ctx.focus(editor);
                            return;
                        }
                        Some(VariableInput::Dropdown { handle, .. }) => {
                            ctx.focus(handle);
                            return;
                        }
                        None => continue,
                    }
                }
            }
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);

        if let Some(templatable_mcp_server) = &self.templatable_mcp_server {
            let mut form_column =
                Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

            form_column.add_child(Self::render_title(
                templatable_mcp_server.name.clone(),
                appearance,
                self.close_button_mouse_state.clone(),
            ));

            if let Some(instructions) = &self.instructions_in_markdown {
                if !instructions.is_empty() {
                    let instructions_result =
                        Self::render_markdown_instructions(instructions, appearance);
                    if let Ok(rendered_instructions) = instructions_result {
                        form_column.add_child(rendered_instructions);
                    }
                }
            }

            form_column = self.render_input_fields(
                form_column,
                templatable_mcp_server.template.variables.clone(),
                appearance,
            );

            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(form_column.finish())
                        .with_uniform_padding(INSTALLATION_MODAL_PADDING)
                        .finish(),
                )
                .with_child(self.render_buttons_row(appearance))
                .finish()
        } else {
            Text::new(
                "No MCP server selected",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .finish()
        }
    }
}

impl TypedActionView for InstallationModalBody {
    type Action = InstallationModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            InstallationModalBodyAction::Cancel => ctx.emit(InstallationModalBodyEvent::Cancel),
            InstallationModalBodyAction::Install => self.process_installation(ctx),
            InstallationModalBodyAction::SelectDropdownValue(selection) => {
                if let Some(VariableInput::Dropdown { selected_value, .. }) =
                    self.variable_inputs.get_mut(&selection.variable_key)
                {
                    *selected_value = Some(selection.selected_value.clone());
                }
            }
        }
    }
}
