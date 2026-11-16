use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{Entity, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, SuggestNewConversationResult,
};

/// Whether the client accepted or rejected the new conversation. We make this a separate type from
/// `SuggestNewConversationResult` for more ergonomic threading of the message_id through
/// the various layers of action handling.
pub enum NewConversationDecision {
    Accept,
    Reject,
}

pub struct SuggestNewConversationExecutor {
    suggest_new_conversation_result_rx: (
        async_channel::Sender<NewConversationDecision>,
        async_channel::Receiver<NewConversationDecision>,
    ),
}

impl SuggestNewConversationExecutor {
    pub fn new() -> Self {
        Self {
            suggest_new_conversation_result_rx: async_channel::unbounded(),
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        false
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> + use<> {
        let AIAgentAction {
            action: AIAgentActionType::SuggestNewConversation { message_id },
            ..
        } = input.action
        else {
            return ActionExecution::InvalidAction;
        };

        let message_id = message_id.clone();
        let receiver = self.suggest_new_conversation_result_rx.clone().1;
        ActionExecution::new_async(async move { receiver.recv().await }, move |result, _ctx| {
            match result {
                Ok(NewConversationDecision::Accept) => {
                    AIAgentActionResultType::SuggestNewConversation(
                        SuggestNewConversationResult::Accepted { message_id },
                    )
                }
                Ok(NewConversationDecision::Reject) => {
                    AIAgentActionResultType::SuggestNewConversation(
                        SuggestNewConversationResult::Rejected,
                    )
                }
                Err(_) => AIAgentActionResultType::SuggestNewConversation(
                    SuggestNewConversationResult::Cancelled,
                ),
            }
        })
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    pub fn complete_suggest_new_conversation_action(&self, decision: NewConversationDecision) {
        let _ = self.suggest_new_conversation_result_rx.0.try_send(decision);
    }
}

impl Default for SuggestNewConversationExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for SuggestNewConversationExecutor {
    type Event = ();
}
