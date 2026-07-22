//! TUI-local presentation for the shared local-to-cloud handoff pipeline.

use std::collections::HashSet;
use std::path::PathBuf;

use warp::settings::{AISettings, PrivacySettings, PrivacySettingsChangedEvent};
use warp::tui_export::{
    AISettingsChangedEvent, HandoffCommitOutcome, HandoffPrepareError, HandoffRestoration, LLMId,
    LLMPreferences, LLMPreferencesEvent, OptionRow, OptionSnapshot, OptionSourceStatus,
    PendingHandoff, ServerApiProvider, TuiCloudEnvironmentProjection, UserWorkspaces,
    UserWorkspacesEvent, commit_handoff, oz_cloud_model_snapshot, persist_environment_selection,
    suggest_tui_handoff_environment,
};
use warpui::SingletonEntity;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiText};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::keybindings::TUI_BINDING_GROUP;
use crate::link::TuiLink;
use crate::option_selector::{OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorEvent};
use crate::tui_builder::TuiUiBuilder;

const HANDOFF_TITLE: &str = "Hand off to cloud";
const ENVIRONMENTS_DOCS_URL: &str =
    "https://docs.warp.dev/agent-platform/cloud-agents/environments";
const CONFIGURING_CONTEXT_FLAG: &str = "TuiHandoffBlockConfiguring";
const NO_ENVIRONMENT_CONTEXT_FLAG: &str = "TuiHandoffBlockNoEnvironment";
const SELECTOR_CONTEXT_FLAG: &str = "TuiHandoffBlockSelector";
const COMMITTED_CONTEXT_FLAG: &str = "TuiHandoffBlockCommitted";
const CREATED_CONTEXT_FLAG: &str = "TuiHandoffBlockCreated";

