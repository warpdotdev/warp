mod apply_diff_model;
mod diff_application;
mod telemetry;

use std::collections::HashMap;
use std::path::PathBuf;

use ai::diff_validation::AIRequestedCodeDiff;
use apply_diff_model::ApplyDiffModel;
use diff_application::DiffApplicationError;
pub(crate) use diff_application::{apply_edits, read_local_file_for_diff};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt;
use itertools::Itertools;
pub(crate) use telemetry::MalformedFinalLineProxyEvent;
#[allow(unused_imports)]
pub use telemetry::{EditAcceptAndContinueClickedEvent, EditAcceptClickedEvent};
pub use telemetry::{
    EditReceivedEvent, EditResolvedEvent, EditStats, RequestFileEditsFormatKind,
    RequestFileEditsTelemetryEvent,
};
use vec1::{vec1, Vec1};
use warp_core::send_telemetry_from_ctx;
use warp_util::file::FileSaveError;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _, ViewHandle};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    AIAgentOutputMessage, AIAgentOutputMessageType, AIIdentifiers, AnyFileContent, FileContext,
    FileLocations, RequestFileEditsResult, UpdatedFileContext,
};
use crate::ai::blocklist::inline_action::code_diff_view::{
    CodeDiffView, CodeDiffViewEvent, DiffSessionType, FileDiff,
};
use crate::ai::blocklist::{BlocklistAIPermissions, RequestedEditResolution};
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::SessionType;
use crate::{safe_warn, BlocklistAIHistoryModel};
const APPLY_DIFF_RESULT_CONTEXT_LINES: usize = 10;

pub struct RequestFileEditsExecutor {
    active_session: ModelHandle<ActiveSession>,
    apply_diff_model: ModelHandle<ApplyDiffModel>,
    diff_views: HashMap<AIAgentActionId, ViewHandle<CodeDiffView>>,
    /// Set of action IDs where diff application failed.
    diff_application_failures: HashMap<AIAgentActionId, Vec1<DiffApplicationError>>,
    terminal_view_id: EntityId,
}

