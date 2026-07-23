//! TUI view for a `RequestCommandOutput` tool call.
//!
//! The GUI and TUI share the action model and the terminal block that records
//! the command's ground-truth execution state. This view adds only TUI chrome:
//! the existing status-aware tool-call header becomes a collapsed disclosure,
//! and expanding it embeds the same terminal-cell renderer used by top-level
//! shell blocks.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionResultType, AIAgentActionType, AIConversationId,
    BlockId, BlocklistAIActionModel, CancellationReason, RequestCommandOutputResult, TerminalModel,
};
use warpui_core::r#async::Timer;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{Modifier, TuiChildView, TuiElement, TuiFlex, tui_collapsible};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{EditableBinding, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::agent_block_sections::render_fallback_tool_call_section;
use crate::editor_view::{TuiEditorView, TuiEditorViewEvent};
use crate::keybindings::{TUI_BINDING_GROUP, is_tui_owned_binding};
use crate::terminal_block::TerminalBlockElement;
use crate::terminal_use::user_controls_running_command;
use crate::tool_call_labels::{
    CommandBlockState, ResolvedCommandBlock, tool_call_display_state, tool_call_label,
};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_cli_subagent_view::{TuiCLISubagentView, TuiCLISubagentViewEvent};
use crate::tui_permission_prompt::{
    TuiPermissionPrompt, TuiPermissionPromptEvent, render_permission_card,
};
const COMMAND_AUTO_EXPAND_DELAY: Duration = Duration::from_secs(3);
const SHELL_COMMAND_EDITING: &str = "TuiShellCommandEditing";

pub(crate) fn init(app: &mut AppContext) {
    let predicate = id!(TuiShellCommandView::ui_name()) & id!(SHELL_COMMAND_EDITING);
    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            TuiShellCommandViewAction::CancelPermission,
            predicate.clone(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-e",
            TuiShellCommandViewAction::SaveCommandEdit,
            predicate.clone(),
        )
        .with_group(TUI_BINDING_GROUP),
    ]);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:shell-permission:save",
            "Save the edited shell command",
            TuiShellCommandViewAction::SaveCommandEdit,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:shell-permission:save",
            "Save the edited shell command",
            TuiShellCommandViewAction::SaveCommandEdit,
        )
        .with_context_predicate(predicate)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("numpadenter"),
    ]);
    app.register_tui_binding_validator::<TuiShellCommandView>(is_tui_owned_binding);
}

struct ShellCommandViewState {
    collapsed: bool,
    manual_override: bool,
    auto_expand_scheduled: bool,
    auto_expanded: bool,
}

impl ShellCommandViewState {
    fn new_collapsed() -> Self {
        Self {
            collapsed: true,
            manual_override: false,
            auto_expand_scheduled: false,
            auto_expanded: false,
        }
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
        self.manual_override = true;
        self.auto_expanded = false;
    }
}

struct ResolvedShellCommandBlock {
    block_id: BlockId,
    details: ResolvedCommandBlock,
}

/// One stateful `RequestCommandOutput` child view in an agent exchange.
pub(super) struct TuiShellCommandView {
    action: AIAgentAction,
    output_streaming: bool,
    action_model: ModelHandle<BlocklistAIActionModel>,
    conversation_id: AIConversationId,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    command_editor: ViewHandle<TuiEditorView>,
    permission_prompt: ViewHandle<TuiPermissionPrompt>,
    command_was_edited: bool,
    state: ShellCommandViewState,
    command_running: Cell<bool>,
    header_mouse_state: MouseStateHandle,
    cli_subagent_view: Option<ViewHandle<TuiCLISubagentView>>,
}

/// Events emitted to the owning agent block.
pub(super) enum TuiShellCommandViewEvent {
    BlockingStateChanged,
    LayoutChanged,
    ReplacementGuidanceSubmitted(String),
}

