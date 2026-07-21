//! Permission-capable view for tool calls without bespoke TUI bodies.

use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionType, AIConversationId, BlocklistAIActionModel,
    CancellationReason, NewConversationDecision,
};
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::{AppContext, Entity, EntityId, ModelHandle, TuiView, ViewContext, ViewHandle};

use crate::agent_block_sections::render_fallback_tool_call_section;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_permission_prompt::{
    TuiPermissionPrompt, TuiPermissionPromptEvent, render_permission_card,
};

/// Events emitted to the agent block that owns this tool call.
pub(super) enum TuiGenericToolCallViewEvent {
    /// The action entered or left its blocking state.
    BlockingStateChanged,
    /// The rendered card changed intrinsic height.
    LayoutChanged,
    /// The user replaced the action with new guidance for the agent.
    ReplacementGuidanceSubmitted(String),
}

/// Stateful permission card for blocked actions without a specialized TUI view.
pub(super) struct TuiGenericToolCallView {
    action: AIAgentAction,
    output_streaming: bool,
    action_model: ModelHandle<BlocklistAIActionModel>,
    conversation_id: AIConversationId,
    permission_prompt: Option<ViewHandle<TuiPermissionPrompt>>,
}

impl TuiGenericToolCallView {
    /// Creates a generic action view and installs its blocked-state subscription.
    pub(super) fn new(
        action: AIAgentAction,
        output_streaming: bool,
        action_model: ModelHandle<BlocklistAIActionModel>,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mut view = Self {
            action,
            output_streaming,
            action_model: action_model.clone(),
            conversation_id,
            permission_prompt: None,
        };
        if view.is_blocked(ctx) {
            view.ensure_permission_prompt(ctx);
        }
        ctx.subscribe_to_model(&action_model, |view, _, event, ctx| {
            if event.action_id() != &view.action.id {
                return;
            }
            if matches!(
                event,
                warp::tui_export::BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
            ) {
                view.ensure_permission_prompt(ctx);
            }
            ctx.emit(TuiGenericToolCallViewEvent::BlockingStateChanged);
            view.invalidate_layout(ctx);
        });
        view
    }

    /// Lazily creates the interactive prompt when this action first blocks.
    fn ensure_permission_prompt(&mut self, ctx: &mut ViewContext<Self>) {
        if self.permission_prompt.is_some() {
            return;
        }
        let action_id = self.action.id.clone();
        let action_model = self.action_model.clone();
        let prompt = ctx.add_typed_action_tui_view(move |ctx| {
            TuiPermissionPrompt::new(action_model, action_id, None, ctx)
        });
        ctx.subscribe_to_view(&prompt, |view, _, event, ctx| match event {
            TuiPermissionPromptEvent::AcceptRequested => view.accept(ctx),
            TuiPermissionPromptEvent::ReplacementGuidanceSubmitted(text) => {
                ctx.emit(TuiGenericToolCallViewEvent::ReplacementGuidanceSubmitted(
                    text.clone(),
                ));
            }
            TuiPermissionPromptEvent::RejectRequested => view.reject(ctx),
            TuiPermissionPromptEvent::BlockingStateChanged => {
                ctx.emit(TuiGenericToolCallViewEvent::BlockingStateChanged);
                view.invalidate_layout(ctx);
            }
            TuiPermissionPromptEvent::LayoutChanged => view.invalidate_layout(ctx),
        });
        self.permission_prompt = Some(prompt);
        self.invalidate_layout(ctx);
    }

    /// Returns whether this action is the front-of-queue blocked action.
    fn is_blocked(&self, app: &AppContext) -> bool {
        self.action_model
            .as_ref(app)
            .get_action_status(&self.action.id)
            .is_some_and(|status| status.is_blocked())
    }

    /// Refreshes streamed action arguments without replacing prompt state.
    pub(super) fn update_action(
        &mut self,
        action: AIAgentAction,
        output_streaming: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.action = action;
        self.output_streaming = output_streaming;
        self.invalidate_layout(ctx);
    }

    /// Returns the prompt while it should replace the normal session input.
    pub(super) fn active_permission_prompt(
        &self,
        app: &AppContext,
    ) -> Option<ViewHandle<TuiPermissionPrompt>> {
        self.permission_prompt
            .as_ref()
            .filter(|prompt| prompt.as_ref(app).is_active(app))
            .cloned()
    }

    /// Approves the action, including executor-specific decision channels.
    fn accept(&self, ctx: &mut ViewContext<Self>) {
        let action_id = self.action.id.clone();
        self.action_model.update(ctx, |action_model, ctx| {
            if matches!(
                self.action.action,
                AIAgentActionType::SuggestNewConversation { .. }
            ) {
                action_model
                    .suggest_new_conversation_executor(ctx)
                    .update(ctx, |executor, _| {
                        executor.complete_suggest_new_conversation_action(
                            NewConversationDecision::Accept,
                        );
                    });
            }
            action_model.execute_action(&action_id, self.conversation_id, ctx);
        });
    }

