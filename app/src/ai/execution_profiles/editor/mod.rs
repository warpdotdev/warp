use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::model_menu_items::available_model_menu_items;
use crate::ai::execution_profiles::{
    profiles::{AIExecutionProfilesModel, AIExecutionProfilesModelEvent, ClientProfileId},
    AIExecutionProfile, ActionPermission, WriteToPtyPermission,
};
use crate::ai::llms::{
    DisableReason, LLMContextWindow, LLMId, LLMInfo, LLMPreferences, LLMPreferencesEvent,
};
use crate::ai::paths::host_native_absolute_path;
use crate::editor::InteractionState;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::settings::{AISettings, AISettingsChangedEvent, AgentModeCommandExecutionPredicate};
use crate::ui_components::icons::Icon;
use crate::view_components::{
    action_button::{ActionButton, DangerSecondaryTheme},
    Dropdown, DropdownItem, FilterableDropdown, SubmittableTextInput, SubmittableTextInputEvent,
};
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspacesEvent;
use crate::TemplatableMCPServerManager;
use crate::UserWorkspaces;
use crate::{
    pane_group::{pane::view, BackingView, PaneConfiguration, PaneEvent},
    Appearance,
};
use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent};
use itertools::Itertools;
use regex::Regex;
use thousands::Separable;
use warp_core::ui::theme::color::internal_colors;
use warpui::fonts::Properties;
use warpui::platform::Cursor;
use warpui::ui_components::slider::SliderStateHandle;
use warpui::ui_components::switch::SwitchStateHandle;

