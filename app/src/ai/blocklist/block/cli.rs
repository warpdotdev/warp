use std::cmp::Ordering;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use lazy_static::lazy_static;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use parking_lot::{FairMutex, RwLock};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::report_error;
use warp_core::semantic_selection::SemanticSelection;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::clipboard::ClipboardContent;
use warpui::elements::new_scrollable::SingleAxisConfig;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, DropShadow, Empty, Expanded, Fill, Flex,
    FormattedTextElement, Highlight, HighlightedHyperlink, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, NewScrollable, OffsetPositioning, ParentElement,
    PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition, SelectableArea,
    SelectionHandle, Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Stack, Text,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::keymap::{EditableBinding, Keystroke};
use warpui::platform::{Cursor, OperatingSystem};
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use super::cli_controller::{CLISubagentController, CLISubagentEvent, UserTakeOverReason};
use super::model::{AIBlockModel, AIBlockModelHelper, AIBlockModelImpl, AIBlockOutputStatus};
use super::view_impl::common::{
    render_debug_footer, render_failed_output, render_informational_footer, render_text_sections,
    DebugFooterProps, FailedOutputProps, TextSectionsProps,
};
use super::view_impl::output::are_all_text_sections_empty;
use super::{EmbeddedCodeEditorView, SecretRedactionState, TableSectionHandles};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::icons::yellow_stop_icon;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionType, AIAgentInput, AIAgentOutput, AIAgentOutputMessageType, AIAgentPtyWriteMode,
    AIAgentText, AIAgentTextSection, CancellationReason, ProgrammingLanguage, WebSearchStatus,
};
use crate::ai::blocklist::block::view_impl::common::{
    render_query_text, UserQueryProps, BLOCKED_ACTION_MESSAGE_FOR_GREP_OR_FILE_GLOB,
    BLOCKED_ACTION_MESSAGE_FOR_READING_FILES, BLOCKED_ACTION_MESSAGE_FOR_SEARCHING_CODEBASE,
    BLOCKED_ACTION_MESSAGE_FOR_WRITE_TO_LONG_RUNNING_SHELL_COMMAND,
    LOAD_OUTPUT_MESSAGE_FOR_FILE_GLOB, LOAD_OUTPUT_MESSAGE_FOR_GREP,
    LOAD_OUTPUT_MESSAGE_FOR_READING_FILES, LOAD_OUTPUT_MESSAGE_FOR_SEARCH_CODEBASE,
    LOAD_OUTPUT_MESSAGE_FOR_WEB_SEARCH,
};
use crate::ai::blocklist::block::TextLocation;
use crate::ai::blocklist::code_block::CodeSnippetButtonHandles;
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ai::blocklist::permissions::is_agent_mode_autonomy_allowed;
use crate::ai::blocklist::{
    BlocklistAIActionModel, BlocklistAIHistoryEvent, BlocklistAIPermissions,
};
use crate::ai::control_code_parser::{parse_control_codes_from_bytes, ParsedControlCodeOutput};
use crate::ai::execution_profiles::profiles::{
    AIExecutionProfilesModel, AIExecutionProfilesModelEvent,
};
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::code::editor_management::CodeSource;
use crate::editor::InteractionState;
use crate::menu::{Event as MenuEvent, Menu, MenuItemFields, MenuVariant};
use crate::server::telemetry::TelemetryEvent;
use crate::settings::AISettings;
use crate::settings_view::SettingsSection;
use crate::terminal::input::SET_INPUT_MODE_TERMINAL_ACTION_NAME;
use crate::terminal::model::block::BlockId;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::{ShellLaunchData, TerminalModel};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::util::link_detection::{detect_links, DetectedLinksState};
use crate::view_components::action_button::{
    ButtonSize, KeystrokeSource, NakedTheme, PrimaryTheme,
};
use crate::view_components::compactible_action_button::{
    render_compact_and_regular_button_rows, CompactibleActionButton, RenderCompactibleActionButton,
};
use crate::view_components::compactible_split_action_button::CompactibleSplitActionButton;
use crate::view_components::DismissibleToast;
use crate::workspace::WorkspaceAction;
use crate::{send_telemetry_from_ctx, BlocklistAIHistoryModel, ToastStack};
const MENU_WIDTH: f32 = 200.0;
const MAX_HEIGHT: f32 = 320.0;
const AVATAR_RIGHT_MARGIN: f32 = 8.;
const CONTENT_PADDING: f32 = 12.;
const ALLOW_ACTION_POSITION_ID: &str = "allow-action-position-id";
const USER_QUERY_POSITION_ID: &str = "cli-subagent-user-query-position-id";

lazy_static! {
    static ref ACCEPT_KEYSTROKE: Keystroke = Keystroke {
        key: "enter".to_owned(),
        ..Default::default()
    };
    static ref REJECT_KEYSTROKE: Keystroke =
        Keystroke::parse("ctrl-c").expect("Failed to parse take over keystroke");
    static ref AUTO_APPROVE_KEYSTROKE: Keystroke = {
        let binding = if OperatingSystem::get().is_mac() {
            "cmd-shift-I"
        } else {
            "ctrl-shift-I"
        };
        Keystroke::parse(binding).expect("Failed to parse auto approve keystroke")
    };
}

const HAS_PENDING_CLI_ACTION_CONTEXT_KEY: &str = "HasPendingCLIAgentAction";
const HAS_PENDING_NON_TRANSFER_CONTROL_ACTION_CONTEXT_KEY: &str =
    "HasPendingNonTransferControlCLIAgentAction";
const BLOCKED_ACTION_MESSAGE_FOR_TRANSFER_CONTROL: &str = "Agent is asking you to take control.";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    use warpui::keymap::FixedBinding;

    app.register_fixed_bindings([
        FixedBinding::new(
            ACCEPT_KEYSTROKE.normalized(),
            CLISubagentAction::ExecuteBlockedAction,
            id!(CLISubagentView::ui_name()) & id!(HAS_PENDING_CLI_ACTION_CONTEXT_KEY),
        ),
        FixedBinding::new(
            REJECT_KEYSTROKE.normalized(),
            CLISubagentAction::RejectBlockedAction {
                should_user_take_over: false,
            },
            id!(CLISubagentView::ui_name()) & id!(HAS_PENDING_CLI_ACTION_CONTEXT_KEY),
        ),
        FixedBinding::new(
            "escape",
            CLISubagentAction::RejectBlockedAction {
                should_user_take_over: true,
            },
            id!(CLISubagentView::ui_name())
                & id!(HAS_PENDING_NON_TRANSFER_CONTROL_ACTION_CONTEXT_KEY),
        ),
        FixedBinding::new(
            AUTO_APPROVE_KEYSTROKE.normalized(),
            CLISubagentAction::ExecuteAndAutoApprove,
            id!(CLISubagentView::ui_name())
                & id!(HAS_PENDING_NON_TRANSFER_CONTROL_ACTION_CONTEXT_KEY),
        ),
    ]);
    app.register_editable_bindings([EditableBinding::new(
        SET_INPUT_MODE_TERMINAL_ACTION_NAME,
        "Take control of running command",
        CLISubagentAction::TakeControlOfRunningCommand,
    )
    .with_mac_key_binding("cmd-i")
    .with_linux_or_windows_key_binding("ctrl-i")
    .with_context_predicate(
        id!(CLISubagentView::ui_name()) & id!(HAS_PENDING_CLI_ACTION_CONTEXT_KEY),
    )]);
}

#[derive(Default)]
struct StateHandles {
    invalid_api_key_button_handle: MouseStateHandle,
    debug_copy_button_handle: MouseStateHandle,
    submit_issue_button_handle: MouseStateHandle,
    query_selection_handle: SelectionHandle,
    output_selection_handle: SelectionHandle,
    action_selection_handle: SelectionHandle,
    speedbump_checkbox_handle: MouseStateHandle,
    ai_settings_link: HighlightedHyperlink,
    output_scroll_state: ClippedScrollStateHandle,
    action_scroll_state: ClippedScrollStateHandle,
    input_scroll_state: ClippedScrollStateHandle,
    query_scroll_state: ClippedScrollStateHandle,
    input_hover_state: MouseStateHandle,
    dismiss_input_mouse_state: MouseStateHandle,
}

