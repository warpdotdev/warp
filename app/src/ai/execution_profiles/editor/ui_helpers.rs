use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    ChildAnchor, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Dismiss, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Shrinkable, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity, ViewHandle};

use super::{ExecutionProfileEditorView, ExecutionProfileEditorViewAction};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::{
    long_context_pricing_warning_title, AIExecutionProfile, AIExecutionProfileAppExt as _,
    ActionPermission,
};
use crate::editor::EditorView;
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::view_components::{
    render_warning_box, Dropdown, DropdownItemAction, FilterableDropdown, SubmittableTextInput,
    WarningBoxConfig,
};
use crate::{Appearance, TemplatableMCPServerManager};

const CONTEXT_WINDOW_SLIDER_WIDTH: f32 = 220.;
const CONTEXT_WINDOW_INPUT_BOX_WIDTH: f32 = 120.;

pub(super) fn context_window_snap_values(min: u32, max: u32) -> Vec<f32> {
    if min >= max {
        return vec![min as f32];
    }
    let range = (max - min) as f64;
    let step = nice_step(range / 8.0);

    let mut values = vec![min as f32];
    let mut v = (min as f64 / step).ceil() * step;
    while v < max as f64 {
        if v > min as f64 {
            values.push(v as f32);
        }
        v += step;
    }
    if values.last().copied() != Some(max as f32) {
        values.push(max as f32);
    }
    values
}

fn nice_step(raw: f64) -> f64 {
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let nice = if normalized < 1.5 {
        1.0
    } else if normalized < 3.5 {
        2.5
    } else if normalized < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

use crate::settings_view::{render_input_list, render_separator, InputListItem};

pub const WORKSPACE_OVERRIDE_TOOLTIP_MESSAGE: &str =
    "This option is enforced by your organization's settings and cannot be customized.";
pub fn render_header_section(
    appearance: &Appearance,
    profile_name_editor: &ViewHandle<EditorView>,
    is_default_profile: bool,
) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_child(render_header_title(appearance))
        .with_child(render_header_name_label(appearance))
        .with_child(
            Container::new(
                appearance
                    .ui_builder()
                    .text_input(profile_name_editor.clone())
                    .build()
                    .finish(),
            )
            .with_margin_top(8.)
            .with_margin_bottom(8.)
            .finish(),
        );

    if is_default_profile {
        column.add_child(render_info_section(
            "Default profile name cannot be changed.",
            None,
            appearance,
        ));
    }

    Container::new(column.finish())
        .with_margin_bottom(24.)
        .finish()
}

fn render_header_title(appearance: &Appearance) -> Box<dyn Element> {
    Text::new_inline("Edit Profile", appearance.ui_font_family(), 16.)
        .with_style(Properties::default().weight(Weight::Bold))
        .with_color(appearance.theme().active_ui_text_color().into())
        .finish()
}

fn render_header_name_label(appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        Text::new("Name", appearance.ui_font_family(), 13.)
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish(),
    )
    .with_margin_top(16.)
    .finish()
}

pub fn render_section_label(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(
        Text::new(label.to_string(), appearance.ui_font_family(), 12.)
            .with_color(appearance.theme().disabled_ui_text_color().into())
            .finish(),
    )
    .with_margin_top(12.)
    .with_margin_bottom(20.)
    .finish()
}

fn render_filterable_dropdown_row<T: DropdownItemAction>(
    appearance: &Appearance,
    label: &str,
    desc: &str,
    dropdown: &ViewHandle<FilterableDropdown<T>>,
) -> Box<dyn Element> {
    let label_elem = Text::new(label.to_string(), appearance.ui_font_family(), 13.)
        .with_color(appearance.theme().active_ui_text_color().into())
        .finish();
    let desc_elem = Text::new(desc.to_string(), appearance.ui_font_family(), 11.)
        .with_color(
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_1())
                .into(),
        )
        .finish();

    let label_desc_column = Flex::column()
        .with_child(label_elem)
        .with_child(desc_elem)
        .finish();

    Container::new(
        Flex::column()
            .with_child(
                Container::new(label_desc_column)
                    .with_margin_bottom(4.)
                    .finish(),
            )
            .with_child(Container::new(ChildView::new(dropdown).finish()).finish())
            .finish(),
    )
    .with_margin_bottom(12.)
    .finish()
}

