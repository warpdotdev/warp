use ai::agent::action_result::{AskUserQuestionAnswerItem, AskUserQuestionResult};
use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{AIAgentActionResultType, AIAgentActionType};
use crate::ai::blocklist::BlocklistAIPermissions;

pub enum AskUserQuestionDecision {
    Completed(Vec<AskUserQuestionAnswerItem>),
    Cancelled,
}

pub struct AskUserQuestionExecutor {
    result_rx: (
        async_channel::Sender<AskUserQuestionDecision>,
        async_channel::Receiver<AskUserQuestionDecision>,
    ),
    terminal_view_id: EntityId,
}

impl AskUserQuestionExecutor {
    pub fn new(terminal_view_id: EntityId) -> Self {
        Self {
            result_rx: async_channel::unbounded(),
            terminal_view_id,
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        !BlocklistAIPermissions::as_ref(ctx).can_ask_user_question(
            &input.conversation_id,
            Some(self.terminal_view_id),
            ctx,
        )
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let questions = match &input.action.action {
            AIAgentActionType::AskUserQuestion { questions } => questions,
            _ => {
                return ActionExecution::InvalidAction;
            }
        };

        if self.should_autoexecute(input, ctx) {
            let question_ids = questions
                .iter()
                .map(|question| question.question_id.clone())
                .collect();
            return ActionExecution::Sync(AIAgentActionResultType::AskUserQuestion(
                AskUserQuestionResult::SkippedByAutoApprove { question_ids },
            ));
        }

        let receiver = self.result_rx.1.clone();
        ActionExecution::new_async(
            async move { receiver.recv().await },
            |result, _ctx| match result {
                Ok(AskUserQuestionDecision::Completed(answers)) => {
                    AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Success {
                        answers,
                    })
                }
                Ok(AskUserQuestionDecision::Cancelled) => {
                    AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Cancelled)
                }
                Err(_) => {
                    AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Cancelled)
                }
            },
        )
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }

    pub fn complete(&self, answers: Vec<AskUserQuestionAnswerItem>) {
        let _ = self
            .result_rx
            .0
            .try_send(AskUserQuestionDecision::Completed(answers));
    }

    pub fn cancel(&self) {
        let _ = self
            .result_rx
            .0
            .try_send(AskUserQuestionDecision::Cancelled);
    }
}

#[cfg(test)]
impl Default for AskUserQuestionExecutor {
    fn default() -> Self {
        Self::new(EntityId::new())
    }
}

impl Entity for AskUserQuestionExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "ask_user_question_tests.rs"]
mod tests;
