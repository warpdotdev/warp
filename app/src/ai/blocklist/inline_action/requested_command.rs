use std::borrow::Cow;
use std::cmp::{Ordering, PartialEq};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use lazy_static::lazy_static;
use parking_lot::FairMutex;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::clipboard::ClipboardContent;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Dismiss, Empty, Expanded, Flex, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, ScrollbarWidth, SelectableArea, SelectionHandle, Stack,
    Text,
};
use warpui::keymap::{Context, EditableBinding, FixedBinding, Keystroke};
use warpui::ui_components::components::UiComponent as _;
use warpui::{
    AppContext, Element, Entity, EntityId, EventContext, ModelHandle, SingletonEntity,
    TypedActionView, UpdateView, View, ViewContext, ViewHandle,
};

use super::inline_action_icons::{self, icon_size};
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResult, AIAgentActionResultType, AIAgentActionType,
    AIAgentCitation, AIAgentOutputMessageType, CallMCPToolResult, RequestCommandOutputResult,
    icons,
};
use crate::ai::blocklist::action_model::AIActionStatus;
use crate::ai::blocklist::block::cli_controller::{
    LongRunningCommandControlState, UserTakeOverReason,
};
use crate::ai::blocklist::block::view_impl::output::action_icon;
use crate::ai::blocklist::block::view_impl::{
    CONTENT_HORIZONTAL_PADDING, CONTENT_ITEM_VERTICAL_MARGIN,
    render_autonomy_checkbox_setting_speedbump_footer, render_citation, render_citation_chips,
};
use crate::ai::blocklist::block::{AIBlockAction, AutonomySettingSpeedbump};
use crate::ai::blocklist::inline_action::inline_action_header::{
    ExpandedConfig, HeaderConfig, INLINE_ACTION_HORIZONTAL_PADDING, InteractionMode,
    RightClickConfig,
};
use crate::ai::blocklist::model::{AIBlockModel, AIBlockModelHelper};
use crate::ai::blocklist::{
    AIBlock, BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIHistoryModel,
    ClientIdentifiers,
};
use crate::cmd_or_ctrl_shift;
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::editor::InteractionState;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::settings::InputModeSettings;
use crate::terminal::TerminalModel;
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::model::block::Block;
use crate::ui_components::blended_colors;
use crate::ui_components::json_tree::{
    CopyJsonFn, JsonTreeColors, JsonTreeState, PathSegment, TREE_FONT_SIZE, ToggleFn,
    ToggleStringFn, render_json_tree,
};
use crate::util::bindings::keybinding_name_to_keystroke;
use crate::view_components::action_button::{ButtonSize, KeystrokeSource, NakedTheme};
use crate::view_components::compactible_action_button::{
    CompactibleActionButton, LARGE_SIZE_SWITCH_THRESHOLD, MEDIUM_SIZE_SWITCH_THRESHOLD,
    RenderCompactibleActionButton, SMALL_SIZE_SWITCH_THRESHOLD,
};
use crate::view_components::compactible_split_action_button::CompactibleSplitActionButton;

/// The vertical padding applied to the requested command row's content body.
/// For horizontal padding, use [`INLINE_ACTION_HORIZONTAL_PADDING`] for consistency.
pub const REQUESTED_COMMAND_BODY_VERTICAL_PADDING: f32 = 16.;

const REQUESTED_COMMAND_REJECT_LABEL: &str = "Reject";
const REQUESTED_COMMAND_ACCEPT_LABEL: &str = "Run";
const REQUESTED_COMMAND_EDIT_LABEL: &str = "Edit";
const REQUESTED_COMMAND_MINIMIZE_LABEL: &str = "Done";

const LOADING_MESSAGE: &str = "Generating command...";
const COMMAND_WAITING_FOR_USER_MESSAGE: &str = "OK if I run this command and read the output?";
const MCP_TOOL_WAITING_FOR_USER_MESSAGE: &str = "OK if I call this MCP tool?";
const MONITORING_COMMAND_MESSAGE: &str = "Agent is monitoring command...";
const AGENT_NEEDS_INPUT_MESSAGE: &str = "Agent needs your input to continue";
const USER_TOOK_CONTROL_COMMAND_MESSAGE: &str = "User is in control.";
const USER_STOPPED_CLI_SUBAGENT_COMMAND_MESSAGE: &str = "Paused agent. User is in control.";
const AGENT_REQUESTED_USER_TAKE_CONTROL_COMMAND_MESSAGE: &str = "User in control";
const AGENT_ERRORED_COMMAND_MESSAGE: &str = "Agent ran into an issue. Take over control.";
pub const VIEWING_COMMAND_DETAIL_MESSAGE: &str = "Viewing command detail";
const VIEWING_MCP_TOOL_DETAIL_MESSAGE: &str = "Viewing MCP tool call detail";

const EDIT_COMMAND_ACTION_NAME: &str = "requested_command:edit";

const EDIT_MODE_OPEN_KEYMAP_CONTEXT: &str = "RequestedCommandViewEditModeOpen";
const REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT: &str = "RequestedActionBlocked";

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const MAX_EDITOR_HEIGHT: f32 = 500.0;

lazy_static! {
    pub static ref CANCEL_REQUESTED_COMMAND_KEYSTROKE: Keystroke = Keystroke {
        ctrl: true,
        key: "c".to_owned(),
        ..Default::default()
    };
    pub static ref ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE: Keystroke = Keystroke {
        key: "enter".to_owned(),
        ..Default::default()
    };
    static ref CMD_ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE: Keystroke =
        Keystroke::parse("cmdorctrl-enter")
            .expect("CMD_ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE is invalid");
    static ref MINIMIZE_REQUESTED_COMMAND_KEYSTROKE: Keystroke = Keystroke {
        key: "escape".to_owned(),
        ..Default::default()
    };
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "ctrl-c",
            RequestedCommandViewAction::Reject,
            id!(RequestedCommandView::ui_name()) & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT),
        ),
        FixedBinding::new(
            "enter",
            RequestedCommandViewAction::Accept,
            id!(RequestedCommandView::ui_name())
                & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT)
                & !id!(EDIT_MODE_OPEN_KEYMAP_CONTEXT),
        ),
        FixedBinding::new(
            "numpadenter",
            RequestedCommandViewAction::Accept,
            id!(RequestedCommandView::ui_name())
                & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT)
                & !id!(EDIT_MODE_OPEN_KEYMAP_CONTEXT),
        ),
        FixedBinding::new(
            "cmdorctrl-enter",
            RequestedCommandViewAction::Accept,
            id!(RequestedCommandView::ui_name())
                & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT)
                & id!(EDIT_MODE_OPEN_KEYMAP_CONTEXT),
        ),
        FixedBinding::new(
            "escape",
            RequestedCommandViewAction::CloseEditMode,
            id!(RequestedCommandView::ui_name())
                & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT)
                & id!(EDIT_MODE_OPEN_KEYMAP_CONTEXT),
        ),
        FixedBinding::new(
            "tab",
            RequestedCommandViewAction::FocusEditor,
            id!(RequestedCommandView::ui_name())
                & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT)
                & id!(EDIT_MODE_OPEN_KEYMAP_CONTEXT),
        ),
    ]);

    app.register_editable_bindings([EditableBinding::new(
        EDIT_COMMAND_ACTION_NAME,
        "Edit requested command",
        RequestedCommandViewAction::OpenEditMode,
    )
    .with_key_binding(cmd_or_ctrl_shift("e"))
    .with_context_predicate(
        id!(RequestedCommandView::ui_name())
            & id!(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT)
            & !id!(EDIT_MODE_OPEN_KEYMAP_CONTEXT),
    )]);
}

/// Structured representation of an MCP tool call request for JSON tree rendering.
///
/// The tool name is derivable from `command_text` and is not duplicated here.
pub struct McpRequest {
    pub args: serde_json::Value,
}

/// The normalized, renderable form of a `CallMCPToolResult`.
pub(crate) enum McpRenderable {
    Tree(serde_json::Value),
    Error(String),
    Cancelled,
}