fn render_info_section(
    text: &str,
    _subtext: Option<&str>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let description_color = appearance.theme().disabled_ui_text_color();
    let alert_icon = Container::new(
        ConstrainedBox::new(
            Icon::AlertCircle
                .to_warpui_icon(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_2()),
                )
                .finish(),
        )
        .with_width(14.)
        .with_height(14.)
        .finish(),
    )
    .with_margin_right(4.)
    .finish();
    let text = Text::new(
        text.to_string(),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(description_color.into())
    .finish();
    let description = Flex::row()
        .with_children([alert_icon, Shrinkable::new(1.0, text).finish()])
        .finish();
    Container::new(description).with_margin_bottom(12.).finish()
}
fn render_long_context_pricing_warning(appearance: &Appearance) -> Box<dyn Element> {
    render_warning_box(
        WarningBoxConfig::formatted_title(long_context_pricing_warning_title()),
        appearance,
    )
}

fn render_permission_row<T: DropdownItemAction>(
    appearance: &Appearance,
    icon: Icon,
    label: &str,
    dropdown: &ViewHandle<Dropdown<T>>,
    info_text: &str,
    show_workspace_override_tooltip: bool,
    tooltip_mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let icon_elem = Container::new(
        ConstrainedBox::new(
            icon.to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish(),
    )
    .with_margin_right(8.)
    .finish();
    let label_elem = Text::new(label.to_string(), appearance.ui_font_family(), 13.)
        .with_color(appearance.theme().active_ui_text_color().into())
        .finish();
    let icon_label_row = Flex::row()
        .with_child(icon_elem)
        .with_child(label_elem)
        .finish();
    let dropdown_element = ChildView::new(dropdown).finish();
    let dropdown_row = if show_workspace_override_tooltip {
        wrap_disabled_with_workspace_override_tooltip(
            dropdown_element,
            tooltip_mouse_state,
            appearance,
        )
    } else {
        dropdown_element
    };
    let info_section = Container::new(render_info_section(info_text, None, appearance))
        .with_margin_bottom(12.)
        .finish();
    Flex::column()
        .with_child(icon_label_row)
        .with_child(dropdown_row)
        .with_child(info_section)
        .finish()
}

pub fn render_models_section(
    appearance: &Appearance,
    view: &ExecutionProfileEditorView,
    app: &AppContext,
) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_child(render_separator(appearance))
        .with_child(render_section_label("MODELS", appearance))
        .with_child(render_filterable_dropdown_row(
            appearance,
            "Base model",
            "This model serves as the primary engine behind the agent. It powers most interactions and invokes other models for tasks like planning or code generation when necessary. Warp may automatically switch to alternate models based on model availability or for auxiliary tasks such as conversation summarization.",
            &view.base_model_dropdown,
        ));

    if let Some(row) = render_context_window_row(appearance, view, app) {
        column.add_child(row);
    }

    column = column.with_child(render_filterable_dropdown_row(
        appearance,
        "Full terminal use model",
        "The model used when the agent operates inside interactive terminal applications like database shells, debuggers, REPLs, or dev servers—reading live output and writing commands to the PTY.",
        &view.full_terminal_use_model_dropdown,
    ));

    if FeatureFlag::LocalComputerUse.is_enabled() {
        column.add_child(render_filterable_dropdown_row(
            appearance,
            "Computer use model",
            "The model used when the agent takes control of your computer to interact with graphical applications through mouse movements, clicks, and keyboard input.",
            &view.computer_use_model_dropdown,
        ));
    }

    Container::new(column.finish())
        .with_margin_bottom(12.)
        .finish()
}