pub struct CLISubagentView {
    block_id: BlockId,
    model: Rc<dyn AIBlockModel<View = CLISubagentView>>,
    subagent_controller: ModelHandle<CLISubagentController>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,

    state_handles: StateHandles,
    code_editor_views: Vec<EmbeddedCodeEditorView>,
    code_editor_buttons: Vec<CodeSnippetButtonHandles>,
    table_section_handles: Vec<TableSectionHandles>,

    secret_redaction_state: SecretRedactionState,
    link_detection_state: DetectedLinksState,
    selected_text: Arc<RwLock<Option<String>>>,

    allow_button: CompactibleSplitActionButton,
    reject_button: CompactibleActionButton,
    take_over_button: CompactibleActionButton,
    transfer_control_button: CompactibleActionButton,
    allow_menu: ViewHandle<Menu<CLISubagentAction>>,
    is_allow_menu_open: bool,
    always_allow_write_to_pty_checked: bool,
    always_allow_read_files_checked: bool,

    is_input_dismissed: bool,
    input_dismiss_timer_handle: Option<SpawnedFutureHandle>,

    current_working_directory: Option<String>,
    shell_launch_data: Option<ShellLaunchData>,
}

impl CLISubagentView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_id: BlockId,
        action_model: ModelHandle<BlocklistAIActionModel>,
        subagent_controller: ModelHandle<CLISubagentController>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        conversation_id: AIConversationId,
        task_id: TaskId,
        current_working_directory: Option<String>,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let allow_button = CompactibleSplitActionButton::new(
            "Allow".to_string(),
            Some(KeystrokeSource::Fixed(ACCEPT_KEYSTROKE.clone())),
            ButtonSize::Small,
            CLISubagentAction::ExecuteBlockedAction,
            CLISubagentAction::ToggleAllowMenu,
            Icon::Check,
            true,
            Some(ALLOW_ACTION_POSITION_ID.to_string()),
            ctx,
        );

        let reject_button = CompactibleActionButton::new(
            "Refine".to_string(),
            Some(KeystrokeSource::Fixed(REJECT_KEYSTROKE.clone())),
            ButtonSize::Small,
            CLISubagentAction::RejectBlockedAction {
                should_user_take_over: false,
            },
            Icon::X,
            Arc::new(NakedTheme),
            ctx,
        );

        let take_over_button = CompactibleActionButton::new(
            "Take over".to_string(),
            Some(KeystrokeSource::Binding(
                SET_INPUT_MODE_TERMINAL_ACTION_NAME,
            )),
            ButtonSize::Small,
            CLISubagentAction::RejectBlockedAction {
                should_user_take_over: true,
            },
            Icon::Hand,
            Arc::new(NakedTheme),
            ctx,
        );
        let transfer_control_button = CompactibleActionButton::new(
            "Take control".to_string(),
            Some(KeystrokeSource::Binding(
                SET_INPUT_MODE_TERMINAL_ACTION_NAME,
            )),
            ButtonSize::Small,
            CLISubagentAction::ExecuteBlockedAction,
            Icon::Hand,
            Arc::new(PrimaryTheme),
            ctx,
        );

        let allow_menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_width(MENU_WIDTH)
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        allow_menu.update(ctx, |menu, ctx| {
            menu.set_items(
                vec![
                    MenuItemFields::new("Accept".to_string())
                        .with_key_shortcut_label(Some(ACCEPT_KEYSTROKE.displayed()))
                        .with_on_select_action(CLISubagentAction::ExecuteBlockedAction)
                        .into_item(),
                    MenuItemFields::new("Auto-approve".to_string())
                        .with_key_shortcut_label(Some(AUTO_APPROVE_KEYSTROKE.displayed()))
                        .with_on_select_action(CLISubagentAction::ExecuteAndAutoApprove)
                        .into_item(),
                ],
                ctx,
            );
        });
        ctx.subscribe_to_view(&allow_menu, |me, _menu, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.is_allow_menu_open = false;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        // We want to default the checkbox to true when rendering the speedbump for the first time.
        // Otherwise, update it when the permission changes.
        let always_allow_write_to_pty_checked = if should_show_write_to_pty_speedbump(ctx) {
            true
        } else {
            BlocklistAIPermissions::as_ref(ctx)
                .can_write_to_pty(&conversation_id, Some(ctx.view_id()), ctx)
                .is_always_allow()
        };

        let always_allow_read_files_checked = if should_show_read_files_speedbump(ctx) {
            true
        } else {
            BlocklistAIPermissions::as_ref(ctx)
                .can_read_files(Some(&conversation_id), Vec::new(), Some(ctx.view_id()), ctx)
                .is_allowed()
        };

        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let mut task_id_clone = task_id.clone();
        ctx.subscribe_to_model(
            &history_model,
            move |me, _history_model, event, ctx| match event {
                BlocklistAIHistoryEvent::UpgradedTask {
                    optimistic_id: old_id,
                    server_id: new_id,
                    ..
                } if *old_id == task_id_clone => {
                    task_id_clone = new_id.clone();
                }
                BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    task_id,
                    conversation_id,
                    ..
                } => {
                    if task_id == &task_id_clone {
                        if let Ok(model) = AIBlockModelImpl::<CLISubagentView>::new(
                            *exchange_id,
                            *conversation_id,
                            false,
                            false,
                            ctx,
                        ) {
                            model.on_updated_output(
                                Box::new(|me, ctx| {
                                    me.handle_updated_exchange_output(ctx);
                                }),
                                ctx,
                            );
                            me.model = Rc::new(model);
                            me.code_editor_views = Default::default();
                            me.code_editor_buttons = Default::default();
                            me.table_section_handles = Default::default();
                            me.secret_redaction_state.reset();
                            me.set_state_from_updated_inputs(ctx);
                        }
                        ctx.notify();
                    }
                }
                _ => {
                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(
            &AIExecutionProfilesModel::handle(ctx),
            move |me, _, event, ctx| {
                let should_update_permissions = match event {
                    AIExecutionProfilesModelEvent::UpdatedActiveProfile { terminal_view_id } => {
                        *terminal_view_id == me.terminal_view_id
                    }
                    AIExecutionProfilesModelEvent::ProfileUpdated(profile_id) => {
                        let active_profile = AIExecutionProfilesModel::as_ref(ctx)
                            .active_profile(Some(me.terminal_view_id), ctx);
                        *profile_id == *active_profile.id()
                    }
                    _ => false,
                };
                if should_update_permissions {
                    let ai_permission = BlocklistAIPermissions::as_ref(ctx);
                    if should_show_write_to_pty_speedbump(ctx) {
                        me.always_allow_write_to_pty_checked = ai_permission
                            .can_write_to_pty(&me.conversation_id, Some(me.terminal_view_id), ctx)
                            .is_always_allow();
                    }
                    if should_show_read_files_speedbump(ctx) {
                        me.always_allow_read_files_checked = ai_permission
                            .get_read_files_setting(ctx, Some(me.terminal_view_id))
                            .is_always_allow();
                    }
                    ctx.notify();
                }
            },
        );
        let exchange_id = history_model
            .as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| {
                c.get_task(&task_id)
                    .and_then(|t| t.last_exchange().map(|e| e.id))
            })
            .expect("Exchange exists.");
        let model = AIBlockModelImpl::<CLISubagentView>::new(
            exchange_id,
            conversation_id,
            false,
            false,
            ctx,
        )
        .expect("Exchange exists.");
        model.on_updated_output(
            Box::new(|me, ctx| {
                me.handle_updated_exchange_output(ctx);
            }),
            ctx,
        );

        ctx.subscribe_to_model(&subagent_controller, |me, _, event, ctx| match event {
            CLISubagentEvent::UpdatedControl { block_id, .. } => {
                if *block_id == me.block_id {
                    ctx.notify();
                }
            }
            CLISubagentEvent::ToggledHideResponses => {
                me.reset_input_dismiss_timer(ctx);
                ctx.notify();
            }
            _ => {}
        });

        let mut view = Self {
            block_id,
            model: Rc::new(model),
            action_model,
            terminal_model,
            subagent_controller,
            conversation_id,
            terminal_view_id: ctx.view_id(),
            link_detection_state: Default::default(),
            code_editor_views: Default::default(),
            code_editor_buttons: Default::default(),
            table_section_handles: Default::default(),
            secret_redaction_state: Default::default(),
            state_handles: Default::default(),
            allow_button,
            reject_button,
            take_over_button,
            transfer_control_button,
            allow_menu,
            is_allow_menu_open: false,
            always_allow_write_to_pty_checked,
            always_allow_read_files_checked,
            is_input_dismissed: false,
            input_dismiss_timer_handle: None,
            current_working_directory,
            shell_launch_data,
            selected_text: Arc::new(RwLock::new(None)),
        };
        view.set_state_from_updated_inputs(ctx);
        view
    }

    fn execute_pending_action(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(blocked_action) = self.model.blocked_action(&self.action_model, ctx) else {
            return;
        };

        self.action_model.update(ctx, |action_model, ctx| {
            action_model.execute_next_action_for_user(self.conversation_id, ctx);
        });

        self.maybe_update_speedbump(&blocked_action.action, ctx);
    }

    fn has_pending_transfer_control_action(&self, app: &AppContext) -> bool {
        self.model
            .blocked_action(&self.action_model, app)
            .is_some_and(|action| {
                matches!(
                    action.action,
                    AIAgentActionType::TransferShellCommandControlToUser { .. }
                )
            })
    }

    fn handle_execute_blocked_action(
        &mut self,
        is_autoexecuted: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.execute_pending_action(ctx);
        if is_autoexecuted {
            self.enable_autoexecute_override(ctx);
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::CLISubagentActionExecuted {
                conversation_id: self.conversation_id,
                block_id: self.block_id.clone(),
                is_autoexecuted,
            },
            ctx
        );
    }

    fn handle_reject_blocked_action(
        &mut self,
        should_user_take_over: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.reject_blocked_action(should_user_take_over, ctx);

        send_telemetry_from_ctx!(
            TelemetryEvent::CLISubagentActionRejected {
                conversation_id: self.conversation_id,
                block_id: self.block_id.clone(),
                user_took_over: should_user_take_over,
            },
            ctx
        );
    }

    fn take_control_of_running_command(&mut self, ctx: &mut ViewContext<Self>) {
        if self.has_pending_transfer_control_action(ctx) {
            self.handle_execute_blocked_action(false, ctx);
        } else {
            self.handle_reject_blocked_action(true, ctx);
        }
    }
    fn reject_blocked_action(&mut self, should_user_take_over: bool, ctx: &mut ViewContext<Self>) {
        let Some(blocked_action) = self.model.blocked_action(&self.action_model, ctx) else {
            return;
        };

        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_action_with_id(
                self.conversation_id,
                &blocked_action.id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });

        if should_user_take_over {
            self.subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
            });
            ctx.notify();
        }

        self.maybe_update_speedbump(&blocked_action.action, ctx);
    }

    fn enable_autoexecute_override(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&self.conversation_id)
        else {
            return;
        };
        if !conversation.autoexecute_any_action() {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(
                    &self.conversation_id,
                    self.terminal_view_id,
                    ctx,
                );
            });
        }
    }
    fn toggle_allow_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_allow_menu_open = !self.is_allow_menu_open;
        if self.is_allow_menu_open {
            ctx.focus(&self.allow_menu);
        }
        ctx.notify();
    }

    // If the speedbump is shown, we update the settings such that the speedbump won't be shown again, and the permission reflect the checked value.
    // This is called on any user action instead of on render time to ensure the state is updated correctly.
    fn maybe_update_speedbump(&mut self, action: &AIAgentActionType, ctx: &mut ViewContext<Self>) {
        match action {
            AIAgentActionType::WriteToLongRunningShellCommand { .. }
                if should_show_write_to_pty_speedbump(ctx) =>
            {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .should_show_agent_mode_write_to_pty_speedbump
                        .set_value(false, ctx);
                });

                BlocklistAIPermissions::handle(ctx).update(ctx, |permissions, ctx| {
                    if let Err(e) = permissions.set_always_allow_write_to_pty(
                        self.always_allow_write_to_pty_checked,
                        self.terminal_view_id,
                        ctx,
                    ) {
                        report_error!(e);
                    }
                });
                ctx.notify();
            }
            AIAgentActionType::SearchCodebase(_)
            | AIAgentActionType::ReadFiles(_)
            | AIAgentActionType::Grep { .. }
            | AIAgentActionType::FileGlobV2 { .. } => {
                if should_show_read_files_speedbump(ctx) {
                    AISettings::handle(ctx).update(ctx, |settings, ctx| {
                        let _ = settings
                            .should_show_agent_mode_autoread_files_speedbump
                            .set_value(false, ctx);
                    });

                    BlocklistAIPermissions::handle(ctx).update(ctx, |permissions, ctx| {
                        if let Err(e) = permissions.set_always_allow_read_files(
                            self.always_allow_read_files_checked,
                            self.terminal_view_id,
                            ctx,
                        ) {
                            report_error!(e);
                        }
                    });
                    ctx.notify();
                }
            }
            _ => {}
        }
    }

    fn handle_updated_exchange_output(&mut self, ctx: &mut ViewContext<Self>) {
        match self.model.status(ctx) {
            AIBlockOutputStatus::Pending => {
                self.secret_redaction_state.reset();
            }
            AIBlockOutputStatus::PartiallyReceived { output } => {
                let output = output.get();
                self.handle_updated_output(&output, ctx);
            }
            AIBlockOutputStatus::Complete { output } => {
                let output = output.get();
                self.handle_updated_output(&output, ctx);
                self.handle_complete_output(&output, ctx);
            }
            AIBlockOutputStatus::Cancelled { partial_output, .. } => {
                if let Some(output) = partial_output.as_ref() {
                    let output = output.get();
                    self.handle_updated_output(&output, ctx);
                }
            }
            AIBlockOutputStatus::Failed { .. } => (),
        }
        ctx.notify();
    }

    fn handle_updated_output(&mut self, output: &AIAgentOutput, ctx: &mut ViewContext<Self>) {
        // Build the views and stream new content for suggested code snippets.
        output
            .all_text()
            .flat_map(|text| text.sections.iter())
            .filter_map(|section| match section {
                AIAgentTextSection::Code {
                    code,
                    language,
                    source,
                } => Some((code, language, source)),
                _ => None,
            })
            .enumerate()
            .for_each(|(index, (code, language, source))| {
                self.handle_code_section_stream_update(index, code, language, source, ctx);
            });

        if get_secret_obfuscation_mode(ctx).should_redact_secret() {
            self.secret_redaction_state
                .run_incremental_redaction_on_partial_output(
                    output,
                    get_secret_obfuscation_mode(ctx).is_visually_obfuscated(),
                );
        }
    }

    fn handle_code_section_stream_update(
        &mut self,
        index: usize,
        code: &str,
        language: &Option<ProgrammingLanguage>,
        source: &Option<CodeSource>,
        ctx: &mut ViewContext<Self>,
    ) {
        match self.code_editor_views.get_mut(index) {
            Some(embedded_view) => {
                embedded_view.view.update(ctx, |view, ctx| {
                    // The language and starting line number may not be specified in the output for the first iteration.
                    // Only set the language/starting line number the first time that they are specified or if they change.
                    if embedded_view.language != *language {
                        embedded_view.language = language.clone();
                        if let Some(extension) = language
                            .as_ref()
                            .and_then(|language| language.to_extension())
                        {
                            // Since this is a code snippet, construct a fake path name for looking up the language.
                            let fake_path = format!("/snippet.{extension}");
                            view.set_language_with_local_path(Path::new(&fake_path), ctx);
                        }
                    }
                    let starting_line_number = source.as_ref().and_then(|s| {
                        if let CodeSource::Link { range_start, .. } = s {
                            range_start.as_ref().map(|ls| ls.line_num)
                        } else {
                            None
                        }
                    });
                    if view.starting_line_number() != starting_line_number {
                        view.set_starting_line_number(starting_line_number);
                    }

                    // Update the buffer with just the new or deleted range.
                    // Assumption: Only the end of the string is updated.
                    // Assumption: The only time text is deleted is at the end of parsing, where it has partially
                    // received the ``` end marker.
                    // Ex: Iteration 57: "a += 12\n``"
                    // Ex: Iteration 58: "a += 12"
                    match code.len().cmp(&embedded_view.length) {
                        Ordering::Greater => {
                            view.append_at_end(&code[embedded_view.length..], ctx);
                            ctx.notify();
                        }
                        Ordering::Less => {
                            view.truncate(code.len(), ctx);
                            ctx.notify();
                        }
                        Ordering::Equal => {}
                    }
                    embedded_view.length = code.len();
                });
            }
            None => {
                let view = ctx.add_typed_action_view(|ctx| {
                    CodeEditorView::new(
                        None,
                        None,
                        CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight),
                        ctx,
                    )
                    .with_can_show_diff_ui(false)
                });
                view.update(ctx, |view, ctx| {
                    view.set_starting_line_number({
                        source.as_ref().and_then(|s| match s {
                            CodeSource::Link { range_start, .. } => {
                                range_start.as_ref().map(|ls| ls.line_num)
                            }
                            _ => None,
                        })
                    });
                    view.set_show_current_line_highlights(false, ctx);
                    view.set_interaction_state(InteractionState::Selectable, ctx);
                    let state = InitialBufferState::plain_text(code);
                    view.reset(state, ctx);
                    ctx.notify();
                });
                ctx.subscribe_to_view(&view, |me, view, event, ctx| match event {
                    CodeEditorEvent::SelectionChanged => {
                        if view.as_ref(ctx).selected_text(ctx).is_some() {
                            me.clear_other_selections(Some(view.id()), ctx);
                            ctx.emit(CLISubagentViewEvent::TextSelected);
                        }
                    }
                    CodeEditorEvent::CopiedEmptyText => {
                        ctx.emit(CLISubagentViewEvent::CopiedEmptyText);
                    }
                    #[cfg(windows)]
                    CodeEditorEvent::WindowsCtrlC { .. } => {
                        ctx.emit(CLISubagentViewEvent::WindowsCtrlC);
                    }
                    _ => {}
                });
                self.code_editor_views.push(EmbeddedCodeEditorView {
                    view,
                    language: Default::default(),
                    length: code.len(),
                });
                self.code_editor_buttons.push(Default::default());
            }
        }
    }

    fn handle_complete_output(&mut self, output: &AIAgentOutput, ctx: &mut ViewContext<Self>) {
        // Run secret detection at the end of the stream to catch any secrets we might've missed while streaming,
        // due to secret patterns that may include whitespace within them (we delimit on whitespace with the optimized
        // secret detection approach).
        if get_secret_obfuscation_mode(ctx).is_visually_obfuscated() {
            self.secret_redaction_state
                .run_redaction_on_complete_output(output);
        }
    }

    fn reset_input_dismiss_timer(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_input_dismissed = false;
        if let Some(handle) = self.input_dismiss_timer_handle.take() {
            handle.abort();
        }

        let has_user_input = self
            .model
            .inputs_to_render(ctx)
            .iter()
            .any(|input| input.is_user_query());
        let should_hide_responses = self
            .terminal_model
            .lock()
            .block_list()
            .active_block()
            .should_hide_responses();

        if has_user_input && should_hide_responses {
            let handle = ctx.spawn_abortable(
                Timer::after(Duration::from_secs(4)),
                |me, _, ctx| {
                    me.is_input_dismissed = true;
                    me.input_dismiss_timer_handle = None;
                    ctx.notify();
                },
                |_, _| {},
            );
            self.input_dismiss_timer_handle = Some(handle);
        }
    }

    fn set_state_from_updated_inputs(&mut self, ctx: &mut ViewContext<Self>) {
        // Clear existing link detection state
        self.link_detection_state.detected_links_by_location.clear();

        self.reset_input_dismiss_timer(ctx);

        // Detect links in all user queries
        for (input_index, input) in self.model.inputs_to_render(ctx).iter().enumerate() {
            if let AIAgentInput::UserQuery { query, .. } = input {
                detect_links(
                    &mut self.link_detection_state,
                    query,
                    TextLocation::Query { input_index },
                    self.current_working_directory.as_ref(),
                    self.shell_launch_data.as_ref(),
                );

                // Run secret redaction on user queries
                let secret_redaction_mode = get_secret_obfuscation_mode(ctx);
                if secret_redaction_mode.should_redact_secret() {
                    let should_obfuscate = secret_redaction_mode.is_visually_obfuscated();
                    self.secret_redaction_state.run_redaction_for_location(
                        query,
                        TextLocation::Query { input_index },
                        should_obfuscate,
                    );
                }
            }
        }
    }

    /// Clears text selections at the `CLISubagentView` level (e.g. user query text).
    /// This does _not_ clear the selection of the child views (code blocks).
    fn clear_view_level_selection(&mut self) {
        self.state_handles.query_selection_handle.clear();
        self.state_handles.output_selection_handle.clear();
        self.state_handles.action_selection_handle.clear();
        *self.selected_text.write() = None;
    }

    /// Clears all text selections in all components within this `CLISubagentView`'s view sub-hierarchy
    /// _other_ than the one that triggered a selection change.
    ///
    /// Call this after text is selected in one part of the view (e.g. a code snippet), to ensure
    /// that there's only one active selection at a time.
    fn clear_other_selections(
        &mut self,
        source_view_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        for editor_view in self.code_editor_views.iter() {
            // Don't clear selections for the view that triggered this change.
            if source_view_id.is_some_and(|entity_id| editor_view.view.id() == entity_id) {
                continue;
            }
            editor_view
                .view
                .update(ctx, |view, ctx| view.clear_selection(ctx));
        }

        // If the event was dispatched by a nested view (i.e. code block),
        // clear the text selection at the `CLISubagentView` level (outside the code block).
        // We want to have only 1 selection active at any one point in time.
        if source_view_id.is_some() {
            self.clear_view_level_selection();
        }
    }

    /// Clears all text selections in all components within this `CLISubagentView`'s view sub-hierarchy.
    /// This includes the `CLISubagentView` level and all child views (code blocks).
    pub fn clear_all_selections(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_other_selections(None, ctx);
        self.clear_view_level_selection();
    }

    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        self.code_editor_views
            .iter()
            .find_map(|editor_view| editor_view.view.as_ref(ctx).selected_text(ctx))
            .or_else(|| self.selected_text.read().clone())
            .filter(|selection| !selection.is_empty())
    }
}

