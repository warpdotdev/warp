mod apply_diff_model;
mod diff_application;
mod telemetry;

use std::collections::HashMap;
use std::path::PathBuf;

use ai::diff_validation::AIRequestedCodeDiff;
use apply_diff_model::ApplyDiffModel;
use diff_application::DiffApplicationError;
pub(crate) use diff_application::{apply_edits, FileReadResult};
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
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    AIAgentOutputMessage, AIAgentOutputMessageType, AIIdentifiers, RequestFileEditsResult,
};
use crate::ai::blocklist::diff_storage::{
    HeadlessDiffStorage, HeadlessDiffStorageModel, RegisteredDiffStorage,
};
use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff};
use crate::ai::blocklist::{BlocklistAIPermissions, RequestedEditResolution};
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::SessionType;
use crate::{safe_warn, BlocklistAIHistoryModel};

/// Per-action state carried from `preprocess_action` to `execute`.
enum PendingFileEdits {
    /// The storage surface that owns the diffs while the action is pending.
    /// Registered by a review surface (GUI/TUI), or a headless placeholder
    /// created by the executor when diffs resolve with no surface registered.
    Storage(Box<dyn RegisteredDiffStorage>),
    /// Diff application failed during preprocess; `execute` reports it to the LLM.
    Failed(Vec1<DiffApplicationError>),
}

pub struct RequestFileEditsExecutor {
    active_session: ModelHandle<ActiveSession>,
    apply_diff_model: ModelHandle<ApplyDiffModel>,
    /// Per-action state produced by preprocess and consumed by execute.
    pending_file_edits: HashMap<AIAgentActionId, PendingFileEdits>,
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
            pending_file_edits: HashMap::new(),
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
        if matches!(
            self.pending_file_edits.get(&input.action.id),
            Some(PendingFileEdits::Failed(_))
        ) {
            return true;
        }

        BlocklistAIPermissions::as_ref(ctx)
            .can_write_files(&conversation_id, &paths, Some(self.terminal_view_id), ctx)
            .is_allowed()
    }

    /// Registers the storage surface that owns an action's diffs.
    ///
    /// May be called before or after preprocess resolves the diffs: when a
    /// placeholder storage (the headless fallback) already holds prepared
    /// diffs, they are handed to the newly registered surface. An existing
    /// storage that does not relinquish its diffs (a review surface, or a
    /// placeholder already saving) stays registered and the new registration
    /// is dropped.
    pub fn register_requested_edits(
        &mut self,
        action_id: &AIAgentActionId,
        storage: Box<dyn RegisteredDiffStorage>,
        ctx: &mut ModelContext<Self>,
    ) {
        match self.pending_file_edits.get(action_id) {
            None => {
                self.pending_file_edits
                    .insert(action_id.clone(), PendingFileEdits::Storage(storage));
            }
            Some(PendingFileEdits::Storage(existing)) => {
                if let Some((diffs, session_type)) = existing.take_candidate_diffs(ctx) {
                    storage.set_candidate_diffs(diffs, session_type, ctx);
                    self.pending_file_edits
                        .insert(action_id.clone(), PendingFileEdits::Storage(storage));
                }
            }
            Some(PendingFileEdits::Failed(_)) => {}
        }
    }

    /// Drops any per-action state for a cancelled or rejected action so
    /// prepared file contents don't outlive the action.
    pub(super) fn discard_pending(&mut self, action_id: &AIAgentActionId) {
        self.pending_file_edits.remove(action_id);
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

        let result_future = match self.pending_file_edits.get(id) {
            // The storage surface persists its (possibly user-edited) diffs and
            // resolves with the assembled result. The entry stays registered
            // until the action's terminal result funnels through
            // `discard_pending`.
            Some(PendingFileEdits::Storage(storage)) => storage.accept_and_save(ctx),
            Some(PendingFileEdits::Failed(errors)) => {
                return ActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                    RequestFileEditsResult::DiffApplicationFailed {
                        error: DiffApplicationError::error_for_conversation(errors),
                    },
                ));
            }
            None => {
                log::warn!("Tried to execute a RequestFileEdits action without prepared diffs");
                return ActionExecution::NotReady;
            }
        };

        let identifiers = self
            .generate_ai_identifiers(&input.conversation_id, id, ctx)
            .unwrap_or_else(|| AIIdentifiers {
                client_conversation_id: Some(input.conversation_id),
                ..Default::default()
            });
        let passive_diff = BlocklistAIHistoryModel::as_ref(ctx)
            .is_entirely_passive_conversation(&input.conversation_id);

        ActionExecution::new_async(result_future, move |result, ctx| {
            if let RequestFileEditsResult::Success {
                updated_files,
                lines_added,
                lines_removed,
                ..
            } = &result
            {
                send_telemetry_from_ctx!(
                    RequestFileEditsTelemetryEvent::EditResolved(EditResolvedEvent {
                        identifiers: identifiers.clone(),
                        response: RequestedEditResolution::Accept,
                        stats: EditStats {
                            files_edited: updated_files.len(),
                            lines_added: *lines_added,
                            lines_removed: *lines_removed,
                        },
                        passive_diff,
                    }),
                    ctx
                );
            }
            AIAgentActionResultType::RequestFileEdits(result)
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

        let applied_diffs = match applied_diffs {
            Ok(diffs) if !diffs.is_empty() => diffs,
            Ok(_) => {
                // We didn't generate any diffs--consider this a failure.
                log::warn!("No diffs generated");
                self.pending_file_edits.insert(
                    id,
                    PendingFileEdits::Failed(vec1![DiffApplicationError::EmptyDiff]),
                );
                return;
            }
            Err(err) => {
                safe_warn!(
                    safe: ("Failed to generate diffs"),
                    full: ("Failed to generate diffs {err:?}")
                );
                self.pending_file_edits
                    .insert(id, PendingFileEdits::Failed(err));
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
            diffs.push(FileDiff::new(diff.original_content, path, diff.diff_type));
        }

        // Seed the registered storage surface with the resolved diffs; when no
        // surface has registered yet (autoexecution racing view creation, or a
        // headless conversation), create the headless placeholder so the action
        // stays executable. A surface registering later takes the diffs over
        // via `register_requested_edits`.
        let session_type = self.resolve_diff_session_type(ctx);
        match self.pending_file_edits.get(&id) {
            Some(PendingFileEdits::Storage(storage)) => {
                storage.set_candidate_diffs(diffs, session_type, ctx);
            }
            Some(PendingFileEdits::Failed(_)) | None => {
                let model =
                    ctx.add_model(|ctx| HeadlessDiffStorageModel::new(diffs, session_type, ctx));
                self.pending_file_edits.insert(
                    id,
                    PendingFileEdits::Storage(Box::new(HeadlessDiffStorage(model))),
                );
            }
        }
    }

    /// Resolves whether file writes target the local filesystem or a remote host.
    fn resolve_diff_session_type(&self, ctx: &mut ModelContext<Self>) -> DiffSessionType {
        match self.active_session.as_ref(ctx).session_type(ctx) {
            Some(SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            }) => DiffSessionType::Remote(host_id.clone()),
            _ => DiffSessionType::Local,
        }
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

impl Entity for RequestFileEditsExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "request_file_edits_tests.rs"]
mod tests;
