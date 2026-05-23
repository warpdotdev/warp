mod helper;
mod model_impl;

use chrono::TimeDelta;
pub use helper::AIBlockModelHelper;
pub use model_impl::*;
use session_sharing_protocol::common::ParticipantId;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, ViewContext};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentExchangeId, AIAgentInput, AIAgentOutput, CancellationReason, PassiveSuggestionTrigger,
    PassiveSuggestionTriggerType, RenderableAIError, ServerOutputId, Shared,
};
use crate::ai::llms::LLMId;

#[derive(Debug, Clone, Copy)]
pub enum PassiveRequestType {
    UnitTestSuggestion,
    CodeDiff,
    PassiveSuggestion(PassiveSuggestionTriggerType),
}

/// The type of request that triggered the AI block.
#[derive(Default, Debug, Clone, Copy)]
pub enum AIRequestType {
    #[default]
    Active,
    Passive(PassiveRequestType),
}

impl AIRequestType {
    pub fn from_passive_trigger(trigger: &PassiveSuggestionTrigger) -> Self {
        match trigger {
            PassiveSuggestionTrigger::CommandRun | PassiveSuggestionTrigger::FilesChanged => {
                AIRequestType::Passive(PassiveRequestType::UnitTestSuggestion)
            }
            _ => AIRequestType::Passive(PassiveRequestType::PassiveSuggestion(trigger.into())),
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, AIRequestType::Active)
    }

    pub fn is_passive(&self) -> bool {
        matches!(self, AIRequestType::Passive(_))
    }

    pub fn is_passive_code_diff(&self) -> bool {
        matches!(self, AIRequestType::Passive(PassiveRequestType::CodeDiff))
            || (FeatureFlag::PromptSuggestionsViaMAA.is_enabled()
                && matches!(
                    self,
                    AIRequestType::Passive(PassiveRequestType::PassiveSuggestion(
                        PassiveSuggestionTriggerType::ShellCommandCompleted
                    ))
                ))
    }

    pub fn is_passive_unit_test_suggestion(&self) -> bool {
        matches!(
            self,
            AIRequestType::Passive(PassiveRequestType::UnitTestSuggestion)
        )
    }
}

/// UI-layer representation of agent output to be rendered in an [`AIBlock`].
#[derive(Debug, Clone)]
pub enum AIBlockOutputStatus {
    Pending,
    PartiallyReceived {
        output: Shared<AIAgentOutput>,
    },
    Complete {
        output: Shared<AIAgentOutput>,
    },
    Cancelled {
        partial_output: Option<Shared<AIAgentOutput>>,
        reason: CancellationReason,
    },
    Failed {
        partial_output: Option<Shared<AIAgentOutput>>,
        error: RenderableAIError,
    },
}

impl AIBlockOutputStatus {
    /// Returns true if the response is still actively being streamed from the server.
    pub fn is_streaming(&self) -> bool {
        matches!(
            self,
            AIBlockOutputStatus::Pending | AIBlockOutputStatus::PartiallyReceived { .. }
        )
    }