/// User interactions handled by the shell-command view.
#[derive(Clone, Debug)]
pub(super) enum TuiShellCommandViewAction {
    CancelPermission,
    SaveCommandEdit,
    ToggleExpanded,
}
impl TuiShellCommandView {
    pub(super) fn new(
        action: AIAgentAction,
        output_streaming: bool,
        action_model: ModelHandle<BlocklistAIActionModel>,
        conversation_id: AIConversationId,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        debug_assert!(matches!(
            &action.action,
            AIAgentActionType::RequestCommandOutput { .. }
        ));
        let command = Self::action_command(&action).to_owned();
        let command_editor = ctx.add_typed_action_tui_view(|ctx| TuiEditorView::multiline(4, ctx));
        command_editor.update(ctx, |editor, ctx| editor.set_text(command, ctx));
        let prompt_action_id = action.id.clone();
        let prompt_action_model = action_model.clone();
        let prompt_command_editor = command_editor.clone();
        let permission_prompt = ctx.add_typed_action_tui_view(move |ctx| {
            TuiPermissionPrompt::new(
                prompt_action_model,
                prompt_action_id,
                Some(prompt_command_editor),
                ctx,
            )
        });
        ctx.subscribe_to_view(&command_editor, |view, _, event, ctx| {
            let TuiEditorViewEvent::Changed(_) = event;
            view.command_was_edited = true;
            view.permission_prompt
                .update(ctx, |prompt, ctx| prompt.set_body_error(None, ctx));
            view.invalidate_layout(ctx);
        });
        ctx.subscribe_to_view(&permission_prompt, |view, _, event, ctx| match event {
            TuiPermissionPromptEvent::AcceptRequested => view.accept(ctx),
            TuiPermissionPromptEvent::ReplacementGuidanceSubmitted(text) => {
                ctx.emit(TuiShellCommandViewEvent::ReplacementGuidanceSubmitted(
                    text.clone(),
                ));
            }
            TuiPermissionPromptEvent::RejectRequested => view.reject(ctx),
            TuiPermissionPromptEvent::BlockingStateChanged => {
                ctx.emit(TuiShellCommandViewEvent::BlockingStateChanged);
                view.invalidate_layout(ctx);
            }
            TuiPermissionPromptEvent::LayoutChanged => view.invalidate_layout(ctx),
        });
        Self {
            action,
            output_streaming,
            action_model,
            conversation_id,
            terminal_model,
            command_editor,
            permission_prompt,
            command_was_edited: false,
            state: ShellCommandViewState::new_collapsed(),
            command_running: Cell::new(false),
            header_mouse_state: MouseStateHandle::default(),
            cli_subagent_view: None,
        }
    }

    /// Refreshes streamed action arguments without replacing interaction state.
    pub(super) fn update_action(
        &mut self,
        action: AIAgentAction,
        output_streaming: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.command_was_edited && !self.command_editor.as_ref(ctx).is_focused() {
            let command = Self::action_command(&action).to_owned();
            self.command_editor
                .update(ctx, |editor, ctx| editor.set_text(command, ctx));
        }
        self.action = action;
        self.output_streaming = output_streaming;
        self.invalidate_layout(ctx);
    }

    pub(super) fn active_permission_prompt(
        &self,
        app: &AppContext,
    ) -> Option<ViewHandle<TuiPermissionPrompt>> {
        self.permission_prompt
            .as_ref(app)
            .is_active(app)
            .then(|| self.permission_prompt.clone())
    }

    fn action_command(action: &AIAgentAction) -> &str {
        match &action.action {
            AIAgentActionType::RequestCommandOutput { command, .. } => command,
            _ => "",
        }
    }

    fn save_command_edit(&self, ctx: &mut ViewContext<Self>) {
        if !self.command_editor.as_ref(ctx).is_focused() {
            return;
        }
        self.permission_prompt
            .update(ctx, |prompt, ctx| prompt.restore_options_focus(ctx));
        self.invalidate_layout(ctx);
    }

