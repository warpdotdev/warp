use std::marker::PhantomData;

use anyhow::{anyhow, Result};
use chrono::{Local, TimeDelta};
use history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use session_sharing_protocol::common::ParticipantId;
use warpui::{AppContext, SingletonEntity, View, ViewContext};

use crate::ai::{
    agent::{
        conversation::AIConversationId, AIAgentExchange, AIAgentExchangeId, AIAgentInput,
        AIAgentOutputStatus, FinishedAIAgentOutput, ServerOutputId, Shared,
    },
    blocklist::{
        history_model,
        model::{AIRequestType, PassiveRequestType},
    },
    llms::LLMId,
};

use super::{AIBlockModel, AIBlockOutputStatus, OutputStatusUpdateCallback};

/// Standard [`AIBlock`] impl for live outputs corresponding to an `OutputStream`.
pub struct AIBlockModelImpl<V> {
    exchange_id: AIAgentExchangeId,
    conversation_id: AIConversationId,
    is_restored: bool,
    is_forked: bool,
    _view: PhantomData<V>,
}

impl<V> AIBlockModelImpl<V>
where
    V: View,
{
    pub fn new(
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        is_restored: bool,
        is_forked: bool,
        app: &AppContext,
    ) -> Result<Self> {
        BlocklistAIHistoryModel::as_ref(app)
            .conversation(&conversation_id)
            .ok_or_else(|| {
                anyhow!(
                    "Failed to find agent conversation data for conversation_id: {:?}",
                    conversation_id
                )
            })
            .and_then(|conversation| {
                conversation.exchange_with_id(exchange_id).ok_or_else(|| {
                    anyhow!(
                        "Failed to find agent exchange data for exchange_id: {:?}",
                        exchange_id
                    )
                })
            })
            .map(|_| Self {
                exchange_id,
                conversation_id,
                is_restored,
                is_forked,
                _view: PhantomData,
            })
    }

    fn exchange<'a>(&self, app: &'a AppContext) -> Result<&'a AIAgentExchange> {
        let res = BlocklistAIHistoryModel::as_ref(app)
            .conversation(&self.conversation_id)
            .and_then(|conversation| conversation.exchange_with_id(self.exchange_id));

        // There is no reason this should ever happen in the normal course of a session.
        if let Some(exchange) = res {
            Ok(exchange)
        } else {
            Err(anyhow!(
                "No exchange found for conversation_id: {:?}, exchange_id: {:?}",
                self.conversation_id,
                self.exchange_id
            ))
        }
    }
}

impl<V> AIBlockModel for AIBlockModelImpl<V>
where
    V: View,
{
    type View = V;

    fn is_restored(&self) -> bool {
        self.is_restored
    }

    fn is_forked(&self) -> bool {
        self.is_forked
    }

    fn status(&self, app: &AppContext) -> AIBlockOutputStatus {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation) = history_model.conversation(&self.conversation_id) else {
            return AIBlockOutputStatus::Pending;
        };
        let Some(exchange) = conversation.exchange_with_id(self.exchange_id) else {
            return AIBlockOutputStatus::Pending;
        };
        match &exchange.output_status {
            AIAgentOutputStatus::Streaming { output: None, .. } => AIBlockOutputStatus::Pending,
            AIAgentOutputStatus::Streaming {
                output: Some(output),
                ..
            } => {
                if output.get().messages.is_empty() {
                    AIBlockOutputStatus::Pending
                } else {
                    AIBlockOutputStatus::PartiallyReceived {
                        output: output.get_owned(),
                    }
                }
            }
            AIAgentOutputStatus::Finished {
                finished_output, ..
            } => match finished_output {
                FinishedAIAgentOutput::Success { output } => AIBlockOutputStatus::Complete {
                    output: output.get_owned(),
                },
                FinishedAIAgentOutput::Cancelled { output, reason } => {
                    AIBlockOutputStatus::Cancelled {
                        partial_output: output.as_ref().map(Shared::get_owned),
                        reason: *reason,
                    }
                }
                FinishedAIAgentOutput::Error { error, output } => AIBlockOutputStatus::Failed {
                    partial_output: output.as_ref().map(Shared::get_owned),
                    error: error.clone(),
                },
            },
        }
    }

    fn time_since_request_start(&self, app: &AppContext) -> Option<TimeDelta> {
        let exchange = self.exchange(app);
        match exchange {
            Ok(exchange) => Some(Local::now().signed_duration_since(exchange.start_time)),
            Err(err) => {
                log::error!("Failed to get time since request start. {err}");
                None
            }
        }
    }

    fn base_model<'a>(&'a self, app: &'a AppContext) -> Option<&'a LLMId> {
        let exchange = self.exchange(app);
        match exchange {
            Ok(exchange) => Some(&exchange.model_id),
            Err(err) => {
                log::error!("Failed to get base model. {err}");
                None
            }
        }
    }

    fn inputs_to_render<'a>(&'a self, app: &'a AppContext) -> &'a [AIAgentInput] {
        self.exchange(app)
            .map(|ex| ex.input.as_slice())
            .unwrap_or(&[])
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        Some(self.conversation_id)
    }

    fn exchange_id(&self, _app: &AppContext) -> Option<AIAgentExchangeId> {
        Some(self.exchange_id)
    }

    fn response_initiator(&self, app: &AppContext) -> Option<ParticipantId> {
        self.exchange(app)
            .ok()
            .and_then(|ex| ex.response_initiator.clone())
    }

    fn server_output_id(&self, app: &AppContext) -> Option<ServerOutputId> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let conversation = history_model.conversation(&self.conversation_id)?;
        let exchange = conversation.exchange_with_id(self.exchange_id)?;
        exchange.output_status.server_output_id()
    }

    fn model_id(&self, app: &AppContext) -> Option<LLMId> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let conversation = history_model.conversation(&self.conversation_id)?;
        let exchange = conversation.exchange_with_id(self.exchange_id)?;
        exchange.output_status.model_id()
    }

    fn on_updated_output(
        &self,
        mut callback: OutputStatusUpdateCallback<V>,
        ctx: &mut ViewContext<V>,
    ) {
        let exchange_id = self.exchange_id;
        let conversation_id = self.conversation_id;
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, _, event, ctx| {
            match event {
                BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id: event_exchange_id,
                    conversation_id: event_conversation_id,
                    ..
                } => {
                    if *event_exchange_id == exchange_id {
                        callback(me, ctx);
                    } else if *event_conversation_id == conversation_id {
                        ctx.notify();
                    }
                }
                BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. } => {
                    // Cross-pane orchestration credit rollup: the collapsed
                    // footer pill on an orchestrator's AIBlock derives its
                    // headline number from descendant credits, which the
                    // existing exchange-scoped notify path above doesn't
                    // cover. Trigger a re-render on any usage metadata
                    // change so the pill stays live. Filtering to only
                    // descendants of this block's conversation would
                    // require an O(depth) walk per event for every AI
                    // block in the app; an unconditional notify is cheap
                    // and tracks live status accurately.
                    ctx.notify();
                }
                _ => {}
            }
        });
    }

    fn request_type(&self, app: &AppContext) -> AIRequestType {
        if self
            .exchange(app)
            .map(|exchange| exchange.has_passive_code_diff())
            .unwrap_or(false)
        {
            AIRequestType::Passive(PassiveRequestType::CodeDiff)
        } else if let Some(trigger) = self
            .exchange(app)
            .ok()
            .and_then(|exchange| exchange.passive_suggestion_trigger())
        {
            AIRequestType::from_passive_trigger(trigger)
        } else {
            AIRequestType::Active
        }
    }
}