    /// Returns `true` if the response stream was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, AIBlockOutputStatus::Cancelled { .. })
    }

    /// Returns the reason for the cancellation, if any.
    pub fn cancellation_reason(&self) -> Option<&CancellationReason> {
        match self {
            AIBlockOutputStatus::Cancelled { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, AIBlockOutputStatus::Complete { .. })
    }

    /// Returns the output to be rendered, if any.
    pub fn output_to_render(&self) -> Option<Shared<AIAgentOutput>> {
        match self {
            AIBlockOutputStatus::Pending => None,
            AIBlockOutputStatus::PartiallyReceived { output } => Some(output.get_owned()),
            AIBlockOutputStatus::Complete { output } => Some(output.get_owned()),
            AIBlockOutputStatus::Cancelled { partial_output, .. } => {
                partial_output.as_ref().map(Shared::get_owned)
            }
            AIBlockOutputStatus::Failed { partial_output, .. } => {
                partial_output.as_ref().map(Shared::get_owned)
            }
        }
    }

    pub fn error(&self) -> Option<&RenderableAIError> {
        match self {
            AIBlockOutputStatus::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Function signature for a callback that may be supplied to
/// [`AIBlockModel::subscribe_to_updates`], to be called whenever a new event is received from the
/// server.
pub type OutputStatusUpdateCallback<V> = Box<dyn FnMut(&mut V, &mut ViewContext<V>)>;

/// Trait to be implemented by data structures that provide the necessary data to back an
/// [`AIBlock`] view.
///
/// You might wonder why this is a trait, as opposed to just a single struct. It's actually quite
/// convenient to have an abstraction layer to completely decouple the model layer for a live agent
/// response stream from data for a restored AI block from history, or an imported AI block for
/// debugging.
pub trait AIBlockModel {
    type View;

    /// Returns the status of the agent output to be rendered in the AI block.
    fn status(&self, app: &AppContext) -> AIBlockOutputStatus;

    /// Returns the `server_output_id` associated with this output rendered in this block, if any.
    fn server_output_id(&self, app: &AppContext) -> Option<ServerOutputId>;

    /// Returns the model ID used to generate the output in this block, which may differ from the
    /// requested model ID because of failover, etc.
    fn model_id(&self, app: &AppContext) -> Option<LLMId>;

    /// Return `true` if the block is a restored-from-history AI block.
    fn is_restored(&self) -> bool {
        false
    }

    /// Return `true` if the block was created in the process of forking a conversation.
    fn is_forked(&self) -> bool {
        false
    }

    /// Returns `true` if this block renders a user query input that was autodetected as AI.
    fn was_autodetected_ai_query(&self, _app: &AppContext) -> bool {
        false
    }

    /// Returns the time elapsed since the request was triggered.
    ///
    /// `None` if there was no request for data in this block (e.g. if it's for a restored AI block).
    fn time_since_request_start(&self, _app: &AppContext) -> Option<TimeDelta> {
        None
    }

    /// Returns the [`LLMId`] for the base model used to generate output in this block.
    fn base_model<'a>(&'a self, app: &'a AppContext) -> Option<&'a LLMId>;

    /// Returns the [`AIAgentInput`]s corresponding to the user input to the Agent to be rendered
    /// in this block.
    fn inputs_to_render<'a>(&'a self, app: &'a AppContext) -> &'a [AIAgentInput];

    /// Returns the conversation ID for this block.
    fn conversation_id(&self, app: &AppContext) -> Option<AIConversationId>;

    /// Returns the exchange ID for this block.
    fn exchange_id(&self, _app: &AppContext) -> Option<AIAgentExchangeId> {
        None
    }

    /// Returns the participant ID who initiated this exchange, for shared sessions.
    /// Returns None for local (non-shared) sessions.
    fn response_initiator(&self, _app: &AppContext) -> Option<ParticipantId> {
        None
    }

    /// Registers the provided `callback` to be called each time an update is received in the agent
    /// response stream.
    fn on_updated_output(
        &self,
        callback: OutputStatusUpdateCallback<Self::View>,
        ctx: &mut ViewContext<Self::View>,
    );

    /// Returns the type of request that triggered the AI block.
    fn request_type(&self, app: &AppContext) -> AIRequestType;
}

#[cfg(any(test, feature = "integration_tests"))]
pub mod testing {
    use warpui::{AppContext, ViewContext};

    use super::{AIBlockModel, AIBlockOutputStatus, OutputStatusUpdateCallback};
    use crate::ai::agent::conversation::AIConversationId;
    use crate::ai::agent::{AIAgentInput, AIAgentOutput, ServerOutputId, Shared};
    use crate::ai::blocklist::model::{
        AIRequestType, PassiveRequestType, PassiveSuggestionTriggerType,
    };
    use crate::ai::blocklist::AIBlock;
    use crate::ai::llms::LLMId;

    pub struct FakeAIBlockModel {
        input: Vec<AIAgentInput>,
        output: Shared<AIAgentOutput>,
        model_id: LLMId,
    }

    impl FakeAIBlockModel {
        pub fn new(input: Vec<AIAgentInput>, output: AIAgentOutput) -> Self {
            Self {
                input,
                output: Shared::new(output),
                model_id: "fake-llm".to_owned().into(),
            }
        }
    }

    impl AIBlockModel for FakeAIBlockModel {
        type View = AIBlock;

        fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
            AIBlockOutputStatus::Complete {
                output: self.output.clone(),
            }
        }

        fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
            None
        }

        fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
            None
        }

        fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
            Some(&self.model_id)
        }

        fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
            &self.input
        }

        fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
            None
        }

        fn on_updated_output(
            &self,
            _callback: OutputStatusUpdateCallback<AIBlock>,
            _ctx: &mut ViewContext<AIBlock>,
        ) {
        }

        fn request_type(&self, app: &AppContext) -> AIRequestType {
            let inputs = self.inputs_to_render(app);
            if inputs
                .iter()
                .any(|input| input.is_passive_suggestion_trigger())
            {
                AIRequestType::Passive(PassiveRequestType::PassiveSuggestion(
                    PassiveSuggestionTriggerType::ShellCommandCompleted,
                ))
            } else if inputs
                .iter()
                .any(|input| input.auto_code_diff_query().is_some())
            {
                AIRequestType::Passive(PassiveRequestType::CodeDiff)
            } else {
                AIRequestType::Active
            }
        }
    }
}
