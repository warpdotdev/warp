use warpui::{AppContext, ViewContext};

use crate::ai::{
    agent::{
        conversation::AIConversationId, AIAgentInput, AIAgentOutput, RenderableAIError,
        ServerOutputId, Shared,
    },
    llms::LLMId,
};

use super::{super::AIBlock, AIBlockModel, AIBlockOutputStatus, OutputStatusUpdateCallback};

pub struct DebugAIBlockModel {
    inputs: Vec<AIAgentInput>,
    output: Option<Shared<AIAgentOutput>>,
    model: LLMId,
}

impl AIBlockModel for DebugAIBlockModel {
    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        match self.output.as_ref() {
            Some(output) => AIBlockOutputStatus::Complete {
                output: output.clone(),
            },
            None => AIBlockOutputStatus::Failed {
                error: RenderableAIError::Other {
                    error_message: "No output received.".to_owned(),
                    will_attempt_resume: false,
                    waiting_for_network: false,
                    is_user_error: false,
                },
            },
        }
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        self.output
            .as_ref()
            .and_then(|output| output.get().server_output_id.clone())
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        self.output
            .as_ref()
            .and_then(|output| output.get().model_id.clone())
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> &'a LLMId {
        &self.model
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a Vec<AIAgentInput> {
        &self.inputs
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        None
    }

    fn on_updated_output(
        &self,
        _callback: OutputStatusUpdateCallback,
        _ctx: &mut ViewContext<AIBlock>,
    ) {
    }

    fn request_type(&self, app: &AppContext) -> AIRequestType {
        let inputs = self.inputs_to_render(app);
        if inputs.iter().any(|input| input.is_suggest_prompt_query()) {
            AIRequestType::Passive(PassiveRequestType::SuggestPrompt)
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