/// Normalizes a `CallMCPToolResult` into a `McpRenderable` for display.
///
/// Prefers `structured_content` when present; otherwise tries to parse joined
/// text content as JSON; falls back to wrapping the raw text as a JSON string value.
pub(crate) fn mcp_result_to_renderable(result: &CallMCPToolResult) -> McpRenderable {
    match result {
        CallMCPToolResult::Success { result } => {
            if let Some(v) = &result.structured_content {
                return McpRenderable::Tree(v.clone());
            }
            let text = result
                .content
                .iter()
                .filter_map(|c| {
                    if let rmcp::model::RawContent::Text(t) = &c.raw {
                        Some(t.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                McpRenderable::Tree(v)
            } else {
                McpRenderable::Tree(serde_json::Value::String(text))
            }
        }
        CallMCPToolResult::Error(e) => McpRenderable::Error(e.clone()),
        CallMCPToolResult::Cancelled => McpRenderable::Cancelled,
    }
}

/// Identifies which of the two JSON trees (request or response) an action targets.
#[derive(Debug, Clone)]
pub enum McpTree {
    Request,
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestedActionViewType {
    Command,
    McpTool,
}

impl RequestedActionViewType {
    fn is_requested_command(&self) -> bool {
        matches!(self, RequestedActionViewType::Command)
    }

    fn is_mcp_tool(&self) -> bool {
        matches!(self, RequestedActionViewType::McpTool)
    }
}

#[derive(Debug, Clone)]
pub enum RequestedCommandViewEvent {
    Accepted,
    EnableAutoexecuteMode,
    Rejected,
    UpdatedExpansionState { is_expanded: bool },
    TextSelected,
    CopiedEmptyText,
    EditorFocused,
    OpenActiveAgentProfileEditor,
}

#[derive(Debug, Clone)]
pub enum RequestedCommandViewAction {
    Accept,
    AcceptAndAutoExecute,
    ToggleAcceptMenu,
    Reject,
    OpenEditMode,
    CloseEditMode,
    FocusEditor,
    ToggleExpanded,
    OpenActiveAgentProfileEditor,
    SelectText,
    /// Toggle the expanded/collapsed state of an object or array node in the
    /// MCP request or response JSON tree.
    ToggleJsonNode {
        path: Vec<PathSegment>,
        tree: McpTree,
    },
    /// Toggle the expanded/collapsed state of a long string value in the MCP
    /// request or response JSON tree.
    ToggleJsonString {
        path: Vec<PathSegment>,
        tree: McpTree,
    },
    /// Write the given JSON text to the system clipboard.
    CopyJsonToClipboard {
        text: String,
    },
    /// Opens the right-click context menu for an MCP JSON tree row, carrying
    /// the serialized subtree JSON and the position anchor ID of the clicked
    /// row so the menu can be positioned below it.
    ShowMcpContextMenu {
        json_text: String,
        anchor_id: String,
    },
    /// Copy the currently selected MCP tree text to the clipboard.
    CopyMcpSelection,
    /// Dismiss the MCP JSON tree right-click context menu.
    CloseMcpContextMenu,
}

pub struct RequestedCommandView {
    action_model: ModelHandle<BlocklistAIActionModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
    ai_block_view_id: EntityId,
    command_text: String,
    editor: Option<ViewHandle<CodeEditorView>>,

    cancel_button: CompactibleActionButton,
    edit_button: CompactibleActionButton,
    minimize_button: CompactibleActionButton,

    // Split accept button and menu state
    accept_and_autoexecute_split_button: CompactibleSplitActionButton,
    is_accept_split_button_menu_open: bool,
    accept_split_button_menu: ViewHandle<Menu<RequestedCommandViewAction>>,

    // For anchoring overlays to unique positions
    position_id_prefix: String,

    action_id: AIAgentActionId,
    client_ids: ClientIdentifiers,
    action_type: RequestedActionViewType,
    autonomy_setting_speedbump: AutonomySettingSpeedbump,

    // Header expansion state components
    is_header_expanded: bool,
    header_mouse_state: MouseStateHandle,
    is_editing: bool,

    // A requested command can either be copied directly off of one citation (such as a Warp Drive
    // object), derived from one or more citations, or be unrelated to any citations.
    // A given citation should only appear once per block.
    copied_from_citation: Option<AIAgentCitation>,
    derived_from_citations: Vec<AIAgentCitation>,
    citation_state_handles: HashMap<AIAgentCitation, MouseStateHandle>,

    autoexecute_readonly_commands_speedbump_checkbox_handle: MouseStateHandle,
    manage_autonomy_settings_link_handle: MouseStateHandle,

    // Selection support for MCP tool call detail text
    mcp_content_selection_handle: SelectionHandle,
    mcp_content_selected_text: Arc<std::sync::RwLock<Option<String>>>,

    // Structured request data and per-tree expansion state for JSON tree rendering.
    // `mcp_request` is populated from the stream as soon as the tool name
    // and arguments are known. Separate states ensure request-tree paths start
    // at depth 0 and are not confused with response-tree paths.
    mcp_request: Option<McpRequest>,
    mcp_request_tree_state: JsonTreeState,
    mcp_response_tree_state: JsonTreeState,
    // Scroll state for the MCP JSON tree body, shared across renders to preserve scroll position.
    mcp_scroll_state: ClippedScrollStateHandle,
    // Right-click context menu for MCP JSON tree rows (Copy / Copy JSON items).
    mcp_context_menu: ViewHandle<Menu<RequestedCommandViewAction>>,
    mcp_context_menu_open: bool,
    // The SavePosition anchor ID of the row that was last right-clicked, used
    // to position the context menu below the correct row.
    mcp_context_menu_anchor_id: Option<String>,
}

impl RequestedCommandView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action_id: AIAgentActionId,
        client_ids: ClientIdentifiers,
        action_type: RequestedActionViewType,
        block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        autonomy_setting_speedbump: AutonomySettingSpeedbump,
        manage_autonomy_settings_link_handle: MouseStateHandle,
        ai_block_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let cancel_button = CompactibleActionButton::new(
            REQUESTED_COMMAND_REJECT_LABEL.to_string(),
            Some(KeystrokeSource::Fixed(
                CANCEL_REQUESTED_COMMAND_KEYSTROKE.clone(),
            )),
            ButtonSize::InlineActionHeader,
            RequestedCommandViewAction::Reject,
            Icon::X,
            Arc::new(NakedTheme),
            ctx,
        );

        let position_id_prefix = format!("{action_id:?}");
        let accept_and_autoexecute_split_button = CompactibleSplitActionButton::new(
            REQUESTED_COMMAND_ACCEPT_LABEL.to_string(),
            Some(KeystrokeSource::Fixed(
                ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE.clone(),
            )),
            ButtonSize::InlineActionHeader,
            RequestedCommandViewAction::Accept,
            RequestedCommandViewAction::ToggleAcceptMenu,
            Icon::Check,
            true,
            Some(Self::get_position_id_for_accept_split_button(
                &position_id_prefix,
            )),
            ctx,
        );

        let edit_button = CompactibleActionButton::new(
            REQUESTED_COMMAND_EDIT_LABEL.to_string(),
            Some(KeystrokeSource::Binding(EDIT_COMMAND_ACTION_NAME)),
            ButtonSize::InlineActionHeader,
            RequestedCommandViewAction::OpenEditMode,
            Icon::Pencil,
            Arc::new(NakedTheme),
            ctx,
        );

        let minimize_button = CompactibleActionButton::new(
            REQUESTED_COMMAND_MINIMIZE_LABEL.to_string(),
            Some(KeystrokeSource::Fixed(
                MINIMIZE_REQUESTED_COMMAND_KEYSTROKE.clone(),
            )),
            ButtonSize::InlineActionHeader,
            RequestedCommandViewAction::CloseEditMode,
            Icon::ArrowBlockLeft,
            Arc::new(NakedTheme),
            ctx,
        );

        let is_finished = action_model
            .as_ref(ctx)
            .get_action_result(&action_id)
            .is_some();

        if !is_finished {
            ctx.subscribe_to_model(action_model, |me, _, event, ctx| {
                match event {
                    BlocklistAIActionEvent::QueuedAction(action_id)
                        if *action_id == me.action_id =>
                    {
                        ctx.notify();
                    }
                    BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(action_id)
                        if *action_id == me.action_id =>
                    {
                        if me.action_type.is_requested_command() {
                            me.ensure_editor(ctx);
                        }
                        me.set_is_header_expanded(true, ctx);
                        ctx.notify();
                    }
                    BlocklistAIActionEvent::ExecutingAction(action_id)
                        if *action_id == me.action_id =>
                    {
                        // For shared-session viewers, sync the command text from the action when it starts executing.
                        if me.action_model.as_ref(ctx).is_view_only() {
                            // Get the action from the block's output to sync the command text.
                            if let Some(command) = me
                                .block_model
                                .status(ctx)
                                .output_to_render()
                                .and_then(|output| {
                                    output
                                        .get()
                                        .actions()
                                        .find(|a| a.id == *action_id)
                                        .and_then(|action| match &action.action {
                                            AIAgentActionType::RequestCommandOutput {
                                                command,
                                                ..
                                            } => Some(command.clone()),
                                            _ => None,
                                        })
                                })
                            {
                                me.apply_streamed_update(&command, ctx);
                            }
                        }

                        me.destroy_editor();

                        if me.is_header_expanded {
                            me.set_is_header_expanded(false, ctx);
                        }
                        ctx.notify();
                    }
                    BlocklistAIActionEvent::FinishedAction { action_id, .. } => {
                        let Some(action_result) = me
                            .action_model
                            .as_ref(ctx)
                            .get_action_result(action_id)
                            .cloned()
                        else {
                            log::info!("Got finished action event without result: {action_id}.");
                            return;
                        };

                        // Else, we only care if the finished action is the original requested command.
                        if *action_id != me.action_id {
                            return;
                        }

                        let is_view_only = me.action_model.as_ref(ctx).is_view_only();
                        me.sync_command_from_result_for_viewer(&action_result, is_view_only);
                        me.destroy_editor();

                        match &action_result.result {
                            AIAgentActionResultType::RequestCommandOutput(command_result) => {
                                if matches!(
                                    command_result,
                                    RequestCommandOutputResult::CancelledBeforeExecution
                                ) {
                                    let terminal_model = me.terminal_model.lock();
                                    if terminal_model
                                        .block_list()
                                        .block_for_ai_action_id(&me.action_id)
                                        .is_none_or(|block| block.finished())
                                    {
                                        drop(terminal_model);
                                        if me.is_header_expanded {
                                            me.set_is_header_expanded(false, ctx);
                                        }
                                    }
                                }
                                ctx.notify();
                            }
                            AIAgentActionResultType::CallMCPTool(..) => {
                                ctx.notify();
                            }
                            _ => (),
                        }
                    }
                    _ => (),
                };
            });

            let conversation_id = client_ids.conversation_id;
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                move |_me, _, event, ctx| {
                    if let crate::ai::blocklist::BlocklistAIHistoryEvent::UpdatedConversationStatus {
                        conversation_id: event_conversation_id,
                        ..
                    } = event
                        && *event_conversation_id == conversation_id {
                            ctx.notify();
                        }
                },
            );
        }

        let accept_menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&accept_menu, |me, _menu, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.is_accept_split_button_menu_open = false;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        let mcp_context_menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&mcp_context_menu, |me, _menu, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.mcp_context_menu_open = false;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        Self {
            command_text: String::new(),
            editor: None,
            cancel_button,
            edit_button,
            minimize_button,
            accept_and_autoexecute_split_button,
            is_accept_split_button_menu_open: false,
            accept_split_button_menu: accept_menu,
            action_id: action_id.clone(),
            client_ids,
            action_type,
            is_editing: false,
            autonomy_setting_speedbump,
            is_header_expanded: false,
            header_mouse_state: Default::default(),
            copied_from_citation: None,
            derived_from_citations: Default::default(),
            citation_state_handles: Default::default(),
            autoexecute_readonly_commands_speedbump_checkbox_handle: Default::default(),
            manage_autonomy_settings_link_handle,
            block_model,
            action_model: action_model.clone(),
            position_id_prefix,
            terminal_model,
            ai_block_view_id,
            mcp_content_selection_handle: SelectionHandle::default(),
            mcp_content_selected_text: Arc::new(std::sync::RwLock::new(None)),
            mcp_request: None,
            mcp_request_tree_state: Default::default(),
            mcp_response_tree_state: Default::default(),
            mcp_scroll_state: Default::default(),
            mcp_context_menu,
            mcp_context_menu_open: false,
            mcp_context_menu_anchor_id: None,
        }
    }

    /// Creates the code editor view if it doesn't already exist, initializing it from
    /// `command_text`. The editor is only needed when it will be visually rendered (i.e.,
    /// when the action is blocked and the header is expanded for command types).
    fn ensure_editor(&mut self, ctx: &mut ViewContext<Self>) {
        if self.editor.is_some() {
            return;
        }

        let command_text = self.command_text.clone();
        let editor = ctx.add_typed_action_view(|ctx| {
            let view = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
                ctx,
            )
            .with_show_line_numbers(false);
            view.set_interaction_state(InteractionState::Selectable, ctx);
            view.set_show_current_line_highlights(false, ctx);

            if !command_text.is_empty() {
                view.system_append_autoscroll_vertical_only(&command_text, ctx);
                view.system_append_autoscroll_vertical_only("", ctx);
            }

            view
        });
        ctx.subscribe_to_view(&editor, |me, view, event, ctx| {
            me.handle_editor_event(event, view, ctx);
        });
        self.editor = Some(editor);
    }

    /// Drops the editor view. Does not sync editor contents back to `command_text`;
    /// use `commit_editor_contents` before calling this if you need to preserve edits.
    fn destroy_editor(&mut self) {
        self.editor = None;
    }

    /// Reads the editor contents into `command_text`, committing any user edits.
    fn commit_editor_contents(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(editor) = &self.editor {
            self.command_text = editor.as_ref(ctx).text(ctx).into_string();
        }
    }

    /// Commits any pending editor edits to `command_text` and returns the committed text.
    /// This should be used by external code paths that accept the command (e.g.,
    /// auto-execute) to ensure user edits are not lost.
    pub fn commit_and_get_command_text(&mut self, ctx: &mut ViewContext<Self>) -> String {
        self.commit_editor_contents(ctx);
        self.command_text.clone()
    }

    fn handle_editor_event(
        &mut self,
        event: &CodeEditorEvent,
        view: ViewHandle<CodeEditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeEditorEvent::Focused => ctx.emit(RequestedCommandViewEvent::EditorFocused),
            CodeEditorEvent::SelectionChanged => {
                // If there's an ongoing text selection, clear all other selections within the
                // `RequestedCommandView`'s view sub-hierarchy to ensure only one component
                // has a selection at a time.
                //
                // The `is_some` check is necessary because `CodeEditorEvent::SelectionChanged` is
                // also emitted when the editor's selection is cleared via external means
                // (i.e. when a text selection is made outside the `CodeEditorView`).
                if view.as_ref(ctx).selected_text(ctx).is_some() {
                    ctx.emit(RequestedCommandViewEvent::TextSelected);
                }
            }
            CodeEditorEvent::CopiedEmptyText => {
                ctx.emit(RequestedCommandViewEvent::CopiedEmptyText);
            }
            #[cfg(windows)]
            CodeEditorEvent::WindowsCtrlC { copied_selection } if !copied_selection => {
                ctx.emit(RequestedCommandViewEvent::Rejected);
            }
            _ => {}
        }
    }

    fn set_is_header_expanded(&mut self, value: bool, ctx: &mut ViewContext<Self>) {
        if value == self.is_header_expanded {
            return;
        }
        self.is_header_expanded = value;

        ctx.emit(RequestedCommandViewEvent::UpdatedExpansionState {
            is_expanded: self.is_header_expanded,
        });
        ctx.notify();
    }

    fn toggle_accept_split_button_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_accept_split_button_menu_open = !self.is_accept_split_button_menu_open;
        if self.is_accept_split_button_menu_open {
            // Accept shows Enter or Cmd/Ctrl+Enter depending on edit state
            let accept_keystroke = if self.is_editing {
                CMD_ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE.displayed()
            } else {
                ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE.displayed()
            };
            let auto_keystroke = keybinding_name_to_keystroke(
                crate::terminal::TOGGLE_AUTOEXECUTE_MODE_KEYBINDING,
                ctx,
            )
            .map(|k| k.displayed())
            .unwrap_or_default();

            let accept_item = MenuItemFields::new_with_label(
                REQUESTED_COMMAND_ACCEPT_LABEL,
                accept_keystroke.as_str(),
            )
            .with_on_select_action(RequestedCommandViewAction::Accept)
            .into_item();

            let auto_item = MenuItemFields::new_with_label("Auto-approve", auto_keystroke.as_str())
                .with_on_select_action(RequestedCommandViewAction::AcceptAndAutoExecute)
                .into_item();

            self.accept_split_button_menu.update(ctx, |menu, ctx| {
                menu.set_items(vec![accept_item, auto_item], ctx);
            });
            self.accept_split_button_menu
                .update(ctx, |menu, ctx| menu.set_selected_by_index(0, ctx));
            ctx.focus(&self.accept_split_button_menu);
        }
        ctx.notify();
    }

    fn get_position_id_for_accept_split_button(prefix: &str) -> String {
        format!("RequestedCommandView-{prefix}-accept-split")
    }

    pub fn is_header_expanded(&self) -> bool {
        self.is_header_expanded
    }

    /// We use the requested command footer to show citations.
    fn maybe_render_footer(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let citations_padding = 4.;
        let citations_font_size = appearance.monospace_font_size() - 2.;
        let citations_footer_props = if self.derived_from_citations.is_empty()
            || self.block_model.status(app).is_streaming()
        {
            // Don't render the footer if there aren't even any citations.
            None
        } else if let Some(copied_citation) = &self.copied_from_citation {
            // If there's exactly one citation that the requested command was copied from,
            // we only show that citation.
            let Some(mouse_state_handle) =
                self.citation_state_handles.get(copied_citation).cloned()
            else {
                log::warn!(
                    "Tried to retrieve mouse state handle for citation, but no mouse state handle exists."
                );
                return None;
            };
            render_citation(
                copied_citation,
                mouse_state_handle,
                citations_font_size,
                citations_padding,
                app,
            )
            .map(|citation| ("Copied from", citation))
        } else {
            // Otherwise, we render all the citations (if any) and mention that the command was derived from them.
            render_citation_chips(
                &self.derived_from_citations,
                &self.citation_state_handles,
                citations_font_size,
                citations_padding,
                app,
            )
            .map(|citations| ("Derived from", citations))
        };

        let citations_footer = citations_footer_props.map(|(prefix, suffix)| {
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(
                        Text::new(
                            format!("{prefix} "),
                            appearance.ui_font_family(),
                            appearance.monospace_font_size() - 1.,
                        )
                        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                        .with_selectable(false)
                        .finish(),
                    )
                    .with_child(suffix)
                    .finish(),
            )
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(4.)
            .with_background(theme.surface_1())
            .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(7.)))
            .finish()
        });

        match (citations_footer, &self.autonomy_setting_speedbump) {
            // If there's a citation footer, prefer showing that instead of the speedbump.
            (Some(citations_footer), _) => Some(citations_footer),
            (
                _,
                AutonomySettingSpeedbump::ShouldShowForAutoexecutingReadonlyCommands {
                    action_id: show_for_action_id,
                    checked,
                    shown,
                },
            ) if show_for_action_id == &self.action_id => {
                *shown.lock() = true;
                Some(render_autonomy_checkbox_setting_speedbump_footer(
                    "Always allow Oz to execute read-only commands (relies on model)",
                    *checked,
                    AIBlockAction::ToggleAutoexecuteReadonlyCommandsSpeedbumpCheckbox,
                    self.autoexecute_readonly_commands_speedbump_checkbox_handle
                        .clone(),
                    self.manage_autonomy_settings_link_handle.clone(),
                    app,
                ))
            }
            (
                _,
                AutonomySettingSpeedbump::ShouldShowForProfileCommandAutoexecution {
                    action_id: show_for_action_id,
                    shown,
                },
            ) if show_for_action_id == &self.action_id => {
                *shown.lock() = true;
                Some(Self::render_profile_autoexecution_info_footer(
                    self.manage_autonomy_settings_link_handle.clone(),
                    app,
                ))
            }
            _ => None,
        }
    }

    fn render_profile_autoexecution_info_footer(
        settings_link_handle: MouseStateHandle,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = appearance.monospace_font_size() - 1.;

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::Info
                                .to_warpui_icon(
                                    blended_colors::text_sub(theme, theme.surface_1()).into(),
                                )
                                .finish(),
                        )
                        .with_height(font_size)
                        .with_width(font_size)
                        .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                )
                .with_child(
                    Text::new(
                        "Your profile is set to always ask for permission to execute commands.",
                        appearance.ui_font_family(),
                        font_size,
                    )
                    .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                    .with_selectable(false)
                    .finish(),
                )
                .with_child(
                    Expanded::new(
                        1.,
                        Align::new(
                            appearance
                                .ui_builder()
                                .link(
                                    "Manage command execution setting".into(),
                                    None,
                                    Some(Box::new(move |ctx| {
                                        ctx.dispatch_typed_action(
                                            RequestedCommandViewAction::OpenActiveAgentProfileEditor,
                                        );
                                    })),
                                    settings_link_handle,
                                )
                                .build()
                                .finish(),
                        )
                        .right()
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_vertical_padding(8.)
        .with_border(Border::top(1.).with_border_fill(theme.surface_1()))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(7.)))
        .finish()
    }

    fn open_edit_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_editing = true;
        self.accept_and_autoexecute_split_button.set_keybinding(
            Some(KeystrokeSource::Fixed(
                CMD_ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE.clone(),
            )),
            ctx,
        );
        self.ensure_editor(ctx);
        if let Some(editor) = &self.editor {
            editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
                ctx.notify();
            });
            ctx.focus(editor);
        }
        ctx.notify();
    }

    fn close_edit_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_editing = false;
        self.accept_and_autoexecute_split_button.set_keybinding(
            Some(KeystrokeSource::Fixed(
                ENTER_ACCEPT_REQUESTED_COMMAND_KEYSTROKE.clone(),
            )),
            ctx,
        );
        if let Some(editor) = &self.editor {
            editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Selectable, ctx);
                ctx.notify();
            });
        }
        ctx.focus_self();
        ctx.notify();
    }

    pub fn command_text(&self) -> &str {
        &self.command_text
    }

    pub fn copied_from_citation(&self) -> Option<&AIAgentCitation> {
        self.copied_from_citation.as_ref()
    }

    pub fn update_copied_from_citation(&mut self, citation: &AIAgentCitation) {
        self.citation_state_handles
            .entry(citation.clone())
            .or_default();
        self.copied_from_citation = Some(citation.clone());
    }

    pub fn update_derived_from_citations(&mut self, citations: &[AIAgentCitation]) {
        for citation in citations {
            self.citation_state_handles
                .entry(citation.clone())
                .or_default();
        }
        self.derived_from_citations = citations.to_vec();
    }

    pub fn set_autonomy_setting_speedbump(
        &mut self,
        speedbump: AutonomySettingSpeedbump,
        ctx: &mut ViewContext<Self>,
    ) {
        self.autonomy_setting_speedbump = speedbump;
        ctx.notify();
    }

    /// For shared-session viewers, reset the command text to the executed command from
    /// the action result. This is important for showing the correct command in the action
    /// header if the command was manually edited by the user.
    fn sync_command_from_result_for_viewer(
        &mut self,
        action_result: &AIAgentActionResult,
        is_view_only: bool,
    ) {
        if !is_view_only {
            return;
        }
        if let Some(command) = action_result.result.command_str()
            && !command.is_empty()
        {
            self.command_text = command.to_string();
        }
    }

    /// Apply a streamed update from the server.
    ///
    /// Note: It is assumed that this is an incremental update of the command text.
    /// Only the range of bytes that have not been appended are appended. It is assumed that earlier bytes are not modified.
    /// This is to reduce flicker.
    ///
    /// If the command length is shorter than the previous update, then the command is truncated to the given byte length.
    pub fn apply_streamed_update(&mut self, command: &str, ctx: &mut ViewContext<Self>) {
        match command.len().cmp(&self.command_text.len()) {
            Ordering::Greater => {
                // Check if the existing length falls on a valid UTF-8 character boundary.
                let existing_length = self.command_text.len();
                if command.is_char_boundary(existing_length) {
                    self.command_text.push_str(&command[existing_length..]);
                } else {
                    self.command_text = command.to_string();
                }
            }
            Ordering::Less => {
                self.command_text.truncate(command.len());
            }
            Ordering::Equal => {}
        }

        // If the editor exists, sync it with the updated command text.
        if let Some(editor) = &self.editor {
            editor.update(ctx, |editor, ctx| {
                let editor_length = editor.text(ctx).as_str().len();
                match self.command_text.len().cmp(&editor_length) {
                    Ordering::Greater => {
                        let slice_to_append = if self.command_text.is_char_boundary(editor_length) {
                            &self.command_text[editor_length..]
                        } else {
                            editor.truncate(0, ctx);
                            &self.command_text
                        };
                        // TODO(Simon): The first insertion into an empty Buffer creates a trailing newline.
                        // If the requested command is streamed in in a single chunk, then there will
                        // be an extra newline rendered at the end of the `CodeEditorView`. This is likely
                        // caused by an initial insertion bug somewhere in the `Buffer` logic.
                        //
                        // To reproduce this bug, simply clear the buffer and type in two letters. You'll
                        // notice that a newline is created on the first letter, but removed on the second.
                        // The temporary workaround is to append an empty string to the end of each chunk,
                        // which acts as the second insertion that clears the trailing newline.
                        editor.system_append_autoscroll_vertical_only(slice_to_append, ctx);
                        editor.system_append_autoscroll_vertical_only("", ctx);
                        ctx.notify();
                    }
                    Ordering::Less => {
                        editor.truncate(self.command_text.len(), ctx);
                        ctx.notify();
                    }
                    Ordering::Equal => {}
                }
            });
        }
    }

    /// Returns the currently selected text.
    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        // Check MCP content selection first, then fall back to editor selection.
        if let Ok(mcp_selection) = self.mcp_content_selected_text.read()
            && mcp_selection.is_some()
        {
            return mcp_selection.clone();
        }
        self.editor
            .as_ref()
            .and_then(|editor| editor.as_ref(ctx).selected_text(ctx))
    }

    pub fn clear_selection(&mut self, ctx: &mut ViewContext<Self>) {
        // Clear MCP content selection if it exists, else fall back to editor selection.
        self.mcp_content_selection_handle.clear();
        match self.mcp_content_selected_text.write() {
            Ok(mut mcp_selection) => {
                *mcp_selection = None;
            }
            _ => {
                if let Some(editor) = &self.editor {
                    editor.update(ctx, |editor, ctx| {
                        editor.clear_selection(ctx);
                    });
                }
            }
        }
    }

    /// Stores the structured MCP tool request data for JSON tree rendering.
    pub(crate) fn update_mcp_request(&mut self, args: serde_json::Value) {
        self.mcp_request = Some(McpRequest { args });
    }

    /// Extracts the tool name from MCP tool command text, removing parameters.
    /// For example, "tool_name(param1, param2)" becomes "tool_name".
    fn extract_mcp_tool_name(&self, command_text: &str) -> String {
        if let Some(paren_pos) = command_text.find('(') {
            command_text[..paren_pos].trim().to_string()
        } else {
            command_text.trim().to_string()
        }
    }

    fn render_header(
        &self,
        should_round_bottom_corners: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let action_status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);

        let mut title: Cow<'static, str>;
        let mut font_override = None;
        let mut font_color_override = None;

        let terminal_model = self.terminal_model.lock();
        let requested_command_block = match &self.action_type {
            RequestedActionViewType::Command => terminal_model
                .block_list()
                .block_for_ai_action_id(&self.action_id),
            RequestedActionViewType::McpTool => None,
        };

        match action_status {
            Some(AIActionStatus::Preprocessing) => {
                title = self.get_header_title_text().into();
                font_override = Some(appearance.monospace_font_family());
                if !self
                    .block_model
                    .is_first_action_in_output(&self.action_id, app)
                {
                    font_color_override = Some(blended_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_2(),
                    ));
                }
            }
            Some(AIActionStatus::Queued) => {
                title = self.get_header_title_text().into();
                font_override = Some(appearance.monospace_font_family());
                font_color_override = Some(blended_colors::text_disabled(
                    appearance.theme(),
                    appearance.theme().surface_2(),
                ));
            }
            Some(AIActionStatus::Blocked) => {
                title = match &self.action_type {
                    RequestedActionViewType::Command => COMMAND_WAITING_FOR_USER_MESSAGE.into(),
                    RequestedActionViewType::McpTool => MCP_TOOL_WAITING_FOR_USER_MESSAGE.into(),
                };
            }
            Some(AIActionStatus::RunningAsync) | Some(AIActionStatus::Finished(..))
                if self.is_header_expanded =>
            {
                title = match &self.action_type {
                    RequestedActionViewType::Command => {
                        if let Some(long_running_command_control_state) = requested_command_block
                            .filter(|block| block.is_executing())
                            .and_then(|block| block.long_running_control_state())
                        {
                            match long_running_command_control_state {
                                LongRunningCommandControlState::Agent { is_blocked, .. } => {
                                    let is_errored =
                                        self.block_model.as_ref().conversation(app).is_some_and(
                                            |conversation| conversation.status().is_error(),
                                        );

                                    if is_errored {
                                        AGENT_ERRORED_COMMAND_MESSAGE.into()
                                    } else if *is_blocked {
                                        AGENT_NEEDS_INPUT_MESSAGE.into()
                                    } else {
                                        MONITORING_COMMAND_MESSAGE.into()
                                    }
                                }
                                LongRunningCommandControlState::User { reason } => {
                                    header_message_for_user_take_over_reason(reason).into()
                                }
                            }
                        } else {
                            VIEWING_COMMAND_DETAIL_MESSAGE.into()
                        }
                    }
                    RequestedActionViewType::McpTool => VIEWING_MCP_TOOL_DETAIL_MESSAGE.into(),
                };
            }
            None => {
                if self.block_model.status(app).is_streaming() {
                    title = LOADING_MESSAGE.into();

                    if !self
                        .block_model
                        .is_first_action_in_output(&self.action_id, app)
                    {
                        font_color_override = Some(blended_colors::text_disabled(
                            appearance.theme(),
                            appearance.theme().surface_2(),
                        ));
                    }
                } else if requested_command_block.is_some_and(|block| block.finished()) {
                    // If a finished command block exists but there's no action status,
                    // treat the same as a finished command (normal text styling).
                    title = self.get_header_title_text().into();
                    font_override = Some(appearance.monospace_font_family());
                } else {
                    // If there is no action status and response is not streaming, it was cancelled
                    // mid-flight.
                    let title_str = self.get_header_title_text();
                    title = if title_str.trim().is_empty() {
                        LOADING_MESSAGE.into()
                    } else {
                        title_str.into()
                    };
                    if self.action_type.is_requested_command() {
                        font_override = Some(appearance.monospace_font_family());
                    }
                    font_color_override = Some(blended_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_2(),
                    ));
                }
            }
            _ => {
                title = self.get_header_title_text().into();

                // Show cancelled command loading message when the command was cancelled during generation,
                // and then restored with an empty title as a result.
                if title.is_empty() {
                    title = LOADING_MESSAGE.into();
                    font_color_override = Some(blended_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_2(),
                    ));
                } else {
                    // Only use monospace font for actual command text
                    font_override = Some(appearance.monospace_font_family());
                }
            }
        };

        let mut config = HeaderConfig::new(title, app)
            .with_selectable_text()
            .with_icon(if let Some(block) = requested_command_block {
                if !block.finished() {
                    if let Some(long_running_command_control_state) =
                        block.long_running_control_state()
                    {
                        match long_running_command_control_state {
                            LongRunningCommandControlState::Agent { is_blocked, .. } => {
                                let is_errored =
                                    self.block_model.as_ref().conversation(app).is_some_and(
                                        |conversation| conversation.status().is_error(),
                                    );
                                if is_errored {
                                    icons::failed_icon(appearance)
                                } else if *is_blocked {
                                    icons::yellow_stop_icon(appearance)
                                } else {
                                    icons::in_progress_icon(appearance)
                                }
                            }
                            LongRunningCommandControlState::User { .. } => {
                                icons::gray_stop_icon(appearance)
                            }
                        }
                    } else {
                        icons::yellow_running_icon(appearance)
                    }
                } else if block.exit_code().is_sigint() {
                    inline_action_icons::cancelled_icon(appearance)
                } else if !block.exit_code().was_successful() {
                    inline_action_icons::red_x_icon(appearance)
                } else {
                    inline_action_icons::green_check_icon(appearance)
                }
            } else {
                action_icon(
                    &self.action_id,
                    &self.action_model,
                    self.block_model.as_ref(),
                    app,
                )
            });

        if should_round_bottom_corners {
            config = config.with_corner_radius_override(CornerRadius::with_all(Radius::Pixels(7.)));
        } else {
            config = config.with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(7.)));
        }

        if let Some(font_override) = font_override {
            config = config.with_font_family(font_override);
        }
        if let Some(font_color_override) = font_color_override {
            config = config.with_font_color(font_color_override);
        }

        match action_status {
            Some(AIActionStatus::Blocked) => {
                let (action_buttons, size_switch_threshold) = if self.is_editing {
                    let action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>> = vec![
                        Rc::new(self.cancel_button.clone()),
                        Rc::new(self.minimize_button.clone()),
                        Rc::new(self.accept_and_autoexecute_split_button.clone()),
                    ];
                    (action_buttons, MEDIUM_SIZE_SWITCH_THRESHOLD)
                } else {
                    match &self.action_type {
                        RequestedActionViewType::Command => {
                            let action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>> = vec![
                                Rc::new(self.cancel_button.clone()),
                                Rc::new(self.edit_button.clone()),
                                Rc::new(self.accept_and_autoexecute_split_button.clone()),
                            ];
                            (action_buttons, LARGE_SIZE_SWITCH_THRESHOLD)
                        }
                        RequestedActionViewType::McpTool => {
                            let action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>> = vec![
                                Rc::new(self.cancel_button.clone()),
                                Rc::new(self.accept_and_autoexecute_split_button.clone()),
                            ];
                            (action_buttons, SMALL_SIZE_SWITCH_THRESHOLD)
                        }
                    }
                };
                config = config.with_interaction_mode(InteractionMode::ActionButtons {
                    action_buttons,
                    size_switch_threshold,
                });
            }
            Some(AIActionStatus::RunningAsync) if self.action_type.is_requested_command() => {
                config = config.with_interaction_mode(InteractionMode::ManuallyExpandable(
                    self.get_expansion_config(requested_command_block, app),
                ));
            }
            Some(AIActionStatus::Finished(result)) => {
                // Determine if command should be expandable based on whether it actually executed.
                // If a finished command block exists for this action, the command definitely ran,
                // so it should be expandable regardless of the action result type. This handles
                // cases where the action result is stale (e.g. a LongRunningCommandSnapshot
                // converted to CancelledBeforeExecution on restore, even though the command
                // completed successfully).
                let has_finished_command_block =
                    requested_command_block.is_some_and(|block| block.finished());
                let should_be_expandable = has_finished_command_block
                    || match &result.result {
                        AIAgentActionResultType::RequestCommandOutput(command_result) => {
                            match command_result {
                                // All completed commands are expandable (including interrupted ones)
                                RequestCommandOutputResult::Completed { .. } => true,
                                // Cancelled before execution are not expandable
                                RequestCommandOutputResult::CancelledBeforeExecution => false,
                                _ => result.result.is_successful() || result.result.is_failed(),
                            }
                        }
                        _ => result.result.is_successful() || result.result.is_failed(),
                    };

                if should_be_expandable {
                    config = config.with_interaction_mode(InteractionMode::ManuallyExpandable(
                        self.get_expansion_config(requested_command_block, app),
                    ));
                } else {
                    // Commands cancelled before execution should be right clickable only.
                    let command_text_for_callback = self.command_text().to_string();
                    config = config.with_interaction_mode(InteractionMode::RightClickable(
                        RightClickConfig::new(
                            Rc::new(move |ctx: &mut EventContext| {
                                ctx.dispatch_typed_action(
                                    AIBlockAction::StoreRightClickedCommand {
                                        command: command_text_for_callback.clone(),
                                    },
                                );
                            }),
                            self.header_mouse_state.clone(),
                        ),
                    ));
                }
            }
            _ => {
                // Even without a known action status, if a finished command block exists
                // for this action, the command ran and the header should be expandable.
                if requested_command_block.is_some_and(|block| block.finished()) {
                    config = config.with_interaction_mode(InteractionMode::ManuallyExpandable(
                        self.get_expansion_config(requested_command_block, app),
                    ));
                }
            }
        };

        config.render(app)
    }

    fn get_header_title_text(&self) -> String {
        match &self.action_type {
            RequestedActionViewType::Command => format_command_text(self.command_text()),
            RequestedActionViewType::McpTool => self.extract_mcp_tool_name(self.command_text()),
        }
    }

    fn get_expansion_config(
        &self,
        requested_command_block: Option<&Block>,
        app: &AppContext,
    ) -> ExpandedConfig {
        let command_text_for_callback = self.command_text().to_string();
        let mut expansion_config =
            ExpandedConfig::new(self.is_header_expanded, self.header_mouse_state.clone())
                .with_right_click_callback(move |ctx| {
                    ctx.dispatch_typed_action(AIBlockAction::StoreRightClickedCommand {
                        command: command_text_for_callback.clone(),
                    });
                });

        let is_active_agent_monitored_command = requested_command_block
            .is_some_and(|block| block.is_agent_monitoring() && !block.finished());
        if !is_active_agent_monitored_command {
            expansion_config = expansion_config.with_toggle_callback(|ctx| {
                ctx.dispatch_typed_action(RequestedCommandViewAction::ToggleExpanded);
            });
        }

        if InputModeSettings::as_ref(app).is_pinned_to_top()
            && self.action_type.is_requested_command()
        {
            expansion_config = expansion_config.with_expands_upwards();
        }
        expansion_config
    }

    fn is_waiting_for_user_confirmation(&self, app: &AppContext) -> bool {
        self.action_model
            .as_ref(app)
            .get_action_status(&self.action_id)
            .is_some_and(|status| status.is_blocked())
    }
}