#[derive(Debug, Clone)]
pub enum CLISubagentViewEvent {
    TextSelected,
    CopiedEmptyText,
    #[cfg(windows)]
    WindowsCtrlC,
}

impl Entity for CLISubagentView {
    type Event = CLISubagentViewEvent;
}

impl View for CLISubagentView {
    fn ui_name() -> &'static str {
        "CLISubagentView"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let terminal_model = self.terminal_model.lock();
        let Some(block) = terminal_model.block_list().block_with_id(&self.block_id) else {
            return Empty::new().finish();
        };

        if !block.is_agent_monitoring() || block.is_eligible_for_agent_handoff() {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let semantic_selection = SemanticSelection::handle(app).as_ref(app);

        let mut result = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Render user queries/follow-ups with avatar and interactive text
        let inputs = self.model.inputs_to_render(app);
        for (input_index, input) in inputs.iter().enumerate() {
            if let AIAgentInput::UserQuery { query, .. } = input {
                let text = render_query_text(
                    UserQueryProps {
                        text: query.to_owned(),
                        query_prefix_highlight_len: None,
                        detected_links_state: &self.link_detection_state,
                        secret_redaction_state: &self.secret_redaction_state,
                        input_index,
                        is_selecting: self.state_handles.query_selection_handle.is_selecting(),
                        is_ai_input_enabled: false,
                        find_context: None,
                        font_properties: &Properties {
                            style: Style::Normal,
                            weight: Weight::Normal,
                        },
                    },
                    app,
                );

                let selected_text = self.selected_text.clone();
                let output_selection_handle = self.state_handles.output_selection_handle.clone();
                let action_selection_handle = self.state_handles.action_selection_handle.clone();
                let mut selectable_text = SelectableArea::new(
                    self.state_handles.query_selection_handle.clone(),
                    move |selection_args, ctx, _| {
                        if let Some(selection) = selection_args
                            .selection
                            .filter(|selection| !selection.is_empty())
                        {
                            output_selection_handle.clear();
                            action_selection_handle.clear();
                            *selected_text.write() = Some(selection);
                            ctx.dispatch_typed_action(CLISubagentAction::SelectText);
                        }
                    },
                    text.finish(),
                )
                .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
                .with_smart_select_fn(semantic_selection.smart_select_fn());

                if FeatureFlag::RectSelection.is_enabled() {
                    selectable_text = selectable_text.should_support_rect_select();
                }

                let scrollable_container = render_scrollable_container(
                    ScrollableContainerProps {
                        scroll_state: self.state_handles.query_scroll_state.clone(),
                        child: selectable_text.finish(),
                        background_color: internal_colors::accent_bg(theme).into(),
                        border: Some(Border::all(1.).with_border_fill(theme.accent())),
                    },
                    app,
                )
                .with_margin_bottom(8.)
                .finish();

                let dismissable_stack = render_dismissable_container(
                    DismissableContainerProps {
                        child: scrollable_container,
                        hover_state: self.state_handles.input_hover_state.clone(),
                        dismiss_mouse_state: self.state_handles.dismiss_input_mouse_state.clone(),
                        position_id: USER_QUERY_POSITION_ID.to_string(),
                    },
                    app,
                );

                if !self.is_input_dismissed {
                    result.add_child(dismissable_stack);
                }
            }
        }

        // Render agent outputs and actions
        let mut output_items = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        let status = self.model.status(app);
        let blocked_action = self.model.blocked_action(&self.action_model, app);
        let should_hide_responses = block.should_hide_responses();

        if let Some(output) = status.output_to_render() {
            let output = output.get();

            let mut code_section_index = 0;
            let mut text_section_index = 0;
            let mut table_section_index = 0;
            let mut image_section_index = 0;

            fn copy_code_action(snippet: String) -> CLISubagentAction {
                CLISubagentAction::CopyCode(snippet)
            }

            fn open_code_block_action(source: CodeSource) -> CLISubagentAction {
                CLISubagentAction::OpenCodeBlock(source)
            }

            for output_message in output.messages.iter() {
                match &output_message.message {
                    AIAgentOutputMessageType::Text(AIAgentText { sections })
                        if !are_all_text_sections_empty(sections) =>
                    {
                        let text_color = blended_colors::text_main(theme, theme.surface_1());
                        output_items.add_child(render_text_sections(
                            TextSectionsProps {
                                model: self.model.as_ref(),
                                starting_text_section_index: &mut text_section_index,
                                starting_code_section_index: &mut code_section_index,
                                starting_table_section_index: &mut table_section_index,
                                starting_image_section_index: &mut image_section_index,
                                sections,
                                is_selecting_text: self
                                    .state_handles
                                    .output_selection_handle
                                    .is_selecting(),
                                selectable: true,
                                text_color,
                                is_ai_input_enabled: false,
                                secret_redaction_state: &self.secret_redaction_state,
                                find_context: None,
                                shell_launch_data: None,
                                current_working_directory: None,
                                embedded_code_editor_views: &self.code_editor_views,
                                code_snippet_button_handles: &self.code_editor_buttons,
                                table_section_handles: &self.table_section_handles,
                                // CLI subagent blocks don't render block-list images yet,
                                // so there are no per-image tooltip handles to thread.
                                image_section_tooltip_handles: &[],
                                open_code_block_action_factory: Some(&open_code_block_action),
                                copy_code_action_factory: Some(&copy_code_action),
                                detected_links: Some(&self.link_detection_state),
                                item_spacing: CONTENT_PADDING,
                                #[cfg(feature = "local_fs")]
                                resolved_code_block_paths: None,
                                #[cfg(feature = "local_fs")]
                                resolved_blocklist_image_sources: None,
                            },
                            app,
                        ));
                    }
                    AIAgentOutputMessageType::Action(action) => {
                        let is_cancelled = self
                            .action_model
                            .as_ref(app)
                            .get_action_status(&action.id)
                            .is_some_and(|status| status.is_cancelled());
                        if blocked_action.is_none() && !is_cancelled && !should_hide_responses {
                            if let Some(rendered_action) = render_action(action.action.clone(), app)
                            {
                                result.add_child(
                                    render_scrollable_container(
                                        ScrollableContainerProps {
                                            scroll_state: self
                                                .state_handles
                                                .action_scroll_state
                                                .clone(),
                                            child: rendered_action,
                                            background_color: internal_colors::neutral_2(
                                                appearance.theme(),
                                            ),
                                            border: Some(Border::all(1.).with_border_fill(
                                                internal_colors::neutral_3(theme),
                                            )),
                                        },
                                        app,
                                    )
                                    .with_margin_bottom(8.)
                                    .finish(),
                                );
                            }
                        }
                    }
                    AIAgentOutputMessageType::WebSearch(WebSearchStatus::Searching { query }) => {
                        if !should_hide_responses {
                            result.add_child(
                                render_scrollable_container(
                                    ScrollableContainerProps {
                                        scroll_state: self
                                            .state_handles
                                            .action_scroll_state
                                            .clone(),
                                        child: render_web_search(query.clone(), app),
                                        background_color: internal_colors::neutral_2(
                                            appearance.theme(),
                                        ),
                                        border: Some(
                                            Border::all(1.).with_border_fill(
                                                internal_colors::neutral_3(theme),
                                            ),
                                        ),
                                    },
                                    app,
                                )
                                .with_margin_bottom(8.)
                                .finish(),
                            );
                        }
                    }
                    _ => (),
                }
            }
        }

        let mut output_border = Border::all(1.).with_border_fill(internal_colors::neutral_3(theme));
        if let AIBlockOutputStatus::Failed { error, .. } = &status {
            output_border = Border::all(1.).with_border_color(theme.ui_error_color());
            output_items.add_child(render_failed_output(
                FailedOutputProps {
                    error,
                    is_ai_input_enabled: false,
                    invalid_api_key_button_handle: &self
                        .state_handles
                        .invalid_api_key_button_handle,
                    aws_bedrock_credentials_error_view: None,
                    icon_right_margin: AVATAR_RIGHT_MARGIN,
                },
                app,
            ));

            if !self.model.is_restored() && !error.is_invalid_api_key() {
                output_items.add_child(
                    Container::new(render_informational_footer(
                        app,
                        "This response won't count towards your usage. \"Take over\" to continue."
                            .to_string(),
                    ))
                    .with_margin_top(8.)
                    .with_margin_left(icon_size(app) + AVATAR_RIGHT_MARGIN)
                    .finish(),
                );

                output_items.add_child(
                    Container::new(render_debug_footer(
                        DebugFooterProps {
                            conversation: self.model.conversation(app),
                            model: self.model.as_ref(),
                            debug_copy_button_handle: self
                                .state_handles
                                .debug_copy_button_handle
                                .clone(),
                            submit_issue_button_handle: self
                                .state_handles
                                .submit_issue_button_handle
                                .clone(),
                            should_render_feedback_below: true,
                        },
                        |debug_id, ctx| {
                            ctx.dispatch_typed_action(CLISubagentAction::CopyDebugId(debug_id))
                        },
                        |ctx| ctx.dispatch_typed_action(CLISubagentAction::OpenFeedbackDocs),
                        app,
                    ))
                    .with_margin_top(8.)
                    .with_margin_left(icon_size(app) + AVATAR_RIGHT_MARGIN)
                    .finish(),
                );
            }
        }

        if !output_items.is_empty() && !should_hide_responses {
            let selected_text = self.selected_text.clone();
            let query_selection_handle = self.state_handles.query_selection_handle.clone();
            let action_selection_handle = self.state_handles.action_selection_handle.clone();
            let mut output = SelectableArea::new(
                self.state_handles.output_selection_handle.clone(),
                move |selection_args, ctx, _| {
                    if let Some(selection) = selection_args
                        .selection
                        .filter(|selection| !selection.is_empty())
                    {
                        query_selection_handle.clear();
                        action_selection_handle.clear();
                        *selected_text.write() = Some(selection);
                        ctx.dispatch_typed_action(CLISubagentAction::SelectText);
                    }
                },
                output_items.finish(),
            )
            .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
            .with_smart_select_fn(semantic_selection.smart_select_fn());

            if FeatureFlag::RectSelection.is_enabled() {
                output = output.should_support_rect_select();
            }

            result.add_child(
                render_scrollable_container(
                    ScrollableContainerProps {
                        scroll_state: self.state_handles.output_scroll_state.clone(),
                        child: output.finish(),
                        background_color: internal_colors::neutral_2(appearance.theme()),
                        border: Some(output_border),
                    },
                    app,
                )
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        if let Some(rendered_action) = blocked_action.and_then(|action| match action.action {
            AIAgentActionType::WriteToLongRunningShellCommand { input, mode, .. } => {
                Some(render_blocked_action(
                    BlockedActionProps {
                        header: BLOCKED_ACTION_MESSAGE_FOR_WRITE_TO_LONG_RUNNING_SHELL_COMMAND
                            .to_string(),
                        description: Some(render_write_to_pty_input(
                            WriteToPtyInputProps {
                                input: input.clone(),
                                mode,
                                scroll_state: self.state_handles.input_scroll_state.clone(),
                            },
                            app,
                        )),
                        is_allow_menu_open: self.is_allow_menu_open,
                        allow_menu: Some(&self.allow_menu),
                        buttons: vec![
                            &self.allow_button,
                            &self.reject_button,
                            &self.take_over_button,
                        ],
                        speedbump: should_show_write_to_pty_speedbump(app).then_some(
                            PermissionsSpeedbumpProps {
                                always_allow_checked: self.always_allow_write_to_pty_checked,
                                speedbump_checkbox_handle: &self
                                    .state_handles
                                    .speedbump_checkbox_handle,
                                speedbump_checkbox_action:
                                    CLISubagentAction::ToggleAlwaysAllowWriteToPty,
                                ai_settings_link: &self.state_handles.ai_settings_link,
                            },
                        ),
                    },
                    app,
                ))
            }
            AIAgentActionType::TransferShellCommandControlToUser { ref reason } => {
                Some(render_blocked_action(
                    BlockedActionProps {
                        header: BLOCKED_ACTION_MESSAGE_FOR_TRANSFER_CONTROL.to_string(),
                        description: Some(render_transfer_control_reason(reason, app)),
                        is_allow_menu_open: false,
                        allow_menu: None,
                        buttons: vec![&self.reject_button, &self.transfer_control_button],
                        speedbump: None,
                    },
                    app,
                ))
            }
            AIAgentActionType::ReadFiles(..)
            | AIAgentActionType::SearchCodebase(..)
            | AIAgentActionType::Grep { .. }
            | AIAgentActionType::FileGlobV2 { .. } => Some(render_blocked_action(
                BlockedActionProps {
                    header: get_blocked_action_header(action.action.clone()).unwrap_or_default(),
                    description: render_search_action_input(action.action.clone(), app),
                    is_allow_menu_open: self.is_allow_menu_open,
                    allow_menu: Some(&self.allow_menu),
                    buttons: vec![
                        &self.allow_button,
                        &self.reject_button,
                        &self.take_over_button,
                    ],
                    speedbump: should_show_read_files_speedbump(app).then_some(
                        PermissionsSpeedbumpProps {
                            always_allow_checked: self.always_allow_read_files_checked,
                            speedbump_checkbox_handle: &self
                                .state_handles
                                .speedbump_checkbox_handle,
                            speedbump_checkbox_action:
                                CLISubagentAction::ToggleAlwaysAllowReadFiles,
                            ai_settings_link: &self.state_handles.ai_settings_link,
                        },
                    ),
                },
                app,
            )),
            _ => None,
        }) {
            let selected_text = self.selected_text.clone();
            let query_selection_handle = self.state_handles.query_selection_handle.clone();
            let output_selection_handle = self.state_handles.output_selection_handle.clone();
            let mut selectable_action = SelectableArea::new(
                self.state_handles.action_selection_handle.clone(),
                move |selection_args, ctx, _| {
                    if let Some(selection) = selection_args
                        .selection
                        .filter(|selection| !selection.is_empty())
                    {
                        query_selection_handle.clear();
                        output_selection_handle.clear();
                        *selected_text.write() = Some(selection);
                        ctx.dispatch_typed_action(CLISubagentAction::SelectText);
                    }
                },
                rendered_action,
            )
            .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
            .with_smart_select_fn(semantic_selection.smart_select_fn());

            if FeatureFlag::RectSelection.is_enabled() {
                selectable_action = selectable_action.should_support_rect_select();
            }

            result.add_child(
                Container::new(selectable_action.finish())
                    .with_margin_bottom(8.)
                    .finish(),
            );
        }

        result.finish()
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        let terminal_model = self.terminal_model.lock();
        let active_block = terminal_model.block_list().active_block();
        if active_block.is_agent_blocked() {
            context.set.insert(HAS_PENDING_CLI_ACTION_CONTEXT_KEY);
            if !self.has_pending_transfer_control_action(app) {
                context
                    .set
                    .insert(HAS_PENDING_NON_TRANSFER_CONTROL_ACTION_CONTEXT_KEY);
            }
        }
        context
    }
}

#[derive(Debug, Clone)]
pub enum CLISubagentAction {
    CopyCode(String),
    OpenCodeBlock(CodeSource),
    ExecuteBlockedAction,
    ExecuteAndAutoApprove,
    RejectBlockedAction { should_user_take_over: bool },
    TakeControlOfRunningCommand,
    ToggleAllowMenu,
    ToggleAlwaysAllowWriteToPty,
    ToggleAlwaysAllowReadFiles,
    DismissInput,
    SelectText,
    CopyDebugId(String),
    OpenFeedbackDocs,
}

impl TypedActionView for CLISubagentView {
    type Action = CLISubagentAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CLISubagentAction::CopyCode(code) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(code.clone()));
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::success(String::from("Copied to clipboard")),
                        window_id,
                        ctx,
                    );
                });
            }
            CLISubagentAction::OpenCodeBlock(source) => {
                // TODO(zachbai): Implement this.
                log::info!("Received open code block action: {source:?}");
            }
            CLISubagentAction::ExecuteBlockedAction => {
                self.handle_execute_blocked_action(false, ctx);
            }
            CLISubagentAction::ExecuteAndAutoApprove => {
                self.handle_execute_blocked_action(true, ctx);
            }
            CLISubagentAction::RejectBlockedAction {
                should_user_take_over,
            } => {
                self.handle_reject_blocked_action(*should_user_take_over, ctx);
            }
            CLISubagentAction::TakeControlOfRunningCommand => {
                self.take_control_of_running_command(ctx);
            }
            CLISubagentAction::ToggleAllowMenu => {
                self.toggle_allow_menu(ctx);
            }
            CLISubagentAction::ToggleAlwaysAllowWriteToPty => {
                self.always_allow_write_to_pty_checked = !self.always_allow_write_to_pty_checked;
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    if let Err(e) = model.set_always_allow_write_to_pty(
                        self.always_allow_write_to_pty_checked,
                        self.terminal_view_id,
                        ctx,
                    ) {
                        report_error!(e);
                    }
                });
                ctx.notify();
            }
            CLISubagentAction::ToggleAlwaysAllowReadFiles => {
                self.always_allow_read_files_checked = !self.always_allow_read_files_checked;
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    if let Err(e) = model.set_always_allow_read_files(
                        self.always_allow_read_files_checked,
                        self.terminal_view_id,
                        ctx,
                    ) {
                        report_error!(e);
                    }
                });
                ctx.notify();
            }
            CLISubagentAction::DismissInput => {
                self.is_input_dismissed = true;
                if let Some(handle) = self.input_dismiss_timer_handle.take() {
                    handle.abort();
                }
                ctx.notify();
                send_telemetry_from_ctx!(
                    TelemetryEvent::CLISubagentInputDismissed {
                        conversation_id: self.conversation_id,
                        block_id: self.block_id.clone(),
                    },
                    ctx
                );
            }
            CLISubagentAction::SelectText => {
                self.clear_other_selections(None, ctx);
                ctx.reset_cursor();
                ctx.focus_self();
                ctx.emit(CLISubagentViewEvent::TextSelected);
            }
            CLISubagentAction::CopyDebugId(debug_id) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(debug_id.clone()));
            }
            CLISubagentAction::OpenFeedbackDocs => {
                ctx.open_url("https://docs.warp.dev/support-and-billing/sending-us-feedback");
            }
        }
    }
}

