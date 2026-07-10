use std::sync::Arc;

use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::{
    add_pending_file_attachments, input_context_for_request, parse_context_attachments,
    BlocklistAIController, BlocklistAIControllerEvent, RequestInput,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentContext, AIAgentInput, CancellationReason, CloneRepositoryURL, EntrypointType,
    InvokeSkillUserQuery, RequestMetadata,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::context_model::{
    BlocklistAIContextModel, PendingAttachment, PendingFile,
};
use crate::ai::blocklist::queued_query::{QueuedQueryId, QueuedQueryModel};
use crate::search::slash_command_menu::static_commands::commands;
use crate::terminal::input::slash_commands::SlashCommandTrigger;
use crate::BlocklistAIHistoryModel;

pub enum SlashCommandRequest {
    CreateNewProject {
        query: String,
    },
    CloneRepository {
        url: String,
    },
    InitProjectRules,
    CreateEnvironment {
        repos: Vec<String>,
        use_current_dir: bool,
    },
    Summarize {
        prompt: Option<String>,
    },
    FetchReviewComments {
        repo_path: String,
    },
    /// Invoke a skill.
    InvokeSkill {
        skill: ai::skills::ParsedSkill,
        user_query: Option<String>,
    },
}

impl SlashCommandRequest {
    /// Parses user input into a SlashCommandRequest for slash commands that are handled
    /// via the AI query flow (as opposed to action-based slash commands handled in input.rs).
    pub fn from_query(query: &str) -> Option<SlashCommandRequest> {
        // Check if this is an exact /init query and route it to InitProjectRules instead
        if query == "/init" {
            return Some(Self::InitProjectRules);
        }

        // Check if query starts with /compact and route to summarize conversation
        if let Some(prompt) = query.strip_prefix(commands::COMPACT.name) {
            return Some(Self::Summarize {
                prompt: prompt.strip_prefix(' ').map(String::from),
            });
        }

        None
    }

    pub(super) fn send_request(
        self,
        controller: &mut BlocklistAIController,
        queued_query_id: Option<QueuedQueryId>,
        conversation_id_override: Option<AIConversationId>,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) {
        let is_queued_prompt = queued_query_id.is_some();
        // A fired queued prompt carries the conversation it was queued on; use it directly
        // instead of re-deriving from the current UI selection (which may point at a different
        // conversation the user navigated to). Falls back to the selection for direct sends.
        let conversation_id =
            conversation_id_override.or_else(|| self.conversation_id(controller, ctx));
        // For skill invocations, include user-attached context (images, blocks, and selected
        // text) so the skill's agent sees the same attachments a non-slash-command user query
        // would. Other slash commands continue to pass `false` to preserve existing behavior.
        let is_invoke_skill = matches!(self, Self::InvokeSkill { .. });
        let prompt_attachments = if is_invoke_skill {
            match (queued_query_id, conversation_id) {
                (Some(query_id), Some(conversation_id)) => QueuedQueryModel::as_ref(ctx)
                    .attachments_for(conversation_id, query_id)
                    .to_vec(),
                (Some(_), None) => vec![],
                (None, _) => controller
                    .context_model
                    .as_ref(ctx)
                    .pending_attachments()
                    .to_vec(),
            }
        } else {
            vec![]
        };
        let mut image_context = Vec::new();
        let mut prompt_files = Vec::new();
        for attachment in prompt_attachments {
            match attachment {
                PendingAttachment::Image(image) => {
                    image_context.push(AIAgentContext::Image(image));
                }
                PendingAttachment::File(file) => prompt_files.push(file),
            }
        }
        let context = input_context_for_request(
            is_invoke_skill,
            controller.context_model.as_ref(ctx),
            controller.active_session.as_ref(ctx),
            conversation_id,
            image_context,
            ctx,
        );
        let entrypoint = self.entrypoint();
        let is_summarize = matches!(self, Self::Summarize { .. });
        let inputs = self.input(
            context,
            prompt_files,
            controller.context_model.as_ref(ctx),
            ctx,
        );
        if inputs.is_empty() {
            return;
        }
        let active_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation_id(controller.terminal_surface_id);

        // If no existing conversation, create a new one.
        // When AgentView is enabled, enter agent view which creates the conversation
        // and ensures AI blocks render correctly in the agent view.
        let Some(conversation_id) = conversation_id.or_else(|| {
            if FeatureFlag::AgentView.is_enabled() {
                controller.context_model.update(ctx, |context_model, ctx| {
                    context_model
                        .try_start_new_conversation(
                            AgentViewEntryOrigin::SlashCommand {
                                trigger: SlashCommandTrigger::input(),
                            },
                            ctx,
                        )
                        .ok()
                })
            } else {
                Some(controller.start_new_conversation_for_request(ctx).id())
            }
        }) else {
            report_error!("Failed to get conversation ID for slash command request");
            return;
        };

        let cancellation_reason = CancellationReason::FollowUpSubmitted {
            is_for_same_conversation: active_conversation_id
                .is_some_and(|id| id == conversation_id),
        };
        if let Some(active_conversation_id) = active_conversation_id {
            controller.cancel_conversation_progress(
                active_conversation_id,
                cancellation_reason,
                ctx,
            );
        }

        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        let task_id = conversation.get_root_task_id().clone();

        let request_input = RequestInput::for_task(
            inputs,
            task_id,
            &controller.active_session,
            controller.get_current_response_initiator(),
            conversation_id,
            controller.terminal_surface_id,
            ctx,
        );
        let model_id = request_input.model_id.clone();

        match controller.send_request_input(
            request_input,
            Some(RequestMetadata {
                is_autodetected_user_query: false,
                entrypoint,
                is_auto_resume_after_error: false,
            }),
            /*can_attempt_resume_on_error*/ true,
            is_queued_prompt,
            ctx,
        ) {
            Ok((_, stream_id)) => {
                // Direct skills consume live pending context; queued skills consume row-owned
                // context and must not clear a new draft's staged attachments.
                if is_invoke_skill && !is_queued_prompt {
                    controller.context_model.update(ctx, |context_model, ctx| {
                        context_model.reset_context_to_default(ctx);
                    });
                }
                // Emit SentRequest event to trigger buffer clearing
                if is_summarize {
                    ctx.emit(BlocklistAIControllerEvent::SentRequest {
                        contains_user_query: true,
                        is_queued_prompt,
                        model_id,
                        stream_id,
                    });
                }
            }
            Err(e) => report_error!(e.context("Failed to send agent slash command request")),
        }
    }

    pub(super) fn conversation_id(
        &self,
        controller: &BlocklistAIController,
        app: &AppContext,
    ) -> Option<AIConversationId> {
        match self {
            Self::Summarize { .. }
            | Self::CreateEnvironment { .. }
            | Self::InvokeSkill { .. }
            | Self::FetchReviewComments { .. } => controller
                .context_model
                .as_ref(app)
                .selected_conversation_id(app),
            _ => None,
        }
    }

    fn input(
        self,
        context: Arc<[AIAgentContext]>,
        prompt_files: Vec<PendingFile>,
        context_model: &BlocklistAIContextModel,
        app: &AppContext,
    ) -> Vec<AIAgentInput> {
        match self {
            SlashCommandRequest::CreateNewProject { query } => {
                vec![AIAgentInput::CreateNewProject { query, context }]
            }
            SlashCommandRequest::CloneRepository { url } => {
                vec![AIAgentInput::CloneRepository {
                    clone_repo_url: CloneRepositoryURL::new(url),
                    context,
                }]
            }
            SlashCommandRequest::InitProjectRules => vec![AIAgentInput::InitProjectRules {
                context,
                display_query: Some("/init".to_string()),
            }],
            SlashCommandRequest::CreateEnvironment {
                mut repos,
                use_current_dir,
            } => {
                let display_query = if repos.is_empty() {
                    "/create-environment".to_string()
                } else {
                    format!("/create-environment {}", repos.join(" "))
                };

                // Add "." to represent the current working directory
                if use_current_dir {
                    repos.push(String::from("."));
                }

                vec![AIAgentInput::CreateEnvironment {
                    context,
                    display_query: Some(display_query),
                    repo_paths: repos,
                }]
            }
            SlashCommandRequest::Summarize { prompt, .. } => {
                vec![AIAgentInput::SummarizeConversation { prompt, context }]
            }
            SlashCommandRequest::FetchReviewComments { repo_path } => {
                vec![AIAgentInput::FetchReviewComments { repo_path, context }]
            }
            SlashCommandRequest::InvokeSkill { skill, user_query } => {
                let user_query = if FeatureFlag::SkillArguments.is_enabled() {
                    let query = user_query
                        .map(|query| query.trim().to_string())
                        .unwrap_or_default();
                    (!query.is_empty() || !prompt_files.is_empty()).then(|| {
                        let mut referenced_attachments =
                            parse_context_attachments(&query, context_model, app);
                        add_pending_file_attachments(&mut referenced_attachments, prompt_files);
                        InvokeSkillUserQuery {
                            referenced_attachments,
                            query,
                        }
                    })
                } else {
                    None
                };
                vec![AIAgentInput::InvokeSkill {
                    skill,
                    user_query,
                    context,
                }]
            }
        }
    }

    fn entrypoint(&self) -> EntrypointType {
        match self {
            SlashCommandRequest::CloneRepository { .. } => EntrypointType::CloneRepository,
            SlashCommandRequest::InitProjectRules => EntrypointType::InitProjectRules,
            SlashCommandRequest::CreateNewProject { .. }
            | SlashCommandRequest::CreateEnvironment { .. }
            | SlashCommandRequest::Summarize { .. }
            | SlashCommandRequest::FetchReviewComments { .. }
            | SlashCommandRequest::InvokeSkill { .. } => EntrypointType::UserInitiated,
        }
    }
}
