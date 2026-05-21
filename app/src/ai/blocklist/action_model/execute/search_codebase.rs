use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use futures::{channel::oneshot, future::BoxFuture, FutureExt};
use itertools::Itertools;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::{
    ai::{
        agent::{
            AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
            SearchCodebaseFailureReason, SearchCodebaseRequest, SearchCodebaseResult,
        },
        blocklist::SessionContext,
        blocklist::{action_model::execute::get_server_output_id, BlocklistAIPermissions},
        get_relevant_files::controller::{
            GetRelevantFilesController, GetRelevantFilesControllerEvent,
            GetRelevantFilesControllerResult, GetRelevantFilesError, GetRelevantFilesRequestTarget,
        },
    },
    features::FeatureFlag,
    send_telemetry_from_ctx,
    terminal::model::session::active_session::ActiveSession,
    TelemetryEvent,
};

use super::{
    read_local_file_context, ActionExecution, AnyActionExecution, ExecuteActionInput,
    PreprocessActionInput,
};

pub struct SearchCodebaseExecutor {
    active_session: ModelHandle<ActiveSession>,
    get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
    /// Per-action response channels for searches that are still waiting on
    /// `GetRelevantFilesController`.
    active_searches: HashMap<AIAgentActionId, oneshot::Sender<SearchCodebaseResult>>,
    /// Cached repo roots derived during preprocessing so permission checks and execution can agree
    /// on which repository the action actually targets.
    root_repo_paths: HashMap<AIAgentActionId, PathBuf>,
    terminal_view_id: EntityId,
}

impl SearchCodebaseExecutor {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&get_relevant_files_controller, |me, event, ctx| {
            if !me.active_searches.contains_key(event.action_id()) {
                return;
            }