fn should_show_write_to_pty_speedbump(app: &AppContext) -> bool {
    is_agent_mode_autonomy_allowed(app)
        && *AISettings::as_ref(app).should_show_agent_mode_write_to_pty_speedbump
}

fn should_show_read_files_speedbump(app: &AppContext) -> bool {
    is_agent_mode_autonomy_allowed(app)
        && *AISettings::as_ref(app).should_show_agent_mode_autoread_files_speedbump
}

fn get_action_loading_text(action: AIAgentActionType) -> Option<String> {
    match action {
        AIAgentActionType::SearchCodebase(_) => {
            Some(LOAD_OUTPUT_MESSAGE_FOR_SEARCH_CODEBASE.to_string())
        }
        AIAgentActionType::ReadFiles(_) => Some(LOAD_OUTPUT_MESSAGE_FOR_READING_FILES.to_string()),
        AIAgentActionType::Grep { .. } => Some(LOAD_OUTPUT_MESSAGE_FOR_GREP.to_string()),
        AIAgentActionType::FileGlobV2 { .. } => Some(LOAD_OUTPUT_MESSAGE_FOR_FILE_GLOB.to_string()),
        _ => None,
    }
}

fn get_action_icon(action: AIAgentActionType) -> Option<Icon> {
    match action {
        AIAgentActionType::SearchCodebase(_)
        | AIAgentActionType::ReadFiles(_)
        | AIAgentActionType::Grep { .. }
        | AIAgentActionType::FileGlobV2 { .. } => Some(Icon::Search),
        _ => None,
    }
}

