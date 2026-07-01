mod apply_diff_model;
mod diff_application;
mod persist_diff_model;
mod telemetry;

use std::collections::HashMap;
use std::path::PathBuf;

use ai::diff_validation::{AIRequestedCodeDiff, DiffDelta, DiffType};
use apply_diff_model::ApplyDiffModel;
use diff_application::DiffApplicationError;
pub(crate) use diff_application::{apply_edits, FileReadResult};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::FutureExt;
use itertools::Itertools;
pub(crate) use persist_diff_model::{PersistDiffModel, ResolvedFileEdit};
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
    AIAgentOutputMessage, AIAgentOutputMessageType, AIIdentifiers, AnyFileContent, FileContext,
    FileLocations, RequestFileEditsResult, UpdatedFileContext,
};
use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff};
use crate::ai::blocklist::{BlocklistAIPermissions, RequestedEditResolution};
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::SessionType;
use crate::{safe_warn, BlocklistAIHistoryModel};
const APPLY_DIFF_RESULT_CONTEXT_LINES: usize = 10;

/// Events emitted by the file-edits executor for review surfaces to observe.
pub enum RequestFileEditsExecutorEvent {
    /// A `RequestFileEdits` action's diffs have been resolved and are ready for a
    /// review surface to display via [`RequestFileEditsExecutor::prepared_diffs`].
    DiffsPrepared(AIAgentActionId),
}

/// Per-action state carried from `preprocess_action` to `execute`.
enum PendingFileEdits {
    /// Diffs resolved and ready to persist. `reviewed` holds GUI-supplied final
    /// content per file (keyed by path) when a review surface edited/accepted
    /// them; `None` on headless surfaces, where final content is derived from
    /// the diff's deltas.
    Prepared {
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        reviewed: Option<Vec<(String, String)>>,
    },
    /// Diff application failed during preprocess; `execute` reports it to the LLM.
    Failed(Vec1<DiffApplicationError>),
}

pub struct RequestFileEditsExecutor {
    active_session: ModelHandle<ActiveSession>,
    apply_diff_model: ModelHandle<ApplyDiffModel>,
    persist_diff_model: ModelHandle<PersistDiffModel>,
    /// Per-action state produced by preprocess and consumed by execute.
    pending: HashMap<AIAgentActionId, PendingFileEdits>,
    terminal_view_id: EntityId,
}

impl RequestFileEditsExecutor {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let apply_diff_model = ctx.add_model(|_| ApplyDiffModel::new(active_session.clone()));
        let persist_diff_model = ctx.add_model(PersistDiffModel::new);
        Self {
            active_session,
            apply_diff_model,
            persist_diff_model,
            pending: HashMap::new(),
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
            self.pending.get(&input.action.id),
            Some(PendingFileEdits::Failed(_))
        ) {
            return true;
        }

