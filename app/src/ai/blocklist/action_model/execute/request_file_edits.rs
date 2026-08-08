mod apply_diff_model;
mod diff_application;
mod telemetry;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use ai::diff_validation::{AIRequestedCodeDiff, DiffType};
use apply_diff_model::ApplyDiffModel;
use diff_application::{AppliedEdits, DiffApplicationError};
pub(crate) use diff_application::{FileReadResult, apply_edits};
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
use crate::ai::blocklist::observed_file_contents::{ContentFingerprint, ObservedFileContents};
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
    /// Set of action IDs where diff application failed.
    diff_application_failures: HashMap<AIAgentActionId, Vec1<DiffApplicationError>>,
    /// Apply-time notices per pending action, surfaced in the success result.
    pending_notes: HashMap<AIAgentActionId, Vec<String>>,
    /// Whole-file contents the model authored per pending action (creations and full
    /// replacements), recorded as observed once the action succeeds without user edits.
    pending_authored_contents: HashMap<AIAgentActionId, Vec<(String, ContentFingerprint)>>,
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
            pending_notes: HashMap::new(),
            pending_authored_contents: HashMap::new(),
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
        self.pending_notes.remove(action_id);
        self.pending_authored_contents.remove(action_id);
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

        // If diff application failed, early exit.
        if let Some(errors) = self.diff_application_failures.remove(id) {
            return ActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed {
                    error: DiffApplicationError::error_for_conversation(&errors),
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

        let identifiers = self
            .generate_ai_identifiers(&input.conversation_id, id, ctx)
            .unwrap_or_else(|| AIIdentifiers {
                client_conversation_id: Some(input.conversation_id),
                ..Default::default()
            });
        let passive_diff = BlocklistAIHistoryModel::as_ref(ctx)
            .is_entirely_passive_conversation(&input.conversation_id);

        let conversation_id = input.conversation_id;
        let apply_notes = self.pending_notes.remove(id).unwrap_or_default();
        let authored_contents = self
            .pending_authored_contents
            .remove(id)
            .unwrap_or_default();

        ActionExecution::new_async(result_future, move |mut result, ctx| {
            if let RequestFileEditsResult::Success {
                updated_files,
                lines_added,
                lines_removed,
                notes,
                ..
            } = &mut result
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

                notes.extend(apply_notes);

                // A file whose entire content the model authored is now observed content —
                // unless the user hand-edited it during review, in which case the model no
                // longer knows what the file holds.
                let user_edited_paths: HashSet<&str> = updated_files
                    .iter()
                    .filter(|file| file.was_edited_by_user)
                    .map(|file| file.file_context.file_name.as_str())
                    .collect();
                let authored_contents: Vec<(String, ContentFingerprint)> = authored_contents
                    .into_iter()
                    .filter(|(path, _)| !user_edited_paths.contains(path.as_str()))
                    .collect();
                if !authored_contents.is_empty() {
                    ObservedFileContents::handle(ctx).update(ctx, |model, _| {
                        for (path, fingerprint) in authored_contents {
                            model.record(conversation_id, path, fingerprint);
                        }
                    });
                }
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
        applied_diffs: Result<AppliedEdits, Vec1<DiffApplicationError>>,
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

        let applied_edits = match applied_diffs {
            Ok(applied) if !applied.diffs.is_empty() => applied,
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

        let mut diffs = Vec::with_capacity(applied_edits.diffs.len());
        let mut authored_contents = Vec::new();
        for diff in applied_edits.diffs {
            let path = host_native_absolute_path(
                diff.file_name.as_str(),
                &shell_launch_data,
                &current_working_directory,
            );
            if let Some(content) = whole_file_authored_content(&diff) {
                authored_contents.push((path.clone(), ContentFingerprint::of(content)));
                // The save path may normalize a missing trailing newline, so treat both
                // variants as authored.
                if !content.ends_with('\n') {
                    authored_contents.push((
                        path.clone(),
                        ContentFingerprint::of(&format!("{content}\n")),
                    ));
                }
            }
            let file_diff = FileDiff::new(diff.original_content, path, diff.diff_type);
            diffs.push(file_diff);
        }
        if !applied_edits.notes.is_empty() {
            self.pending_notes.insert(id.clone(), applied_edits.notes);
        }
        if !authored_contents.is_empty() {
            self.pending_authored_contents
                .insert(id.clone(), authored_contents);
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

/// The complete file content the model authored in `diff`, when the diff writes the whole
/// file: a creation, or a full replacement (a single delta spanning every line of the
/// original).
fn whole_file_authored_content(diff: &AIRequestedCodeDiff) -> Option<&str> {
    match &diff.diff_type {
        DiffType::Create { delta } => Some(&delta.insertion),
        DiffType::Update {
            deltas,
            rename: None,
        } => match deltas.as_slice() {
            [delta]
                if is_full_file_range(&delta.replacement_line_range, &diff.original_content) =>
            {
                Some(&delta.insertion)
            }
            _ => None,
        },
        DiffType::Update { .. } | DiffType::Delete { .. } => None,
    }
}

fn is_full_file_range(range: &Range<usize>, original_content: &str) -> bool {
    let line_count = original_content.lines().count();
    if line_count == 0 {
        *range == (0..0)
    } else {
        *range == (1..line_count.saturating_add(1))
    }
}

impl Entity for RequestFileEditsExecutor {
    type Event = ();
}

#[cfg(test)]
#[path = "request_file_edits_tests.rs"]
mod tests;