fn render_action(action: AIAgentActionType, app: &AppContext) -> Option<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let text = get_action_loading_text(action.clone())?;
    let icon = get_action_icon(action)?;

    let icon = Container::new(
        ConstrainedBox::new(
            warpui::elements::Icon::new(icon.into(), internal_colors::neutral_5(theme)).finish(),
        )
        .with_width(icon_size(app))
        .with_height(icon_size(app))
        .finish(),
    )
    .with_margin_right(AVATAR_RIGHT_MARGIN)
    .finish();

    let text = Expanded::new(
        1.,
        Text::new(
            text,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_main(theme, theme.surface_1()))
        .finish(),
    )
    .finish();

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_children([icon, text])
        .finish();

    Some(row)
}

fn render_web_search(query: Option<String>, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let text = if let Some(q) = query {
        format!("Searching the web for \"{q}\"")
    } else {
        LOAD_OUTPUT_MESSAGE_FOR_WEB_SEARCH.to_string()
    };

    let icon = Container::new(
        ConstrainedBox::new(
            warpui::elements::Icon::new(Icon::Search.into(), internal_colors::neutral_5(theme))
                .finish(),
        )
        .with_width(icon_size(app))
        .with_height(icon_size(app))
        .finish(),
    )
    .with_margin_right(AVATAR_RIGHT_MARGIN)
    .finish();

    let text = Expanded::new(
        1.,
        Text::new(
            text,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_main(theme, theme.surface_1()))
        .finish(),
    )
    .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_children([icon, text])
        .finish()
}