use std::path::{Path, PathBuf};
use warpui::{
    elements::{
        Align, Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
        Container, CrossAxisAlignment, Expanded, Flex, Highlight, MouseStateHandle, ParentElement,
        PartialClickableElement, ScrollbarWidth, Text,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const MODEL_MENU_WIDTH: f32 = 250.;

/// Renders a footer banner for model dropdowns informing free-plan users that
/// frontier models require an upgrade, with a clickable "Upgrade" link.
fn render_upgrade_footer(
    upgrade_mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let surface = theme.surface_2();
    let text_color = theme.main_text_color(surface);

    let info_icon = ConstrainedBox::new(
        warp_core::ui::Icon::Info
            .to_warpui_icon(text_color)
            .finish(),
    )
    .with_width(16.)
    .with_height(16.)
    .finish();

    let label = "Frontier models are unavailable on free plans. Upgrade";
    let upgrade_start = label.len() - "Upgrade".len();
    let info_text = Text::new(
        label,
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(text_color.into())
    .with_single_highlight(
        Highlight::new()
            .with_properties(Properties::default())
            .with_foreground_color(internal_colors::accent_fg(theme).into()),
        (upgrade_start..label.len()).collect(),
    )
    .with_hoverable_char_range(
        upgrade_start..label.len(),
        upgrade_mouse_state,
        Some(Cursor::PointingHand),
        |_is_hovered, _ctx, _app| {},
    )
    .with_clickable_char_range(upgrade_start..label.len(), move |_modifiers, ctx, _app| {
        ctx.dispatch_typed_action(WorkspaceAction::ShowUpgrade);
    })
    .finish();

    let inner = Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Container::new(info_icon)
                    .with_margin_right(6.)
                    .with_margin_top(2.)
                    .finish(),
            )
            .with_child(Expanded::new(1., info_text).finish())
            .finish(),
    )
    .with_horizontal_padding(16.)
    .with_vertical_padding(6.)
    .with_background(internal_colors::fg_overlay_1(theme))
    .with_border(Border::top(1.).with_border_color(internal_colors::neutral_3(theme)))
    .finish();

    Container::new(inner).with_background(surface).finish()
}

#[derive(Default)]
struct TooltipMouseStateHandles {
    // Separate mouse state handles for each permission dropdown (for workspace override tooltips)
    apply_code_diffs_tooltip_mouse_state: MouseStateHandle,
    read_files_tooltip_mouse_state: MouseStateHandle,
    execute_commands_tooltip_mouse_state: MouseStateHandle,
    write_to_pty_tooltip_mouse_state: MouseStateHandle,
    computer_use_tooltip_mouse_state: MouseStateHandle,
    ask_user_question_tooltip_mouse_state: MouseStateHandle,
    call_mcp_servers_tooltip_mouse_state: MouseStateHandle,
    // Separate mouse state handles for text input editors (for workspace override tooltips)
    command_allowlist_editor_tooltip_mouse_state: MouseStateHandle,
    directory_allowlist_editor_tooltip_mouse_state: MouseStateHandle,
    mcp_allowlist_editor_tooltip_mouse_state: MouseStateHandle,
    mcp_denylist_editor_tooltip_mouse_state: MouseStateHandle,
}

pub mod manager;
pub use manager::*;

pub const HEADER_TEXT: &str = "Profile Editor";

#[derive(Debug, Clone)]
pub enum ExecutionProfileEditorViewEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionProfileEditorViewAction {
    Save,
    Close,
    SetBaseModel {
        id: LLMId,
    },
    /// Fired continuously while the user drags the context window slider.
    ContextWindowSliderDragged {
        value: u32,
    },
    /// Fired when the user commits a new context window value (slider drop,
    /// track click, or input box commit).
    SetContextWindowSize {
        value: u32,
    },
    SetCodingModel {
        id: LLMId,
    },
    SetFullTerminalUseModel {
        id: LLMId,
    },
    SetComputerUseModel {
        id: LLMId,
    },

    SetApplyCodeDiffs {
        permission: ActionPermission,
    },
    SetReadFiles {
        permission: ActionPermission,
    },

    SetExecuteCommands {
        permission: ActionPermission,
    },
    SetWriteToPty {
        permission: WriteToPtyPermission,
    },
    SetCallMcpServers {
        permission: ActionPermission,
    },
    SetComputerUse {
        permission: super::ComputerUsePermission,
    },
    SetAskUserQuestion {
        permission: super::AskUserQuestionPermission,
    },
    AddToCommandAllowlist {
        predicate: AgentModeCommandExecutionPredicate,
    },
    RemoveFromCommandAllowlist {
        predicate: AgentModeCommandExecutionPredicate,
    },
    AddToCommandDenylist {
        predicate: AgentModeCommandExecutionPredicate,
    },
    RemoveFromCommandDenylist {
        predicate: AgentModeCommandExecutionPredicate,
    },
    AddToDirectoryAllowlist {
        path: PathBuf,
    },
    RemoveFromDirectoryAllowlist {
        path: PathBuf,
    },
    AddToMCPAllowlist {
        id: uuid::Uuid,
    },
    RemoveFromMCPAllowlist {
        id: uuid::Uuid,
    },
    AddToMCPDenylist {
        id: uuid::Uuid,
    },
    RemoveFromMCPDenylist {
        id: uuid::Uuid,
    },
    DeleteProfile,
    SetPlanAutoSync {
        enabled: bool,
    },
    SetWebSearchEnabled {
        enabled: bool,
    },
}

pub struct ExecutionProfileEditorView {
    profile_id: ClientProfileId,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    clipped_scroll_state: ClippedScrollStateHandle,
    base_model_dropdown: ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
    context_window_slider_state: SliderStateHandle,
    context_window_editor: ViewHandle<EditorView>,
    last_synced_context_window_editor_value: Option<u32>,
    coding_model_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    full_terminal_use_model_dropdown:
        ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
    computer_use_model_dropdown: ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
    apply_code_diffs_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    read_files_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    execute_commands_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    write_to_pty_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    call_mcp_servers_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    computer_use_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    ask_user_question_dropdown: ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
    command_allowlist_editor: ViewHandle<SubmittableTextInput>,
    command_denylist_editor: ViewHandle<SubmittableTextInput>,
    directory_allowlist_editor: ViewHandle<SubmittableTextInput>,
    command_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    command_denylist_mouse_state_handles: Vec<MouseStateHandle>,
    command_denylist_tooltip_mouse_state_handles: Vec<MouseStateHandle>,
    directory_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    mcp_allowlist_dropdown: ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
    mcp_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    mcp_denylist_dropdown: ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
    mcp_denylist_mouse_state_handles: Vec<MouseStateHandle>,
    profile_name_editor: ViewHandle<EditorView>,
    delete_button: ViewHandle<ActionButton>,
    tooltip_mouse_state_handles: TooltipMouseStateHandles,
    plan_auto_sync_switch: SwitchStateHandle,
    web_search_switch: SwitchStateHandle,
    upgrade_footer_mouse_state: MouseStateHandle,
}

impl ExecutionProfileEditorView {
    pub fn new(profile_id: ClientProfileId, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(HEADER_TEXT));

        let apply_code_diffs_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        ExecutionProfileEditorViewAction::SetApplyCodeDiffs {
                            permission: ActionPermission::AgentDecides,
                        },
                    ),
                    DropdownItem::new(
                        "Always allow",
                        ExecutionProfileEditorViewAction::SetApplyCodeDiffs {
                            permission: ActionPermission::AlwaysAllow,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetApplyCodeDiffs {
                            permission: ActionPermission::AlwaysAsk,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let read_files_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        ExecutionProfileEditorViewAction::SetReadFiles {
                            permission: ActionPermission::AgentDecides,
                        },
                    ),
                    DropdownItem::new(
                        "Always allow",
                        ExecutionProfileEditorViewAction::SetReadFiles {
                            permission: ActionPermission::AlwaysAllow,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetReadFiles {
                            permission: ActionPermission::AlwaysAsk,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let execute_commands_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        ExecutionProfileEditorViewAction::SetExecuteCommands {
                            permission: ActionPermission::AgentDecides,
                        },
                    ),
                    DropdownItem::new(
                        "Always allow",
                        ExecutionProfileEditorViewAction::SetExecuteCommands {
                            permission: ActionPermission::AlwaysAllow,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetExecuteCommands {
                            permission: ActionPermission::AlwaysAsk,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let write_to_pty_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Always allow",
                        ExecutionProfileEditorViewAction::SetWriteToPty {
                            permission: WriteToPtyPermission::AlwaysAllow,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetWriteToPty {
                            permission: WriteToPtyPermission::AlwaysAsk,
                        },
                    ),
                    DropdownItem::new(
                        "Ask on first write",
                        ExecutionProfileEditorViewAction::SetWriteToPty {
                            permission: WriteToPtyPermission::AskOnFirstWrite,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let call_mcp_servers_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        ExecutionProfileEditorViewAction::SetCallMcpServers {
                            permission: ActionPermission::AgentDecides,
                        },
                    ),
                    DropdownItem::new(
                        "Always allow",
                        ExecutionProfileEditorViewAction::SetCallMcpServers {
                            permission: ActionPermission::AlwaysAllow,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetCallMcpServers {
                            permission: ActionPermission::AlwaysAsk,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let computer_use_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Never",
                        ExecutionProfileEditorViewAction::SetComputerUse {
                            permission: super::ComputerUsePermission::Never,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetComputerUse {
                            permission: super::ComputerUsePermission::AlwaysAsk,
                        },
                    ),
                    DropdownItem::new(
                        "Always allow",
                        ExecutionProfileEditorViewAction::SetComputerUse {
                            permission: super::ComputerUsePermission::AlwaysAllow,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let ask_user_question_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Never ask",
                        ExecutionProfileEditorViewAction::SetAskUserQuestion {
                            permission: super::AskUserQuestionPermission::Never,
                        },
                    ),
                    DropdownItem::new(
                        "Ask unless auto-approve",
                        ExecutionProfileEditorViewAction::SetAskUserQuestion {
                            permission: super::AskUserQuestionPermission::AskExceptInAutoApprove,
                        },
                    ),
                    DropdownItem::new(
                        "Always ask",
                        ExecutionProfileEditorViewAction::SetAskUserQuestion {
                            permission: super::AskUserQuestionPermission::AlwaysAsk,
                        },
                    ),
                ],
                ctx,
            );
            dropdown
        });

        let mcp_allowlist_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_menu_header_to_static("Select MCP servers");
            dropdown
        });

        let mcp_denylist_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_menu_header_to_static("Select MCP servers");
            dropdown
        });

        let permissions = BlocklistAIPermissions::as_ref(ctx);
        let profile_data = permissions.permissions_profile_for_id(ctx, profile_id);

        let mcp_allowlist_mouse_state_handles = profile_data
            .mcp_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        let mcp_denylist_mouse_state_handles = profile_data
            .mcp_denylist
            .iter()
            .map(|_| Default::default())
            .collect();

        let base_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_menu_width(MODEL_MENU_WIDTH, ctx);
            dropdown
        });

        // Initialize the context window editor buffer with the profile's
        // persisted limit (or the active model's max as a sensible default).
        // The slider's current position is derived from the profile on each
        // render, so no local Cell is needed.
        let initial_context_window_value = initial_context_window_display_value(&profile_data, ctx);
        let context_window_slider_state = SliderStateHandle::default();
        let context_window_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(Appearance::as_ref(ctx).ui_font_size()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_buffer_text(&initial_context_window_value.separate_with_commas(), ctx);
            editor
        });
        let last_synced_context_window_editor_value = Some(initial_context_window_value);

        let coding_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_menu_width(MODEL_MENU_WIDTH, ctx);
            dropdown
        });
        let full_terminal_use_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_menu_width(MODEL_MENU_WIDTH, ctx);
            dropdown
        });
        let computer_use_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_menu_width(MODEL_MENU_WIDTH, ctx);
            dropdown
        });
        let command_allowlist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text("e.g. ls .*", ctx);
            input
        });

        let command_allowlist_mouse_state_handles = profile_data
            .command_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        let command_denylist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text("e.g. rm .*", ctx);
            input
        });

        let command_denylist_mouse_state_handles = profile_data
            .command_denylist
            .iter()
            .map(|_| Default::default())
            .collect();

        let directory_allowlist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx).validate_on_submit(|s| {
                let expanded = host_native_absolute_path(s, &None, &None);
                Path::new(&expanded).is_dir()
            });
            input.set_placeholder_text("e.g. ~/code-repos/repo", ctx);
            input
        });

        let directory_allowlist_mouse_state_handles = profile_data
            .directory_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        let profile_name_editor = ctx.add_view(|ctx| {
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    max_buffer_len: Some(super::PROFILE_NAME_MAX_LENGTH),
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("e.g. \"YOLO code\"", ctx);
            editor
        });

        let font_family = Appearance::as_ref(ctx).ui_font_family();

        profile_name_editor.update(ctx, |editor, ctx| {
            editor.set_font_size(12., ctx);
            editor.set_font_family(font_family, ctx);
        });

        Self::update_profile_name_editor(&profile_name_editor, &profile_data, ctx);

        let delete_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Delete profile", DangerSecondaryTheme)
                .with_icon(Icon::Trash)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ExecutionProfileEditorViewAction::DeleteProfile);
                })
        });

        let mut view = Self {
            profile_id,
            pane_configuration,
            focus_handle: None,
            clipped_scroll_state: Default::default(),
            base_model_dropdown,
            context_window_slider_state,
            context_window_editor,
            last_synced_context_window_editor_value,
            coding_model_dropdown,
            full_terminal_use_model_dropdown,
            computer_use_model_dropdown,
            apply_code_diffs_dropdown,
            read_files_dropdown,
            execute_commands_dropdown,
            write_to_pty_dropdown,
            call_mcp_servers_dropdown,
            computer_use_dropdown,
            ask_user_question_dropdown,
            command_allowlist_editor,
            command_denylist_editor,
            directory_allowlist_editor,
            command_allowlist_mouse_state_handles,
            command_denylist_mouse_state_handles,
            command_denylist_tooltip_mouse_state_handles: profile_data
                .command_denylist
                .iter()
                .map(|_| Default::default())
                .collect(),
            directory_allowlist_mouse_state_handles,
            mcp_allowlist_dropdown,
            mcp_allowlist_mouse_state_handles,
            mcp_denylist_dropdown,
            mcp_denylist_mouse_state_handles,
            profile_name_editor,
            delete_button,
            tooltip_mouse_state_handles: Default::default(),
            plan_auto_sync_switch: Default::default(),
            web_search_switch: Default::default(),
            upgrade_footer_mouse_state: Default::default(),
        };

        ctx.subscribe_to_view(&view.profile_name_editor, |view, _, event, ctx| {
            if let EditorEvent::Edited(_) = event {
                view.save_profile_name_if_valid(ctx);
            }
        });

        ctx.subscribe_to_view(&view.context_window_editor, |view, _, event, ctx| {
            view.handle_context_window_editor_event(event, ctx);
        });

        ctx.subscribe_to_view(&view.command_allowlist_editor, |view, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let predicate = match AgentModeCommandExecutionPredicate::new_regex(s) {
                    Ok(regex) => regex,
                    Err(e) => {
                        log::warn!(
                            "Failed to convert string to regex for cmd execution allowlist: {e}"
                        );
                        return;
                    }
                };
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_command_allowlist(view.profile_id, &predicate, ctx);
                });
                ctx.notify();
            }
        });

        ctx.subscribe_to_view(&view.command_denylist_editor, |view, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let predicate = match AgentModeCommandExecutionPredicate::new_regex(s) {
                    Ok(regex) => regex,
                    Err(e) => {
                        log::warn!(
                            "Failed to convert string to regex for cmd execution denylist: {e}"
                        );
                        return;
                    }
                };
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_command_denylist(view.profile_id, &predicate, ctx);
                });
                ctx.notify();
            }
        });

        ctx.subscribe_to_view(&view.directory_allowlist_editor, |view, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let expanded = host_native_absolute_path(s, &None, &None);
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_directory_allowlist(
                        view.profile_id,
                        &PathBuf::from(expanded),
                        ctx,
                    );
                });
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            let permissions = BlocklistAIPermissions::as_ref(ctx);
            let current_permissions = permissions.permissions_profile_for_id(ctx, me.profile_id);

            match event {
                LLMPreferencesEvent::UpdatedAvailableLLMs => {
                    Self::refresh_filterable_model_dropdown(
                        &me.base_model_dropdown,
                        current_permissions.base_model.clone(),
                        |prefs, app| prefs.get_base_llm_choices_for_agent_mode(app).collect_vec(),
                        |id| ExecutionProfileEditorViewAction::SetBaseModel { id },
                        |prefs| prefs.get_default_base_model().id.clone(),
                        &me.upgrade_footer_mouse_state,
                        ctx,
                    );
                    Self::refresh_coding_model_dropdown(
                        &me.coding_model_dropdown,
                        current_permissions.coding_model.clone(),
                        ctx,
                    );
                    Self::refresh_filterable_model_dropdown(
                        &me.full_terminal_use_model_dropdown,
                        current_permissions.cli_agent_model.clone(),
                        |prefs, app| prefs.get_cli_agent_llm_choices(app).collect_vec(),
                        |id| ExecutionProfileEditorViewAction::SetFullTerminalUseModel { id },
                        |prefs| prefs.get_default_cli_agent_model().id.clone(),
                        &me.upgrade_footer_mouse_state,
                        ctx,
                    );
                    Self::refresh_filterable_model_dropdown(
                        &me.computer_use_model_dropdown,
                        current_permissions.computer_use_model.clone(),
                        |prefs, _| prefs.get_computer_use_llm_choices().collect_vec(),
                        |id| ExecutionProfileEditorViewAction::SetComputerUseModel { id },
                        |prefs| prefs.get_default_computer_use_model().id.clone(),
                        &me.upgrade_footer_mouse_state,
                        ctx,
                    );
                    me.sync_context_window_editor(ctx, false);
                }
                LLMPreferencesEvent::UpdatedActiveAgentModeLLM => {
                    Self::refresh_filterable_model_dropdown(
                        &me.base_model_dropdown,
                        current_permissions.base_model.clone(),
                        |prefs, app| prefs.get_base_llm_choices_for_agent_mode(app).collect_vec(),
                        |id| ExecutionProfileEditorViewAction::SetBaseModel { id },
                        |prefs| prefs.get_default_base_model().id.clone(),
                        &me.upgrade_footer_mouse_state,
                        ctx,
                    );
                    me.sync_context_window_editor(ctx, false);
                }
                LLMPreferencesEvent::UpdatedActiveCodingLLM => {
                    Self::refresh_coding_model_dropdown(
                        &me.coding_model_dropdown,
                        current_permissions.coding_model.clone(),
                        ctx,
                    );
                }
            }
        });

        // Refresh model dropdowns when BYO API keys update so key icons reflect current state.
        ctx.subscribe_to_model(
            &ApiKeyManager::handle(ctx),
            |me, _model, _event: &ApiKeyManagerEvent, ctx| {
                let permissions = BlocklistAIPermissions::as_ref(ctx);
                let current_permissions =
                    permissions.permissions_profile_for_id(ctx, me.profile_id);
                Self::refresh_filterable_model_dropdown(
                    &me.base_model_dropdown,
                    current_permissions.base_model.clone(),
                    |prefs, app| prefs.get_base_llm_choices_for_agent_mode(app).collect_vec(),
                    |id| ExecutionProfileEditorViewAction::SetBaseModel { id },
                    |prefs| prefs.get_default_base_model().id.clone(),
                    &me.upgrade_footer_mouse_state,
                    ctx,
                );
                Self::refresh_coding_model_dropdown(
                    &me.coding_model_dropdown,
                    current_permissions.coding_model.clone(),
                    ctx,
                );
                me.sync_context_window_editor(ctx, false);
                ctx.notify();
            },
        );

        ctx.subscribe_to_model(
            &AIExecutionProfilesModel::handle(ctx),
            |me, _, event, ctx| {
                if matches!(event, AIExecutionProfilesModelEvent::ProfileUpdated(profile_id) if *profile_id == me.profile_id) {
                    me.refresh_profile_state(ctx);
                    me.update_mouse_state_handles(ctx);
                }
            },
        );

        let workspace = UserWorkspaces::handle(ctx);
        ctx.subscribe_to_model(&workspace, |me, workspace, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                Self::update_all_editor_interaction_states(me, workspace, ctx);
                me.update_mouse_state_handles(ctx);
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if let AISettingsChangedEvent::IsAnyAIEnabled { .. } = event {
                let workspace = UserWorkspaces::handle(ctx);
                Self::update_all_editor_interaction_states(me, workspace, ctx);
                me.sync_context_window_editor(ctx, true);
                ctx.notify();
            }
        });

        Self::update_all_editor_interaction_states(&view, workspace, ctx);

        view.refresh_profile_state(ctx);

        view.update_mouse_state_handles(ctx);

        view
    }

    pub fn profile_id(&self) -> ClientProfileId {
        self.profile_id
    }

    fn update_mouse_state_handles(&mut self, ctx: &mut ViewContext<Self>) {
        let app = ctx;
        let permissions = BlocklistAIPermissions::as_ref(app);
        let current_permissions = permissions.permissions_profile_for_id(app, self.profile_id);

        self.command_allowlist_mouse_state_handles = current_permissions
            .command_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        self.command_denylist_mouse_state_handles = current_permissions
            .command_denylist
            .iter()
            .map(|_| Default::default())
            .collect();

        self.command_denylist_tooltip_mouse_state_handles = current_permissions
            .command_denylist
            .iter()
            .map(|_| Default::default())
            .collect();

        self.directory_allowlist_mouse_state_handles = current_permissions
            .directory_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        self.mcp_allowlist_mouse_state_handles = current_permissions
            .mcp_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        self.mcp_denylist_mouse_state_handles = current_permissions
            .mcp_denylist
            .iter()
            .map(|_| Default::default())
            .collect();
    }

    fn refresh_profile_state(&mut self, ctx: &mut ViewContext<Self>) {
        let permissions = BlocklistAIPermissions::as_ref(ctx);
        let current_permissions = permissions.permissions_profile_for_id(ctx, self.profile_id);
        let ai_settings = AISettings::as_ref(ctx);

        let apply_code_diffs_disabled = !ai_settings.is_code_diffs_permissions_editable(ctx);
        let read_files_disabled = !ai_settings.is_read_files_permissions_editable(ctx);
        let execute_commands_disabled = !ai_settings.is_execute_commands_permissions_editable(ctx);
        let write_to_pty_disabled = !ai_settings.is_write_to_pty_permissions_editable(ctx);
        let computer_use_disabled = !ai_settings.is_computer_use_permissions_editable(ctx);
        let ask_user_question_disabled =
            !ai_settings.is_ask_user_question_permissions_editable(ctx);
        let mcp_disabled = !ai_settings.is_mcp_permission_editable(ctx);

        Self::refresh_filterable_model_dropdown(
            &self.base_model_dropdown,
            current_permissions.base_model.clone(),
            |prefs, app| prefs.get_base_llm_choices_for_agent_mode(app).collect_vec(),
            |id| ExecutionProfileEditorViewAction::SetBaseModel { id },
            |prefs| prefs.get_default_base_model().id.clone(),
            &self.upgrade_footer_mouse_state,
            ctx,
        );
        Self::refresh_coding_model_dropdown(
            &self.coding_model_dropdown,
            current_permissions.coding_model.clone(),
            ctx,
        );
        Self::refresh_filterable_model_dropdown(
            &self.full_terminal_use_model_dropdown,
            current_permissions.cli_agent_model.clone(),
            |prefs, app| prefs.get_cli_agent_llm_choices(app).collect_vec(),
            |id| ExecutionProfileEditorViewAction::SetFullTerminalUseModel { id },
            |prefs| prefs.get_default_cli_agent_model().id.clone(),
            &self.upgrade_footer_mouse_state,
            ctx,
        );
        Self::refresh_filterable_model_dropdown(
            &self.computer_use_model_dropdown,
            current_permissions.computer_use_model.clone(),
            |prefs, _| prefs.get_computer_use_llm_choices().collect_vec(),
            |id| ExecutionProfileEditorViewAction::SetComputerUseModel { id },
            |prefs| prefs.get_default_computer_use_model().id.clone(),
            &self.upgrade_footer_mouse_state,
            ctx,
        );

        Self::refresh_execution_profile_dropdown_menu(
            &self.apply_code_diffs_dropdown,
            current_permissions.apply_code_diffs,
            apply_code_diffs_disabled,
            ctx,
        );
        Self::refresh_execution_profile_dropdown_menu(
            &self.read_files_dropdown,
            current_permissions.read_files,
            read_files_disabled,
            ctx,
        );
        Self::refresh_execution_profile_dropdown_menu(
            &self.execute_commands_dropdown,
            current_permissions.execute_commands,
            execute_commands_disabled,
            ctx,
        );
        Self::refresh_write_to_pty_dropdown_menu(
            &self.write_to_pty_dropdown,
            current_permissions.write_to_pty,
            write_to_pty_disabled,
            ctx,
        );
        Self::refresh_execution_profile_dropdown_menu(
            &self.call_mcp_servers_dropdown,
            current_permissions.mcp_permissions,
            mcp_disabled,
            ctx,
        );
        Self::refresh_computer_use_dropdown_menu(
            &self.computer_use_dropdown,
            current_permissions.computer_use,
            computer_use_disabled,
            ctx,
        );
        Self::refresh_ask_user_question_dropdown_menu(
            &self.ask_user_question_dropdown,
            current_permissions.ask_user_question,
            ask_user_question_disabled,
            ctx,
        );
        Self::refresh_mcp_dropdown(
            &self.mcp_allowlist_dropdown,
            |uuid| ExecutionProfileEditorViewAction::AddToMCPAllowlist { id: uuid },
            &current_permissions.mcp_allowlist,
            &current_permissions.mcp_denylist,
            ctx,
        );
        Self::refresh_mcp_dropdown(
            &self.mcp_denylist_dropdown,
            |uuid| ExecutionProfileEditorViewAction::AddToMCPDenylist { id: uuid },
            &current_permissions.mcp_allowlist,
            &current_permissions.mcp_denylist,
            ctx,
        );

        Self::update_profile_name_editor(&self.profile_name_editor, &current_permissions, ctx);
        self.sync_context_window_editor(ctx, false);
    }

    fn refresh_execution_profile_dropdown_menu(
        menu: &ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
        current_permission: ActionPermission,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if !disabled {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let active = match current_permission {
                ActionPermission::AgentDecides | ActionPermission::Unknown => 0,
                ActionPermission::AlwaysAllow => 1,
                ActionPermission::AlwaysAsk => 2,
            };

            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_write_to_pty_dropdown_menu(
        menu: &ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
        current_permission: WriteToPtyPermission,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if !disabled {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let active = match current_permission {
                WriteToPtyPermission::AlwaysAllow => 0,
                WriteToPtyPermission::AlwaysAsk | WriteToPtyPermission::Unknown => 1,
                WriteToPtyPermission::AskOnFirstWrite => 2,
            };

            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_computer_use_dropdown_menu(
        menu: &ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
        current_permission: super::ComputerUsePermission,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if !disabled {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let active = match current_permission {
                super::ComputerUsePermission::Never | super::ComputerUsePermission::Unknown => 0,
                super::ComputerUsePermission::AlwaysAsk => 1,
                super::ComputerUsePermission::AlwaysAllow => 2,
            };

            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_ask_user_question_dropdown_menu(
        menu: &ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
        current_permission: super::AskUserQuestionPermission,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if !disabled {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let active = match current_permission {
                super::AskUserQuestionPermission::Never => 0,
                super::AskUserQuestionPermission::AskExceptInAutoApprove
                | super::AskUserQuestionPermission::Unknown => 1,
                super::AskUserQuestionPermission::AlwaysAsk => 2,
            };

            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_filterable_model_dropdown<G, A, D>(
        menu: &ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
        profile_model: Option<LLMId>,
        get_choices: G,
        create_action: A,
        get_default_id: D,
        upgrade_mouse_state: &MouseStateHandle,
        ctx: &mut ViewContext<Self>,
    ) where
        G: for<'a> FnOnce(&'a LLMPreferences, &AppContext) -> Vec<&'a LLMInfo>,
        A: Fn(LLMId) -> ExecutionProfileEditorViewAction,
        D: FnOnce(&LLMPreferences) -> LLMId,
    {
        menu.update(ctx, |dropdown, ctx| {
            let disabled_by_ai_toggle = !AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

            if disabled_by_ai_toggle {
                dropdown.set_disabled(ctx);
            } else {
                dropdown.set_enabled(ctx);
            }

            let llm_prefs = LLMPreferences::handle(ctx);
            let llm_prefs = llm_prefs.as_ref(ctx);
            let choices = get_choices(llm_prefs, ctx);

            let has_upgrade_gated_models = choices
                .iter()
                .any(|llm| matches!(llm.disable_reason, Some(DisableReason::RequiresUpgrade)));

            let items = available_model_menu_items(
                choices,
                |llm| create_action(llm.id.clone()).into(),
                None,
                None,
                false,
                false,
                ctx,
            );
            dropdown.set_rich_items(items, ctx);

            if has_upgrade_gated_models {
                let mouse_state = upgrade_mouse_state.clone();
                dropdown.set_footer(
                    move |app| render_upgrade_footer(mouse_state.clone(), app),
                    ctx,
                );
            } else {
                dropdown.clear_footer(ctx);
            }

            let llm_prefs = LLMPreferences::handle(ctx);
            let llm_prefs = llm_prefs.as_ref(ctx);
            let model_to_select = profile_model.unwrap_or_else(|| get_default_id(llm_prefs));
            dropdown.set_selected_by_action(create_action(model_to_select), ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_coding_model_dropdown(
        menu: &ViewHandle<Dropdown<ExecutionProfileEditorViewAction>>,
        profile_coding_model: Option<LLMId>,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |dropdown, ctx| {
            let disabled_by_ai_toggle = !AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

            if disabled_by_ai_toggle {
                dropdown.set_disabled(ctx);
            } else {
                dropdown.set_enabled(ctx);
            }

            let choices = LLMPreferences::as_ref(ctx)
                .get_coding_llm_choices(ctx)
                .collect_vec();

            let items = available_model_menu_items(
                choices,
                |llm| {
                    ExecutionProfileEditorViewAction::SetCodingModel { id: llm.id.clone() }.into()
                },
                None,
                None,
                false,
                false,
                ctx,
            );
            dropdown.set_rich_items(items, ctx);

            let model_to_select = profile_coding_model.unwrap_or_else(|| {
                LLMPreferences::as_ref(ctx)
                    .get_default_coding_model()
                    .id
                    .clone()
            });
            dropdown.set_selected_by_action(
                ExecutionProfileEditorViewAction::SetCodingModel {
                    id: model_to_select,
                },
                ctx,
            );
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_mcp_dropdown<F>(
        dropdown: &ViewHandle<FilterableDropdown<ExecutionProfileEditorViewAction>>,
        action_creator: F,
        profile_mcp_allowlist: &[uuid::Uuid],
        profile_mcp_denylist: &[uuid::Uuid],
        ctx: &mut ViewContext<Self>,
    ) where
        F: Fn(uuid::Uuid) -> ExecutionProfileEditorViewAction,
    {
        let all_mcp_servers = TemplatableMCPServerManager::get_all_cloud_synced_mcp_servers(ctx);
        dropdown.update(ctx, |dropdown, ctx| {
            let mcps_in_dropdown: Vec<(uuid::Uuid, String)> = all_mcp_servers
                .into_iter()
                .filter(|(uuid, _server_name)| {
                    !profile_mcp_allowlist.contains(uuid) && !profile_mcp_denylist.contains(uuid)
                })
                .collect();

            dropdown.set_items(
                mcps_in_dropdown
                    .iter()
                    .map(|(uuid, server_name)| {
                        DropdownItem::new(server_name, action_creator(*uuid))
                    })
                    .collect(),
                ctx,
            );
            ctx.notify()
        });
        ctx.notify();
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn save_profile_name_if_valid(&self, ctx: &mut ViewContext<Self>) {
        let new_name = self.profile_name_editor.read(ctx, |editor, ctx| {
            editor.buffer_text(ctx).trim().to_string()
        });

        if new_name.is_empty() {
            return;
        }

        let current_name = BlocklistAIPermissions::as_ref(ctx)
            .permissions_profile_for_id(ctx, self.profile_id)
            .name;

        if current_name != new_name {
            AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                profiles_model.set_profile_name(self.profile_id, &new_name, ctx);
            });
        }
    }

    fn update_profile_name_editor(
        profile_name_editor: &ViewHandle<EditorView>,
        profile_data: &AIExecutionProfile,
        ctx: &mut ViewContext<Self>,
    ) {
        profile_name_editor.update(ctx, |editor, ctx| {
            let display_name = if profile_data.is_default_profile {
                "Default".to_string()
            } else {
                profile_data.name.clone()
            };

            // Only update the buffer text if it's different from what's currently displayed
            // This preserves the cursor position when the text hasn't changed
            let current_text = editor.buffer_text(ctx);
            if current_text != display_name {
                editor.set_buffer_text(&display_name, ctx);
            }

            if profile_data.is_default_profile {
                editor.set_interaction_state(InteractionState::Disabled, ctx);
            }
        });
    }

    fn update_all_editor_interaction_states(
        view: &Self,
        workspace: ModelHandle<UserWorkspaces>,
        ctx: &mut ViewContext<Self>,
    ) {
        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let ai_autonomy_settings = workspace.as_ref(ctx).ai_autonomy_settings();

        Self::update_editor_interaction_state(
            view.command_denylist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled,
            ctx,
        );

        Self::update_editor_interaction_state(
            view.command_allowlist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled
                && !ai_autonomy_settings.has_override_for_execute_commands_allowlist(),
            ctx,
        );

        Self::update_editor_interaction_state(
            view.directory_allowlist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled && !ai_autonomy_settings.has_override_for_read_files_allowlist(),
            ctx,
        );
    }

    fn update_editor_interaction_state(
        editor: ViewHandle<EditorView>,
        is_editable: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        editor.update(ctx, |editor, ctx| {
            if !is_editable {
                editor.set_interaction_state(InteractionState::Disabled, ctx);
            } else {
                editor.set_interaction_state(InteractionState::Editable, ctx);
            }
        });
    }

    fn configurable_context_window(&self, app: &AppContext) -> Option<LLMContextWindow> {
        let profile =
            BlocklistAIPermissions::as_ref(app).permissions_profile_for_id(app, self.profile_id);
        profile.configurable_context_window(app)
    }

    fn current_context_window_display_value(&self, app: &AppContext) -> Option<u32> {
        let profile =
            BlocklistAIPermissions::as_ref(app).permissions_profile_for_id(app, self.profile_id);
        profile.context_window_display_value(app)
    }

    fn handle_context_window_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => {
                if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    self.sync_context_window_editor(ctx, true);
                    return;
                }
                let Some(cw) = self.configurable_context_window(ctx) else {
                    return;
                };
                let buffer_text = self.context_window_editor.as_ref(ctx).buffer_text(ctx);
                let cleaned: String = buffer_text
                    .chars()
                    .filter(|c| !c.is_whitespace() && *c != ',')
                    .collect();
                if let Ok(parsed) = cleaned.parse::<u32>() {
                    let clamped = parsed.clamp(cw.min, cw.max);
                    if Some(clamped) != self.current_context_window_display_value(ctx) {
                        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                            profiles_model.set_context_window_limit(
                                self.profile_id,
                                Some(clamped),
                                ctx,
                            );
                        });
                    }
                }
                self.sync_context_window_editor(ctx, true);
                ctx.notify();
            }
            _ => {}
        }
    }

    fn sync_context_window_editor(&mut self, ctx: &mut ViewContext<Self>, force: bool) {
        let Some(value) = self.current_context_window_display_value(ctx) else {
            self.last_synced_context_window_editor_value = None;
            self.context_window_slider_state.reset_offset();
            ctx.notify();
            return;
        };

        let formatted = value.separate_with_commas();
        let should_update = if force {
            true
        } else {
            match self.last_synced_context_window_editor_value {
                Some(last_value) => {
                    self.context_window_editor.as_ref(ctx).buffer_text(ctx)
                        == last_value.separate_with_commas()
                }
                None => true,
            }
        };

        if should_update {
            self.context_window_editor.update(ctx, |editor, ctx| {
                if editor.buffer_text(ctx) != formatted {
                    editor.system_reset_buffer_text(&formatted, ctx);
                }
            });
            self.last_synced_context_window_editor_value = Some(value);
            self.context_window_slider_state.reset_offset();
            ctx.notify();
        }
    }
}

fn initial_context_window_display_value(
    profile_data: &AIExecutionProfile,
    app: &AppContext,
) -> u32 {
    profile_data
        .context_window_display_value(app)
        .unwrap_or_else(|| {
            LLMPreferences::as_ref(app)
                .get_default_base_model()
                .context_window
                .default_max
        })
}

mod ui_helpers;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

impl View for ExecutionProfileEditorView {
    fn ui_name() -> &'static str {
        "ExecutionProfileEditorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        use ui_helpers::*;

        let permissions = BlocklistAIPermissions::as_ref(app);
        let profile_data = permissions.permissions_profile_for_id(app, self.profile_id);

        let mut column = Flex::column()
            .with_child(render_header_section(
                appearance,
                &self.profile_name_editor,
                profile_data.is_default_profile,
            ))
            .with_child(render_models_section(appearance, self, app))
            .with_child(render_permissions_section(
                appearance,
                self,
                &profile_data,
                app,
            ));

        if !profile_data.is_default_profile {
            column.add_child(ChildView::new(&self.delete_button).finish());
        }

        let content = Container::new(column.finish())
            .with_uniform_padding(16.)
            .finish();

        ClippedScrollable::vertical(
            self.clipped_scroll_state.clone(),
            Align::new(content).top_center().finish(),
            ScrollbarWidth::Auto,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish()
    }
}

impl Entity for ExecutionProfileEditorView {
    type Event = ExecutionProfileEditorViewEvent;
}

impl TypedActionView for ExecutionProfileEditorView {
    type Action = ExecutionProfileEditorViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ExecutionProfileEditorViewAction::Save => {
                // TODO: Implement save logic
                log::info!("Save profile");
            }
            ExecutionProfileEditorViewAction::Close => {
                ctx.emit(ExecutionProfileEditorViewEvent::Pane(PaneEvent::Close));
            }
            ExecutionProfileEditorViewAction::SetBaseModel { id } => {
                // Changing the base model resets any persisted context window
                // override — the new model may have a different range (or not
                // be configurable at all). The user can pick a new value for
                // the new model if they want one.
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_base_model(self.profile_id, Some(id.clone()), ctx);
                    profiles_model.set_context_window_limit(self.profile_id, None, ctx);
                });
                self.sync_context_window_editor(ctx, true);
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::ContextWindowSliderDragged { value } => {
                if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    self.sync_context_window_editor(ctx, true);
                    return;
                }
                // Transient drag update: reflect the current slider position
                // in the input box without persisting to the profile yet.
                // Persistence happens on SetContextWindowSize (drop / commit).
                if self.configurable_context_window(ctx).is_some() {
                    let formatted = value.separate_with_commas();
                    self.context_window_editor.update(ctx, |editor, ctx| {
                        editor.system_reset_buffer_text(&formatted, ctx);
                    });
                    ctx.notify();
                }
            }
            ExecutionProfileEditorViewAction::SetContextWindowSize { value } => {
                if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    self.sync_context_window_editor(ctx, true);
                    return;
                }
                let Some(cw) = self.configurable_context_window(ctx) else {
                    return;
                };
                let clamped = (*value).clamp(cw.min, cw.max);
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_context_window_limit(self.profile_id, Some(clamped), ctx);
                });
                self.sync_context_window_editor(ctx, true);
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetCodingModel { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_coding_model(self.profile_id, Some(id.clone()), ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetFullTerminalUseModel { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_cli_agent_model(self.profile_id, Some(id.clone()), ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetComputerUseModel { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_computer_use_model(self.profile_id, Some(id.clone()), ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetApplyCodeDiffs { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_apply_code_diffs(self.profile_id, permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetReadFiles { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_read_files(self.profile_id, permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetExecuteCommands { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_execute_commands(self.profile_id, permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetWriteToPty { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_write_to_pty(self.profile_id, permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetCallMcpServers { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_mcp_permissions(self.profile_id, permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetComputerUse { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_computer_use(self.profile_id, permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetAskUserQuestion { permission } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_ask_user_question(self.profile_id, *permission, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::AddToCommandAllowlist { predicate } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_command_allowlist(self.profile_id, predicate, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::RemoveFromCommandAllowlist { predicate } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.remove_from_command_allowlist(self.profile_id, predicate, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::AddToCommandDenylist { predicate } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_command_denylist(self.profile_id, predicate, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::RemoveFromCommandDenylist { predicate } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.remove_from_command_denylist(self.profile_id, predicate, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::AddToDirectoryAllowlist { path } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_directory_allowlist(self.profile_id, path, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::RemoveFromDirectoryAllowlist { path } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.remove_from_directory_allowlist(self.profile_id, path, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::AddToMCPAllowlist { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_mcp_allowlist(self.profile_id, id, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::RemoveFromMCPAllowlist { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.remove_from_mcp_allowlist(self.profile_id, id, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::AddToMCPDenylist { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.add_to_mcp_denylist(self.profile_id, id, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::RemoveFromMCPDenylist { id } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.remove_from_mcp_denylist(self.profile_id, id, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::DeleteProfile => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.delete_profile(self.profile_id, ctx);
                });
                ctx.emit(ExecutionProfileEditorViewEvent::Pane(PaneEvent::Close));
            }
            ExecutionProfileEditorViewAction::SetPlanAutoSync { enabled } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_autosync_plans_to_warp_drive(self.profile_id, *enabled, ctx);
                });
                ctx.notify();
            }
            ExecutionProfileEditorViewAction::SetWebSearchEnabled { enabled } => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_web_search_enabled(self.profile_id, *enabled, ctx);
                });
                ctx.notify();
            }
        }
    }
}

impl BackingView for ExecutionProfileEditorView {
    type PaneHeaderOverflowMenuAction = ExecutionProfileEditorViewAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut warpui::ViewContext<Self>,
    ) {
        self.handle_action(_action, _ctx)
    }

    fn close(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        self.save_profile_name_if_valid(ctx);
        ctx.emit(ExecutionProfileEditorViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::Standard(view::StandardHeader {
            title: HEADER_TEXT.into(),
            title_secondary: None,
            title_style: None,
            title_clip_config: warpui::text_layout::ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: view::StandardHeaderOptions {
                always_show_icons: true,
                ..Default::default()
            },
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
