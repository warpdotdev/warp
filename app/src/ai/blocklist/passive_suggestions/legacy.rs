use super::static_prompt_suggestions::static_suggested_query;
use crate::ai::blocklist::controller::{
    response_stream::ResponseStreamId, BlocklistAIController, BlocklistAIControllerEvent,
};
use crate::settings::AISettings;
use crate::terminal::event::{BlockType, UserBlockCompleted};
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::terminal_model::TerminalModel;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::view::AgentModePromptSuggestion;
use crate::workspaces::user_workspaces::UserWorkspaces;
use parking_lot::FairMutex;
use std::sync::Arc;
use warp_core::features::FeatureFlag;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

#[derive(Clone, Debug)]
pub enum PassiveSuggestionsEvent {
    PromptSuggestionsGenerated {
        prompt_suggestion: AgentModePromptSuggestion,
        block_id: BlockId,
    },
}

pub struct PassiveSuggestionsModel {
    _active_session: ModelHandle<ActiveSession>,
    _terminal_model: Arc<FairMutex<TerminalModel>>,
    _ai_controller: ModelHandle<BlocklistAIController>,
    _terminal_view_id: EntityId,
}

impl PassiveSuggestionsModel {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ai_controller: ModelHandle<BlocklistAIController>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(model_event_dispatcher, |me, event, ctx| {
            me.handle_model_event(event, ctx);
        });
        ctx.subscribe_to_model(&ai_controller, |me, event, ctx| {
            me.handle_controller_event(event, ctx);
        });

        Self {
            _active_session: active_session,
            _terminal_model: terminal_model,
            _ai_controller: ai_controller,
            _terminal_view_id: terminal_view_id,
        }
    }

    pub fn is_passive_code_diff_being_generated(&self) -> bool {
        false
    }

    pub fn abort_pending_requests(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<ResponseStreamId> {
        let _ = ctx;
        Vec::new()
    }

    fn handle_model_event(&mut self, event: &ModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            ModelEvent::AfterBlockStarted { .. } => {
                self.abort_pending_requests(ctx);
            }
            ModelEvent::AfterBlockCompleted(after_block_completed_event) => {
                if FeatureFlag::PromptSuggestionsViaMAA.is_enabled() {
                    self.abort_pending_requests(ctx);
                    return;
                }
                let BlockType::User(block_completed) = &after_block_completed_event.block_type
                else {
                    return;
                };
                self.handle_user_block_completed(block_completed, ctx);
            }
            _ => {}
        }
    }

    fn handle_controller_event(
        &mut self,
        event: &BlocklistAIControllerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let BlocklistAIControllerEvent::SentRequest { .. } = event {
            self.abort_pending_requests(ctx);
        }
    }

    fn handle_user_block_completed(
        &mut self,
        block_completed: &UserBlockCompleted,
        ctx: &mut ModelContext<Self>,
    ) {
        if block_completed.was_part_of_agent_interaction {
            return;
        }

        self.abort_pending_requests(ctx);

        if should_generate_prompt_suggestions(block_completed, ctx) {
            self.generate_prompt_suggestions(block_completed.clone(), ctx);
        }
    }

    fn generate_prompt_suggestions(
        &mut self,
        block_completed: UserBlockCompleted,
        ctx: &mut ModelContext<Self>,
    ) {
        let block_id = block_completed.serialized_block.id.clone();
        if let Some(suggestion) = fetch_static_prompt_suggestion(&block_completed) {
            ctx.emit(PassiveSuggestionsEvent::PromptSuggestionsGenerated {
                prompt_suggestion: suggestion.clone(),
                block_id: block_id.clone(),
            });
        }
    }
}

impl Entity for PassiveSuggestionsModel {
    type Event = PassiveSuggestionsEvent;
}

fn should_generate_prompt_suggestions(
    block_completed: &UserBlockCompleted,
    ctx: &ModelContext<PassiveSuggestionsModel>,
) -> bool {
    if block_completed.command.trim().is_empty() {
        return false;
    }

    AISettings::as_ref(ctx).is_prompt_suggestions_enabled(ctx)
        && UserWorkspaces::as_ref(ctx).is_prompt_suggestions_toggleable()
}

fn fetch_static_prompt_suggestion(block: &UserBlockCompleted) -> Option<AgentModePromptSuggestion> {
    if !block.serialized_block.exit_code.was_successful() {
        return None;
    }
    static_suggested_query(&block.command).map(AgentModePromptSuggestion::Success)
}