struct DismissableContainerProps {
    child: Box<dyn Element>,
    hover_state: MouseStateHandle,
    dismiss_mouse_state: MouseStateHandle,
    position_id: String,
}

fn render_dismissable_container(
    props: DismissableContainerProps,
    app: &AppContext,
) -> Box<dyn Element> {
    let DismissableContainerProps {
        child,
        hover_state,
        dismiss_mouse_state,
        position_id,
    } = props;

    let hoverable = Hoverable::new(hover_state, |mouse_state| {
        let mut stack = Stack::new().with_child(SavePosition::new(child, &position_id).finish());
        if mouse_state.is_hovered() {
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            let ui_builder = appearance.ui_builder();

            let dismiss_button = Container::new(
                ui_builder
                    .close_button(16., dismiss_mouse_state.clone())
                    .with_style(UiComponentStyles {
                        font_color: Some(blended_colors::text_main(theme, theme.surface_1())),
                        background: Some(internal_colors::accent_bg(theme).into()),
                        border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                        border_width: Some(1.),
                        border_color: Some(theme.accent().into()),
                        padding: Some(Coords::uniform(2.)),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CLISubagentAction::DismissInput);
                    })
                    .finish(),
            )
            .finish();

            stack.add_positioned_child(
                dismiss_button,
                OffsetPositioning::offset_from_save_position_element(
                    position_id,
                    vec2f(4., -4.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
        }
        stack.finish()
    });

    hoverable
        .with_hover_out_delay(Duration::from_millis(500))
        .finish()
}
struct ScrollableContainerProps {
    scroll_state: ClippedScrollStateHandle,
    child: Box<dyn Element>,
    background_color: ColorU,
    border: Option<Border>,
}

fn render_scrollable_container(props: ScrollableContainerProps, _app: &AppContext) -> Container {
    let ScrollableContainerProps {
        scroll_state,
        child,
        background_color,
        border,
    } = props;

    let scrollable = NewScrollable::vertical(
        SingleAxisConfig::Clipped {
            handle: scroll_state,
            child,
        },
        Fill::None,
        Fill::None,
        Fill::None,
    )
    .with_propagate_mousewheel_if_not_handled(true)
    .finish();

    let clipped = ConstrainedBox::new(scrollable)
        .with_max_height(MAX_HEIGHT)
        .finish();

    let mut container = Container::new(clipped)
        .with_background_color(background_color)
        .with_horizontal_padding(CONTENT_PADDING)
        .with_vertical_padding(CONTENT_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_drop_shadow(DropShadow::default());

    if let Some(border) = border {
        container = container.with_border(border);
    }

    container
}

fn render_action_buttons(
    buttons: Vec<&dyn RenderCompactibleActionButton>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let (regular_row, compact_row) =
        render_compact_and_regular_button_rows(buttons, None, appearance, app);

    let regular_wrapped = Container::new(regular_row)
        .with_vertical_padding(8.)
        .with_horizontal_padding(CONTENT_PADDING)
        .with_background_color(internal_colors::neutral_2(theme))
        .with_border(Border::top(1.).with_border_color(internal_colors::neutral_3(theme)))
        .finish();

    let compact_wrapped = Container::new(compact_row)
        .with_vertical_padding(8.)
        .with_horizontal_padding(CONTENT_PADDING)
        .with_background_color(internal_colors::neutral_2(theme))
        .with_border(Border::top(1.).with_border_color(internal_colors::neutral_3(theme)))
        .finish();

    let size_switch_threshold = 250. * appearance.monospace_ui_scalar();
    SizeConstraintSwitch::new(
        regular_wrapped,
        vec![(
            SizeConstraintCondition::WidthLessThan(size_switch_threshold),
            compact_wrapped,
        )],
    )
    .finish()
}

struct PermissionsSpeedbumpProps<'a> {
    always_allow_checked: bool,
    speedbump_checkbox_handle: &'a MouseStateHandle,
    speedbump_checkbox_action: CLISubagentAction,
    ai_settings_link: &'a HighlightedHyperlink,
}

fn render_permissions_speedbump(
    props: PermissionsSpeedbumpProps<'_>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_size = appearance.monospace_font_size() - 2.;
    let font_family = appearance.ui_font_family();
    let font_color = internal_colors::neutral_6(theme);

    let checkbox = appearance
        .ui_builder()
        .checkbox(props.speedbump_checkbox_handle.clone(), Some(font_size))
        .check(props.always_allow_checked)
        .with_style(UiComponentStyles {
            font_color: Some(font_color),
            font_size: Some(font_size),
            ..Default::default()
        })
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(props.speedbump_checkbox_action.clone());
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

    let checkbox_text = appearance
        .ui_builder()
        .span("Always allow")
        .with_style(UiComponentStyles {
            font_color: Some(font_color),
            font_size: Some(font_size),
            padding: Some(Coords::default().left(4.)),
            ..Default::default()
        })
        .with_soft_wrap()
        .build()
        .finish();

    let formatted_text = FormattedTextElement::new(
        FormattedText::new([FormattedTextLine::Line(vec![
            FormattedTextFragment::hyperlink("Manage Agent permissions", "Settings > AI"),
        ])]),
        font_size,
        font_family,
        font_family,
        font_color,
        props.ai_settings_link.clone(),
    )
    .with_hyperlink_font_color(blended_colors::accent_fg_strong(theme).into())
    .register_default_click_handlers(|_, ctx, _| {
        ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPage(
            SettingsSection::WarpAgent,
        ));
    })
    .finish();

    Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(checkbox)
                        .with_child(Shrinkable::new(1.0, checkbox_text).finish())
                        .finish(),
                )
                .finish(),
            )
            .with_child(Shrinkable::new(1.0, formatted_text).finish())
            .finish(),
    )
    .with_vertical_padding(8.)
    .with_horizontal_padding(CONTENT_PADDING)
    .with_background_color(internal_colors::neutral_2(theme))
    .with_border(Border::top(1.).with_border_color(internal_colors::neutral_3(theme)))
    .finish()
}

