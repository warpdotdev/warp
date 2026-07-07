use itertools::Itertools;
use serde::{Deserialize, Serialize};
use warp_graphql::mutations::generate_metadata_for_command::{
    GenerateMetadataForCommandFailureType, GenerateMetadataForCommandSuccess,
};
use warpui::{SingletonEntity, ViewContext};

use super::arguments::ArgumentsState;
use super::modal::{AiAssistState, WorkflowModal, WorkflowModalEvent};
use crate::ai::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::workflows::workflow::{Argument, Workflow};
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Generated command metadata from server.
#[derive(Debug)]
pub struct GeneratedCommandMetadata {
    pub command: String,
    pub title: String,
    pub description: String,
    pub arguments: Vec<GeneratedArgument>,
}

/// Metadata for a parameter in the workflow.
#[derive(Debug)]
pub struct GeneratedArgument {
    pub name: String,
    pub description: String,
    pub default_value: String,
}

impl From<GenerateMetadataForCommandSuccess> for GeneratedCommandMetadata {
    fn from(value: GenerateMetadataForCommandSuccess) -> Self {
        GeneratedCommandMetadata {
            command: value.parameterized_command,
            title: value.title,
            description: value.description,
            arguments: value
                .parameters
                .into_iter()
                .map(|p| GeneratedArgument {
                    name: p.name,
                    description: p.description,
                    default_value: p.value,
                })
                .collect_vec(),
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum GeneratedCommandMetadataError {
    /// OpenAI failed to generate a parsable response.
    BadCommand,
    /// Request to OpenAI failed
    AiProviderError,
    /// User is over rate limit.
    RateLimited,
    Other,
}

impl GeneratedCommandMetadataError {
    pub fn user_facing_message(&self) -> String {
        match self {
            Self::BadCommand => {
                "Failed to generate metadata. Please try again with a different command."
            }
            Self::AiProviderError => "Something went wrong. Please try again.",
            Self::RateLimited => "Looks like you're out of AI credits. Please try again later.",
            Self::Other => "Something went wrong. Please try again.",
        }
        .to_string()
    }
}

impl From<GenerateMetadataForCommandFailureType> for GeneratedCommandMetadataError {
    fn from(value: GenerateMetadataForCommandFailureType) -> Self {
        match value {
            GenerateMetadataForCommandFailureType::BadCommand => Self::BadCommand,
            GenerateMetadataForCommandFailureType::AiProviderError => Self::AiProviderError,
            GenerateMetadataForCommandFailureType::RateLimited => Self::RateLimited,
            GenerateMetadataForCommandFailureType::Other => Self::Other,
        }
    }
}

impl WorkflowModal {
    /// Send request to generate metadata for the command in command editor.
    pub(super) fn issue_request(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_client = self.ai_client.clone();
        let content = self.content_editor.as_ref(ctx).buffer_text(ctx);
        let raw_request = content.trim().to_string();

        ctx.spawn(
            async move { ai_client.generate_metadata_for_command(raw_request).await },
            move |modal, response, ctx| {
                match response {
                    Ok(metadata) => {
                        modal.ai_metadata_assist_state = AiAssistState::Generated;
                        modal.enable_editors(ctx);

                        let arguments = metadata
                            .arguments
                            .into_iter()
                            .map(|parameter| Argument {
                                name: parameter.name,
                                description: Some(parameter.description),
                                default_value: Some(parameter.default_value),
                                arg_type: Default::default()
                            })
                            .collect_vec();

                        let workflow = Workflow::Command {
                            name: metadata.title,
                            description: Some(metadata.description),
                            command: metadata.command,
                            arguments,
                            tags: vec![],
                            source_url: None,
                            author: None,
                            author_url: None,
                            shells: vec![],
                            environment_variables: None,
                        };

                        send_telemetry_from_ctx!(
                            TelemetryEvent::AutoGenerateMetadataSuccess,
                            ctx
                        );

                        modal.populate_missing_field_with_suggestion(workflow, ctx);
                        ctx.notify();
                    }
                    Err(err) => {
                        let message = err.user_facing_message();
                        if let GeneratedCommandMetadataError::RateLimited = err {
                            let auth_state = AuthStateProvider::as_ref(ctx).get();
                            let current_user_id = auth_state.user_id().unwrap_or_default();
                            if let Some(team) = UserWorkspaces::as_ref(ctx).current_team() {
                                let current_user_email =
                                    auth_state.user_email().unwrap_or_default();
                                let has_admin_permissions = team.has_admin_permissions(&current_user_email);
                                if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                                    if has_admin_permissions {
                                        ctx.emit(WorkflowModalEvent::AiAssistUpgradeError(Some(team.uid), current_user_id));
                                    } else {
                                        ctx.emit(WorkflowModalEvent::AiAssistError("Looks like you're out of AI credits. Contact a team admin to upgrade for more credits.".to_string()));
                                    }
                                } else {
                                    ctx.emit(WorkflowModalEvent::AiAssistError(message.clone()));
                                }
                            } else {
                                ctx.emit(WorkflowModalEvent::AiAssistUpgradeError(None, current_user_id));
                            }
                        } else {
                            ctx.emit(WorkflowModalEvent::AiAssistError(message.clone()));
                        }

                        send_telemetry_from_ctx!(
                            TelemetryEvent::AutoGenerateMetadataError {
                                error_payload: serde_json::json!(err)
                            },
                            ctx
                        );

                        modal.ai_metadata_assist_state = AiAssistState::PreRequest;
                        modal.enable_editors(ctx);
                        ctx.notify();
                    }
                }
                AIRequestUsageModel::handle(ctx).update(ctx, |request_usage_model, ctx| {
                    request_usage_model.refresh_request_usage_async(ctx);
                });
            }
        );

        self.ai_metadata_assist_state = AiAssistState::RequestInFlight;
        self.disable_editors(ctx);
        ctx.notify();
    }

    // Populate only the missing field in the workflow editor with the generated suggestion from AI.
    pub(super) fn populate_missing_field_with_suggestion(
        &mut self,
        workflow: Workflow,
        ctx: &mut ViewContext<Self>,
    ) {
        self.title_editor.update(ctx, |editor, ctx| {
            if editor.is_empty(ctx) {
                editor.set_buffer_text(workflow.name(), ctx);
            }
        });

        self.description_editor.update(ctx, |editor, ctx| {
            if editor.is_empty(ctx) {
                editor.set_buffer_text(
                    workflow
                        .description()
                        .map(String::as_str)
                        .unwrap_or_default(),
                    ctx,
                );
            }
        });

        let content_parsed = !self.arguments_state.arguments.is_empty();
        if !content_parsed {
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(workflow.content(), ctx);
            });

            // note: normally, we wouldn't have to do this, since editing the command
            // editor's text will trigger the event that does this automatically.
            // however, that happens in a callback, yet we need to know what the args
            // are right away to populate the description/default value editors.
            self.arguments_state = ArgumentsState::for_command_workflow(
                &self.arguments_state,
                workflow.content().to_string(),
            );
            self.update_arguments_rows(ctx);

            workflow
                .arguments()
                .iter()
                .enumerate()
                .for_each(|(index, argument)| {
                    // Since suggestion generated by AI is non-deterministic, we should make sure to handle each
                    // operation safely.
                    if index >= self.arguments_rows.len() {
                        return;
                    }

                    if let Some(description) = &argument.description {
                        self.arguments_rows[index]
                            .description_editor
                            .update(ctx, |editor, ctx| {
                                editor.set_buffer_text(description.as_str(), ctx);
                            });
                    }

                    if let Some(default_value) = &argument.default_value {
                        self.arguments_rows[index].default_value_editor.update(
                            ctx,
                            |editor, ctx| {
                                editor.set_buffer_text(default_value.as_str(), ctx);
                            },
                        );
                    }
                });
        }
    }
}