impl RequestFileEditsExecutor {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let apply_diff_model = ctx.add_model(|_| ApplyDiffModel::new(active_session.clone()));
        Self {
            active_session,
            apply_diff_model,
            diff_views: HashMap::new(),
            diff_application_failures: HashMap::new(),
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
                    action: AIAgentActionType::RequestFileEdits { file_edits, .. },
                    ..
                },
            conversation_id,
        } = input
        else {
            return false;
        };

        let paths: Vec<PathBuf> = file_edits
            .iter()
            .filter_map(|edit| edit.file().map(PathBuf::from))
            .collect();

        // Don't allow autoexecution if the diff was generated passively.
        let Some(latest_exchange) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.latest_exchange())
        else {
            return false;
        };
        if latest_exchange.has_passive_request() {
            return false;
        }

        // Allow "autoexecution" if the diff application failed so that we can continue execution.
        // This is a terrible hack--but allows us to continue execution and let the LLM potentially recover
        // from the LLM.
        // If we don't do this, a failed diff application will block execution of the entire AI conversation
        // without any possibility of recovery.
        if self
            .diff_application_failures
            .contains_key(&input.action.id)
        {
            return true;
        }

        BlocklistAIPermissions::as_ref(ctx)
            .can_write_files(&conversation_id, &paths, Some(self.terminal_view_id), ctx)
            .is_allowed()
    }

    /// Registers a diff view to handle a RequestFileEdits action.
    /// Note this MUST be called before `execute` or `preprocess_action` is invoked in
    /// order for the necessary state to be set to handle the action.
    pub fn register_requested_edits(
        &mut self,
        action_id: &AIAgentActionId,
        view: &ViewHandle<CodeDiffView>,
    ) {
        self.diff_views.insert(action_id.clone(), view.clone());
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput {
            action:
                AIAgentAction {
                    id,
                    action: AIAgentActionType::RequestFileEdits { .. },
                    ..
                },
            ..
        } = input
        else {
            return ActionExecution::InvalidAction;
        };

        let Some(diff_view) = self.diff_views.get(id) else {
            log::warn!("Tried to execute a RequestFileEdits action without a diff view");
            return ActionExecution::NotReady;
        };

        // If diff application failed, early exit.
        if let Some(errors) = self.diff_application_failures.remove(id) {
            return ActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed {
                    error: DiffApplicationError::error_for_conversation(&errors),
                },
            ));
        }

        let identifiers = self
            .generate_ai_identifiers(&input.conversation_id, id, ctx)
            .unwrap_or_else(|| AIIdentifiers {
                client_conversation_id: Some(input.conversation_id),
                ..Default::default()
            });

        let (result_tx, result_rx) = oneshot::channel();
        let mut result_tx = Some(result_tx);

        ctx.subscribe_to_view(diff_view, move |_me, _, event, ctx| match event {
            CodeDiffViewEvent::Rejected => {
                let Some(result_tx) = result_tx.take() else {
                    return;
                };
                let _ = result_tx.send(RequestFileEditsResult::Cancelled);
            }
            CodeDiffViewEvent::SavedAcceptedDiffs {
                diff,
                updated_files,
                file_contents,
                deleted_files,
                save_errors,
            } => {
                let Some(result_tx) = result_tx.take() else {
                    return;
                };

                // If saving any file failed, report it as an error to the LLM. Other files may
                // have saved successfully, but we're ignoring this edge case for now.
                if !save_errors.is_empty() {
                    let error = save_errors
                        .iter()
                        .filter_map(|err| match err.as_ref() {
                            FileSaveError::IOError { error, path } => {
                                Some(format!("Failed to save file {path:?}: {error}"))
                            }
                            _ => None,
                        })
                        .join("\n");

                    let _ = result_tx.send(RequestFileEditsResult::DiffApplicationFailed { error });
                    return;
                }

                let passive_diff = BlocklistAIHistoryModel::as_ref(ctx)
                    .is_entirely_passive_conversation(&input.conversation_id);
                send_telemetry_from_ctx!(
                    RequestFileEditsTelemetryEvent::EditResolved(EditResolvedEvent {
                        identifiers: identifiers.clone(),
                        response: RequestedEditResolution::Accept,
                        stats: EditStats {
                            files_edited: updated_files.len(),
                            lines_added: diff.lines_added,
                            lines_removed: diff.lines_removed,
                        },
                        passive_diff,
                    },),
                    ctx
                );

                // Build a map of file path → content from the editor buffers.
                // This avoids re-reading files from disk or the remote server.
                let content_map: HashMap<String, String> = file_contents.iter().cloned().collect();

                let _ = result_tx.send(RequestFileEditsResult::Success {
                    diff: diff.unified_diff.clone(),
                    updated_files: updated_file_contexts_from_editor_buffers(
                        updated_files,
                        &content_map,
                    ),
                    deleted_files: deleted_files.clone(),
                    lines_added: diff.lines_added,
                    lines_removed: diff.lines_removed,
                });
            }
            _ => (),
        });
        diff_view.update(ctx, |diff_view, ctx| {
            diff_view.accept_and_save(ctx);
        });

        ActionExecution::new_async(result_rx, |result, _ctx| match result {
            Ok(result) => AIAgentActionResultType::RequestFileEdits(result),
            Err(oneshot::Canceled) => {
                AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Cancelled)
            }
        })
    }

    pub(super) fn preprocess_action(
        &mut self,
        input: PreprocessActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        let AIAgentAction {
            id,
            action: AIAgentActionType::RequestFileEdits { file_edits, .. },
            ..
        } = input.action
        else {
            return futures::future::ready(()).boxed();
        };

        let ai_identifiers = self
            .generate_ai_identifiers(&input.conversation_id, id, ctx)
            .unwrap_or_else(|| AIIdentifiers {
                client_conversation_id: Some(input.conversation_id),
                ..Default::default()
            });

        let passive_diff = BlocklistAIHistoryModel::as_ref(ctx)
            .is_entirely_passive_conversation(&input.conversation_id);

        send_telemetry_from_ctx!(
            RequestFileEditsTelemetryEvent::EditReceived(EditReceivedEvent {
                identifiers: ai_identifiers.clone(),
                unique_files: file_edits.iter().map(|file| file.file()).unique().count(),
                diffs: file_edits.len(),
                passive_diff,
            }),
            ctx
        );

        let (tx, rx) = oneshot::channel();
        let files = file_edits.clone();
        let id = id.clone();

        let apply_future = self.apply_diff_model.update(ctx, |model, ctx| {
            model.apply_diffs(files, &ai_identifiers, passive_diff, ctx)
        });

        ctx.spawn(
            async move {
                let applied_diffs = apply_future.await;
                (applied_diffs, id, tx)
            },
            |me, (diffs, id, tx), ctx| {
                me.on_diffs_applied(diffs, id, tx, ctx);
            },
        );

        async {
            rx.await.ok();
        }
        .boxed()
    }

    fn on_diffs_applied(
        &mut self,
        applied_diffs: Result<Vec<AIRequestedCodeDiff>, Vec1<DiffApplicationError>>,
        id: AIAgentActionId,
        tx: oneshot::Sender<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        tx.send(()).ok();

        let Some(diff_view) = self.diff_views.get(&id) else {
            log::warn!(
                "Tried to apply diffs for a RequestFileEdits action without a corresponding diff view"
            );
            return;
        };

        let applied_diffs = match applied_diffs {
            Ok(diffs) if !diffs.is_empty() => diffs,
            Ok(_) => {
                // We didn't generate any diffs--consider this a failure.
                log::warn!("No diffs generated");
                self.diff_application_failures
                    .insert(id, vec1![DiffApplicationError::EmptyDiff]);
                return;
            }
            Err(err) => {
                safe_warn!(
                    safe: ("Failed to generate diffs"),
                    full: ("Failed to generate diffs {err:?}")
                );
                self.diff_application_failures.insert(id, err);
                return;
            }
        };

        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();

        let shell_launch_data = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        let mut diffs = Vec::with_capacity(applied_diffs.len());
        for diff in applied_diffs {
            let path = host_native_absolute_path(
                diff.file_name.as_str(),
                &shell_launch_data,
                &current_working_directory,
            );
            let file_diff = FileDiff::new(diff.original_content, path, diff.diff_type);
            diffs.push(file_diff);
        }

        // Set the session type on the diff view so save/delete/create routes
        // through the correct FileModel backend.
        let diff_session_type = match self.active_session.as_ref(ctx).session_type(ctx) {
            Some(SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            }) => DiffSessionType::Remote(host_id.clone()),
            _ => DiffSessionType::Local,
        };

        diff_view.update(ctx, |diff_view, ctx| {
            diff_view.set_diff_session_type(diff_session_type);
            diff_view.set_candidate_diffs(diffs, ctx);
        });
    }

    fn generate_ai_identifiers(
        &self,
        conversation_id: &AIConversationId,
        action_id: &AIAgentActionId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AIIdentifiers> {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let conversation = history_model.conversation(conversation_id)?;

        // Find the `AIAgentExchange` and its corresponding `AIAgentOutput` for this given action.
        let (exchange, output) = conversation.all_exchanges().into_iter().find_map(|exchange| {
            let output = exchange.output_status.output()?;
            let contains_action = output.get().messages.iter().any(|step| {
                matches!(step, AIAgentOutputMessage{ message: AIAgentOutputMessageType::Action(AIAgentAction { id, .. }), .. } if id == action_id)
            });

            contains_action.then_some((exchange, output))
        })?;

        let server_output_id = output.get().server_output_id.clone();
        let model_id = output.get().model_info.as_ref().map(|m| m.model_id.clone());
        Some(AIIdentifiers {
            client_conversation_id: Some(*conversation_id),
            client_exchange_id: Some(exchange.id),
            server_output_id,
            server_conversation_id: conversation
                .server_conversation_token()
                .cloned()
                .map(Into::into),
            model_id,
        })
    }
}

