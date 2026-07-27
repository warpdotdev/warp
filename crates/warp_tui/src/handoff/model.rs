//! State and execution model for the TUI local-to-cloud handoff flow.
//!
//! The terminal session supplies its concrete terminal/controller handles once
//! at preparation time. After that, this model owns handoff data, catalog
//! subscriptions, validation, asynchronous execution, and lifecycle outcomes.
//! [`super::block::TuiHandoffBlock`] only presents and edits this state.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::settings::{AISettings, PrivacySettings, PrivacySettingsChangedEvent};
use warp::tui_export::{
    AIConversationId, AISettingsChangedEvent, AttachmentInput, BlocklistAIContextModel,
    BlocklistAIController, BlocklistAIHistoryModel, CloudEnvironmentCatalog, HandoffCommitOutcome,
    HandoffEntryPoint, HandoffLaunchAttachments, HandoffPrepareError, HandoffPrepareInput,
    HandoffRestoration, HandoffSurface, LLMId, LLMPreferences, LLMPreferencesEvent, OptionRow,
    OptionSnapshot, OptionSourceStatus, PendingCloudLaunch, PendingHandoff, ServerApiProvider,
    SnapshotUploadTarget, TerminalModel, UserWorkspaces, UserWorkspacesEvent, execute_handoff,
    oz_model_snapshot, prepare_handoff, suggest_handoff_environment,
};
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _};

/// Editable selector pages in their handoff configuration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiHandoffSelectorKind {
    Environment,
    Model,
}

impl TuiHandoffSelectorKind {
    /// User-facing question shown above this selector page.
    pub(crate) fn question(self) -> &'static str {
        match self {
            Self::Environment => "Which environment should run this conversation?",
            Self::Model => "Which model should run this conversation?",
        }
    }
}

/// Editable presentation state for a prepared handoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiHandoffEditableState {
    Acceptance { validation_error: Option<String> },
    Configuring { page: TuiHandoffSelectorKind },
}

/// Model-owned lifecycle state for one handoff.
pub(crate) enum TuiHandoffPhase {
    Editable {
        state: TuiHandoffEditableState,
        pending: Box<PendingHandoff>,
    },
    Committed {
        operation_id: u64,
    },
    Created {
        url: String,
    },
    Persisted {
        url: String,
    },
}

/// Outcomes that require the owning terminal session to change surfaces.
#[derive(Clone)]
pub(crate) enum TuiHandoffModelEvent {
    Changed {
        focus_block: bool,
    },
    Cancelled(Option<HandoffRestoration>),
    Failed {
        restoration: Option<HandoffRestoration>,
        message: String,
    },
    ContinueLocally,
    StartNewConversation,
}

/// Preparation failure already reduced to the local input and message the TUI
/// should display.
pub(crate) struct TuiHandoffPreparationFailure {
    replacement_input: Option<String>,
    message: String,
}

impl TuiHandoffPreparationFailure {
    pub(crate) fn into_parts(self) -> (Option<String>, String) {
        (self.replacement_input, self.message)
    }
}

/// Model backing one TUI handoff card.
pub(crate) struct TuiHandoffModel {
    source_conversation_id: Option<AIConversationId>,
    phase: TuiHandoffPhase,
    environments: ModelHandle<CloudEnvironmentCatalog>,
    forked_existing_conversation: bool,
    next_operation_id: u64,
    dismissed: bool,
}

impl TuiHandoffModel {
    /// Prepares a handoff and registers its retained model.
    pub(crate) fn new(
        terminal_surface_id: EntityId,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        controller: ModelHandle<BlocklistAIController>,
        context: ModelHandle<BlocklistAIContextModel>,
        current_working_directory: Option<String>,
        argument: Option<String>,
        ctx: &mut AppContext,
    ) -> Result<ModelHandle<Self>, TuiHandoffPreparationFailure> {
        if !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx) {
            return Err(TuiHandoffPreparationFailure {
                replacement_input: None,
                message: "Cloud handoff is unavailable.".to_owned(),
            });
        }

        let history = BlocklistAIHistoryModel::handle(ctx);
        let selected_conversation = history.as_ref(ctx).active_conversation(terminal_surface_id);
        let source_model_id = selected_conversation
            .and_then(|conversation| conversation.latest_exchange())
            .map(|exchange| exchange.model_id.to_string());
        let source_conversation_id = selected_conversation.map(|conversation| conversation.id());
        let source_was_active = selected_conversation.is_some_and(|conversation| {
            conversation.status().is_in_progress() || conversation.status().is_blocked()
        });
        let has_long_running_command = terminal_model
            .lock()
            .block_list()
            .active_block()
            .is_active_and_long_running();
        if has_long_running_command {
            return Err(Self::preparation_failure(
                HandoffPrepareError::LongRunningCommand,
                source_was_active,
                argument.as_ref(),
            ));
        }