pub(crate) fn header_message_for_user_take_over_reason(
    reason: &UserTakeOverReason,
) -> &'static str {
    match reason {
        UserTakeOverReason::Manual => USER_TOOK_CONTROL_COMMAND_MESSAGE,
        UserTakeOverReason::Stop { .. } => USER_STOPPED_CLI_SUBAGENT_COMMAND_MESSAGE,
        UserTakeOverReason::TransferFromAgent { .. } => {
            AGENT_REQUESTED_USER_TAKE_CONTROL_COMMAND_MESSAGE
        }
    }
}

impl Entity for RequestedCommandView {
    type Event = RequestedCommandViewEvent;
}

impl View for RequestedCommandView {
    fn ui_name() -> &'static str {
        "RequestedCommandView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let action_status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);

        let is_last_output_message_in_output = self
            .block_model
            .status(app)
            .output_to_render()
            .is_some_and(|output| {
                let output_data = output.get();
                output_data.messages.last().is_some_and(|message| {
                    matches!(
                        &message.message,
                        AIAgentOutputMessageType::Action(action) if action.id == self.action_id
                    )
                })
            });

        let is_input_pinned_to_top =
            *InputModeSettings::as_ref(app).input_mode.value() == InputMode::PinnedToTop;

        // When expanded details are rendered using a regular block, having a non-zero horizontal
        // margin while toggled expanded will cause the body to look wider than the header.
        // The expanded details should also appear connected to the header, so we remove bottom margin in this case.
        let is_rendered_above_expanded_command_block = {
            let terminal_model = self.terminal_model.lock();

            is_last_output_message_in_output
                && self.action_type.is_requested_command()
                && action_status.as_ref().is_some_and(|status| {
                    status.is_running() || (status.is_success() || status.is_failed())
                })
                && !is_input_pinned_to_top
                && self.is_header_expanded
                && terminal_model
                    .block_list()
                    .is_requested_command_block_immediately_after_ai_block(
                        self.ai_block_view_id,
                        &self.action_id,
                    )
        };

