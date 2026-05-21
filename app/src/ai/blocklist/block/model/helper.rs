use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use crate::{
    ai::{
        agent::{
            conversation::AIConversation, AIAgentAction, AIAgentActionId, AIAgentActionType,
            AIAgentInput, AIAgentOutputMessageType, SummarizationType,
        },
        blocklist::BlocklistAIActionModel,
    },
    BlocklistAIHistoryModel,
};

use super::AIBlockModel;

// Helper methods for accessing data on an impl of `AIBlockModel`.
//
// These are defined within a separate trait rather than default implementations of `AIBlockModel`
// so implementations cannot errantly override them.
pub trait AIBlockModelHelper {
    fn is_first_action_in_output(&self, action_id: &AIAgentActionId, app: &AppContext) -> bool;
    fn conversation<'a>(&self, app: &'a AppContext) -> Option<&'a AIConversation>;

    fn contains_static_prompt_suggestion_input(&self, app: &AppContext) -> bool;

    fn contains_create_document_action(&self, app: &AppContext) -> bool;

    fn contains_update_document_action(&self, app: &AppContext) -> bool;

    fn is_latest_visible_exchange_in_root_task(&self, app: &AppContext) -> bool;

    fn is_latest_exchange_in_terminal_pane(
        &self,
        terminal_view_id: EntityId,
        app: &AppContext,
    ) -> bool;

    fn is_conversation_summarization_active(&self, app: &AppContext) -> bool;

    fn blocked_action(
        &self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        app: &AppContext,
    ) -> Option<AIAgentAction>;
}

impl<T: ?Sized + AIBlockModel> AIBlockModelHelper for T {
    fn is_first_action_in_output(&self, action_id: &AIAgentActionId, app: &AppContext) -> bool {
        self.status(app).output_to_render().is_some_and(|output| {
            output
                .get()
                .actions()
                .next()
                .is_some_and(|action| action.id == *action_id)
        })
    }

    fn conversation<'a>(&self, app: &'a AppContext) -> Option<&'a AIConversation> {
        self.conversation_id(app)
            .and_then(|id| BlocklistAIHistoryModel::as_ref(app).conversation(&id))
    }

    fn contains_static_prompt_suggestion_input(&self, app: &AppContext) -> bool {
        self.inputs_to_render(app)
                .iter()
                .any(|input| matches!(input, AIAgentInput::UserQuery { static_query_type, .. } if static_query_type .is_some()))
    }

    fn contains_create_document_action(&self, app: &AppContext) -> bool {
        if let Some(output) = self.status(app).output_to_render() {
            let output = output.get();
            output.messages.iter().any(|m| {
                matches!(
                    m.message,
                    AIAgentOutputMessageType::Action(AIAgentAction {
                        action: AIAgentActionType::CreateDocuments { .. },
                        ..
                    })
                )
            })
        } else {
            false
        }
    }

    fn contains_update_document_action(&self, app: &AppContext) -> bool {
        if let Some(output) = self.status(app).output_to_render() {
            let output = output.get();
            output.messages.iter().any(|m| {
                matches!(
                    m.message,
                    AIAgentOutputMessageType::Action(AIAgentAction {
                        action: AIAgentActionType::EditDocuments { .. },
                        ..
                    })
                )
            })
        } else {
            false
        }
    }

    fn is_latest_visible_exchange_in_root_task(&self, app: &AppContext) -> bool {
        self.conversation(app).is_some_and(|conversation| {
            match (
                conversation.latest_visible_exchange(),
                self.exchange_id(app),
            ) {
                (Some(latest_exchange), Some(id)) => latest_exchange.id == id,
                _ => false,
            }
        })
    }

    fn is_latest_exchange_in_terminal_pane(
        &self,
        terminal_view_id: EntityId,
        app: &AppContext,
    ) -> bool {
        match (
            BlocklistAIHistoryModel::as_ref(app)
                .latest_exchange_across_all_conversations(terminal_view_id),
            self.exchange_id(app),
        ) {
            (Some(latest_exchange), Some(id)) => latest_exchange.id == id,
            _ => false,
        }
    }

    fn is_conversation_summarization_active(&self, app: &AppContext) -> bool {
        let Some(output) = self.status(app).output_to_render() else {
            return false;
        };
        let output = output.get();
        output.messages.last().is_some_and(|m| {
            matches!(
                m.message,
                crate::ai::agent::AIAgentOutputMessageType::Summarization {
                    finished_duration: None,
                    summarization_type: SummarizationType::ConversationSummary,
                    ..
                }
            )
        })
    }

    fn blocked_action(
        &self,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        app: &AppContext,
    ) -> Option<AIAgentAction> {
        let output = self.status(app).output_to_render()?;
        let output = output.get();
        output.messages.iter().find_map(|message| {
            if let AIAgentOutputMessageType::Action(action) = &message.message {
                if let Some(status) = action_model.as_ref(app).get_action_status(&action.id) {
                    return status.is_blocked().then_some(action.clone());
                }
            }
            None
        })
    }
}
