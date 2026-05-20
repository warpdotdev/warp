use std::path::{Path, PathBuf};

use futures::{future::BoxFuture, FutureExt};
use itertools::Itertools;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, AIAgentAction, AIAgentActionResultType,
            AIAgentActionType, FileLocations, GetFilesRequestType, GetFilesResult,
        },
        blocklist::BlocklistAIPermissions,
        get_relevant_files::controller::{
            GetRelevantFilesController, GetRelevantFilesError, GetRelevantFilesRequestTarget,
            GetRelevantFilesStatus,
        },
        paths::host_native_absolute_path,
    },
    terminal::model::session::active_session::ActiveSession,
};

use super::{
    read_local_file_context, ActionExecution, AnyActionExecution, ExecuteActionInput,
    PreprocessActionInput,
};

pub struct GetFilesExecutor {
    active_session: ModelHandle<ActiveSession>,
    get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
    terminal_view_id: EntityId,
}

impl GetFilesExecutor {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
        terminal_view_id: EntityId,
    ) -> Self {
        Self {
            active_session,
            get_relevant_files_controller,
            terminal_view_id,
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ExecuteActionInput {
            action:
                AIAgentAction {
                    id,
                    action: AIAgentActionType::GetFiles(get_files_request),
                    ..
                },
            conversation_id,
        } = input
        else {
            return false;
        };

        // TODO: figure out how to avoid constructing the full paths in `should_execute`
        // and then again in `execute`, and then again on every render.
        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        match get_files_request {
            GetFilesRequestType::RelevantFileQuery { .. } => {
                match self.get_relevant_files_controller.as_ref(ctx).status(id) {
                    Some(relevant_files_status) => match relevant_files_status {
                        GetRelevantFilesStatus::Pending { root_repo_path } => {
                            // If we have access to read the repo, we can auto-execute the search.
                            BlocklistAIPermissions::handle(ctx)
                                .as_ref(ctx)
                                .can_read_files_with_conversation(
                                    &conversation_id,
                                    vec![root_repo_path.to_owned()],
                                    self.terminal_view_id,
                                    ctx,
                                )
                                .is_allowed()
                        }
                        // We can autoexecute if the request has not yet been sent or has failed,
                        // in which case we report the failure back to the LLM.
                        GetRelevantFilesStatus::InFlight { .. }
                        | GetRelevantFilesStatus::Failed { .. } => true,
                        GetRelevantFilesStatus::Success { file_paths, .. } => {
                            // If we've retrieved the relevant file paths, auto-execution (to read
                            // the file contents) depends on the user's permission.
                            BlocklistAIPermissions::handle(ctx)
                                .as_ref(ctx)
                                .can_read_files_with_conversation(
                                    &conversation_id,
                                    file_paths
                                        .iter()
                                        .map(|file| {
                                            PathBuf::from(host_native_absolute_path(
                                                &file.as_os_str().to_string_lossy(),
                                                &shell,
                                                &current_working_directory,
                                            ))
                                        })
                                        .collect(),
                                    self.terminal_view_id,
                                    ctx,
                                )
                                .is_allowed()
                        }
                    },
                    // Shouldn't be possible.
                    None => false,
                }
            }
            GetFilesRequestType::FileLocations(file_locations) => {
                BlocklistAIPermissions::handle(ctx)
                    .as_ref(ctx)
                    .can_read_files_with_conversation(
                        &conversation_id,
                        file_locations
                            .iter()
                            .map(|file| {
                                PathBuf::from(host_native_absolute_path(
                                    &file.name,
                                    &shell,
                                    &current_working_directory,
                                ))
                            })
                            .collect(),
                        self.terminal_view_id,
                        ctx,
                    )
                    .is_allowed()
            }
        }
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput {
            action,
            conversation_id,
            ..
        } = input;
        let AIAgentAction {
            id,
            action: AIAgentActionType::GetFiles(get_files_request),
            ..
        } = action
        else {
            return ActionExecution::InvalidAction;
        };

        let Some(current_working_directory) = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .map(PathBuf::from)
        else {
            // This should really never happen; it implies that we don't know what the
            // current working directory is, which is never the case.
            return ActionExecution::Sync(AIAgentActionResultType::GetFiles(
                GetFilesResult::Error(
                    "The search failed. Try another way to locate the relevant files.".to_string(),
                ),
            ));
        };

        match get_files_request {
            GetFilesRequestType::RelevantFileQuery {
                query,
                partial_paths,
            } => {
                match self
                    .get_relevant_files_controller
                    .as_ref(ctx)
                    .status(id)
                    .cloned()
                {
                    Some(GetRelevantFilesStatus::Pending { root_repo_path }) => {
                        // Add the repo root as a temporary permission; if the user gave us permission to
                        // search the repo, we can certainly search files within it for the rest of the convo.
                        BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
                            model.add_temporary_file_read_permissions(
                                conversation_id,
                                vec![root_repo_path.to_owned()],
                            );
                        });
                        // Start the actual search.
                        match self
                            .get_relevant_files_controller
                            .update(ctx, |controller, ctx| {
                                controller.send_request(
                                    GetRelevantFilesRequestTarget::Local {
                                        directory: current_working_directory.clone(),
                                    },
                                    query.clone(),
                                    partial_paths.as_ref(),
                                    id.clone(),
                                    ctx,
                                )
                            }) {
                            Ok(_) => ActionExecution::NotReady,
                            Err(e) => {
                                log::warn!(
                                    "Failed to send get_relevant_files request for directory: {:?}",
                                    e
                                );

                                let error_message = match e {
                                    GetRelevantFilesError::Pending => {
                                        "The current git repository is still being indexed, so search is unavailable right now. You can try again later".to_owned()
                                    }
                                    GetRelevantFilesError::CreateFailed => {
                                        "Relevant file search in the current directory is not available".to_owned()
                                    }
                                    GetRelevantFilesError::Missing => {
                                        "The current directory isn't within a git repository, which is necessary to search for relevant files.".to_owned()
                                    }
                                };
                                ActionExecution::Sync(AIAgentActionResultType::GetFiles(
                                    GetFilesResult::Error(error_message),
                                ))
                            }
                        }
                    }
                    Some(GetRelevantFilesStatus::InFlight { .. }) => ActionExecution::NotReady,
                    // The search succeeded so now we can look up the specific files.
                    Some(GetRelevantFilesStatus::Success { file_paths, .. }) => self
                        .execute_get_file_by_location_action(
                            file_paths
                                .iter()
                                .map(|path| FileLocations {
                                    name: path.to_string_lossy().to_string(),
                                    lines: vec![],
                                })
                                .collect_vec(),
                            conversation_id,
                            ctx,
                        ),
                    Some(GetRelevantFilesStatus::Failed { .. }) => ActionExecution::Sync(
                        AIAgentActionResultType::GetFiles(GetFilesResult::Error(
                            "The search failed. Try another way to locate the relevant files."
                                .to_owned(),
                        )),
                    ),
                    None => {
                        log::warn!(
                            "Tried to execute a GetFiles action without a corresponding status"
                        );
                        ActionExecution::InvalidAction
                    }
                }
            }
            GetFilesRequestType::FileLocations(file_locations) => self
                .execute_get_file_by_location_action(file_locations.clone(), conversation_id, ctx),
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        input: PreprocessActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        let Some(pwd) = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
        else {
            log::warn!("Failed to preprocess GetRelevantFiles action because no pwd");
            return futures::future::ready(()).boxed();
        };
        self.get_relevant_files_controller
            .update(ctx, |controller, ctx| {
                controller.queue_request(input.action.id.clone(), Path::new(&pwd), ctx);
            });
        futures::future::ready(()).boxed()
    }

    fn execute_get_file_by_location_action(
        &self,
        files: Vec<FileLocations>,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> ActionExecution<anyhow::Result<GetFilesResult>> {
        BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
            model.add_temporary_file_read_permissions(
                conversation_id,
                files.iter().map(|file| Path::new(&file.name)),
            );
        });

        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();

        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        ActionExecution::Async {
            execute_future: Box::pin(async move {
                let result =
                    read_local_file_context(&files, current_working_directory, shell, None, None)
                        .await?;
                if result.missing_files.is_empty() {
                    Ok(GetFilesResult::Success {
                        files: result.file_contexts,
                    })
                } else {
                    let missing_files = result.missing_files.join(", ");
                    Ok(GetFilesResult::Error(format!(
                        "These files do not exist: {}",
                        missing_files
                    )))
                }
            }),
            on_complete: Box::new(|res, _ctx| {
                let action_result = res.unwrap_or_else(|e| GetFilesResult::Error(e.to_string()));
                AIAgentActionResultType::GetFiles(action_result)
            }),
        }
    }
}

impl Entity for GetFilesExecutor {
    type Event = ();
}