fn updated_file_contexts_from_editor_buffers(
    updated_files: &[(FileLocations, bool)],
    content_map: &HashMap<String, String>,
) -> Vec<UpdatedFileContext> {
    updated_files
        .iter()
        .flat_map(|(file_location, was_edited)| {
            let content = content_map
                .get(&file_location.name)
                .cloned()
                .unwrap_or_default();
            let line_count = content.lines().count();

            let mut file_location = file_location.clone();
            file_location.expand_surrounding_context(APPLY_DIFF_RESULT_CONTEXT_LINES);
            clamp_to_file_context_range_start(&mut file_location);

            if file_location.lines.is_empty() {
                return vec![UpdatedFileContext {
                    was_edited_by_user: *was_edited,
                    file_context: FileContext {
                        file_name: file_location.name,
                        content: AnyFileContent::StringContent(content),
                        line_range: None,
                        last_modified: None,
                        line_count,
                    },
                }];
            }

            let lines = content.lines().collect_vec();
            file_location
                .lines
                .into_iter()
                .map(|range| {
                    let start = range.start.saturating_sub(1).min(lines.len());
                    let end = range.end.saturating_sub(1).min(lines.len());
                    let fragment = if start >= end {
                        String::new()
                    } else {
                        lines[start..end].join("\n")
                    };

                    UpdatedFileContext {
                        was_edited_by_user: *was_edited,
                        file_context: FileContext {
                            file_name: file_location.name.clone(),
                            content: AnyFileContent::StringContent(fragment),
                            line_range: Some(range),
                            last_modified: None,
                            line_count,
                        },
                    }
                })
                .collect_vec()
        })
        .collect()
}

fn clamp_to_file_context_range_start(file_location: &mut FileLocations) {
    for range in &mut file_location.lines {
        range.start = range.start.max(1);
        range.end = range.end.max(range.start);
    }
}

impl Entity for RequestFileEditsExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "request_file_edits_tests.rs"]
mod tests;
