use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::{
    read_local_file_context, ActionExecution, AnyActionExecution, ExecuteActionInput,
    PreprocessActionInput,
};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, ReadFilesFailedFile,
    ReadFilesRequest, ReadFilesResult,
};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::SessionType;

pub struct ReadFilesExecutor {
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
}

impl ReadFilesExecutor {
    pub fn new(active_session: ModelHandle<ActiveSession>, terminal_view_id: EntityId) -> Self {
        Self {
            active_session,
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
                    action: AIAgentActionType::ReadFiles(ReadFilesRequest { locations }),
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

        BlocklistAIPermissions::as_ref(ctx)
            .can_read_files_with_conversation(
                &conversation_id,
                locations
                    .iter()
                    .map(|file| {
                        PathBuf::from(host_native_absolute_path(
                            &file.name,
                            &shell,
                            &current_working_directory,
                        ))
                    })
                    .collect(),
                Some(self.terminal_view_id),
                ctx,
            )
            .is_allowed()
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
            action: AIAgentActionType::ReadFiles(ReadFilesRequest { locations }),
            ..
        } = action
        else {
            return ActionExecution::InvalidAction;
        };

        BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
            model.add_temporary_file_read_permissions(
                conversation_id,
                locations.iter().map(|file| Path::new(&file.name)),
            );
        });

        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        let locations = locations.clone();

        // Check if this is a remote session with a connected host.
        let session_type = self.active_session.as_ref(ctx).session_type(ctx);
        let remote_client = match &session_type {
            Some(SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            }) => remote_server::manager::RemoteServerManager::as_ref(ctx)
                .client_for_host(host_id)
                .cloned(),
            _ => None,
        };

        // Remote session without a usable remote server client. File reading
        // requires either local access or a connected remote server, neither
        // of which is available.
        if matches!(session_type, Some(SessionType::WarpifiedRemote { .. }))
            && remote_client.is_none()
        {
            return ActionExecution::Sync(AIAgentActionResultType::ReadFiles(
                ReadFilesResult::Error(
                    "The file read/edit tool is not available on this remote session. \
                     Try using a different tool."
                        .to_string(),
                ),
            ));
        }

        if let Some(client) = remote_client {
            return ActionExecution::Async {
                execute_future: Box::pin(async move {
                    let request = remote_server::proto::ReadFileContextRequest {
                        files: locations
                            .iter()
                            .map(|loc| {
                                let absolute_path = host_native_absolute_path(
                                    &loc.name,
                                    &shell,
                                    &current_working_directory,
                                );
                                remote_server::proto::ReadFileContextFile {
                                    path: absolute_path,
                                    line_ranges: loc
                                        .lines
                                        .iter()
                                        .map(|r| remote_server::proto::LineRange {
                                            start: r.start as u32,
                                            end: r.end as u32,
                                        })
                                        .collect(),
                                }
                            })
                            .collect(),
                        max_file_bytes: None,
                        max_batch_bytes: None,
                    };

                    let response = client
                        .read_file_context(request)
                        .await
                        .map_err(|e| anyhow::anyhow!("Remote read failed: {e}"))?;

                    let failed_files = response
                        .failed_files
                        .into_iter()
                        .map(|f| ReadFilesFailedFile {
                            path: f.path,
                            message: f.error.map(|e| e.message).unwrap_or_else(|| {
                                "File not found or could not be read".to_string()
                            }),
                        })
                        .collect::<Vec<_>>();

                    if !failed_files.is_empty() && response.file_contexts.is_empty() {
                        let failed = failed_files
                            .iter()
                            .map(|f| format!("{}: {}", f.path, f.message))
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Ok(ReadFilesResult::Error(format!(
                            "Failed to read files: {failed}"
                        )));
                    }

                    let file_contexts = response
                        .file_contexts
                        .into_iter()
                        .filter_map(|fc| {
                            let content = match fc.content? {
                                remote_server::proto::file_context_proto::Content::TextContent(
                                    text,
                                ) => crate::ai::agent::AnyFileContent::StringContent(text),
                                remote_server::proto::file_context_proto::Content::BinaryContent(
                                    bytes,
                                ) => crate::ai::agent::AnyFileContent::BinaryContent(bytes),
                            };
                            let line_range = match (fc.line_range_start, fc.line_range_end) {
                                (Some(start), Some(end)) => Some(start as usize..end as usize),
                                _ => None,
                            };
                            let last_modified = fc.last_modified_epoch_millis.map(|ms| {
                                std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms)
                            });
                            Some(crate::ai::agent::FileContext {
                                file_name: fc.file_name,
                                content,
                                line_range,
                                last_modified,
                                line_count: fc.line_count as usize,
                            })
                        })
                        .collect();

                    Ok(ReadFilesResult::Success {
                        files: file_contexts,
                        failed_files,
                    })
                }),
                on_complete: Box::new(|res: Result<ReadFilesResult, anyhow::Error>, _ctx| {
                    let action_result =
                        res.unwrap_or_else(|e| ReadFilesResult::Error(e.to_string()));
                    AIAgentActionResultType::ReadFiles(action_result)
                }),
            };
        }

        // Local path.
        ActionExecution::Async {
            execute_future: Box::pin(async move {
                let result = read_local_file_context(
                    &locations,
                    current_working_directory,
                    shell,
                    None,
                    None,
                )
                .await?;
                if result.failed_files.is_empty() {
                    Ok(ReadFilesResult::Success {
                        files: result.file_contexts,
                        failed_files: Vec::new(),
                    })
                } else if result.file_contexts.is_empty() {
                    let failed_files = result
                        .failed_files
                        .iter()
                        .map(|f| format!("{}: {}", f.path, f.message))
                        .collect::<Vec<_>>()
                        .join(", ");
                    Ok(ReadFilesResult::Error(format!(
                        "Failed to read files: {failed_files}"
                    )))
                } else {
                    Ok(ReadFilesResult::Success {
                        files: result.file_contexts,
                        failed_files: result.failed_files,
                    })
                }
            }),
            on_complete: Box::new(|res: Result<ReadFilesResult, anyhow::Error>, _ctx| {
                let action_result = res.unwrap_or_else(|e| ReadFilesResult::Error(e.to_string()));
                AIAgentActionResultType::ReadFiles(action_result)
            }),
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for ReadFilesExecutor {
    type Event = ();
}
