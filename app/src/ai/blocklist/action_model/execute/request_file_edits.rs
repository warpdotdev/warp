mod apply_diff_model;
mod diff_application;
mod telemetry;

use std::collections::HashMap;
use std::path::PathBuf;

use apply_diff_model::ApplyDiffModel;
pub(crate) use diff_application::{ApplyEditsOutcome, FileReadResult, apply_edits};
use diff_application::{DiffApplicationError, errors_to_conversation_message};
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use itertools::Itertools;
pub(crate) use telemetry::MalformedFinalLineProxyEvent;
#[allow(unused_imports)]
pub use telemetry::{EditAcceptAndContinueClickedEvent, EditAcceptClickedEvent};
pub use telemetry::{
    EditReceivedEvent, EditResolvedEvent, EditStats, RequestFileEditsFormatKind,
    RequestFileEditsTelemetryEvent,
};
use vec1::{Vec1, vec1};
use warp_core::send_telemetry_from_ctx;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    AIAgentOutputMessage, AIAgentOutputMessageType, AIIdentifiers, RequestFileEditsResult,
};
use crate::ai::blocklist::diff_storage::RegisteredDiffStorage;
use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff};
use crate::ai::blocklist::{BlocklistAIPermissions, RequestedEditResolution};
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::SessionType;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::{BlocklistAIHistoryModel, safe_warn};

pub struct RequestFileEditsExecutor {
    active_session: ModelHandle<ActiveSession>,
    apply_diff_model: ModelHandle<ApplyDiffModel>,
    /// The registered diff storage surface for each pending action.
    diff_storages: HashMap<AIAgentActionId, Box<dyn RegisteredDiffStorage>>,
    /// Set of action IDs where diff application failed completely (no diffs were applied).
    diff_application_failures: HashMap<AIAgentActionId, Vec1<DiffApplicationError>>,
    /// Error message for actions where some files applied successfully but others failed.
    /// Unlike `diff_application_failures`, storage has been seeded with the successful
    /// diffs; this message is appended to the result after the user accepts them.
    diff_application_partial_errors: HashMap<AIAgentActionId, String>,
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
            diff_storages: HashMap::new(),
            diff_application_failures: HashMap::new(),
            diff_application_partial_errors: HashMap::new(),
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
        //
        // Note: partial-success batches are NOT auto-executed here — they contain successful diffs
        // that still need to go through the normal `can_write_files` / user-acceptance gate.
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

    /// Registers the diff storage surface that handles a RequestFileEdits action.
    /// Note this MUST be called before `execute` or `preprocess_action` is invoked in
    /// order for the necessary state to be set to handle the action.
    pub fn register_requested_edits(
        &mut self,
        action_id: &AIAgentActionId,
        storage: Box<dyn RegisteredDiffStorage>,
    ) {
        self.diff_storages.insert(action_id.clone(), storage);
    }

    /// Drops any per-action state for a cancelled or rejected action so
    /// prepared file contents don't outlive the action.
    pub(super) fn discard_pending(&mut self, action_id: &AIAgentActionId) {
        self.diff_storages.remove(action_id);
        self.diff_application_failures.remove(action_id);
        self.diff_application_partial_errors.remove(action_id);
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> + use<> {
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

        // If diff application failed completely (no files were applied), early exit so the
        // model receives the error immediately without waiting for user interaction.
        if let Some(errors) = self.diff_application_failures.remove(id) {
            return ActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed {
                    error: errors_to_conversation_message(&errors),
                },
            ));
        }

        // The storage surface persists its (possibly user-edited) diffs and
        // resolves with the assembled result. The entry stays registered until
        // the action's terminal result funnels through `discard_pending`.
        let Some(storage) = self.diff_storages.get(id) else {
            log::warn!("Tried to execute a RequestFileEdits action without a registered storage");
            return ActionExecution::NotReady;
        };
        let result_future = storage.accept_and_save(ctx);