        // When the requested command is expanded but there is no subsequent block containing
        // command details beneath, then the command details must be rendered inline.
        let should_render_editor = self.is_header_expanded
            && action_status
                .as_ref()
                .is_some_and(|status| status.is_blocked())
            && !self.command_text.is_empty()
            && self.action_type.is_requested_command()
            && self.editor.is_some();

        // For MCP tools, when expanded, show either the tool call details or the JSON response.
        let should_render_mcp_content = self.is_header_expanded
            && self.action_type.is_mcp_tool()
            && !self.command_text.is_empty();

        let has_citations_footer =
            !self.derived_from_citations.is_empty() && !self.block_model.status(app).is_streaming();
        let header_element = self.render_header(
            !should_render_editor
                && !should_render_mcp_content
                && !is_rendered_above_expanded_command_block
                && !has_citations_footer,
            app,
        );

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Clipped::new(header_element).finish());

        if let (true, Some(editor)) = (should_render_editor, &self.editor) {
            content.add_child(
                ConstrainedBox::new(
                    Container::new(ChildView::new(editor).finish())
                        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                        .with_padding_top(REQUESTED_COMMAND_BODY_VERTICAL_PADDING)
                        .with_padding_bottom(
                            REQUESTED_COMMAND_BODY_VERTICAL_PADDING - SCROLLBAR_WIDTH.as_f32() - 2.,
                        )
                        .with_background_color(theme.background().into_solid())
                        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(7.)))
                        .finish(),
                )
                .with_max_height(MAX_EDITOR_HEIGHT)
                .finish(),
            );
        }

        if should_render_mcp_content {
            if FeatureFlag::McpJsonTreeView.is_enabled() {
                let colors = JsonTreeColors::from_theme(theme);
                let font_family = appearance.monospace_font_family();

                let mut tree_column =
                    Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

                // Request section: show the tree if args are known, or a placeholder.
                let request_section: Box<dyn Element> = if let Some(mcp_request) = &self.mcp_request
                {
                    let on_toggle_req: Arc<ToggleFn> = Arc::new(|ctx, path, _depth| {
                        ctx.dispatch_typed_action(RequestedCommandViewAction::ToggleJsonNode {
                            path,
                            tree: McpTree::Request,
                        });
                    });
                    let on_copy_req: Arc<CopyJsonFn> = Arc::new(|ctx, _path, value, anchor_id| {
                        let json_text = serde_json::to_string_pretty(&value).unwrap_or_default();
                        ctx.dispatch_typed_action(RequestedCommandViewAction::ShowMcpContextMenu {
                            json_text,
                            anchor_id,
                        });
                    });
                    let on_toggle_string_req: Arc<ToggleStringFn> = Arc::new(|ctx, path| {
                        ctx.dispatch_typed_action(RequestedCommandViewAction::ToggleJsonString {
                            path,
                            tree: McpTree::Request,
                        });
                    });
                    render_json_tree(
                        &mcp_request.args,
                        Some("Request"),
                        &self.mcp_request_tree_state,
                        &colors,
                        &format!("{}-req", self.position_id_prefix),
                        on_toggle_req,
                        on_toggle_string_req,
                        on_copy_req,
                        appearance,
                    )
                } else {
                    let mut col =
                        Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
                    col.add_child(
                        Text::new_inline("Request".to_string(), font_family, TREE_FONT_SIZE)
                            .with_color(colors.annotation)
                            .soft_wrap(false)
                            .finish(),
                    );
                    col.add_child(
                        Text::new_inline("(no arguments)".to_string(), font_family, TREE_FONT_SIZE)
                            .with_color(colors.annotation)
                            .soft_wrap(false)
                            .finish(),
                    );
                    col.finish()
                };
                tree_column.add_child(request_section);

                // Response section: present only when a finished result exists.
                if let Some(AIAgentActionResultType::CallMCPTool(result)) = action_status
                    .as_ref()
                    .and_then(|status| status.finished_result().map(|r| &r.result))
                {
                    tree_column.add_child(
                        Container::new(Empty::new().finish())
                            .with_padding_top(8.)
                            .finish(),
                    );

                    let renderable = mcp_result_to_renderable(result);
                    let response_element: Box<dyn Element> = match renderable {
                        McpRenderable::Tree(value) => {
                            let on_toggle_resp: Arc<ToggleFn> = Arc::new(|ctx, path, _depth| {
                                ctx.dispatch_typed_action(
                                    RequestedCommandViewAction::ToggleJsonNode {
                                        path,
                                        tree: McpTree::Response,
                                    },
                                );
                            });
                            let on_copy_resp: Arc<CopyJsonFn> =
                                Arc::new(|ctx, _path, value, anchor_id| {
                                    let json_text =
                                        serde_json::to_string_pretty(&value).unwrap_or_default();
                                    ctx.dispatch_typed_action(
                                        RequestedCommandViewAction::ShowMcpContextMenu {
                                            json_text,
                                            anchor_id,
                                        },
                                    );
                                });
                            let on_toggle_string_resp: Arc<ToggleStringFn> =
                                Arc::new(|ctx, path| {
                                    ctx.dispatch_typed_action(
                                        RequestedCommandViewAction::ToggleJsonString {
                                            path,
                                            tree: McpTree::Response,
                                        },
                                    );
                                });
                            render_json_tree(
                                &value,
                                Some("Response"),
                                &self.mcp_response_tree_state,
                                &colors,
                                &format!("{}-resp", self.position_id_prefix),
                                on_toggle_resp,
                                on_toggle_string_resp,
                                on_copy_resp,
                                appearance,
                            )
                        }
                        McpRenderable::Error(e) => {
                            let mut col = Flex::column()
                                .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
                            col.add_child(
                                Text::new_inline(
                                    "Response".to_string(),
                                    font_family,
                                    TREE_FONT_SIZE,
                                )
                                .with_color(colors.annotation)
                                .soft_wrap(false)
                                .finish(),
                            );
                            col.add_child(
                                Text::new(format!("Error: {e}"), font_family, TREE_FONT_SIZE)
                                    .with_color(theme.ui_error_color())
                                    .with_selectable(true)
                                    .finish(),
                            );
                            col.finish()
                        }
                        McpRenderable::Cancelled => {
                            let mut col = Flex::column()
                                .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
                            col.add_child(
                                Text::new_inline(
                                    "Response".to_string(),
                                    font_family,
                                    TREE_FONT_SIZE,
                                )
                                .with_color(colors.annotation)
                                .soft_wrap(false)
                                .finish(),
                            );
                            col.add_child(
                                Text::new_inline(
                                    "Cancelled".to_string(),
                                    font_family,
                                    TREE_FONT_SIZE,
                                )
                                .with_color(colors.annotation)
                                .soft_wrap(false)
                                .finish(),
                            );
                            col.finish()
                        }
                    };
                    tree_column.add_child(response_element);
                }

                // Height cap prevents a large tree from pushing subsequent blocks off-screen.
                // Padding is on the outer Container so it applies outside the scrollable viewport.
                let scrollable = NewScrollable::vertical(
                    SingleAxisConfig::Clipped {
                        handle: self.mcp_scroll_state.clone(),
                        child: tree_column.finish(),
                    },
                    theme.nonactive_ui_detail().into(),
                    theme.active_ui_detail().into(),
                    warpui::elements::Fill::None,
                )
                .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
                .with_propagate_mousewheel_if_not_handled(true)
                .finish();

                let constrained = ConstrainedBox::new(scrollable)
                    .with_max_height(MAX_EDITOR_HEIGHT)
                    .finish();

                // SelectableArea enables text drag-selection across the tree rows.
                // Per-row Hoverables receive LeftMouseDown before SelectableArea sees it
                // (depth-first dispatch), so click handlers are unaffected.
                let mcp_selected_text = self.mcp_content_selected_text.clone();
                let selectable_content = SelectableArea::new(
                    self.mcp_content_selection_handle.clone(),
                    #[allow(clippy::unwrap_used)]
                    move |selection_args, _, _| {
                        *mcp_selected_text.write().unwrap() = selection_args.selection;
                    },
                    constrained,
                )
                .on_selection_updated(|ctx, _| {
                    ctx.dispatch_typed_action(RequestedCommandViewAction::SelectText);
                })
                .finish();

                content.add_child(
                    Container::new(selectable_content)
                        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                        .with_vertical_padding(REQUESTED_COMMAND_BODY_VERTICAL_PADDING)
                        .with_background(theme.background())
                        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(7.)))
                        .finish(),
                );
            } else {
                // Fallback: flat pretty-printed JSON.
                let command_text = self.command_text();
                let content_text = if let Some(AIAgentActionResultType::CallMCPTool(result)) =
                    action_status
                        .as_ref()
                        .and_then(|status| status.finished_result().map(|result| &result.result))
                {
                    let result_text = match result {
                        CallMCPToolResult::Success { result } => {
                            serde_json::to_string_pretty(result)
                                .unwrap_or_else(|_| "Error formatting JSON".to_string())
                        }
                        CallMCPToolResult::Error(error) => format!("Error: {error}"),
                        CallMCPToolResult::Cancelled => "Tool call was cancelled".to_string(),
                    };
                    format!("{command_text}\n\nResponse: {result_text}")
                } else if self.is_header_expanded {
                    command_text.to_string()
                } else {
                    self.extract_mcp_tool_name(command_text)
                };
                let text_element = Text::new(
                    content_text,
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(blended_colors::text_main(theme, theme.background()))
                .with_selectable(true)
                .finish();
                let mcp_selected_text = self.mcp_content_selected_text.clone();
                let selectable_text = SelectableArea::new(
                    self.mcp_content_selection_handle.clone(),
                    #[allow(clippy::unwrap_used)]
                    move |selection_args, _, _| {
                        *mcp_selected_text.write().unwrap() = selection_args.selection;
                    },
                    text_element,
                )
                .on_selection_updated(|ctx, _| {
                    ctx.dispatch_typed_action(RequestedCommandViewAction::SelectText);
                })
                .finish();
                content.add_child(
                    Container::new(selectable_text)
                        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                        .with_vertical_padding(REQUESTED_COMMAND_BODY_VERTICAL_PADDING)
                        .with_background(theme.background())
                        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(7.)))
                        .finish(),
                );
            }
        }

        if let Some(footer) = self.maybe_render_footer(app) {
            content.add_child(Clipped::new(footer).finish());
        }

        let border_color = if action_status
            .as_ref()
            .is_some_and(|status| status.is_blocked())
        {
            theme.accent()
        } else {
            theme.surface_2()
        };

        // If the requested command state is completed and input isn't pinned to the top, we're
        // going to have a regular block directly below this one with the output of the executed
        // command. Since we can't control the top padding of the AI block that comes _after_ the
        // subsequent regular block, we'll simply need to eliminate the bottom margin on this block
        // and have the next AI block take care of the vertical spacing. Moreover, having a non-zero
        // bottom margin while expanded will cause the body to look disconnected from the header.
        let should_remove_bottom_margin = is_rendered_above_expanded_command_block
            || ((self.action_type.is_requested_command() || self.action_type.is_mcp_tool())
                && is_last_output_message_in_output
                && (BlocklistAIHistoryModel::as_ref(app)
                    .conversation(&self.client_ids.conversation_id)
                    .is_some_and(|conversation| {
                        // Prevents an issue where the bottom margin is removed when the requested command is the last message and we cancel it.
                        // We want to keep the margin in this case so that there's visual separation between the cancelled command and footer.
                        conversation.status() != &ConversationStatus::Cancelled
                            // If the next exchange doesn't contain a user query, don't render bottom margin for continuity.
                            && conversation
                                .root_task_exchanges()
                                .skip_while(|exchange| {
                                    exchange.id != self.client_ids.client_exchange_id
                                })
                                .nth(1)
                                .is_some_and(|exchange| {
                                    !exchange
                                        .input
                                        .iter()
                                        .any(|input| input.display_query().is_some())
                                })
                    }))
                && !is_input_pinned_to_top);

        let container = Container::new(content.finish())
            .with_margin_left(if is_rendered_above_expanded_command_block {
                0.
            } else if action_status.is_some_and(|status| status.is_blocked()) {
                CONTENT_HORIZONTAL_PADDING
            } else {
                CONTENT_HORIZONTAL_PADDING + icon_size(app) + 16.
            })
            .with_margin_right(if is_rendered_above_expanded_command_block {
                0.
            } else {
                CONTENT_HORIZONTAL_PADDING
            })
            .with_margin_bottom(if should_remove_bottom_margin {
                0.
            } else {
                CONTENT_ITEM_VERTICAL_MARGIN
            })
            .with_corner_radius(if is_rendered_above_expanded_command_block {
                CornerRadius::with_top(Radius::Pixels(8.))
            } else {
                CornerRadius::with_all(Radius::Pixels(8.))
            })
            .with_border(Border::all(1.).with_border_fill(border_color))
            .finish();

        let mut root_stack = Stack::new();
        root_stack.add_child(container);

        if self.is_accept_split_button_menu_open {
            root_stack.add_positioned_child(
                ChildView::new(&self.accept_split_button_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    Self::get_position_id_for_accept_split_button(&self.position_id_prefix),
                    vec2f(0., 8.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        if self.mcp_context_menu_open
            && let Some(anchor_id) = &self.mcp_context_menu_anchor_id
        {
            root_stack.add_positioned_child(
                Dismiss::new(ChildView::new(&self.mcp_context_menu).finish())
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(RequestedCommandViewAction::CloseMcpContextMenu);
                    })
                    .prevent_interaction_with_other_elements()
                    .finish(),
                OffsetPositioning::offset_from_save_position_element(
                    anchor_id.as_str(),
                    vec2f(0., 0.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        root_stack.finish()
    }

    fn keymap_context(&self, app: &AppContext) -> Context {
        let mut context = Self::default_keymap_context();
        if self.is_waiting_for_user_confirmation(app) {
            context.set.insert(REQUESTED_ACTION_BLOCKED_KEYMAP_CONTEXT);
        }

        if self.is_editing {
            context.set.insert(EDIT_MODE_OPEN_KEYMAP_CONTEXT);
        }
        context
    }
}

impl TypedActionView for RequestedCommandView {
    type Action = RequestedCommandViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RequestedCommandViewAction::Accept => {
                self.commit_editor_contents(ctx);
                ctx.emit(RequestedCommandViewEvent::Accepted);
            }
            RequestedCommandViewAction::AcceptAndAutoExecute => {
                self.commit_editor_contents(ctx);
                ctx.emit(RequestedCommandViewEvent::Accepted);
                ctx.emit(RequestedCommandViewEvent::EnableAutoexecuteMode);
            }
            RequestedCommandViewAction::ToggleAcceptMenu => {
                self.toggle_accept_split_button_menu(ctx)
            }
            RequestedCommandViewAction::Reject => ctx.emit(RequestedCommandViewEvent::Rejected),
            RequestedCommandViewAction::OpenEditMode => self.open_edit_mode(ctx),
            RequestedCommandViewAction::CloseEditMode => self.close_edit_mode(ctx),
            RequestedCommandViewAction::FocusEditor => {
                if let Some(editor) = &self.editor {
                    ctx.focus(editor);
                }
            }
            RequestedCommandViewAction::ToggleExpanded => {
                self.set_is_header_expanded(!self.is_header_expanded, ctx)
            }
            RequestedCommandViewAction::OpenActiveAgentProfileEditor => {
                ctx.emit(RequestedCommandViewEvent::OpenActiveAgentProfileEditor)
            }
            RequestedCommandViewAction::SelectText => {
                ctx.emit(RequestedCommandViewEvent::TextSelected);
            }
            RequestedCommandViewAction::ToggleJsonNode { path, tree } => {
                // A node's depth in the tree always equals its path length: the root
                // has an empty path (depth 0) and each level down adds one segment.
                let depth = path.len();
                match tree {
                    McpTree::Request => self.mcp_request_tree_state.toggle(path, depth),
                    McpTree::Response => self.mcp_response_tree_state.toggle(path, depth),
                }
                ctx.notify();
            }
            RequestedCommandViewAction::ToggleJsonString { path, tree } => {
                match tree {
                    McpTree::Request => self.mcp_request_tree_state.toggle_string(path),
                    McpTree::Response => self.mcp_response_tree_state.toggle_string(path),
                }
                ctx.notify();
            }
            RequestedCommandViewAction::CopyJsonToClipboard { text } => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()));
            }
            RequestedCommandViewAction::ShowMcpContextMenu {
                json_text,
                anchor_id,
            } => {
                // Determine whether the Copy item should be enabled based on whether
                // there is currently a non-empty text selection in the MCP section.
                #[allow(clippy::unwrap_used)]
                let has_selection = self
                    .mcp_content_selected_text
                    .read()
                    .unwrap()
                    .as_deref()
                    .is_some_and(|t| !t.is_empty());

                let copy_item: MenuItem<RequestedCommandViewAction> = MenuItemFields::new("Copy")
                    .with_on_select_action(RequestedCommandViewAction::CopyMcpSelection)
                    .with_disabled(!has_selection)
                    .into_item();

                let json_for_menu = json_text.clone();
                let copy_json_item: MenuItem<RequestedCommandViewAction> =
                    MenuItemFields::new("Copy JSON")
                        .with_on_select_action(RequestedCommandViewAction::CopyJsonToClipboard {
                            text: json_for_menu,
                        })
                        .into_item();

                self.mcp_context_menu.update(ctx, move |menu, ctx| {
                    menu.set_items(vec![copy_item, copy_json_item], ctx);
                });
                self.mcp_context_menu_anchor_id = Some(anchor_id.clone());
                self.mcp_context_menu_open = true;
                ctx.notify();
            }
            RequestedCommandViewAction::CopyMcpSelection => {
                #[allow(clippy::unwrap_used)]
                if let Some(text) = self
                    .mcp_content_selected_text
                    .read()
                    .unwrap()
                    .clone()
                    .filter(|t| !t.is_empty())
                {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                }
            }
            RequestedCommandViewAction::CloseMcpContextMenu => {
                self.mcp_context_menu_open = false;
                ctx.notify();
            }
        }
    }
}

/// Convenience wrapper around a [`RequestedCommandView`].
pub struct RequestedCommand {
    pub view: ViewHandle<RequestedCommandView>,
}

impl RequestedCommand {
    pub fn render(&self) -> Box<dyn Element> {
        ChildView::new(&self.view).finish()
    }

    pub fn force_expand(&self, ctx: &mut impl UpdateView) {
        self.view.update(ctx, |command, ctx| {
            command.set_is_header_expanded(true, ctx);
        })
    }

    pub fn force_collapse(&self, ctx: &mut impl UpdateView) {
        self.view.update(ctx, |command, ctx| {
            command.set_is_header_expanded(false, ctx);
        })
    }
}

/// Formats the command text to truncate at the first newline and add an ellipsis.
/// Extracted for unit testing.
pub fn format_command_text(text: &str) -> String {
    if let Some(newline_pos) = text.find('\n') {
        let first_line = &text[..newline_pos];
        if text[newline_pos..].trim().is_empty() {
            first_line.to_string()
        } else {
            format!("{first_line}…")
        }
    } else {
        text.to_string()
    }
}

#[cfg(test)]
#[path = "requested_command_tests.rs"]
mod tests;