pub(crate) fn init(app: &mut AppContext) {
    let card = || id!(TuiHandoffBlock::ui_name());
    let configuring = || card() & id!(CONFIGURING_CONTEXT_FLAG);
    let no_environment = || card() & id!(NO_ENVIRONMENT_CONTEXT_FLAG);
    let selector = || card() & id!(SELECTOR_CONTEXT_FLAG);
    let committed = || card() & id!(COMMITTED_CONTEXT_FLAG);
    let created = || card() & id!(CREATED_CONTEXT_FLAG);
    app.register_fixed_bindings([
        FixedBinding::new(
            "e",
            TuiHandoffBlockAction::OpenEnvironmentSelector,
            configuring(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("m", TuiHandoffBlockAction::OpenModelSelector, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("enter", TuiHandoffBlockAction::Confirm, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("numpadenter", TuiHandoffBlockAction::Confirm, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "enter",
            TuiHandoffBlockAction::OpenEnvironmentDocs,
            no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "numpadenter",
            TuiHandoffBlockAction::OpenEnvironmentDocs,
            no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "r",
            TuiHandoffBlockAction::RefreshEnvironments,
            no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiHandoffBlockAction::Cancel, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiHandoffBlockAction::Cancel, no_environment())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiHandoffBlockAction::Back, selector())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-c",
            TuiHandoffBlockAction::Cancel,
            configuring() | no_environment() | selector(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-c",
            TuiHandoffBlockAction::ConsumeInterrupt,
            committed(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("enter", TuiHandoffBlockAction::OpenRun, created())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("numpadenter", TuiHandoffBlockAction::OpenRun, created())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("c", TuiHandoffBlockAction::ContinueLocally, created())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("n", TuiHandoffBlockAction::StartNewConversation, created())
            .with_group(TUI_BINDING_GROUP),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorKind {
    Environment,
    Model,
}

#[derive(Debug)]
enum HandoffPhase {
    Configuring,
    Selecting(SelectorKind),
    Committed { operation_id: u64 },
    Created { url: String },
}

#[derive(Clone)]
pub(crate) enum TuiHandoffBlockEvent {
    Cancelled(Option<HandoffRestoration>),
    Failed {
        restoration: Option<HandoffRestoration>,
        message: String,
    },
    ContinueLocally,
    StartNewConversation,
}

#[derive(Clone, Debug)]
pub(crate) enum TuiHandoffBlockAction {
    OpenEnvironmentSelector,
    OpenModelSelector,
    Confirm,
    OpenEnvironmentDocs,
    RefreshEnvironments,
    Back,
    Cancel,
    ConsumeInterrupt,
    OpenRun,
    ContinueLocally,
    StartNewConversation,
}

pub(crate) struct TuiHandoffBlock {
    pending: Option<PendingHandoff>,
    phase: HandoffPhase,
    selector: ViewHandle<TuiOptionSelector>,
    environments: ModelHandle<TuiCloudEnvironmentProjection>,
    forked_existing_conversation: bool,
    validation_error: Option<String>,
    next_operation_id: u64,
    dismissed: bool,
    link: TuiLink,
}

impl TuiHandoffBlock {
    pub(crate) fn new(
        pending: PendingHandoff,
        current_working_directory: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let selector = ctx.add_typed_action_tui_view(TuiOptionSelector::new);
        ctx.subscribe_to_view(&selector, |block, _, event, ctx| {
            block.handle_selector_event(event, ctx);
        });
        let environments = ctx.add_model(TuiCloudEnvironmentProjection::new);
        ctx.subscribe_to_model(&environments, |block, _, _, ctx| {
            block.handle_environment_change(ctx);
        });
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |block, _, event, ctx| {
            if matches!(event, LLMPreferencesEvent::UpdatedAvailableLLMs) {
                block.refresh_selector(ctx);
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(
            &AISettings::handle(ctx),
            |block, _, _: &AISettingsChangedEvent, ctx| {
                if block.is_editable() && !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx) {
                    block.cancel(ctx);
                }
            },
        );
        ctx.subscribe_to_model(&PrivacySettings::handle(ctx), |block, _, event, ctx| {
            if matches!(
                event,
                PrivacySettingsChangedEvent::UpdateIsCloudConversationStorageEnabled { .. }
            ) && block.is_editable()
                && !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)
            {
                block.cancel(ctx);
            }
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |block, _, event, ctx| {
            if matches!(event, UserWorkspacesEvent::TeamsChanged)
                && block.is_editable()
                && !AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)
            {
                block.cancel(ctx);
            }
        });

        let forked_existing_conversation =
            pending.presentation_snapshot().forked_existing_conversation;
        let mut block = Self {
            pending: Some(pending),
            phase: HandoffPhase::Configuring,
            selector,
            environments,
            forked_existing_conversation,
            validation_error: None,
            next_operation_id: 0,
            dismissed: false,
            link: TuiLink::default(),
        };
        block.refresh_pending_environments(ctx);

        if let Some(path) = current_working_directory.map(PathBuf::from) {
            let suggestion = suggest_tui_handoff_environment(path, ctx);
            ctx.spawn(suggestion, |block, environment_id, ctx| {
                if !block.is_editable() || block.dismissed {
                    return;
                }
                if let Some(environment_id) = environment_id
                    && let Some(pending) = block.pending.as_mut()
                {
                    pending.set_environment_id(Some(environment_id), false);
                    block.refresh_selector(ctx);
                    ctx.notify();
                }
            });
        }
        block
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.dismissed
    }

    #[cfg(test)]
    pub(crate) fn environments_for_test(&self) -> ModelHandle<TuiCloudEnvironmentProjection> {
        self.environments.clone()
    }

    #[cfg(test)]
    pub(crate) fn selector_for_test(&self) -> ViewHandle<TuiOptionSelector> {
        self.selector.clone()
    }

    #[cfg(test)]
    pub(crate) fn is_configuring_for_test(&self) -> bool {
        matches!(self.phase, HandoffPhase::Configuring)
    }

    #[cfg(test)]
    pub(crate) fn set_committed_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.phase = HandoffPhase::Committed { operation_id: 1 };
        ctx.focus_self();
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn set_model_for_test(&mut self, model_id: String, ctx: &mut ViewContext<Self>) {
        self.pending
            .as_mut()
            .expect("editable handoff has pending state")
            .set_model_id(model_id, true, ctx);
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn select_first_environment_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        let environment_id = self
            .environments
            .as_ref(ctx)
            .environments()
            .first()
            .expect("test environment exists")
            .id;
        self.pending
            .as_mut()
            .expect("editable handoff has pending state")
            .set_environment_id(Some(environment_id), true);
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn set_created_for_test(
        &mut self,
        url: String,
        forked_existing_conversation: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.phase = HandoffPhase::Created { url };
        self.forked_existing_conversation = forked_existing_conversation;
        ctx.focus_self();
        ctx.notify();
    }

    pub(crate) fn focus(&self, ctx: &mut ViewContext<Self>) {
        match self.phase {
            HandoffPhase::Selecting(_) => ctx.focus(&self.selector),
            HandoffPhase::Configuring
            | HandoffPhase::Committed { .. }
            | HandoffPhase::Created { .. } => ctx.focus_self(),
        }
    }

    fn is_editable(&self) -> bool {
        matches!(
            self.phase,
            HandoffPhase::Configuring | HandoffPhase::Selecting(_)
        )
    }

    fn no_environments(&self, ctx: &AppContext) -> bool {
        self.environments.as_ref(ctx).environments().is_empty()
    }

    fn refresh_pending_environments(&mut self, ctx: &AppContext) {
        let valid_ids = self
            .environments
            .as_ref(ctx)
            .environments()
            .iter()
            .map(|environment| environment.id)
            .collect::<HashSet<_>>();
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        pending.refresh_valid_environment_ids(valid_ids);
        if pending.presentation_snapshot().environment_id.is_none()
            && let Some(environment_id) = self.environments.as_ref(ctx).default_environment_id(ctx)
        {
            pending.set_environment_id(Some(environment_id), false);
        }
    }

    fn handle_environment_change(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_editable() {
            return;
        }
        self.refresh_pending_environments(ctx);
        self.refresh_selector(ctx);
        ctx.notify();
    }

    fn environment_snapshot(&self, ctx: &AppContext) -> OptionSnapshot {
        let selected_id = self
            .pending
            .as_ref()
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
            .pending
            .as_ref()
            .expect("editable handoff has pending state")
            .presentation_snapshot()
            .model_id;
        oz_cloud_model_snapshot(&selected_model_id, ctx)
    }

    fn open_selector(&mut self, kind: SelectorKind, ctx: &mut ViewContext<Self>) {
        if !matches!(self.phase, HandoffPhase::Configuring)
            || (kind == SelectorKind::Environment && self.no_environments(ctx))
        {
            return;
        }
        let snapshot = match kind {
            SelectorKind::Environment => self.environment_snapshot(ctx),
            SelectorKind::Model => self.model_snapshot(ctx),
        };
        self.phase = HandoffPhase::Selecting(kind);
        self.validation_error = None;
        self.selector.update(ctx, |selector, ctx| {
            selector.set_page(
                OptionSelectorPage {
                    header: None,
                    snapshot,
                    searchable: kind == SelectorKind::Model,
                    row_shortcuts: Default::default(),
                },
                ctx,
            );
        });
        ctx.focus(&self.selector);
        ctx.notify();
    }

    fn return_to_configuration(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(self.phase, HandoffPhase::Selecting(_)) {
            self.phase = HandoffPhase::Configuring;
            ctx.focus_self();
            ctx.notify();
        }
    }

    fn refresh_selector(&mut self, ctx: &mut ViewContext<Self>) {
        let HandoffPhase::Selecting(kind) = self.phase else {
            return;
        };
        let snapshot = match kind {
            SelectorKind::Environment => self.environment_snapshot(ctx),
            SelectorKind::Model => self.model_snapshot(ctx),
        };
        self.selector.update(ctx, |selector, ctx| {
            selector.refresh_snapshot(snapshot, ctx);
        });
    }

    fn handle_selector_event(
        &mut self,
        event: &TuiOptionSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiOptionSelectorEvent::Confirmed { id } => {
                let HandoffPhase::Selecting(kind) = self.phase else {
                    return;
                };
                let Some(pending) = self.pending.as_mut() else {
                    return;
                };
                match kind {
                    SelectorKind::Environment => {
                        let environment_id = self
                            .environments
                            .as_ref(ctx)
                            .environments()
                            .iter()
                            .find(|environment| environment.id.to_string() == *id)
                            .map(|environment| environment.id);
                        let Some(environment_id) = environment_id else {
                            return;
                        };
                        pending.set_environment_id(Some(environment_id), true);
                        persist_environment_selection(id, ctx);
                    }
                    SelectorKind::Model => {
                        pending.set_model_id(id.clone(), true, ctx);
                    }
                }
                self.return_to_configuration(ctx);
            }
            TuiOptionSelectorEvent::Dismissed => self.return_to_configuration(ctx),
            TuiOptionSelectorEvent::CustomTextSubmitted { .. }
            | TuiOptionSelectorEvent::CustomTextOpened
            | TuiOptionSelectorEvent::CustomTextClosed
            | TuiOptionSelectorEvent::RetryRequested
            | TuiOptionSelectorEvent::LayoutInvalidated => {}
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

    fn confirm(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(self.phase, HandoffPhase::Configuring) || self.no_environments(ctx) {
            return;
        }
        let validation = self
            .pending
            .as_ref()
            .expect("configuring handoff has pending state")
            .validate();
        if let Err(error) = validation {
            self.validation_error = Some(Self::validation_message(&error).to_owned());
            ctx.notify();
            return;
        }

        let pending = self
            .pending
            .take()
            .expect("configuring handoff has pending state");
        self.next_operation_id = self.next_operation_id.wrapping_add(1);
        let operation_id = self.next_operation_id;
        self.phase = HandoffPhase::Committed { operation_id };
        self.validation_error = None;
        ctx.focus_self();
        ctx.notify();

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let commit = commit_handoff(pending, ai_client, None, ctx);
        ctx.spawn(commit, move |block, outcome, ctx| {
            if !matches!(
                block.phase,
                HandoffPhase::Committed {
                    operation_id: active_operation_id
                } if active_operation_id == operation_id
            ) || block.dismissed
            {
                return;
            }
            match outcome {
                HandoffCommitOutcome::Rejected { pending, error } => {
                    block.pending = Some(*pending);
                    block.phase = HandoffPhase::Configuring;
                    block.validation_error = Some(Self::validation_message(&error).to_owned());
                    block.refresh_pending_environments(ctx);
                    ctx.focus_self();
                    ctx.notify();
                }
                HandoffCommitOutcome::Failed(failure) => {
                    block.dismissed = true;
                    ctx.emit(TuiHandoffBlockEvent::Failed {
                        restoration: failure.restoration,
                        message:
                            "Couldn't start the handoff. Check your network connection and try again."
                                .to_owned(),
                    });
                    ctx.notify();
                }
                HandoffCommitOutcome::Created(created) => {
                    block.phase = HandoffPhase::Created { url: created.url };
                    ctx.focus_self();
                    ctx.notify();
                }
            }
        });
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_editable() || self.dismissed {
            return;
        }
        let restoration = self
            .pending
            .as_mut()
            .and_then(PendingHandoff::take_restoration);
        self.dismissed = true;
        ctx.emit(TuiHandoffBlockEvent::Cancelled(restoration));
        ctx.notify();
    }

    fn handle_back(&mut self, ctx: &mut ViewContext<Self>) {
        let handled = self
            .selector
            .update(ctx, |selector, ctx| selector.handle_back(ctx));
        if !handled {
            self.return_to_configuration(ctx);
        }
    }

    fn open_run(&self, ctx: &mut ViewContext<Self>) {
        if let HandoffPhase::Created { url } = &self.phase {
            ctx.open_url(url);
        }
    }

    fn continue_locally(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(self.phase, HandoffPhase::Created { .. })
            || !self.forked_existing_conversation
            || self.dismissed
        {
            return;
        }
        self.dismissed = true;
        ctx.emit(TuiHandoffBlockEvent::ContinueLocally);
        ctx.notify();
    }

    fn start_new_conversation(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(self.phase, HandoffPhase::Created { .. }) || self.dismissed {
            return;
        }
        self.dismissed = true;
        ctx.emit(TuiHandoffBlockEvent::StartNewConversation);
        ctx.notify();
    }

    fn environment_label(&self, ctx: &AppContext) -> String {
        let selected = self
            .pending
            .as_ref()
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

    fn model_label(&self, ctx: &AppContext) -> String {
        let Some(pending) = self.pending.as_ref() else {
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

    fn render_title(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        TuiText::from_spans([
            ("■ ".to_owned(), builder.option_selector_selected_style()),
            (HANDOFF_TITLE.to_owned(), builder.primary_text_style()),
        ])
        .finish()
    }

    fn render_configuration(
        &self,
        ctx: &AppContext,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        if self.no_environments(ctx) {
            return TuiFlex::column()
                .child(
                    TuiText::new("A cloud environment is required to hand off this conversation.")
                        .with_style(builder.primary_text_style())
                        .finish(),
                )
                .child(
                    TuiText::new("Create one in Warp, then refresh this card.")
                        .with_style(builder.muted_text_style())
                        .finish(),
                )
                .finish();
        }
        let mut content = TuiFlex::column()
            .child(
                TuiText::from_spans([
                    ("Environment: ".to_owned(), builder.primary_text_style()),
                    (
                        self.environment_label(ctx),
                        builder.orchestration_selected_value_style(),
                    ),
                ])
                .finish(),
            )
            .child(
                TuiText::from_spans([
                    ("Model: ".to_owned(), builder.primary_text_style()),
                    (
                        self.model_label(ctx),
                        builder.orchestration_selected_value_style(),
                    ),
                ])
                .finish(),
            );
        if let Some(error) = &self.validation_error {
            content = content.child(
                TuiText::new(error.clone())
                    .with_style(builder.error_text_style())
                    .finish(),
            );
        }
        content.finish()
    }

    fn render_body(&self, ctx: &AppContext, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        match self.phase {
            HandoffPhase::Configuring => self.render_configuration(ctx, builder),
            HandoffPhase::Selecting(_) => TuiChildView::new(&self.selector).finish(),
            HandoffPhase::Committed { .. } => TuiText::new("Creating cloud run…")
                .with_style(builder.primary_text_style())
                .finish(),
            HandoffPhase::Created { ref url } => TuiFlex::column()
                .child(
                    TuiText::new("Cloud run created.")
                        .with_style(builder.primary_text_style())
                        .finish(),
                )
                .child(self.link.render(url.clone(), ctx, move |event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiHandoffBlockAction::OpenRun);
                }))
                .finish(),
        }
    }

    fn render_footer(&self, ctx: &AppContext, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let spans = match self.phase {
            HandoffPhase::Configuring if self.no_environments(ctx) => vec![
                ("Enter ".to_owned(), builder.primary_text_style()),
                ("open setup guide  ".to_owned(), builder.muted_text_style()),
                ("R ".to_owned(), builder.primary_text_style()),
                ("refresh  ".to_owned(), builder.muted_text_style()),
                ("Esc ".to_owned(), builder.primary_text_style()),
                ("cancel".to_owned(), builder.muted_text_style()),
            ],
            HandoffPhase::Configuring => vec![
                ("Enter ".to_owned(), builder.primary_text_style()),
                ("hand off  ".to_owned(), builder.muted_text_style()),
                ("E ".to_owned(), builder.primary_text_style()),
                ("environment  ".to_owned(), builder.muted_text_style()),
                ("M ".to_owned(), builder.primary_text_style()),
                ("model  ".to_owned(), builder.muted_text_style()),
                ("Esc ".to_owned(), builder.primary_text_style()),
                ("cancel".to_owned(), builder.muted_text_style()),
            ],
            HandoffPhase::Selecting(_) => vec![
                ("Enter ".to_owned(), builder.primary_text_style()),
                ("select  ".to_owned(), builder.muted_text_style()),
                ("↑ ↓ ".to_owned(), builder.primary_text_style()),
                ("navigate  ".to_owned(), builder.muted_text_style()),
                ("Esc ".to_owned(), builder.primary_text_style()),
                ("back".to_owned(), builder.muted_text_style()),
            ],
            HandoffPhase::Committed { .. } => vec![(
                "Handoff is in progress…".to_owned(),
                builder.muted_text_style(),
            )],
            HandoffPhase::Created { .. } => {
                let mut spans = vec![
                    ("Enter ".to_owned(), builder.primary_text_style()),
                    ("open cloud run  ".to_owned(), builder.muted_text_style()),
                ];
                if self.forked_existing_conversation {
                    spans.extend([
                        ("C ".to_owned(), builder.primary_text_style()),
                        ("continue locally  ".to_owned(), builder.muted_text_style()),
                    ]);
                }
                spans.extend([
                    ("N ".to_owned(), builder.primary_text_style()),
                    ("new conversation".to_owned(), builder.muted_text_style()),
                ]);
                spans
            }
        };
        TuiText::from_spans(spans).finish()
    }
}

impl Entity for TuiHandoffBlock {
    type Event = TuiHandoffBlockEvent;
}

impl TuiView for TuiHandoffBlock {
    fn ui_name() -> &'static str {
        "TuiHandoffBlock"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![self.selector.id()]
    }

    fn keymap_context(&self, ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert(Self::ui_name());
        match self.phase {
            HandoffPhase::Configuring if self.no_environments(ctx) => {
                context.set.insert(NO_ENVIRONMENT_CONTEXT_FLAG);
            }
            HandoffPhase::Configuring => {
                context.set.insert(CONFIGURING_CONTEXT_FLAG);
            }
            HandoffPhase::Selecting(_) => {
                context.set.insert(SELECTOR_CONTEXT_FLAG);
            }
            HandoffPhase::Committed { .. } => {
                context.set.insert(COMMITTED_CONTEXT_FLAG);
            }
            HandoffPhase::Created { .. } => {
                context.set.insert(CREATED_CONTEXT_FLAG);
            }
        }
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        let header = TuiContainer::new(self.render_title(&builder))
            .with_background(builder.orchestration_header_background())
            .with_padding_x(1)
            .finish();
        let body = TuiContainer::new(self.render_body(ctx, &builder))
            .with_background(builder.orchestration_surface_background())
            .with_padding_x(3)
            .with_padding_y(1)
            .finish();
        TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(header)
            .child(body)
            .child(
                TuiContainer::new(self.render_footer(ctx, &builder))
                    .with_padding_top(1)
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for TuiHandoffBlock {
    type Action = TuiHandoffBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiHandoffBlockAction::OpenEnvironmentSelector => {
                self.open_selector(SelectorKind::Environment, ctx);
            }
            TuiHandoffBlockAction::OpenModelSelector => {
                self.open_selector(SelectorKind::Model, ctx);
            }
            TuiHandoffBlockAction::Confirm => self.confirm(ctx),
            TuiHandoffBlockAction::OpenEnvironmentDocs => ctx.open_url(ENVIRONMENTS_DOCS_URL),
            TuiHandoffBlockAction::RefreshEnvironments => {
                self.environments.update(ctx, |projection, ctx| {
                    projection.refresh_from_server(ctx);
                });
            }
            TuiHandoffBlockAction::Back => self.handle_back(ctx),
            TuiHandoffBlockAction::Cancel => self.cancel(ctx),
            TuiHandoffBlockAction::ConsumeInterrupt => {}
            TuiHandoffBlockAction::OpenRun => self.open_run(ctx),
            TuiHandoffBlockAction::ContinueLocally => self.continue_locally(ctx),
            TuiHandoffBlockAction::StartNewConversation => self.start_new_conversation(ctx),
        }
    }
}
