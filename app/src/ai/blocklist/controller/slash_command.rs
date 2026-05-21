use std::sync::Arc;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, AIAgentContext, AIAgentInput, CancellationReason,
            CloneRepositoryURL, EntrypointType, RequestMetadata,
        },
        blocklist::agent_view::AgentViewEntryOrigin,
    },
    search::slash_command_menu::static_commands::commands,
    terminal::input::slash_commands::SlashCommandTrigger,
    BlocklistAIHistoryModel,
};

use super::{
    input_context_for_request, parse_context_attachments, BlocklistAIController,
    BlocklistAIControllerEvent, RequestInput,
};

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
        is_queued_prompt: bool,
        ctx: &mut ModelContext<BlocklistAIController>,
    ) {
        let conversation_id = self.conversation_id(controller, ctx);
        // For skill invocations, include user-attached context (images, blocks, and selected
        // text) so the skill's agent sees the same attachments a non-slash-command user query
        // would. Other slash commands continue to pass `false` to preserve existing behavior.
        let is_invoke_skill = matches!(self, Self::InvokeSkill { .. });
        let context = input_context_for_request(
            is_invoke_skill,
            controller.context_model.as_ref(ctx),
            controller.active_session.as_ref(ctx),
            conversation_id,
            vec![],
            ctx,
        );
        let entrypoint = self.entrypoint();
        let is_summarize = matches!(self, Self::Summarize { .. });
        let inputs = self.input(context, controller.context_model.as_ref(ctx), ctx);
        if inputs.is_empty() {
            return;
        }
        let active_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation_id(controller.terminal_view_id);

        // If no existing conversation, create a new one.
        // When AgentView is enabled, enter agent view which creates the conversation
        // and ensures AI blocks render correctly in the agent view.
        let Some(conversation_id) = conversation_id.or_else(|| {
            if FeatureFlag::AgentView.is_enabled() {
                controller.context_model.update(ctx, |context_model, ctx| {
                    context_model
                        .try_enter_agent_view_for_new_conversation(
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
            log::error!("Failed to get conversation ID for slash command request");
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

        let request_input = RequestInput::for_task(
            inputs,
            conversation.get_root_task_id().clone(),
            &controller.active_session,
            controller.get_current_response_initiator(),
            conversation_id,
            controller.terminal_view_id,
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
            /*default_to_follow_up_on_success*/ true,
            /*can_attempt_resume_on_error*/ true,
            is_queued_prompt,
            ctx,
        ) {
            Ok((_, stream_id)) => {
                // Skill invocations now consume user-attached context (images, blocks, and
                // selected text) the same way regular user queries do. `send_request_input`
                // only clears that context for `AIAgentInput::UserQuery`, so we mirror its
                // reset here for `InvokeSkill` to avoid pending attachments sticking around
                // and getting re-sent on subsequent messages.
                if is_invoke_skill {
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
            Err(e) => log::error!("Failed to send agent slash command request: {e:?}"),
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
        context_model: &crate::ai::blocklist::BlocklistAIContextModel,
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
                    user_query
                        .map(|query| query.trim().to_string())
                        .filter(|query| !query.is_empty())
                        .map(|query| crate::ai::agent::InvokeSkillUserQuery {
                            referenced_attachments: parse_context_attachments(
                                &query,
                                context_model,
                                app,
                            ),
                            query,
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