        BlocklistAIPermissions::as_ref(ctx)
            .can_write_files(&conversation_id, &paths, Some(self.terminal_view_id), ctx)
            .is_allowed()
    }

    /// Records the GUI-reviewed final content for an action before it executes.
    /// Keyed by file path; consumed by `execute` on the review (GUI) path.
    pub fn set_reviewed_content(
        &mut self,
        action_id: &AIAgentActionId,
        files: Vec<(String, String)>,
    ) {
        if let Some(PendingFileEdits::Prepared { reviewed, .. }) = self.pending.get_mut(action_id) {
            *reviewed = Some(files);
        }
    }

    /// Returns the prepared diffs and session backend for an action so a review
    /// surface can display them. `None` if the action is not prepared.
    pub fn prepared_diffs(
        &self,
        action_id: &AIAgentActionId,
    ) -> Option<(Vec<FileDiff>, DiffSessionType)> {
        match self.pending.get(action_id) {
            Some(PendingFileEdits::Prepared {
                diffs,
                session_type,
                ..
            }) => Some((diffs.clone(), session_type.clone())),
            _ => None,
        }
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

        let (diffs, session_type, reviewed) = match self.pending.remove(id) {
            Some(PendingFileEdits::Prepared {
                diffs,
                session_type,
                reviewed,
            }) => (diffs, session_type, reviewed),
            Some(PendingFileEdits::Failed(errors)) => {
                return ActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                    RequestFileEditsResult::DiffApplicationFailed {
                        error: DiffApplicationError::error_for_conversation(&errors),
                    },
                ));
            }
            None => {
                log::warn!("Tried to execute a RequestFileEdits action without prepared diffs");
                return ActionExecution::NotReady;
            }
        };

        // GUI review supplies final content per path; headless applies the diff deltas.
        let reviewed: HashMap<String, String> = reviewed.unwrap_or_default().into_iter().collect();
        let resolved = match build_resolved_edits(diffs, &reviewed) {
            Ok(resolved) => resolved,
            Err(error) => {
                return ActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                    RequestFileEditsResult::DiffApplicationFailed { error },
                ));
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

        let result_future = self
            .persist_diff_model
            .update(ctx, |model, ctx| model.persist(resolved, session_type, ctx));

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
                self.pending.insert(
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
                self.pending.insert(id, PendingFileEdits::Failed(err));
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

        let session_type = self.resolve_diff_session_type(ctx);
        self.pending.insert(
            id.clone(),
            PendingFileEdits::Prepared {
                diffs,
                session_type,
                reviewed: None,
            },
        );
        ctx.emit(RequestFileEditsExecutorEvent::DiffsPrepared(id));
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

/// Builds resolved file edits, using GUI-reviewed content per path when present
/// and otherwise applying the diff's deltas to the base content.
fn build_resolved_edits(
    diffs: Vec<FileDiff>,
    reviewed: &HashMap<String, String>,
) -> Result<Vec<ResolvedFileEdit>, String> {
    let mut resolved = Vec::with_capacity(diffs.len());
    for diff in diffs {
        let path = diff.file_path();
        let base_content = diff.base.content;
        let op = diff.diff_type;
        let final_content = match reviewed.get(&path) {
            Some(content) => content.clone(),
            None => final_content_from_op(&base_content, &op)?,
        };
        resolved.push(ResolvedFileEdit {
            path,
            base_content,
            op,
            final_content,
        });
    }
    Ok(resolved)
}

/// Derives the final on-disk content for a diff from its base content and deltas.
fn final_content_from_op(base_content: &str, op: &DiffType) -> Result<String, String> {
    match op {
        DiffType::Create { delta } => Ok(delta.insertion.clone()),
        DiffType::Update { deltas, .. } => apply_deltas_to_content(base_content, deltas),
        DiffType::Delete { .. } => Ok(String::new()),
    }
}

/// Applies line-range replacement deltas to `content`, producing the new content.
fn apply_deltas_to_content(content: &str, deltas: &[DiffDelta]) -> Result<String, String> {
    let mut lines = split_lines_preserving_newlines(content);
    let mut deltas = deltas.to_vec();
    deltas.sort_by_key(|delta| delta.replacement_line_range.start);

    for delta in deltas.into_iter().rev() {
        let start = delta.replacement_line_range.start.saturating_sub(1);
        let end = delta.replacement_line_range.end.saturating_sub(1);
        if start > lines.len() || end > lines.len() || start > end {
            return Err(format!(
                "Diff range {:?} is out of bounds for file with {} lines",
                delta.replacement_line_range,
                lines.len()
            ));
        }
        let replacement = split_lines_preserving_newlines(&delta.insertion);
        lines.splice(start..end, replacement);
    }

    Ok(lines.concat())
}

/// Splits content into lines while keeping trailing newlines, so reassembly via
/// `concat` reproduces the original byte-for-byte.
fn split_lines_preserving_newlines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.split_inclusive('\n').map(str::to_string).collect()
    }
}

impl Entity for RequestFileEditsExecutor {
    type Event = RequestFileEditsExecutorEvent;
}

#[cfg(test)]
#[path = "request_file_edits_tests.rs"]
mod tests;