fn render_transfer_control_reason(reason: &str, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let text = Text::new(
        reason.to_string(),
        appearance.ai_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(
        appearance.theme(),
        appearance.theme().surface_1(),
    ))
    .finish();

    Container::new(text)
        .with_background_color(internal_colors::neutral_2(appearance.theme()))
        .with_horizontal_padding(CONTENT_PADDING)
        .with_vertical_padding(8.)
        .finish()
}

fn get_blocked_action_header(action: AIAgentActionType) -> Option<String> {
    match action {
        AIAgentActionType::WriteToLongRunningShellCommand { .. } => {
            Some(BLOCKED_ACTION_MESSAGE_FOR_WRITE_TO_LONG_RUNNING_SHELL_COMMAND.to_string())
        }
        AIAgentActionType::ReadFiles(..) => {
            Some(BLOCKED_ACTION_MESSAGE_FOR_READING_FILES.to_string())
        }
        AIAgentActionType::SearchCodebase(..) => {
            Some(BLOCKED_ACTION_MESSAGE_FOR_SEARCHING_CODEBASE.to_string())
        }
        AIAgentActionType::Grep { .. } | AIAgentActionType::FileGlobV2 { .. } => {
            Some(BLOCKED_ACTION_MESSAGE_FOR_GREP_OR_FILE_GLOB.to_string())
        }
        _ => None,
    }
}