    fn accept(&mut self, ctx: &mut ViewContext<Self>) {
        let command = self.command_editor.as_ref(ctx).text(ctx);
        if command.trim().is_empty() {
            self.permission_prompt.update(ctx, |prompt, ctx| {
                prompt.set_body_error(Some("Enter a command to continue.".to_owned()), ctx);
            });
            self.invalidate_layout(ctx);
            return;
        }
        let action_id = self.action.id.clone();
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.handle_requested_command_accepted(&action_id, command, ctx);
        });
    }

    fn reject(&self, ctx: &mut ViewContext<Self>) {
        let action_id = self.action.id.clone();
        self.action_model.update(ctx, |action_model, ctx| {
            action_model.cancel_action_with_id(
                self.conversation_id,
                &action_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
    }

    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiShellCommandViewEvent::LayoutChanged);
        ctx.notify();
    }

    fn render_blocked(&self, app: &AppContext) -> Box<dyn TuiElement> {
        render_permission_card(
            &self.permission_prompt,
            "Is it OK if I run this command and read the output?",
            None,
            app,
        )
    }

    pub(super) fn set_cli_subagent_view(
        &mut self,
        view: Option<ViewHandle<TuiCLISubagentView>>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(view) = view.as_ref() {
            ctx.subscribe_to_view(view, |_, _, event, ctx| match event {
                TuiCLISubagentViewEvent::LayoutChanged => {
                    ctx.emit(TuiShellCommandViewEvent::LayoutChanged);
                    ctx.notify();
                }
            });
            if !self.state.manual_override {
                self.state.collapsed = false;
                self.state.auto_expanded = true;
            }
        }
        if view.is_none() && self.state.auto_expanded && !self.state.manual_override {
            self.state.collapsed = true;
            self.state.auto_expanded = false;
        }
        self.cli_subagent_view = view;
        ctx.emit(TuiShellCommandViewEvent::LayoutChanged);
        ctx.notify();
    }

    pub(super) fn schedule_auto_expand(&mut self, ctx: &mut ViewContext<Self>) {
        if self.state.auto_expand_scheduled || self.state.manual_override {
            return;
        }
        self.state.auto_expand_scheduled = true;
        ctx.spawn(Timer::after(COMMAND_AUTO_EXPAND_DELAY), |view, _, ctx| {
            view.state.auto_expand_scheduled = false;
            if view.state.manual_override {
                return;
            }
            let is_running = view
                .terminal_model
                .lock()
                .block_list()
                .block_for_ai_action_id(&view.action.id)
                .is_some_and(|block| !block.finished());
            if is_running {
                view.state.collapsed = false;
                view.state.auto_expanded = true;
                ctx.emit(TuiShellCommandViewEvent::LayoutChanged);
                ctx.notify();
            }
        });
    }
    /// Whether expanded command output can still grow between layout events.
    pub(super) fn needs_continuous_height_measurement(&self) -> bool {
        !self.state.is_collapsed() && self.command_running.get()
    }
    pub(super) fn is_expanded(&self) -> bool {
        !self.state.is_collapsed()
    }

    /// Resolves the shared terminal block exactly as the GUI requested-command
    /// view does: first by agent action metadata, then by a long-running
    /// snapshot's block ID for restored/view-only cases.
    fn resolved_block(&self, status: Option<&AIActionStatus>) -> Option<ResolvedShellCommandBlock> {
        let snapshot_block_id = match status
            .and_then(AIActionStatus::finished_result)
            .map(|result| &result.result)
        {
            Some(AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::LongRunningCommandSnapshot { block_id, .. },
            )) => Some(block_id),
            _ => None,
        };
        let model = self.terminal_model.lock();
        let block_list = model.block_list();
        let block = block_list
            .block_for_ai_action_id(&self.action.id)
            .or_else(|| snapshot_block_id.and_then(|id| block_list.block_with_id(id)))?;
        let command = block
            .command_with_secrets_obfuscated(false)
            .trim()
            .to_owned();
        let state = if block.finished() {
            CommandBlockState::Finished {
                exit_code: block.exit_code(),
            }
        } else {
            CommandBlockState::Running
        };
        Some(ResolvedShellCommandBlock {
            block_id: block.id().clone(),
            details: ResolvedCommandBlock {
                command: (!command.is_empty()).then_some(command),
                state,
            },
        })
    }

    fn user_controls_command(&self) -> bool {
        self.terminal_model
            .lock()
            .block_list()
            .block_for_ai_action_id(&self.action.id)
            .is_some_and(user_controls_running_command)
    }
}