            match event {
                GetRelevantFilesControllerEvent::Success {
                    action_id,
                    result: GetRelevantFilesControllerResult::SearchResult(result),
                } => {
                    let Some(result_tx) = me.active_searches.remove(action_id) else {
                        return;
                    };
                    if let Err(e) = result_tx.send(result.clone()) {
                        log::warn!("Failed to send search codebase results to receiver {e:?}.");
                    }
                }
                GetRelevantFilesControllerEvent::Success {
                    result: GetRelevantFilesControllerResult::Locations(fragments),
                    ..
                } => {
                    let action_id = event.action_id().clone();
                    let locations = fragments
                        .iter()
                        .map(|location| location.into())
                        .collect_vec();
                    let current_working_directory = me
                        .active_session
                        .as_ref(ctx)
                        .current_working_directory()
                        .cloned();
                    let shell = me.active_session.as_ref(ctx).shell_launch_data(ctx);
                    ctx.spawn(
                        async move {
                            match read_local_file_context(
                                &locations,
                                current_working_directory,
                                shell,
                                None,
                                None,
                            )
                            .await
                            {
                                Ok(result) => {
                                    if !result.missing_files.is_empty() {
                                        let missing_files = result.missing_files.join(", ");
                                        SearchCodebaseResult::Failed {
                                            message: format!(
                                                "These files do not exist: {missing_files}"
                                            ),
                                            reason: SearchCodebaseFailureReason::InvalidFilePaths,
                                        }
                                    } else {
                                        SearchCodebaseResult::Success {
                                            files: result.file_contexts,
                                        }
                                    }
                                }
                                Err(e) => SearchCodebaseResult::Failed {
                                    reason: SearchCodebaseFailureReason::ClientError,
                                    message: e.to_string(),
                                },
                            }
                        },
                        move |me, result, _| {
                            let Some(result_tx) = me.active_searches.remove(&action_id) else {
                                return;
                            };
                            if let Err(e) = result_tx.send(result) {
                                log::warn!(
                                    "Failed to send search codebase results to receiver {e:?}."
                                );
                            }
                        },
                    );
                }
                GetRelevantFilesControllerEvent::Error { action_id } => {
                    let Some(result_tx) = me.active_searches.remove(action_id) else {
                        return;
                    };
                    if let Err(e) = result_tx.send(SearchCodebaseResult::Failed {
                        message: "The search failed. Try another way to locate the relevant files."
                            .to_owned(),
                        reason: SearchCodebaseFailureReason::GetRelevantFilesError,
                    }) {
                        log::warn!("Failed to send search codebase results to receiver {e:?}.");
                    }
                }
            }
        });

        Self {
            active_session,
            get_relevant_files_controller,
            active_searches: HashMap::new(),
            root_repo_paths: HashMap::new(),
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
                    action: AIAgentActionType::SearchCodebase(..),
                    ..
                },
            conversation_id,
        } = input
        else {
            return false;
        };

        self.root_repo_paths.get(id).is_none_or(|root_repo_path| {
            // If we have access to read the repo, we can auto-execute the search.
            BlocklistAIPermissions::as_ref(ctx)
                .can_read_files_with_conversation(
                    &conversation_id,
                    vec![root_repo_path.to_owned()],
                    Some(self.terminal_view_id),
                    ctx,
                )
                .is_allowed()
        })
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
            action:
                AIAgentActionType::SearchCodebase(SearchCodebaseRequest {
                    query,
                    partial_paths,
                    codebase_path,
                }),
            ..
        } = action
        else {
            return ActionExecution::InvalidAction;
        };

        let session_context = SessionContext::from_session(self.active_session.as_ref(ctx), ctx);
        if session_context.is_remote() {
            let requested_codebase_path = codebase_path
                .as_deref()
                .filter(|path| !path.is_empty() && *path != ".")
                .map(ToOwned::to_owned);
            let server_output_id = get_server_output_id(conversation_id, ctx);
            send_telemetry_from_ctx!(
                TelemetryEvent::SearchCodebaseRequested {
                    action_id: id.clone(),
                    server_output_id,
                    is_cross_repo: requested_codebase_path.is_some(),
                },
                ctx
            );

            let root_dir_for_search = self.root_repo_paths.get(id).cloned().or_else(|| {
                self.get_relevant_files_controller
                    .as_ref(ctx)
                    .root_directory_for_remote_search(
                        &session_context,
                        requested_codebase_path.as_deref(),
                        ctx,
                    )
            });
            let Some(root_dir_for_search) = root_dir_for_search else {
                return ActionExecution::Sync(AIAgentActionResultType::SearchCodebase(
                    SearchCodebaseResult::Failed {
                        reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                        message: "The search failed because the codebase is not available. Try another way to locate the relevant files.".to_owned(),
                    },
                ));
            };

            // Add the repo root as a temporary permission; if the user gave us permission to
            // search the repo, we can certainly search files within it for the rest of the convo.
            BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
                model.add_temporary_file_read_permissions(
                    conversation_id,
                    vec![root_dir_for_search.to_owned()],
                );
            });

            let (result_tx, result_rx) = oneshot::channel();
            self.active_searches.insert(id.clone(), result_tx);

            match self
                .get_relevant_files_controller
                .update(ctx, |controller, ctx| {
                    controller.send_request(
                        GetRelevantFilesRequestTarget::Remote {
                            session_context,
                            requested_codebase_path,
                        },
                        query.clone(),
                        partial_paths.as_ref(),
                        id.clone(),
                        ctx,
                    )
                }) {
                Ok(_) => ActionExecution::Async {
                    execute_future: Box::pin(result_rx),
                    on_complete: Box::new(
                        |res: Result<SearchCodebaseResult, oneshot::Canceled>, _ctx| {
                            let action_result =
                                res.unwrap_or_else(|e| SearchCodebaseResult::Failed {
                                    message: e.to_string(),
                                    reason: SearchCodebaseFailureReason::ClientError,
                                });
                            AIAgentActionResultType::SearchCodebase(action_result)
                        },
                    ),
                },
                Err(e) => {
                    log::warn!("Failed to send remote get_relevant_files request: {e:?}");
                    ActionExecution::Sync(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Failed {
                            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                            message: "Remote codebase search is unavailable.".to_owned(),
                        },
                    ))
                }
            }
        } else {
            let codebase_path = codebase_path.as_ref().map(PathBuf::from);

            let Some(current_working_directory) = self
                .active_session
                .as_ref(ctx)
                .current_working_directory()
                .map(PathBuf::from)
            else {
                // This should really never happen; it implies that we don't know what the
                // current working directory is, which is never the case.
                return ActionExecution::Sync(AIAgentActionResultType::SearchCodebase(
                    SearchCodebaseResult::Failed {
                        reason: SearchCodebaseFailureReason::MissingCurrentWorkingDirectory,
                        message: "The search failed. Try another way to locate the relevant files."
                            .to_string(),
                    },
                ));
            };

            let search_dir;
            let is_cross_repo;
            if FeatureFlag::CrossRepoContext.is_enabled() {
                is_cross_repo = codebase_path
                    .as_ref()
                    .is_some_and(|path| !current_working_directory.starts_with(path));
                search_dir = codebase_path.unwrap_or(current_working_directory);
            } else {
                is_cross_repo = false;
                search_dir = current_working_directory;
            }
            let server_output_id = get_server_output_id(conversation_id, ctx);
            send_telemetry_from_ctx!(
                TelemetryEvent::SearchCodebaseRequested {
                    action_id: id.clone(),
                    server_output_id,
                    is_cross_repo,
                },
                ctx
            );

            let Some(root_dir_for_search) = self.root_repo_paths.get(id) else {
                let action_id = id.clone();

                // Check if directory exists on background thread since its a sys call; no need to block
                // main thread since its just for telemetry.
                let _ = ctx.spawn(async move { search_dir.exists() }, |_, exists, ctx| {
                    let error = if exists {
                        "The codebase isn't indexed".to_string()
                    } else {
                        "The codebase doesn't exist".to_string()
                    };
                    send_telemetry_from_ctx!(
                        TelemetryEvent::SearchCodebaseRepoUnavailable { action_id, error },
                        ctx
                    );
                });
                return ActionExecution::Sync(AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Failed {
                    message: "The search failed because the codebase is not available. Try another way to locate the relevant files.".to_owned(),
                    reason: SearchCodebaseFailureReason::CodebaseNotIndexed
                }));
            };

            // Add the repo root as a temporary permission; if the user gave us permission to
            // search the repo, we can certainly search files within it for the rest of the convo.
            BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
                model.add_temporary_file_read_permissions(
                    conversation_id,
                    vec![root_dir_for_search.to_owned()],
                );
            });

            let (result_tx, result_rx) = oneshot::channel();
            self.active_searches.insert(id.clone(), result_tx);

            // Start the actual search.
            match self
                .get_relevant_files_controller
                .update(ctx, |controller, ctx| {
                    controller.send_request(
                        GetRelevantFilesRequestTarget::Local {
                            directory: root_dir_for_search.clone(),
                        },
                        query.clone(),
                        partial_paths.as_ref(),
                        id.clone(),
                        ctx,
                    )
                }) {
                Ok(_) => ActionExecution::Async {
                    execute_future: Box::pin(result_rx),
                    on_complete: Box::new(
                        |res: Result<SearchCodebaseResult, oneshot::Canceled>, _ctx| {
                            let action_result =
                                res.unwrap_or_else(|e| SearchCodebaseResult::Failed {
                                    message: e.to_string(),
                                    reason: SearchCodebaseFailureReason::ClientError,
                                });
                            AIAgentActionResultType::SearchCodebase(action_result)
                        },
                    ),
                },
                Err(e) => {
                    log::warn!("Failed to send get_relevant_files request for directory: {e:?}");

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
                    ActionExecution::Sync(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Failed {
                            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                            message: error_message,
                        },
                    ))
                }
            }
        }
    }

    pub fn root_repo_for_action(&self, id: &AIAgentActionId) -> Option<&Path> {
        self.root_repo_paths.get(id).map(|path| path.as_path())
    }

    pub(super) fn cancel_execution(
        &mut self,
        action_id: &AIAgentActionId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Drop the waiting sender first so any late completion from the controller becomes a no-op.
        self.active_searches.remove(action_id);
        self.get_relevant_files_controller
            .update(ctx, |controller, ctx| {
                controller.cancel_request_for_action(action_id, ctx)
            });
    }

    fn get_root_repo_path_for_request(
        &self,
        request: &SearchCodebaseRequest,
        app: &AppContext,
    ) -> Option<PathBuf> {
        let SearchCodebaseRequest { codebase_path, .. } = request;
        let session_context = SessionContext::from_session(self.active_session.as_ref(app), app);
        if session_context.is_remote() {
            let requested_codebase_path = codebase_path
                .as_deref()
                .filter(|path| !path.is_empty() && *path != ".");
            return self
                .get_relevant_files_controller
                .as_ref(app)
                .root_directory_for_remote_search(&session_context, requested_codebase_path, app);
        }

        let codebase_path = codebase_path.as_deref().map(PathBuf::from);
        let Some(pwd) = self
            .active_session
            .as_ref(app)
            .current_working_directory()
            .map(PathBuf::from)
        else {
            // This should never really happen, since we should always have a pwd.
            log::warn!("No pwd found for search codebase request");
            return None;
        };

        let search_dir = if FeatureFlag::CrossRepoContext.is_enabled() {
            match codebase_path {
                Some(codebase_path) if codebase_path == Path::new(".") => pwd,
                Some(codebase_path) => codebase_path,
                None => pwd,
            }
        } else {
            pwd
        };

        self.get_relevant_files_controller
            .as_ref(app)
            .root_directory_for_search(&search_dir, app)
    }

    /// In the preprocessing step, we determine the root of the repo path for the codebase to be
    /// searched, and cache it. This is used downstream to render UI thats derived from the root
    /// repo path, which isn't really trivially computable from the `action` itself.
    pub(super) fn preprocess_action(
        &mut self,
        input: PreprocessActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        let AIAgentAction {
            id,
            action: AIAgentActionType::SearchCodebase(request),
            ..
        } = input.action
        else {
            log::error!("Expected a SearchCodebase action when preprocessing action");
            return futures::future::ready(()).boxed();
        };

        if let Some(root_repo_path) = self.get_root_repo_path_for_request(request, ctx) {
            self.root_repo_paths.insert(id.clone(), root_repo_path);
        }
        futures::future::ready(()).boxed()
    }
}

impl Entity for SearchCodebaseExecutor {
    type Event = ();
}
