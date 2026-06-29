use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use crate::{
    ai::{
        agent::{
            conversation::AIConversation, AIAgentAction, AIAgentActionId, AIAgentActionType,
            AIAgentExchange, AIAgentInput, AIAgentOutputMessageType, ServerOutputId,
            SummarizationType,
        },
        blocklist::BlocklistAIActionModel,
    },
    BlocklistAIHistoryModel,
};

use super::AIBlockModel;

fn is_visible_non_passive_exchange(
    conversation: &AIConversation,
    exchange: &AIAgentExchange,
) -> bool {
    !exchange.has_passive_request() && !conversation.is_exchange_hidden(exchange.id)
}

fn shares_server_output_id(
    exchange: &AIAgentExchange,
    server_output_id: &Option<ServerOutputId>,
) -> bool {
    server_output_id.as_ref().is_some_and(|server_output_id| {
        exchange
            .output_status
            .server_output_id()
            .as_ref()
            .is_some_and(|exchange_server_output_id| exchange_server_output_id == server_output_id)
    })
}

fn starts_visible_user_turn(conversation: &AIConversation, exchange: &AIAgentExchange) -> bool {
    exchange.has_user_query() && is_visible_non_passive_exchange(conversation, exchange)
}

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
    fn is_in_latest_visible_turn(&self, app: &AppContext) -> bool;

    fn is_last_visible_exchange_in_current_turn(&self, app: &AppContext) -> bool;

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

    fn is_in_latest_visible_turn(&self, app: &AppContext) -> bool {
        let Some(current_exchange_id) = self.exchange_id(app) else {
            return false;
        };
        let Some(conversation) = self.conversation(app) else {
            return false;
        };
        let Some(latest_exchange) = conversation.latest_visible_exchange() else {
            return false;
        };

        if latest_exchange.id == current_exchange_id {
            return true;
        }

        let Some(current_exchange) = conversation.exchange_with_id(current_exchange_id) else {
            return false;
        };
        let current_server_output_id = current_exchange.output_status.server_output_id();

        // Server actions can split a single assistant response into multiple root-task
        // exchanges, and shared-session reconstruction can attach the original user query to
        // those continuation exchanges. Prefer the server output ID when it is available so
        // those continuations still count as the same visible turn.
        if current_server_output_id.is_some()
            && shares_server_output_id(latest_exchange, &current_server_output_id)
        {
            return true;
        }

        let mut previous_server_output_id = current_server_output_id;
        let mut found_current_exchange = false;
        for exchange in conversation.root_task_exchanges() {
            if exchange.id == current_exchange_id {
                found_current_exchange = true;
                continue;
            }
            if !found_current_exchange {
                continue;
            }

            let is_same_server_response =
                shares_server_output_id(exchange, &previous_server_output_id);
            if !is_same_server_response && starts_visible_user_turn(conversation, exchange) {
                return false;
            }

            if exchange.id == latest_exchange.id {
                return true;
            }

            if let Some(server_output_id) = exchange.output_status.server_output_id() {
                previous_server_output_id = Some(server_output_id);
            }
        }

        false
    }
    fn is_last_visible_exchange_in_current_turn(&self, app: &AppContext) -> bool {
        let Some(current_exchange_id) = self.exchange_id(app) else {
            return true;
        };
        let Some(conversation) = self.conversation(app) else {
            return true;
        };
        let mut previous_server_output_id = conversation
            .exchange_with_id(current_exchange_id)
            .and_then(|exchange| exchange.output_status.server_output_id());

        let mut found_current_exchange = false;
        for exchange in conversation.root_task_exchanges() {
            if exchange.id == current_exchange_id {
                found_current_exchange = true;
                continue;
            }
            if !found_current_exchange {
                continue;
            }

            let is_visible_exchange = is_visible_non_passive_exchange(conversation, exchange);
            let is_same_server_response =
                shares_server_output_id(exchange, &previous_server_output_id);

            // A new visible user query starts the next turn, so later exchanges belong to that
            // turn. Same-response continuations are handled above because some restored/shared
            // exchanges can repeat the original user query.
            if !is_same_server_response && starts_visible_user_turn(conversation, exchange) {
                break;
            }

            if is_visible_exchange {
                return false;
            }

            if let Some(server_output_id) = exchange.output_status.server_output_id() {
                previous_server_output_id = Some(server_output_id);
            }
        }

        true
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