struct WriteToPtyInputProps {
    input: bytes::Bytes,
    mode: AIAgentPtyWriteMode,
    scroll_state: ClippedScrollStateHandle,
}

fn render_write_to_pty_input(props: WriteToPtyInputProps, app: &AppContext) -> Box<dyn Element> {
    let WriteToPtyInputProps {
        input,
        mode,
        scroll_state,
    } = props;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let decorated_bytes = mode.decorate_bytes(input.to_vec(), false);
    let parsed = if let AIAgentPtyWriteMode::Block = mode {
        ParsedControlCodeOutput {
            display: String::from_utf8_lossy(&input).to_string(),
            control_code_ranges: vec![],
        }
    } else {
        parse_control_codes_from_bytes(&decorated_bytes)
    };

    let text = Text::new(
        parsed.display,
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(theme.sub_text_color(theme.background()).into())
    .with_single_highlight(
        Highlight::new()
            .with_foreground_color(theme.hint_text_color(theme.surface_2()).into())
            .with_properties(Properties {
                style: Style::Italic,
                ..Default::default()
            }),
        parsed.control_code_ranges.into_iter().flatten().collect(),
    )
    .finish();

    let scrollable = NewScrollable::vertical(
        SingleAxisConfig::Clipped {
            handle: scroll_state,
            child: text,
        },
        Fill::None,
        Fill::None,
        Fill::None,
    )
    .with_propagate_mousewheel_if_not_handled(true)
    .finish();

    let clipped = ConstrainedBox::new(scrollable)
        .with_max_height(MAX_HEIGHT)
        .finish();

    Container::new(clipped)
        .with_background_color(internal_colors::neutral_2(theme))
        .with_horizontal_padding(CONTENT_PADDING)
        .with_vertical_padding(8.)
        .finish()
}

fn render_search_action_input(
    action: AIAgentActionType,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let description_text = match action {
        AIAgentActionType::ReadFiles(ref request) => request
            .locations
            .iter()
            .map(|loc| loc.name.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        AIAgentActionType::SearchCodebase(ref request) => {
            let repo = request.codebase_path.as_deref()?;
            repo.to_string()
        }
        AIAgentActionType::Grep {
            ref queries,
            ref path,
        } => {
            let display_path = if path == "." {
                "the current directory"
            } else {
                path.as_str()
            };

            if queries.len() == 1 {
                format!("Grep for `{}` in {}", queries[0], display_path)
            } else {
                let patterns_list = queries
                    .iter()
                    .map(|q| format!(" - `{q}`"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Grep for the following patterns in {display_path}:\n{patterns_list}")
            }
        }
        AIAgentActionType::FileGlobV2 {
            ref patterns,
            ref search_dir,
        } => {
            let display_path = search_dir.as_deref().unwrap_or("the current directory");

            if patterns.len() == 1 {
                format!(
                    "Search for files that match `{}` in {}",
                    patterns[0], display_path
                )
            } else {
                let patterns_list = patterns
                    .iter()
                    .map(|p| format!(" - `{p}`"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "Find files that match the following patterns in {display_path}:\n{patterns_list}"
                )
            }
        }
        _ => return None,
    };

    let text = Text::new(
        description_text,
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(theme, theme.surface_1()))
    .finish();

    Some(
        Container::new(text)
            .with_background_color(internal_colors::neutral_2(theme))
            .with_uniform_padding(CONTENT_PADDING)
            .finish(),
    )
}

struct BlockedActionProps<'a> {
    header: String,
    description: Option<Box<dyn Element>>,

    is_allow_menu_open: bool,
    allow_menu: Option<&'a ViewHandle<Menu<CLISubagentAction>>>,
    buttons: Vec<&'a dyn RenderCompactibleActionButton>,
    speedbump: Option<PermissionsSpeedbumpProps<'a>>,
}

fn render_blocked_action(props: BlockedActionProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let header_text = props.header.clone();
    let text = Text::new(
        header_text,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(theme.active_ui_text_color().into())
    .finish();

    let icon = Container::new(
        ConstrainedBox::new(yellow_stop_icon(appearance).finish())
            .with_width(icon_size(app))
            .with_height(icon_size(app))
            .finish(),
    )
    .with_margin_right(AVATAR_RIGHT_MARGIN)
    .finish();

    let header = Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children(vec![icon, Shrinkable::new(1.0, text).finish()])
            .finish(),
    )
    .with_background_color(internal_colors::neutral_3(theme))
    .with_uniform_padding(CONTENT_PADDING)
    .finish();

    let mut body_children = vec![header];

    if let Some(description) = props.description {
        body_children.push(description);
    }

    let buttons = render_action_buttons(props.buttons, app);
    body_children.push(buttons);
    if let Some(speedbump) = props.speedbump {
        body_children.push(render_permissions_speedbump(speedbump, app));
    }

    let body = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_children(body_children)
        .finish();

    let mut stack = Stack::new();
    stack.add_child(
        Container::new(body)
            .with_drop_shadow(DropShadow::default())
            .finish(),
    );

    if props.is_allow_menu_open {
        if let Some(allow_menu) = props.allow_menu {
            stack.add_positioned_child(
                ChildView::new(allow_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    ALLOW_ACTION_POSITION_ID.to_string(),
                    vec2f(0., 8.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }
    }

    Expanded::new(
        1.0,
        Container::new(stack.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_color(internal_colors::neutral_3(theme)))
            .finish(),
    )
    .finish()
}