    /// Rejects the action using its executor-specific or standard cancel path.
    fn reject(&self, ctx: &mut ViewContext<Self>) {
        let action_id = self.action.id.clone();
        self.action_model.update(ctx, |action_model, ctx| {
            if matches!(
                self.action.action,
                AIAgentActionType::SuggestNewConversation { .. }
            ) {
                action_model
                    .suggest_new_conversation_executor(ctx)
                    .update(ctx, |executor, _| {
                        executor.complete_suggest_new_conversation_action(
                            NewConversationDecision::Reject,
                        );
                    });
                action_model.execute_action(&action_id, self.conversation_id, ctx);
            } else {
                action_model.cancel_action_with_id(
                    self.conversation_id,
                    &action_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            }
        });
    }

    /// Requests remeasurement by the owning transcript block.
    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiGenericToolCallViewEvent::LayoutChanged);
        ctx.notify();
    }

    /// Builds the user-facing question shown above the action details.
    fn permission_question(&self) -> String {
        match &self.action.action {
            AIAgentActionType::ReadFiles(_) => "Is it OK if I read these files?".to_owned(),
            AIAgentActionType::UploadArtifact(_) => {
                "Is it OK if I upload this artifact?".to_owned()
            }
            AIAgentActionType::SearchCodebase(_) => {
                "Is it OK if I search this codebase?".to_owned()
            }
            AIAgentActionType::Grep { .. } => "Is it OK if I search these files?".to_owned(),
            AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => {
                "Is it OK if I find files matching these patterns?".to_owned()
            }
            AIAgentActionType::CallMCPTool { .. } => "Is it OK if I call this MCP tool?".to_owned(),
            AIAgentActionType::ReadMCPResource { .. } => {
                "Is it OK if I read this MCP resource?".to_owned()
            }
            AIAgentActionType::RequestComputerUse(_) => {
                "Is it OK if I use the computer?".to_owned()
            }
            AIAgentActionType::WriteToLongRunningShellCommand { .. } => {
                "Is it OK if I write this input to the running command?".to_owned()
            }
            AIAgentActionType::SuggestNewConversation { .. } => {
                "Should I start a new conversation?".to_owned()
            }
            AIAgentActionType::TransferShellCommandControlToUser { .. } => {
                "Is it OK if I hand control of the running command to you?".to_owned()
            }
            action => format!("Is it OK if I {}?", action.user_friendly_name()),
        }
    }

    /// Formats the action arguments needed to make an approval decision.
    fn details(&self) -> String {
        match &self.action.action {
            AIAgentActionType::ReadFiles(request) => request
                .locations
                .iter()
                .map(|location| format!("  - {}", location.name))
                .collect::<Vec<_>>()
                .join("\n"),
            AIAgentActionType::UploadArtifact(request) => match &request.description {
                Some(description) => format!("{}\n{}", request.file_path, description),
                None => request.file_path.clone(),
            },
            AIAgentActionType::SearchCodebase(request) => match &request.codebase_path {
                Some(path) => format!("{}\n  in {path}", request.query),
                None => request.query.clone(),
            },
            AIAgentActionType::Grep { queries, path } => {
                format!("{}\n  in {path}", queries.join("\n"))
            }
            AIAgentActionType::FileGlob { patterns, path } => {
                let path = path.as_deref().unwrap_or(".");
                format!("{}\n  in {path}", patterns.join("\n"))
            }
            AIAgentActionType::FileGlobV2 {
                patterns,
                search_dir,
            } => {
                let path = search_dir.as_deref().unwrap_or(".");
                format!("{}\n  in {path}", patterns.join("\n"))
            }
            AIAgentActionType::CallMCPTool { name, input, .. } => {
                let input =
                    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
                if input == "{}" || input == "null" {
                    name.clone()
                } else {
                    format!("{name}\n{input}")
                }
            }
            AIAgentActionType::ReadMCPResource { name, uri, .. } => {
                uri.clone().unwrap_or_else(|| name.clone())
            }
            AIAgentActionType::RequestComputerUse(request) => request.task_summary.clone(),
            AIAgentActionType::WriteToLongRunningShellCommand { input, .. } => {
                String::from_utf8_lossy(input).into_owned()
            }
            AIAgentActionType::SuggestNewConversation { .. } => {
                "Continue the agent's next step in a fresh conversation.".to_owned()
            }
            AIAgentActionType::TransferShellCommandControlToUser { reason } => reason.clone(),
            action => action.user_friendly_name(),
        }
    }

    /// Renders the complete blocked-action card.
    fn render_blocked(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let prompt = self
            .permission_prompt
            .as_ref()
            .expect("blocked generic actions should own a permission prompt");
        render_permission_card(
            prompt,
            self.permission_question(),
            Some(
                TuiText::new(self.details())
                    .with_style(builder.primary_text_style())
                    .finish(),
            ),
            app,
        )
    }
}

#[cfg(test)]
#[path = "tui_generic_tool_call_view_tests.rs"]
mod tests;

impl Entity for TuiGenericToolCallView {
    type Event = TuiGenericToolCallViewEvent;
}

impl TuiView for TuiGenericToolCallView {
    fn ui_name() -> &'static str {
        "TuiGenericToolCallView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.permission_prompt.iter().map(ViewHandle::id).collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action.id);
        if matches!(status, Some(AIActionStatus::Blocked)) {
            self.render_blocked(app)
        } else {
            render_fallback_tool_call_section(
                &self.action,
                status.as_ref(),
                self.output_streaming,
                None,
                app,
            )
        }
    }
}