/// Renders a `[min — slider — max] [input]` row beneath the base model
/// dropdown. Returns `None` if the active base model doesn't advertise a
/// configurable context window or global AI is disabled.
fn render_context_window_row(
    appearance: &Appearance,
    view: &ExecutionProfileEditorView,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    if !AISettings::as_ref(app).is_any_ai_enabled(app) {
        return None;
    }
    let cw = view.configurable_context_window(app)?;
    let min = cw.min;
    let max = cw.max;

    let label = Text::new(
        "Context window".to_string(),
        appearance.ui_font_family(),
        13.,
    )
    .with_color(appearance.theme().active_ui_text_color().into())
    .finish();
    let min_label_text = min.separate_with_commas();
    let max_label_text = max.separate_with_commas();
    let desc = Text::new(
        "The base model's working memory — how many tokens of your conversation, code, and documents it can consider at once. Larger windows enable longer conversations and more coherent responses over bigger codebases, at the cost of higher latency and compute usage.".to_string(),
        appearance.ui_font_family(),
        11.,
    )
    .with_color(
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into(),
    )
    .finish();
    let label_desc = Flex::column().with_child(label).with_child(desc).finish();

    let min_label = Text::new(min_label_text.clone(), appearance.ui_font_family(), 11.)
        .with_color(
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_1())
                .into(),
        )
        .finish();
    let max_label = Text::new(max_label_text.clone(), appearance.ui_font_family(), 11.)
        .with_color(
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_1())
                .into(),
        )
        .finish();

    let current_value = view
        .current_context_window_display_value(app)
        .unwrap_or(cw.default_max)
        .clamp(min, max);
    let slider = appearance
        .ui_builder()
        .slider(view.context_window_slider_state.clone())
        .with_range(min as f32..max as f32)
        .with_snap_values(context_window_snap_values(min, max))
        .with_default_value(current_value as f32)
        .with_style(UiComponentStyles {
            width: Some(CONTEXT_WINDOW_SLIDER_WIDTH),
            margin: Some(Coords::default().left(8.).right(8.)),
            ..Default::default()
        })
        .on_drag(|ctx, _, val| {
            ctx.dispatch_typed_action(
                ExecutionProfileEditorViewAction::ContextWindowSliderDragged {
                    value: val.round() as u32,
                },
            );
        })
        .on_change(|ctx, _, val| {
            ctx.dispatch_typed_action(ExecutionProfileEditorViewAction::SetContextWindowSize {
                value: val.round() as u32,
            });
        })
        .build()
        .finish();

    let context_window_editor = view.context_window_editor.clone();
    let input_box = Dismiss::new(
        appearance
            .ui_builder()
            .text_input(view.context_window_editor.clone())
            .with_style(UiComponentStyles {
                width: Some(CONTEXT_WINDOW_INPUT_BOX_WIDTH),
                padding: Some(Coords {
                    top: 6.,
                    bottom: 6.,
                    left: 10.,
                    right: 10.,
                }),
                margin: Some(Coords::default().left(12.)),
                background: Some(appearance.theme().surface_2().into()),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .on_dismiss(move |ctx, app| {
        let buffer_text = context_window_editor.as_ref(app).buffer_text(app);
        let cleaned: String = buffer_text
            .chars()
            .filter(|c| !c.is_whitespace() && *c != ',')
            .collect();
        if let Ok(parsed) = cleaned.parse::<u32>() {
            ctx.dispatch_typed_action(ExecutionProfileEditorViewAction::SetContextWindowSize {
                value: parsed,
            });
        }
    })
    .finish();

    let slider_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(min_label)
        .with_child(Shrinkable::new(1., slider).finish())
        .with_child(max_label)
        .with_child(input_box)
        .finish();

    let mut column = Flex::column()
        .with_child(Container::new(label_desc).with_margin_bottom(4.).finish())
        .with_child(slider_row);
    if BlocklistAIPermissions::as_ref(app)
        .permissions_profile_for_id(app, view.profile_id())
        .should_show_long_context_pricing_warning(view.dragged_context_window_value, app)
    {
        column.add_child(render_long_context_pricing_warning(appearance));
    }

    Some(
        Container::new(column.finish())
            .with_margin_bottom(12.)
            .finish(),
    )
}

pub fn render_permissions_section(
    appearance: &Appearance,
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
    app: &warpui::AppContext,
) -> Box<dyn Element> {
    let ai_settings = AISettings::as_ref(app);
    let mut column = Flex::column().with_children([
        render_separator(appearance),
        render_section_label("PERMISSIONS", appearance),
        render_permission_row(
            appearance,
            Icon::Code2,
            "Apply code diffs",
            &view.apply_code_diffs_dropdown,
            profile_data.apply_code_diffs.description(),
            !ai_settings.is_code_diffs_permissions_editable(app),
            view.tooltip_mouse_state_handles
                .apply_code_diffs_tooltip_mouse_state
                .clone(),
        ),
        render_permission_row(
            appearance,
            Icon::Notebook,
            "Read files",
            &view.read_files_dropdown,
            profile_data.read_files.description(),
            !ai_settings.is_read_files_permissions_editable(app),
            view.tooltip_mouse_state_handles
                .read_files_tooltip_mouse_state
                .clone(),
        ),
    ]);

    if profile_data.read_files == ActionPermission::AlwaysAsk
        || profile_data.read_files == ActionPermission::AgentDecides
    {
        column.add_child(render_directory_allowlist_section(
            view,
            profile_data,
            appearance,
            app,
        ));
    }

    column.add_child(render_permission_row(
        appearance,
        Icon::Terminal,
        "Execute commands",
        &view.execute_commands_dropdown,
        profile_data.execute_commands.description(),
        !ai_settings.is_execute_commands_permissions_editable(app),
        view.tooltip_mouse_state_handles
            .execute_commands_tooltip_mouse_state
            .clone(),
    ));

    match profile_data.execute_commands {
        ActionPermission::AlwaysAllow => {
            column.add_child(render_command_denylist_section(
                view,
                profile_data,
                appearance,
                app,
            ));
        }
        ActionPermission::AlwaysAsk => {
            column.add_child(render_command_allowlist_section(
                view,
                profile_data,
                appearance,
                app,
            ));
        }
        ActionPermission::AgentDecides | ActionPermission::Unknown => {
            column.add_children([
                render_command_allowlist_section(view, profile_data, appearance, app),
                render_command_denylist_section(view, profile_data, appearance, app),
            ]);
        }
    }

    column.add_child(render_permission_row(
        appearance,
        Icon::Workflow,
        "Interact with running commands",
        &view.write_to_pty_dropdown,
        profile_data.write_to_pty.description(),
        !ai_settings.is_write_to_pty_permissions_editable(app),
        view.tooltip_mouse_state_handles
            .write_to_pty_tooltip_mouse_state
            .clone(),
    ));

    if FeatureFlag::LocalComputerUse.is_enabled() {
        column.add_child(render_permission_row(
            appearance,
            Icon::Laptop,
            "Computer use",
            &view.computer_use_dropdown,
            profile_data.computer_use.description(),
            !ai_settings.is_computer_use_permissions_editable(app),
            view.tooltip_mouse_state_handles
                .computer_use_tooltip_mouse_state
                .clone(),
        ));
    }

    column.add_child(render_permission_row(
        appearance,
        Icon::MessageText,
        "Ask questions",
        &view.ask_user_question_dropdown,
        profile_data.ask_user_question.description(),
        !ai_settings.is_ask_user_question_permissions_editable(app),
        view.tooltip_mouse_state_handles
            .ask_user_question_tooltip_mouse_state
            .clone(),
    ));
    column.add_child(render_permission_row(
        appearance,
        Icon::Atom,
        "Run orchestrated agents",
        &view.run_agents_dropdown,
        profile_data.run_agents.description(),
        !ai_settings.is_run_agents_permissions_editable(app),
        view.tooltip_mouse_state_handles
            .run_agents_tooltip_mouse_state
            .clone(),
    ));

    column.add_child(render_permission_row(
        appearance,
        Icon::Dataflow,
        "Call MCP servers",
        &view.call_mcp_servers_dropdown,
        profile_data.mcp_permissions.description(),
        !ai_settings.is_mcp_permission_editable(app), // Use MCP override for this permission
        view.tooltip_mouse_state_handles
            .call_mcp_servers_tooltip_mouse_state
            .clone(),
    ));

    match profile_data.mcp_permissions {
        ActionPermission::AlwaysAllow => {
            column.add_child(render_mcp_denylist_section(
                view,
                profile_data,
                app,
                appearance,
            ));
        }
        ActionPermission::AlwaysAsk => {
            column.add_child(render_mcp_allowlist_section(
                view,
                profile_data,
                app,
                appearance,
            ));
        }
        ActionPermission::AgentDecides | ActionPermission::Unknown => {
            column.add_children([
                render_mcp_allowlist_section(view, profile_data, app, appearance),
                render_mcp_denylist_section(view, profile_data, app, appearance),
            ]);
        }
    }

    if FeatureFlag::WebSearchUI.is_enabled() {
        column.add_child(
            Container::new(render_web_search_toggle(appearance, view, profile_data))
                .with_margin_top(16.)
                .finish(),
        );
    }

    column.add_child(
        Container::new(render_plan_auto_sync_toggle(appearance, view, profile_data))
            .with_margin_top(16.)
            .finish(),
    );

    Container::new(column.finish())
        .with_margin_bottom(24.)
        .finish()
}

fn create_section_header(
    label: &str,
    description: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let label_elem = Text::new(label.to_string(), appearance.ui_font_family(), 13.)
        .with_color(appearance.theme().active_ui_text_color().into())
        .finish();

    let desc_elem = Text::new(description.to_string(), appearance.ui_font_family(), 11.)
        .with_color(
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_1())
                .into(),
        )
        .finish();

    Container::new(
        Flex::column()
            .with_child(label_elem)
            .with_child(desc_elem)
            .finish(),
    )
    .with_margin_bottom(4.)
    .finish()
}

#[allow(clippy::too_many_arguments)]
fn render_list_section<T, F, D>(
    label: &str,
    description: &str,
    items: &[T],
    mouse_handles: &[MouseStateHandle],
    editor: Option<&ViewHandle<SubmittableTextInput>>,
    dropdown: Option<&ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>>,
    on_remove_action: F,
    display_fn: D,
    appearance: &Appearance,
    is_editable: bool,
    tooltip_mouse_state: MouseStateHandle,
) -> Box<dyn Element>
where
    T: Clone,
    F: Fn(T) -> ExecutionProfileEditorViewAction,
    D: Fn(&T) -> String,
{
    let input_items: Vec<InputListItem<ExecutionProfileEditorViewAction>> = items
        .iter()
        .cloned()
        .zip(mouse_handles.iter().cloned())
        .rev()
        .map(|(item, mouse_state_handle)| InputListItem {
            item: display_fn(&item),
            mouse_state_handle,
            on_remove_action: on_remove_action(item),
            is_disabled: !is_editable,
            tooltip_mouse_state: None,
        })
        .collect();

    let list = render_input_list(None, input_items, editor, appearance);
    let list_element = if !is_editable {
        wrap_disabled_with_workspace_override_tooltip(list, tooltip_mouse_state, appearance)
    } else {
        list
    };

    let mut column =
        Flex::column().with_child(create_section_header(label, description, appearance));

    // Add dropdown if provided (for MCP lists)
    if let Some(dropdown) = dropdown {
        let dropdown_row = Container::new(ChildView::new(dropdown).finish()).finish();
        column = column.with_child(dropdown_row);
    }

    column = column.with_child(list_element);

    Container::new(column.finish())
        .with_margin_bottom(16.)
        .finish()
}

fn render_directory_allowlist_section(
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
    appearance: &Appearance,
    app: &warpui::AppContext,
) -> Box<dyn Element> {
    let ai_settings = AISettings::as_ref(app);
    let is_editable = ai_settings.is_directory_allowlist_editable(app);

    render_list_section(
        "Directory allowlist",
        "Give the agent file access to certain directories.",
        &profile_data.directory_allowlist,
        &view.directory_allowlist_mouse_state_handles,
        Some(&view.directory_allowlist_editor),
        None,
        |path| ExecutionProfileEditorViewAction::RemoveFromDirectoryAllowlist { path },
        |path| path.display().to_string(),
        appearance,
        is_editable,
        view.tooltip_mouse_state_handles
            .directory_allowlist_editor_tooltip_mouse_state
            .clone(),
    )
}
fn render_command_allowlist_section(
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
    appearance: &Appearance,
    app: &warpui::AppContext,
) -> Box<dyn Element> {
    let ai_settings = AISettings::as_ref(app);
    let is_editable = ai_settings.is_command_allowlist_editable(app);

    render_list_section(
        "Command allowlist",
        "Regular expressions to match commands that can be automatically executed by Oz.",
        &profile_data.command_allowlist,
        &view.command_allowlist_mouse_state_handles,
        Some(&view.command_allowlist_editor),
        None,
        |predicate| ExecutionProfileEditorViewAction::RemoveFromCommandAllowlist { predicate },
        |item| item.to_string(),
        appearance,
        is_editable,
        view.tooltip_mouse_state_handles
            .command_allowlist_editor_tooltip_mouse_state
            .clone(),
    )
}

fn render_command_denylist_section(
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
    appearance: &Appearance,
    app: &warpui::AppContext,
) -> Box<dyn Element> {
    use crate::ai::blocklist::BlocklistAIPermissions;

    let ai_disabled = !AISettings::as_ref(app).is_any_ai_enabled(app);
    let org_denylist = BlocklistAIPermissions::get_org_execute_commands_denylist(app);
    let mut tooltip_idx = 0usize;

    let input_items: Vec<InputListItem<ExecutionProfileEditorViewAction>> = profile_data
        .command_denylist
        .iter()
        .cloned()
        .zip(view.command_denylist_mouse_state_handles.iter().cloned())
        .rev()
        .map(|(predicate, mouse_state_handle)| {
            let is_org = org_denylist.contains(&predicate);
            let tooltip_mouse_state = if is_org {
                let handle = view
                    .command_denylist_tooltip_mouse_state_handles
                    .get(tooltip_idx)
                    .cloned();
                tooltip_idx += 1;
                handle
            } else {
                None
            };
            InputListItem {
                item: predicate.to_string(),
                mouse_state_handle,
                on_remove_action: ExecutionProfileEditorViewAction::RemoveFromCommandDenylist {
                    predicate,
                },
                is_disabled: is_org || ai_disabled,
                tooltip_mouse_state,
            }
        })
        .collect();

    let list = render_input_list(
        None,
        input_items,
        Some(&view.command_denylist_editor),
        appearance,
    );

    let mut column = Flex::column().with_child(create_section_header(
        "Command denylist",
        "Regular expressions to match commands that Oz should always ask permission to execute.",
        appearance,
    ));
    column = column.with_child(list);

    Container::new(column.finish())
        .with_margin_bottom(16.)
        .finish()
}

fn display_mcp_name(uuid: &Uuid, app: &AppContext) -> String {
    TemplatableMCPServerManager::get_mcp_name(uuid, app).unwrap_or({
        log::warn!("Expected a name for MCP server {uuid} but could not find one.");
        format!("MCP Server {uuid}")
    })
}

fn render_mcp_allowlist_section(
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
    app: &warpui::AppContext,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let ai_settings = AISettings::as_ref(app);
    let is_editable = ai_settings.is_mcp_permission_editable(app);

    render_list_section(
        "MCP allowlist",
        "MCP servers that are allowed to be called by Oz.",
        &profile_data.mcp_allowlist,
        &view.mcp_allowlist_mouse_state_handles,
        None,
        Some(&view.mcp_allowlist_dropdown),
        |id| ExecutionProfileEditorViewAction::RemoveFromMCPAllowlist { id },
        |uuid| display_mcp_name(uuid, app),
        appearance,
        is_editable,
        view.tooltip_mouse_state_handles
            .mcp_allowlist_editor_tooltip_mouse_state
            .clone(),
    )
}

fn render_mcp_denylist_section(
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
    app: &warpui::AppContext,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let ai_settings = AISettings::as_ref(app);
    let is_editable = ai_settings.is_mcp_permission_editable(app);

    render_list_section(
        "MCP denylist",
        "MCP servers that are not allowed to be called by Oz.",
        &profile_data.mcp_denylist,
        &view.mcp_denylist_mouse_state_handles,
        None,
        Some(&view.mcp_denylist_dropdown),
        |id| ExecutionProfileEditorViewAction::RemoveFromMCPDenylist { id },
        |uuid| display_mcp_name(uuid, app),
        appearance,
        is_editable,
        view.tooltip_mouse_state_handles
            .mcp_denylist_editor_tooltip_mouse_state
            .clone(),
    )
}
pub fn render_plan_auto_sync_toggle(
    appearance: &Appearance,
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
) -> Box<dyn Element> {
    let icon_size = 16.0;
    let icon_elem = Container::new(
        ConstrainedBox::new(
            Icon::Compass
                .to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_width(icon_size)
        .with_height(icon_size)
        .finish(),
    )
    .with_margin_right(8.)
    .finish();

    let label_elem = Text::new(
        "Plan auto-sync".to_string(),
        appearance.ui_font_family(),
        13.,
    )
    .with_color(appearance.theme().active_ui_text_color().into())
    .finish();

    let desc_elem = Text::new(
        "The plans this agent creates will be automatically added and synced to Warp Drive."
            .to_string(),
        appearance.ui_font_family(),
        11.,
    )
    .with_color(
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into(),
    )
    .finish();

    let current_value = profile_data.autosync_plans_to_warp_drive;
    let switch = appearance
        .ui_builder()
        .switch(view.plan_auto_sync_switch.clone())
        .check(current_value)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ExecutionProfileEditorViewAction::SetPlanAutoSync {
                enabled: !current_value,
            });
        })
        .finish();

    let left_content = Flex::column()
        .with_child(
            Flex::row()
                .with_child(icon_elem)
                .with_child(label_elem)
                .finish(),
        )
        .with_child(desc_elem)
        .finish();

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(8.)
        .with_child(Shrinkable::new(1., left_content).finish())
        .with_child(switch)
        .finish()
}