        // Capture partial errors so we can append them to the result after the successful
        // diffs have been accepted by the user.
        let partial_error_msg = self.diff_application_partial_errors.remove(id);

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

            // For a partial-success batch: some files were applied (shown to the user above)
            // but others failed. Surface the partial-failure notice through the dedicated
            // `partial_errors` field on Success rather than appending it to `diff`. The
            // `diff` field is a legacy field that is zeroed on conversation reload and is
            // rendered inside a ```diff fence by the driver output formatter; `partial_errors`
            // survives the round-trip and is displayed as plain prose by the Display impl.
            // When accept_and_save returned non-Success (e.g. Cancelled), fall through.
            if let Some(error_msg) = partial_error_msg
                && let RequestFileEditsResult::Success {
                    diff,
                    updated_files,
                    deleted_files,
                    lines_added,
                    lines_removed,
                    ..
                } = result
            {
                return AIAgentActionResultType::RequestFileEdits(
                    RequestFileEditsResult::Success {
                        diff,
                        updated_files,
                        deleted_files,
                        lines_added,
                        lines_removed,
                        partial_errors: Some(error_msg),
                    },
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
                let outcome = apply_future.await;
                (outcome, id, tx)
            },
            |me, (outcome, id, tx), ctx| {
                me.on_diffs_applied(outcome, id, tx, ctx);
            },
        );

        async {
            rx.await.ok();
        }
        .boxed()
    }

    fn on_diffs_applied(
        &mut self,
        outcome: ApplyEditsOutcome,
        id: AIAgentActionId,
        tx: oneshot::Sender<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        tx.send(()).ok();

        // Expected when the action reached a terminal result (e.g. was
        // cancelled) mid-apply and its storage was discarded; a storage that
        // was never registered still warns at execute time.
        let Some(storage) = self.diff_storages.get(&id) else {
            log::info!("No registered storage for RequestFileEdits action at apply completion");
            return;
        };

        let ApplyEditsOutcome {
            applied_diffs,
            errors,
        } = outcome;

        if applied_diffs.is_empty() && errors.is_empty() {
            // We didn't generate any diffs and had no errors--consider this a failure.
            log::warn!("No diffs generated");
            self.diff_application_failures
                .insert(id, vec1![DiffApplicationError::EmptyDiff]);
            return;
        }

        if applied_diffs.is_empty() {
            // Every file in the batch failed; report all errors immediately.
            let Some(errors_nonempty) = Vec1::try_from_vec(errors).ok() else {
                // Should not happen given the check above, but be safe.
                self.diff_application_failures
                    .insert(id, vec1![DiffApplicationError::EmptyDiff]);
                return;
            };
            safe_warn!(
                safe: ("Failed to generate diffs"),
                full: ("Failed to generate diffs {errors_nonempty:?}")
            );
            self.diff_application_failures.insert(id, errors_nonempty);
            return;
        }

        // At least some files applied successfully. If there were also errors
        // (partial-success batch), build a combined message so the model knows
        // which files succeeded and which to retry.
        if !errors.is_empty() {
            let succeeded_names: Vec<&str> =
                applied_diffs.iter().map(|d| d.file_name.as_str()).collect();
            let success_prefix = format!(
                "Applied edits to {} successfully. ",
                succeeded_names.join(", ")
            );
            let error_details = DiffApplicationError::errors_to_conversation_message(&errors);
            self.diff_application_partial_errors
                .insert(id.clone(), format!("{success_prefix}{error_details}"));
        }

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

        // Set the session type so save/delete/create routes through the
        // correct FileModel backend.
        let diff_session_type = match self.active_session.as_ref(ctx).session_type(ctx) {
            Some(SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            }) => DiffSessionType::Remote(host_id.clone()),
            _ => DiffSessionType::Local,
        };

        storage.set_candidate_diffs(diffs, diff_session_type, ctx);
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