impl Entity for TuiShellCommandView {
    type Event = TuiShellCommandViewEvent;
}

impl TuiView for TuiShellCommandView {
    fn ui_name() -> &'static str {
        "TuiShellCommandView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        let mut ids = vec![self.permission_prompt.id()];
        ids.extend(self.cli_subagent_view.iter().map(|view| view.id()));
        ids
    }

    fn keymap_context(&self, app: &AppContext) -> warpui_core::keymap::Context {
        let mut context = Self::default_keymap_context();
        let blocked = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action.id)
            .is_some_and(|status| status.is_blocked());
        if blocked
            && self
                .permission_prompt
                .as_ref(app)
                .body_editor_is_focused(app)
        {
            context.set.insert(SHELL_COMMAND_EDITING);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action.id);
        if matches!(status, Some(AIActionStatus::Blocked)) {
            self.command_running.set(false);
            return self.render_blocked(app);
        }
        let Some(block) = self.resolved_block(status.as_ref()) else {
            self.command_running.set(false);
            return render_fallback_tool_call_section(
                &self.action,
                status.as_ref(),
                self.output_streaming,
                None,
                app,
            );
        };
        self.command_running
            .set(matches!(block.details.state, CommandBlockState::Running));

        let builder = TuiUiBuilder::from_app(app);
        let display_state =
            tool_call_display_state(status.as_ref(), false, Some(block.details.state));
        let glyph_style = display_state.glyph_style(&builder);
        let mut label_style = display_state.label_style(&builder);
        if self.header_mouse_state.lock().unwrap().is_hovered() {
            label_style = label_style.add_modifier(Modifier::BOLD);
        }
        let collapsed = self.state.is_collapsed() && !self.user_controls_command();
        let label = tool_call_label(&self.action, status.as_ref(), false, Some(&block.details));
        let header_spans = vec![
            (format!("{} ", display_state.glyph()), glyph_style),
            (format!("{label} "), label_style),
        ];

        let terminal_model = self.terminal_model.clone();
        let block_id = block.block_id;
        let command = tui_collapsible(
            collapsed,
            header_spans,
            label_style,
            self.header_mouse_state.clone(),
            move || TerminalBlockElement::content(terminal_model, block_id).finish(),
            move |event_ctx, _app| {
                event_ctx.dispatch_typed_action(TuiShellCommandViewAction::ToggleExpanded);
            },
        );
        if let Some(view) = self.cli_subagent_view.as_ref() {
            TuiFlex::column()
                .child(command)
                .child(TuiChildView::new(view).finish())
                .finish()
        } else {
            command
        }
    }
}
impl TypedActionView for TuiShellCommandView {
    type Action = TuiShellCommandViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiShellCommandViewAction::CancelPermission => self.reject(ctx),
            TuiShellCommandViewAction::SaveCommandEdit => self.save_command_edit(ctx),
            TuiShellCommandViewAction::ToggleExpanded => {
                if self.user_controls_command() {
                    return;
                }
                self.state.toggle();
                ctx.emit(TuiShellCommandViewEvent::LayoutChanged);
                ctx.notify();
            }
        }
    }
}
#[cfg(test)]
#[path = "tui_shell_command_view_tests.rs"]
mod tests;