pub fn render_web_search_toggle(
    appearance: &Appearance,
    view: &ExecutionProfileEditorView,
    profile_data: &AIExecutionProfile,
) -> Box<dyn Element> {
    let icon_size = 16.0;
    let icon_elem = Container::new(
        ConstrainedBox::new(
            Icon::Globe
                .to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_width(icon_size)
        .with_height(icon_size)
        .finish(),
    )
    .with_margin_right(8.)
    .finish();

    let label_elem = Text::new(
        "Call web tools".to_string(),
        appearance.ui_font_family(),
        13.,
    )
    .with_color(appearance.theme().active_ui_text_color().into())
    .finish();

    let desc_elem = Text::new(
        "The agent may use web search when helpful for completing tasks.".to_string(),
        appearance.ui_font_family(),
        11.,
    )
    .with_color(
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into(),
    )
    .finish();

    let current_value = profile_data.web_search_enabled;
    let switch = appearance
        .ui_builder()
        .switch(view.web_search_switch.clone())
        .check(current_value)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ExecutionProfileEditorViewAction::SetWebSearchEnabled {
                enabled: !current_value,
            });
        })
        .finish();

    let left_content = Flex::column()
        .with_child(
            Flex::row()
                .with_child(icon_elem)
                .with_child(label_elem)
                .finish(),
        )
        .with_child(desc_elem)
        .finish();

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(8.)
        .with_child(Shrinkable::new(1., left_content).finish())
        .with_child(switch)
        .finish()
}

pub fn wrap_disabled_with_workspace_override_tooltip(
    child: Box<dyn Element>,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // Wrap the disabled element in a hoverable container that can show tooltips
    Hoverable::new(mouse_state, |state| {
        let mut stack = Stack::new().with_child(child);
        if state.is_hovered() {
            let tooltip = appearance
                .ui_builder()
                .tool_tip(WORKSPACE_OVERRIDE_TOOLTIP_MESSAGE.to_string())
                .build()
                .finish();

            stack.add_positioned_child(
                tooltip,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -4.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}