        let launch = PendingCloudLaunch {
            prompt: argument.clone().unwrap_or_default(),
            attachments: Self::collect_attachments(&context, ctx),
        };
        let provider = ServerApiProvider::as_ref(ctx);
        let pending = prepare_handoff(
            HandoffPrepareInput::new(
                terminal_surface_id,
                history,
                controller,
                context,
                SnapshotUploadTarget::Local {
                    ai_client: provider.get_ai_client(),
                    http: provider.get_http_client(),
                },
                HandoffEntryPoint::SlashCommand,
                HandoffSurface::Tui,
            )
            .with_expected_conversation_id(source_conversation_id)
            .with_current_working_directory(current_working_directory.clone())
            .with_long_running_command(has_long_running_command)
            .with_launch(Some(launch))
            .with_environment_required(true),
            ctx,
        )
        .map_err(|error| Self::preparation_failure(error, source_was_active, argument.as_ref()))?;
        let mut pending = pending;
        if let Some(source_model_id) = source_model_id {
            pending.set_model_id(source_model_id, false, ctx);
        }

        Ok(ctx.add_model(move |ctx: &mut ModelContext<Self>| {
            let environments = CloudEnvironmentCatalog::handle(ctx);
            ctx.subscribe_to_model(&environments, |model, _, _, ctx| {
                model.handle_environment_change(ctx);
            });
            ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |_, _, event, ctx| {
                if matches!(event, LLMPreferencesEvent::UpdatedAvailableLLMs) {
                    ctx.emit(TuiHandoffModelEvent::Changed { focus_block: false });
                    ctx.notify();
                }
            });
            ctx.subscribe_to_model(
                &AISettings::handle(ctx),
                |model, _, _: &AISettingsChangedEvent, ctx| {
                    if model.is_editable() && !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)
                    {
                        model.cancel(ctx);
                    }
                },
            );
            ctx.subscribe_to_model(&PrivacySettings::handle(ctx), |model, _, event, ctx| {
                if matches!(
                    event,
                    PrivacySettingsChangedEvent::UpdateIsCloudConversationStorageEnabled { .. }
                ) && model.is_editable()
                    && !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)
                {
                    model.cancel(ctx);
                }
            });
            ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |model, _, event, ctx| {
                if matches!(event, UserWorkspacesEvent::TeamsChanged)
                    && model.is_editable()
                    && !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)
                {
                    model.cancel(ctx);
                }
            });

            let forked_existing_conversation =
                pending.presentation_snapshot().forked_existing_conversation;
            let mut model = Self {
                source_conversation_id,
                phase: TuiHandoffPhase::Editable {
                    state: TuiHandoffEditableState::Acceptance {
                        validation_error: None,
                    },
                    pending: Box::new(pending),
                },
                environments,
                forked_existing_conversation,
                next_operation_id: 0,
                dismissed: false,
            };
            model.refresh_pending_environments(ctx);

            if let Some(path) = current_working_directory.map(PathBuf::from) {
                let suggestion = suggest_handoff_environment(path, ctx);
                ctx.spawn(suggestion, |model, environment_id, ctx| {
                    if !model.is_editable() || model.dismissed {
                        return;
                    }
                    if let Some(environment_id) = environment_id
                        && let Some(pending) = model.pending_mut()
                    {
                        pending.set_environment_id(Some(environment_id), false);
                        ctx.emit(TuiHandoffModelEvent::Changed { focus_block: false });
                        ctx.notify();
                    }
                });
            }
            model
        }))
    }

    fn collect_attachments(
        context: &ModelHandle<BlocklistAIContextModel>,
        ctx: &AppContext,
    ) -> HandoffLaunchAttachments {
        let context = context.as_ref(ctx);
        let request_attachments = context
            .pending_images()
            .into_iter()
            .map(|image| AttachmentInput {
                file_name: image.file_name.clone(),
                mime_type: image.mime_type.clone(),
                data: image.data.clone(),
            })
            .collect();
        HandoffLaunchAttachments {
            request_attachments,
            display_attachments: context.pending_attachments().to_vec(),
        }
    }

    fn preparation_failure(
        error: HandoffPrepareError,
        source_was_active: bool,
        argument: Option<&String>,
    ) -> TuiHandoffPreparationFailure {
        let replacement_input = (source_was_active
            && matches!(error, HandoffPrepareError::MissingServerConversationToken))
        .then(|| {
            argument
                .map(|argument| argument.trim())
                .unwrap_or_default()
                .to_owned()
        });
        TuiHandoffPreparationFailure {
            replacement_input,
            message: Self::prepare_error_message(&error).to_owned(),
        }
    }

    fn prepare_error_message(error: &HandoffPrepareError) -> &'static str {
        match error {
            HandoffPrepareError::LongRunningCommand => {
                "Can't hand off while a command is running. Cancel it or wait for it to finish."
            }
            HandoffPrepareError::ActiveOrBlockedChild => {
                "Can't hand off while child work is active or waiting for input."
            }
            HandoffPrepareError::EmptySourceAndPrompt => {
                "Nothing to hand off — start a conversation or add a prompt."
            }
            HandoffPrepareError::MissingServerConversationToken => {
                "This conversation hasn't synced yet. Send another message, then try again."
            }
            HandoffPrepareError::InvalidModel => "The selected model can't run in Oz cloud.",
            HandoffPrepareError::SourceConversationChanged
            | HandoffPrepareError::SourceNotInProgress
            | HandoffPrepareError::HandoffDisabled
            | HandoffPrepareError::MissingRequiredEnvironment
            | HandoffPrepareError::InvalidEnvironment => {
                "Couldn't start the handoff. Check the current conversation and try again."
            }
        }
    }

    fn validation_message(error: &HandoffPrepareError) -> &'static str {
        match error {
            HandoffPrepareError::MissingRequiredEnvironment => {
                "Select an environment before starting the handoff."
            }
            HandoffPrepareError::InvalidEnvironment => {
                "The selected environment is no longer available."
            }
            HandoffPrepareError::InvalidModel => {
                "The selected model cannot run in Oz cloud. Choose a compatible model."
            }
            HandoffPrepareError::HandoffDisabled => "Cloud handoff is no longer available.",
            HandoffPrepareError::SourceConversationChanged
            | HandoffPrepareError::EmptySourceAndPrompt
            | HandoffPrepareError::SourceNotInProgress
            | HandoffPrepareError::LongRunningCommand
            | HandoffPrepareError::ActiveOrBlockedChild
            | HandoffPrepareError::MissingServerConversationToken => {
                "The handoff can no longer start. Return to local input and try again."
            }
        }
    }

    pub(crate) fn phase(&self) -> &TuiHandoffPhase {
        &self.phase
    }

    pub(crate) fn source_conversation_id(&self) -> Option<AIConversationId> {
        self.source_conversation_id
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.dismissed && !matches!(self.phase, TuiHandoffPhase::Persisted { .. })
    }

    pub(crate) fn is_editable(&self) -> bool {
        matches!(self.phase, TuiHandoffPhase::Editable { .. })
    }

    pub(crate) fn no_environments(&self, ctx: &AppContext) -> bool {
        self.environments.as_ref(ctx).environments().is_empty()
    }

    pub(crate) fn forked_existing_conversation(&self) -> bool {
        self.forked_existing_conversation
    }

    pub(crate) fn validation_error(&self) -> Option<&str> {
        match &self.phase {
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { validation_error },
                ..
            } => validation_error.as_deref(),
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Configuring { .. },
                ..
            }
            | TuiHandoffPhase::Committed { .. }
            | TuiHandoffPhase::Created { .. }
            | TuiHandoffPhase::Persisted { .. } => None,
        }
    }

    pub(crate) fn url(&self) -> Option<&str> {
        match &self.phase {
            TuiHandoffPhase::Created { url } | TuiHandoffPhase::Persisted { url } => Some(url),
            TuiHandoffPhase::Editable { .. } | TuiHandoffPhase::Committed { .. } => None,
        }
    }

    fn pending(&self) -> Option<&PendingHandoff> {
        match &self.phase {
            TuiHandoffPhase::Editable { pending, .. } => Some(pending.as_ref()),
            TuiHandoffPhase::Committed { .. }
            | TuiHandoffPhase::Created { .. }
            | TuiHandoffPhase::Persisted { .. } => None,
        }
    }

    fn pending_mut(&mut self) -> Option<&mut PendingHandoff> {
        match &mut self.phase {
            TuiHandoffPhase::Editable { pending, .. } => Some(pending.as_mut()),
            TuiHandoffPhase::Committed { .. }
            | TuiHandoffPhase::Created { .. }
            | TuiHandoffPhase::Persisted { .. } => None,
        }
    }

    pub(crate) fn selector_snapshot(
        &self,
        page: TuiHandoffSelectorKind,
        ctx: &AppContext,
    ) -> OptionSnapshot {
        match page {
            TuiHandoffSelectorKind::Environment => self.environment_snapshot(ctx),
            TuiHandoffSelectorKind::Model => self.model_snapshot(ctx),
        }
    }

    fn environment_snapshot(&self, ctx: &AppContext) -> OptionSnapshot {
        let selected_id = self
            .pending()
            .and_then(|pending| pending.presentation_snapshot().environment_id)
            .map(|id| id.to_string());
        let rows = self
            .environments
            .as_ref(ctx)
            .environments()
            .iter()
            .map(|environment| OptionRow {
                id: environment.id.to_string(),
                label: environment.name.clone(),
                harness: None,
                badge: None,
                disabled_reason: None,
            })
            .collect::<Vec<_>>();
        let selected_id =
            selected_id.filter(|selected_id| rows.iter().any(|row| row.id == *selected_id));
        OptionSnapshot {
            status: if rows.is_empty() {
                OptionSourceStatus::Empty {
                    message: "No cloud environments available".to_owned(),
                }
            } else {
                OptionSourceStatus::Ready
            },
            rows,
            selected_id,
            footer: None,
        }
    }

    fn model_snapshot(&self, ctx: &AppContext) -> OptionSnapshot {
        let selected_model_id = self
            .pending()
            .expect("editable handoff has pending state")
            .presentation_snapshot()
            .model_id;
        oz_model_snapshot(&selected_model_id, false, ctx)
    }

    pub(crate) fn environment_label(&self, ctx: &AppContext) -> String {
        let selected = self
            .pending()
            .and_then(|pending| pending.presentation_snapshot().environment_id);
        selected
            .and_then(|selected| {
                self.environments
                    .as_ref(ctx)
                    .environments()
                    .iter()
                    .find(|environment| environment.id == selected)
                    .map(|environment| environment.name.clone())
            })
            .unwrap_or_else(|| "Select an environment".to_owned())
    }

    pub(crate) fn model_label(&self, ctx: &AppContext) -> String {
        let Some(pending) = self.pending() else {
            return String::new();
        };
        let presentation = pending.presentation_snapshot();
        let snapshot = self.model_snapshot(ctx);
        let label = snapshot
            .rows
            .iter()
            .find(|row| row.id == presentation.model_id)
            .map(|row| row.label.clone())
            .unwrap_or_else(|| presentation.model_id.clone());
        if !LLMPreferences::as_ref(ctx)
            .is_cloud_runnable_oz_model_id(&LLMId::from(presentation.model_id.as_str()))
        {
            format!("{label} (incompatible)")
        } else {
            label
        }
    }

    pub(crate) fn open_page(
        &mut self,
        page: TuiHandoffSelectorKind,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if !self.is_editable()
            || (page == TuiHandoffSelectorKind::Environment && self.no_environments(ctx))
        {
            return false;
        }
        let TuiHandoffPhase::Editable { state, .. } = &mut self.phase else {
            return false;
        };
        *state = TuiHandoffEditableState::Configuring { page };
        ctx.notify();
        true
    }

    pub(crate) fn return_to_acceptance(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiHandoffPhase::Editable { state, .. } = &mut self.phase else {
            return;
        };
        *state = TuiHandoffEditableState::Acceptance {
            validation_error: None,
        };
        ctx.notify();
    }

    pub(crate) fn apply_selection(
        &mut self,
        page: TuiHandoffSelectorKind,
        id: &str,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        match page {
            TuiHandoffSelectorKind::Environment => {
                let environment_id = self
                    .environments
                    .as_ref(ctx)
                    .environments()
                    .iter()
                    .find(|environment| environment.id.to_string() == id)
                    .map(|environment| environment.id);
                let Some(environment_id) = environment_id else {
                    return false;
                };
                let Some(pending) = self.pending_mut() else {
                    return false;
                };
                pending.set_environment_id(Some(environment_id), true);
                self.environments.update(ctx, |catalog, ctx| {
                    catalog.persist_selection(environment_id, ctx);
                });
            }
            TuiHandoffSelectorKind::Model => {
                let Some(pending) = self.pending_mut() else {
                    return false;
                };
                pending.set_model_id(id.to_owned(), true, ctx);
            }
        }
        ctx.notify();
        true
    }

    pub(crate) fn confirm(&mut self, ctx: &mut ModelContext<Self>) {
        if !matches!(
            self.phase,
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            }
        ) || self.no_environments(ctx)
        {
            return;
        }
        let validation = self
            .pending()
            .expect("editable handoff has pending state")
            .validate();
        if let Err(error) = validation {
            let TuiHandoffPhase::Editable { state, .. } = &mut self.phase else {
                unreachable!("validated handoff is editable");
            };
            *state = TuiHandoffEditableState::Acceptance {
                validation_error: Some(Self::validation_message(&error).to_owned()),
            };
            ctx.emit(TuiHandoffModelEvent::Changed { focus_block: false });
            ctx.notify();
            return;
        }
        self.next_operation_id = self.next_operation_id.wrapping_add(1);
        let operation_id = self.next_operation_id;
        let editable =
            std::mem::replace(&mut self.phase, TuiHandoffPhase::Committed { operation_id });
        let TuiHandoffPhase::Editable { pending, .. } = editable else {
            unreachable!("confirmed handoff is editable");
        };
        ctx.notify();

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let execution = execute_handoff(*pending, ai_client, None, ctx);
        ctx.spawn(execution, move |model, outcome, ctx| {
            if !matches!(
                model.phase,
                TuiHandoffPhase::Committed {
                    operation_id: active_operation_id
                } if active_operation_id == operation_id
            ) || model.dismissed
            {
                return;
            }
            match outcome {
                HandoffCommitOutcome::Rejected { pending, error } => {
                    model.phase = TuiHandoffPhase::Editable {
                        state: TuiHandoffEditableState::Acceptance {
                            validation_error: Some(Self::validation_message(&error).to_owned()),
                        },
                        pending,
                    };
                    model.refresh_pending_environments(ctx);
                    ctx.emit(TuiHandoffModelEvent::Changed { focus_block: true });
                    ctx.notify();
                }
                HandoffCommitOutcome::Failed(failure) => {
                    model.dismissed = true;
                    ctx.emit(TuiHandoffModelEvent::Failed {
                        restoration: failure.restoration,
                        message:
                            "Couldn't start the handoff. Check your network connection and try again."
                                .to_owned(),
                    });
                    ctx.notify();
                }
                HandoffCommitOutcome::Cancelled => {
                    model.dismissed = true;
                    ctx.emit(TuiHandoffModelEvent::Cancelled(None));
                    ctx.notify();
                }
                HandoffCommitOutcome::Created(created) => {
                    model.phase = TuiHandoffPhase::Created { url: created.url };
                    ctx.emit(TuiHandoffModelEvent::Changed { focus_block: true });
                    ctx.notify();
                }
            }
        });
    }

    pub(crate) fn cancel(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_editable() || self.dismissed {
            return;
        }
        let restoration = self
            .pending_mut()
            .and_then(PendingHandoff::take_restoration);
        self.dismissed = true;
        ctx.emit(TuiHandoffModelEvent::Cancelled(restoration));
        ctx.notify();
    }

    pub(crate) fn continue_locally(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiHandoffPhase::Created { url } = &self.phase else {
            return;
        };
        if !self.forked_existing_conversation || self.dismissed {
            return;
        }
        self.phase = TuiHandoffPhase::Persisted { url: url.clone() };
        ctx.emit(TuiHandoffModelEvent::ContinueLocally);
        ctx.notify();
    }

    pub(crate) fn start_new_conversation(&mut self, ctx: &mut ModelContext<Self>) {
        if !matches!(self.phase, TuiHandoffPhase::Created { .. }) || self.dismissed {
            return;
        }
        self.dismissed = true;
        ctx.emit(TuiHandoffModelEvent::StartNewConversation);
        ctx.notify();
    }

    pub(crate) fn refresh_environments(&self, ctx: &mut ModelContext<Self>) {
        self.environments.update(ctx, |catalog, ctx| {
            catalog.refresh_from_server(ctx);
        });
    }

    fn refresh_pending_environments(&mut self, ctx: &AppContext) {
        let valid_ids = self
            .environments
            .as_ref(ctx)
            .environments()
            .iter()
            .map(|environment| environment.id)
            .collect::<HashSet<_>>();
        let default_environment_id = self.environments.as_ref(ctx).default_environment_id(ctx);
        let Some(pending) = self.pending_mut() else {
            return;
        };
        pending.set_valid_environment_ids(valid_ids);
        if pending.presentation_snapshot().environment_id.is_none()
            && let Some(environment_id) = default_environment_id
        {
            pending.set_environment_id(Some(environment_id), false);
        }
    }

    fn handle_environment_change(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_editable() {
            return;
        }
        self.refresh_pending_environments(ctx);
        ctx.emit(TuiHandoffModelEvent::Changed { focus_block: false });
        ctx.notify();
    }
}

impl Entity for TuiHandoffModel {
    type Event = TuiHandoffModelEvent;
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
