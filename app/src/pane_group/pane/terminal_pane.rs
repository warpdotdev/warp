//! Implementation of terminal panes.
#[cfg(feature = "local_fs")]
use crate::pane_group::CodeSource;
use std::{collections::HashMap, sync::mpsc::SyncSender};

use warpui::{
    AppContext, EntityId, ModelHandle, SingletonEntity, ViewContext, ViewHandle, WindowId,
};

use crate::{
    ai::{
        active_agent_views_model::ActiveAgentViewsModel,
        agent::{conversation::AIConversationId, LifecycleEventType, StartAgentExecutionMode},
        blocklist::{
            agent_view::AgentViewEntryOrigin, orchestration_events::OrchestrationEventService,
            BlocklistAIHistoryModel,
        },
        llms::LLMPreferences,
    },
    app_state::{LeafContents, TerminalPaneSnapshot},
    pane_group::child_agent::{
        create_error_child_agent_conversation, create_hidden_child_agent_conversation,
        HiddenChildAgentConversation,
    },
    pane_group::{self, Direction, Event::OpenConversationHistory, PaneGroup},
    persistence::{BlockCompleted, ModelEvent},
    session_management::SessionNavigationData,
    terminal::cli_agent_sessions::CLIAgentSessionsModel,
    terminal::{general_settings::GeneralSettings, view::Event, TerminalManager, TerminalView},
    view_components::ToastFlavor,
    workspace::{sync_inputs::SyncedInputState, PaneViewLocator},
};

#[cfg(feature = "local_fs")]
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use warp_core::execution_mode::AppExecutionMode;

#[cfg(not(target_family = "wasm"))]
use super::local_harness_launch::{prepare_local_harness_child_launch, PreparedLocalHarnessLaunch};
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneId, PaneStackEvent, PaneView, ShareableLink,
    ShareableLinkError, TerminalPaneId,
};

pub type TerminalPaneView = PaneView<TerminalView>;

/// Data kept for terminal panes.
pub struct TerminalPane {
    model_event_sender: Option<SyncSender<ModelEvent>>,

    /// Used to uniquely identify the pane, even across separate runs of the app.
    uuid: Vec<u8>,

    pane_configuration: ModelHandle<PaneConfiguration>,

    /// Defining `terminal_manager` before `view` means that `terminal_manager`
    /// gets dropped first (guaranteed by the language), which halts the event
    /// loop and avoids possible deadlocks during session cleanup. This is enforced
    /// by the `PaneStack`, since the terminal manager is the associated data for
    /// the backing pane view.
    view: ViewHandle<TerminalPaneView>,
}

fn register_legacy_local_lifecycle_subscription(
    parent_conversation_id: AIConversationId,
    child_conversation_id: AIConversationId,
    lifecycle_subscription: Option<Vec<LifecycleEventType>>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    if let Some(parent_agent_id) = BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&parent_conversation_id)
        .and_then(|conversation| {
            conversation
                .server_conversation_token()
                .map(|token| token.as_str().to_string())
        })
    {
        OrchestrationEventService::handle(ctx).update(ctx, |svc, _| {
            svc.register_lifecycle_subscription(
                child_conversation_id,
                parent_agent_id,
                lifecycle_subscription,
            );
        });
    }
}

impl TerminalPane {
    pub(in crate::pane_group) fn new(
        uuid: Vec<u8>,
        terminal_manager: ModelHandle<Box<dyn TerminalManager>>,
        terminal_view: ViewHandle<TerminalView>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Self {
        let pane_configuration = terminal_view.as_ref(ctx).pane_configuration().to_owned();
        let view = ctx.add_typed_action_view(|ctx| {
            let pane_id = PaneId::from_terminal_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                terminal_view,
                terminal_manager,
                pane_configuration.clone(),
                ctx,
            )
        });

        Self {
            model_event_sender,
            uuid,
            pane_configuration,
            view,
        }
    }

    /// The [`PaneView<TerminalView>`] for this pane.
    #[cfg(any(test, feature = "integration_tests"))]
    pub(in crate::pane_group) fn pane_view(&self) -> ViewHandle<TerminalPaneView> {
        self.view.to_owned()
    }

    /// The [`TerminalView`] backing the [`PaneView`] for this terminal pane.
    pub(crate) fn terminal_view(&self, ctx: &AppContext) -> ViewHandle<TerminalView> {
        self.view.as_ref(ctx).child(ctx)
    }

    /// The UUID that identifies this terminal session across app restarts.
    pub(in crate::pane_group) fn session_uuid(&self) -> Vec<u8> {
        self.uuid.clone()
    }

    /// The terminal manager responsible for this session's event loop.
    pub(in crate::pane_group) fn terminal_manager(
        &self,
        ctx: &AppContext,
    ) -> ModelHandle<Box<dyn TerminalManager>> {
        self.view.as_ref(ctx).child_data(ctx).clone()
    }

    /// Instructs the SQLite thread to delete blocks for this session.
    pub(in crate::pane_group) fn delete_blocks(&self, ctx: &AppContext) {
        if !AppExecutionMode::as_ref(ctx).can_save_session() {
            return;
        }

        if let Some(sender) = &self.model_event_sender {
            let model_event = ModelEvent::DeleteBlocks(self.uuid.clone());
            if let Err(err) = sender.send(model_event) {
                log::error!(
                    "Error sending blocks deleted event for terminal id {} {:?}",
                    self.terminal_view(ctx).id(),
                    err
                );
            }
        }
    }

    pub fn session_navigation_data(
        &self,
        pane_group_id: EntityId,
        window_id: WindowId,
        app: &AppContext,
    ) -> SessionNavigationData {
        let view = self.terminal_view(app).as_ref(app);
        SessionNavigationData::new(
            view.full_prompt(app),
            view.prompt_elements(app),
            view.session_command_context(app),
            PaneViewLocator {
                pane_group_id,
                pane_id: self.id(),
            },
            view.last_focus_ts(),
            view.is_read_only(),
            window_id,
        )
    }

    pub fn terminal_pane_id(&self) -> TerminalPaneId {
        self.id()
            .as_terminal_pane_id()
            .expect("Should be able to derive a TerminalPaneId from TerminalPane")
    }
}

impl PaneContent for TerminalPane {
    fn id(&self) -> PaneId {
        PaneId::from_terminal_pane_view(&self.view)
    }

    fn attach(
        &self,
        group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // TODO(ben): As much as possible, logic from PaneGroup::add_session should go here.
        //  This will simplify PaneGroup, especially when implementing pane management.
        let terminal_pane_id = self.terminal_pane_id();

        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        // Attach the initial terminal view in the stack.
        attach_terminal_view(&self.terminal_view(ctx), terminal_pane_id, ctx);

        // Subscribe to the pane stack to handle views being pushed/popped.
        let pane_stack = self.view.as_ref(ctx).pane_stack().clone();
        ctx.subscribe_to_model(&pane_stack, move |group, _, event, ctx| {
            handle_pane_stack_event(group, event, terminal_pane_id, ctx);
        });

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(terminal_pane_id.into(), event, ctx);
        });

        if SyncedInputState::as_ref(ctx).should_sync_this_pane_group(ctx.view_id(), ctx.window_id())
        {
            if let Some(active_pane_view) = group.active_session_view(ctx) {
                let event = active_pane_view
                    .as_ref(ctx)
                    .create_sync_event_based_on_terminal_state(ctx);

                group.send_sync_event_to_session(terminal_pane_id, &event, ctx);
            }
        }

        let terminal_view_id = self.terminal_view(ctx).id();

        #[cfg(feature = "local_fs")]
        {
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                move |group, _, event, ctx| {
                    let Some(model_event_sender) = group.model_event_sender.clone() else {
                        return;
                    };

                    handle_ai_history_event(
                        event,
                        terminal_view_id,
                        terminal_pane_id,
                        model_event_sender,
                        ctx,
                    );
                },
            );
        }

        // Store the pane group entity ID on the agent view controller so the
        // message bar can perform pane-group-scoped visibility checks.
        let pane_group_id = ctx.view_id();
        let terminal_view = self.terminal_view(ctx);
        let agent_view_controller = terminal_view.as_ref(ctx).agent_view_controller().clone();
        agent_view_controller.update(ctx, |controller, _ctx| {
            controller.set_pane_group_id(pane_group_id);
        });
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_agent_view_controller(&agent_view_controller, terminal_view_id, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        if matches!(detach_type, DetachType::Closed) {
            // Only immediately clear conversations and delete blocks if the session is being
            // permanently closed.
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model
                    .clear_conversations_in_terminal_view(self.terminal_view(ctx).id(), ctx);
            });
            self.delete_blocks(ctx);
        }

        // Unsubscribe from all views in the pane stack.
        let pane_stack = self.view.as_ref(ctx).pane_stack().clone();
        let contents = pane_stack.as_ref(ctx).entries().to_vec();
        for (manager, view) in contents {
            // Notify the view that it's being detached so it can react appropriately.
            manager.update(ctx, |terminal_manager, ctx| {
                terminal_manager.on_view_detached(detach_type, ctx);
            });
            ctx.unsubscribe_to_view(&view);
        }

        // Notify the active agent views model that the terminal view has been closed
        // (and that any active views are no longer active). On a `HiddenForClose` detach,
        // `attach` will re-register via `register_agent_view_controller` when the tab is
        // restored, so this is safe to run unconditionally.
        let terminal_view_id = self.terminal_view(ctx).id();
        ActiveAgentViewsModel::handle(ctx).update(ctx, |model, ctx| {
            model.unregister_agent_view_controller(terminal_view_id, ctx);
        });

        // Clean up any active CLI agent session so its notification is removed.
        // Skip this for moves — the session is still running and will re-register in the new tab.
        if !matches!(detach_type, DetachType::Moved) {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.remove_session(terminal_view_id, ctx);
            });
        }

        ctx.unsubscribe_to_model(&pane_stack);

        ctx.unsubscribe_to_view(&self.view);

        #[cfg(feature = "local_fs")]
        {
            ctx.unsubscribe_to_model(&BlocklistAIHistoryModel::handle(ctx));
        }
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let view = self.terminal_view(app).as_ref(app);
        let is_active = view.is_active_session(app);

        // Capture the current input_config from the AI input model
        let current_input_config = view.input_config(app.as_ref());

        if !view.model.lock().is_conversation_transcript_viewer() {
            let llm_model_override =
                LLMPreferences::as_ref(app).get_base_llm_override(self.terminal_view(app).id());

            let active_profile_id = None;

            // Collect all conversation IDs for this terminal view
            let conversation_ids_to_restore = BlocklistAIHistoryModel::as_ref(app)
                .all_live_conversations_for_terminal_view(self.terminal_view(app).id())
                .map(|conversation| conversation.id())
                .collect();

            // Capture agent view state: if fullscreen, store the active conversation ID
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(app)
                .agent_view_state()
                .display_mode()
                .filter(|mode| mode.is_fullscreen())
                .and_then(|_| {
                    view.agent_view_controller()
                        .as_ref(app)
                        .agent_view_state()
                        .active_conversation_id()
                });

            LeafContents::Terminal(TerminalPaneSnapshot {
                uuid: self.uuid.clone(),
                cwd: view.pwd_if_local(app),
                is_active,
                is_read_only: false,
                shell_launch_data: view.shell_launch_data_if_local(app),
                input_config: Some(current_input_config),
                llm_model_override,
                active_profile_id,
                conversation_ids_to_restore,
                active_conversation_id,
            })
        } else {
            LeafContents::Terminal(TerminalPaneSnapshot {
                uuid: self.uuid.clone(),
                cwd: None,
                is_active,
                is_read_only: false,
                shell_launch_data: None,
                input_config: None,
                llm_model_override: None,
                active_profile_id: None,
                conversation_ids_to_restore: vec![],
                active_conversation_id: None,
            })
        }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.terminal_view(ctx)
            .update(ctx, |view, ctx| view.redetermine_global_focus(ctx));
    }

    fn shareable_link(
        &self,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        let manager = self.terminal_manager(ctx);
        let the_model = manager.as_ref(ctx).model();
        let lock = the_model.lock();

        // Check if this is a conversation transcript viewer
        if lock.is_conversation_transcript_viewer() {
            // Try to get the conversation token from the history model
            let history_model = crate::ai::blocklist::BlocklistAIHistoryModel::handle(ctx);
            let terminal_view_id = self.terminal_view(ctx).id();

            // Find the conversation for this terminal view
            // We're assuming the conversation transcript view only has one conversation.
            // TODO(roland): store conversation id or server conversation token on the model ConversationTranscriptViewerStatus
            if let Some(conversation) = history_model
                .as_ref(ctx)
                .all_live_conversations_for_terminal_view(terminal_view_id)
                .next()
            {
                if let Some(token) = conversation.server_conversation_token() {
                    let url_string = token.conversation_link();
                    if let Ok(url) = url::Url::parse(&url_string) {
                        return Ok(ShareableLink::Pane { url });
                    }
                }
            }

            // If we can't get the conversation link yet (still loading or not available),
            // return Expected error to preserve the current browser URL
            return Err(ShareableLinkError::Expected);
        }

        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}

/// Attaches a terminal view to the pane group by subscribing to its events
/// and setting the file tree code model.
fn attach_terminal_view(
    terminal_view: &ViewHandle<TerminalView>,
    terminal_pane_id: TerminalPaneId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.subscribe_to_view(
        terminal_view,
        move |group: &mut PaneGroup, _, event, ctx| {
            handle_terminal_view_event(group, terminal_pane_id, event, ctx);
        },
    );
}

/// Handles events from the pane stack when views are added or removed.
fn handle_pane_stack_event(
    group: &mut PaneGroup,
    event: &PaneStackEvent<TerminalView>,
    terminal_pane_id: TerminalPaneId,
    ctx: &mut ViewContext<PaneGroup>,
) {
    match event {
        PaneStackEvent::ViewAdded(terminal_view) => {
            attach_terminal_view(terminal_view, terminal_pane_id, ctx);
        }
        PaneStackEvent::ViewRemoved(terminal_view) => {
            ctx.unsubscribe_to_view(terminal_view);
        }
    }

    // Ensure we use the new top-level view's title and active session status.
    // TODO(ben): This shouldn't be necessary once titles are set declaratively.
    if let Some(active_terminal) = group.terminal_view_from_pane_id(terminal_pane_id, ctx) {
        active_terminal.update(ctx, |view, ctx| view.on_pane_state_change(ctx));
    }
}

fn handle_terminal_view_event(
    group: &mut PaneGroup,
    terminal_pane_id: TerminalPaneId,
    event: &Event,
    ctx: &mut ViewContext<PaneGroup>,
) {
    let pane_id = terminal_pane_id.into();

    if group.pane_contents.contains_key(&pane_id) {
        match event {
            Event::Escape => ctx.emit(pane_group::Event::Escape),
            Event::ExecuteCommand(event) => {
                ctx.emit(pane_group::Event::ExecuteCommand(event.clone()));
            }
            Event::Exited => {
                // If the shell process exited before it successfully bootstrapped,
                // keep the pane open.  There might be useful information visible
                // in the output, and if this was the first shell spawned when the
                // user started the app, it will prevent it from suddenly quitting.
                if group
                    .terminal_view_from_pane_id(terminal_pane_id, ctx)
                    .is_some_and(|terminal_view| {
                        !terminal_view.as_ref(ctx).is_login_shell_bootstrapped()
                    })
                {
                    return;
                }

                group.close_pane(pane_id, ctx);
            }
            Event::CloseRequested => {
                group.close_pane_with_confirmation(pane_id, ctx);
            }
            Event::Pane(pane_event) => group.handle_pane_event(pane_id, pane_event, ctx),
            Event::BlockListCleared => {
                // Capture CMD-K to clear blocks here so we could remove
                // all the associated blocks stored in the history.
                if let Some(terminal_pane) = group.terminal_session_by_id(pane_id) {
                    terminal_pane.delete_blocks(ctx);
                }
            }
            Event::SendNotification(notification) => {
                ctx.emit(pane_group::Event::SendNotification {
                    notification: notification.clone(),
                    pane_id,
                })
            }
            Event::PluggableNotification { title, body } => {
                let message = if let Some(t) = title {
                    format!("{t}: {body}")
                } else {
                    body.clone()
                };
                ctx.emit(pane_group::Event::ShowToast {
                    message,
                    flavor: ToastFlavor::Default,
                    pane_id: Some(pane_id),
                })
            }
            Event::AppStateChanged => {
                ctx.emit(pane_group::Event::AppStateChanged);
            }
            Event::BlockCompleted { block, is_local } => {
                match group.terminal_session_by_id(pane_id) {
                    Some(pane) => {
                        if *GeneralSettings::as_ref(ctx).restore_session
                            && AppExecutionMode::as_ref(ctx).can_save_session()
                        {
                            if let Some(sender) = &group.model_event_sender {
                                let block_completed_event = ModelEvent::SaveBlock(BlockCompleted {
                                    pane_id: pane.session_uuid(),
                                    block: block.clone(),
                                    is_local: *is_local,
                                });

                                let sender_clone = sender.clone();
                                let _ = ctx.spawn(async move {
                                // Sending over a sync sender can block the current thread, so we do this async.
                                sender_clone.send(block_completed_event)
                            }, move |_, res, _| {
                                if let Err(err) = res {
                                    log::error!("Error sending block completed event for terminal id {terminal_pane_id:?} {err:?}");
                                }
                            });
                            }
                        }
                        ctx.emit(pane_group::Event::ActiveSessionChanged);
                    }
                    None => {
                        log::error!("Could not find uuid for terminal id: {terminal_pane_id:?}");
                    }
                };
            }
            Event::SessionBootstrapped => {
                ctx.emit(pane_group::Event::ActiveSessionChanged);
            }
            Event::OpenSettings(section) => {
                ctx.emit(pane_group::Event::OpenSettings(*section));
            }
            #[cfg(not(target_family = "wasm"))]
            Event::OpenPluginInstructionsPane(agent, kind) => {
                ctx.emit(pane_group::Event::OpenPluginInstructionsPane(*agent, *kind));
            }
            Event::SyncInput(sync_event) => {
                if SyncedInputState::as_ref(ctx)
                    .should_sync_this_pane_group(ctx.view_id(), ctx.window_id())
                {
                    ctx.emit(pane_group::Event::SyncInput(sync_event.clone()));
                }
            }
            Event::ShowCommandSearch(options) => {
                ctx.emit(pane_group::Event::ShowCommandSearch(options.clone()));
            }
            Event::TerminalViewStateChanged => {
                ctx.emit(pane_group::Event::TerminalViewStateChanged);
            }
            Event::OpenWorkflowModalWithCommand(command) => {
                ctx.emit(pane_group::Event::OpenWorkflowModalWithCommand(
                    command.clone(),
                ));
            }
            Event::OpenWorkflowModalWithLocalWorkflow(workflow_id) => {
                ctx.emit(pane_group::Event::OpenLocalWorkflowForEdit(
                    workflow_id.clone(),
                ));
            }
            Event::OpenWorkflowModalWithTemporary(workflow) => {
                ctx.emit(pane_group::Event::OpenWorkflowModalWithTemporary(
                    workflow.clone(),
                ));
            }
            Event::OpenPromptEditor => {
                ctx.emit(pane_group::Event::OpenPromptEditor);
            }
            Event::OpenAgentToolbarEditor => {
                ctx.emit(pane_group::Event::OpenAgentToolbarEditor);
            }
            Event::OpenCLIAgentToolbarEditor => {
                ctx.emit(pane_group::Event::OpenCLIAgentToolbarEditor);
            }
            Event::OpenFileInWarp { path, session } => {
                ctx.emit(pane_group::Event::OpenFileInWarp {
                    path: path.clone(),
                    session: session.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::PreviewCodeInWarp { source } => {
                ctx.emit(pane_group::Event::PreviewCodeInWarp {
                    source: source.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::OpenCodeInWarp { source, layout } => {
                ctx.emit(pane_group::Event::OpenCodeInWarp {
                    source: source.clone(),
                    layout: *layout,
                    line_col: if let CodeSource::Link { range_start, .. } = source {
                        *range_start
                    } else {
                        None
                    },
                });
            }
            Event::OpenCodeDiff { view } => {
                ctx.emit(pane_group::Event::OpenCodeDiff { view: view.clone() });
            }
            Event::OpenCodeReviewPane(arg) => {
                ctx.emit(pane_group::Event::OpenCodeReviewPane(arg.clone()));
            }
            Event::OpenCodeReviewPaneAndScrollToComment {
                open_code_review,
                comment,
                diff_mode,
            } => {
                ctx.emit(pane_group::Event::OpenCodeReviewPaneAndScrollToComment {
                    open_code_review: open_code_review.clone(),
                    comment: comment.clone(),
                    diff_mode: diff_mode.clone(),
                });
            }
            Event::ImportAllCodeReviewComments {
                open_code_review,
                comments,
                diff_mode,
            } => {
                ctx.emit(pane_group::Event::ImportAllCodeReviewComments {
                    open_code_review: open_code_review.clone(),
                    comments: comments.clone(),
                    diff_mode: diff_mode.clone(),
                });
            }
            Event::ToggleCodeReviewPane(arg) => {
                ctx.emit(pane_group::Event::ToggleCodeReviewPane(arg.clone()));
            }
            Event::FocusSession => {
                group.focus_pane(terminal_pane_id.into(), true, ctx);
                ctx.emit(pane_group::Event::FocusPaneGroup);
            }
            Event::OpenAIFactCollection { sync_id } => {
                ctx.emit(pane_group::Event::OpenAIFactCollection {
                    sync_id: sync_id.clone(),
                });
            }
            Event::SummarizationCancelDialogToggled { is_open } => {
                group.terminal_with_open_summarization_dialog = is_open.then_some(terminal_pane_id);
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            Event::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                ctx.emit(pane_group::Event::OpenFileWithTarget {
                    path: path.clone(),
                    target: target.clone(),
                    line_col: *line_col,
                });
            }
            Event::CopyFileToRemote { command, upload_id } => {
                let new_pane_id = group.insert_terminal_pane(
                    Direction::Right,
                    pane_id,
                    None, /*chosen_shell*/
                    ctx,
                );

                group.hide_pane_for_job(new_pane_id.into(), ctx);

                let new_terminal_view = group
                    .active_session_view(ctx)
                    .expect("should have new terminal view");
                new_terminal_view.update(ctx, |terminal_view, ctx| {
                    terminal_view.set_pending_command(command, ctx);
                    terminal_view.set_is_ssh_uploader(true);
                });

                ctx.emit(pane_group::Event::FileUploadCommand {
                    upload_id: *upload_id,
                    command: command.to_owned(),
                    remote_pane_id: terminal_pane_id,
                    local_pane_id: new_pane_id,
                });

                group.focus_pane(pane_id, true, ctx);
            }
            Event::FileUploadPasswordPending => {
                ctx.emit(pane_group::Event::FileUploadPasswordPending {
                    local_pane_id: terminal_pane_id,
                });
            }
            Event::OpenConversationHistory => {
                ctx.emit(OpenConversationHistory);
            }
            Event::FileUploadFinished(exit_code) => {
                ctx.emit(pane_group::Event::FileUploadFinished {
                    local_pane_id: terminal_pane_id,
                    exit_code: *exit_code,
                });

                // Each upload spawns its own new terminal pane. Once an upload
                // has finished, we know that its terminal session will no
                // longer be responsible for any UI-based uploads.
                if let Some(uploader_terminal_view) =
                    group.terminal_view_from_pane_id(terminal_pane_id, ctx)
                {
                    uploader_terminal_view.update(ctx, |terminal_view, _ctx| {
                        terminal_view.set_is_ssh_uploader(false);
                    });
                }
            }
            Event::OpenFileUploadSession(upload_id) => {
                ctx.emit(pane_group::Event::OpenFileUploadSession {
                    remote_pane_id: terminal_pane_id,
                    upload_id: *upload_id,
                })
            }
            Event::TerminateFileUploadSession(upload_id) => {
                ctx.emit(pane_group::Event::TerminateFileUploadSession {
                    remote_pane_id: terminal_pane_id,
                    upload_id: *upload_id,
                })
            }
            Event::OpenThemeChooser => {
                ctx.emit(pane_group::Event::OpenThemeChooser);
            }
            Event::OpenMCPSettingsPage { page } => {
                ctx.emit(pane_group::Event::OpenMCPSettingsPage { page: *page });
            }
            Event::OpenFilesPalette { source } => {
                ctx.emit(pane_group::Event::OpenFilesPalette { source: *source })
            }
            Event::OpenAddRulePane => {
                ctx.emit(crate::pane_group::Event::OpenAddRulePane);
            }
            Event::OpenRulesPane => {
                ctx.emit(crate::pane_group::Event::OpenAIFactCollection { sync_id: None });
            }
            Event::OpenAddPromptPane { initial_content } => {
                ctx.emit(crate::pane_group::Event::OpenAddPromptPane {
                    initial_content: initial_content.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::FileRenamed { old_path, new_path } => {
                ctx.emit(pane_group::Event::FileRenamed {
                    old_path: old_path.clone(),
                    new_path: new_path.clone(),
                });
            }
            #[cfg(feature = "local_fs")]
            Event::FileDeleted { path } => {
                ctx.emit(pane_group::Event::FileDeleted { path: path.clone() });
            }
            Event::ToggleLeftPanel {
                target_view,
                force_open,
            } => {
                ctx.emit(pane_group::Event::ToggleLeftPanel {
                    target_view: *target_view,
                    force_open: *force_open,
                });
            }
            Event::ToggleAIDocumentPane {
                document_id,
                document_version,
            } => {
                if let Some(conversation_id) =
                    crate::ai::document::ai_document_model::AIDocumentModel::as_ref(ctx)
                        .get_conversation_id_for_document_id(document_id)
                {
                    group.toggle_ai_document_pane(
                        conversation_id,
                        *document_id,
                        *document_version,
                        ctx,
                    );
                }
            }
            Event::HideAIDocumentPanes => {
                group.close_all_ai_document_panes(ctx);
            }
            Event::OpenAIDocumentPane {
                document_id,
                document_version,
                is_auto_open,
            } => {
                let should_open = if *is_auto_open {
                    // Auto-open: only open if there's already a visible plan pane
                    // (to replace it with the newest plan) or if there's enough space.
                    let has_visible_ai_doc_pane = group
                        .ai_document_panes()
                        .any(|pane_id| !group.is_pane_hidden_for_close(pane_id));

                    has_visible_ai_doc_pane
                        || group
                            .terminal_view_from_pane_id(terminal_pane_id, ctx)
                            .is_some_and(|tv| tv.as_ref(ctx).can_auto_open_panel())
                } else {
                    // User-triggered: always open.
                    true
                };

                if should_open {
                    if let Some(conversation_id) =
                        crate::ai::document::ai_document_model::AIDocumentModel::as_ref(ctx)
                            .get_conversation_id_for_document_id(document_id)
                    {
                        group.open_ai_document_pane(
                            conversation_id,
                            *document_id,
                            *document_version,
                            ctx,
                        );
                    }
                }
            }
            Event::OpenAgentProfileEditor { profile_id } => {
                ctx.emit(pane_group::Event::OpenAgentProfileEditor {
                    profile_id: *profile_id,
                });
            }
            Event::InsertCodeReviewComments {
                repo_path,
                comments,
                diff_mode,
                open_code_review,
            } => {
                ctx.emit(pane_group::Event::InsertCodeReviewComments {
                    repo_path: repo_path.to_path_buf(),
                    comments: comments.to_owned(),
                    diff_mode: diff_mode.to_owned(),
                    open_code_review: open_code_review.clone(),
                });
            }
            Event::RevealChildAgent { conversation_id } => {
                if let Some(&child_pane_id) = group.child_agent_panes.get(conversation_id) {
                    group.panes.show_pane_for_child_agent(child_pane_id);
                    group.handle_pane_count_change(ctx);
                    group.focus_pane(child_pane_id, true, ctx);
                } else {
                    log::warn!("No hidden pane found for child conversation {conversation_id:?}");
                }
            }
            Event::StartAgentConversation(request) => {
                let request = request.clone();
                match request.execution_mode.clone() {
                    StartAgentExecutionMode::Local { harness_type: None } => {
                        if let Some(HiddenChildAgentConversation {
                            terminal_view: new_terminal_view,
                            conversation_id,
                            ..
                        }) = create_hidden_child_agent_conversation(
                            group,
                            pane_id,
                            request.name,
                            request.parent_conversation_id,
                            HashMap::new(),
                            ctx,
                        ) {
                            register_legacy_local_lifecycle_subscription(
                                request.parent_conversation_id,
                                conversation_id,
                                request.lifecycle_subscription,
                                ctx,
                            );

                            new_terminal_view.update(ctx, |terminal_view, ctx| {
                                terminal_view
                                    .ai_controller()
                                    .update(ctx, |controller, ctx| {
                                        controller.send_agent_query_in_conversation(
                                            request.prompt,
                                            conversation_id,
                                            ctx,
                                        );
                                    });

                                terminal_view.enter_agent_view(
                                    None,
                                    Some(conversation_id),
                                    AgentViewEntryOrigin::ChildAgent,
                                    ctx,
                                );
                            });
                        }
                    }
                    #[cfg(not(target_family = "wasm"))]
                    StartAgentExecutionMode::Local {
                        harness_type: Some(harness_type),
                    } => {
                        let startup_directory =
                            group.startup_path_for_new_session(Some(terminal_pane_id), ctx);
                        let parent_pane_id = pane_id;
                        let request_name = request.name.clone();
                        let parent_conversation_id = request.parent_conversation_id;
                        let prompt = request.prompt.clone();
                        let shell_type = group
                            .terminal_view_from_pane_id(parent_pane_id, ctx)
                            .and_then(|terminal_view| {
                                terminal_view.as_ref(ctx).active_session_shell_type(ctx)
                            });

                        let _ = ctx.spawn(
                            async move {
                                prepare_local_harness_child_launch(
                                    prompt,
                                    harness_type,
                                    shell_type,
                                    startup_directory,
                                )
                                .await
                            },
                            move |group, result, ctx| match result {
                                Ok(launch) => {
                                    let PreparedLocalHarnessLaunch {
                                        command,
                                        env_vars,
                                        run_id,
                                        local_run_id,
                                    } = launch;
                                    if let Some(HiddenChildAgentConversation {
                                        terminal_view: new_terminal_view,
                                        terminal_view_id,
                                        conversation_id,
                                        ..
                                    }) = create_hidden_child_agent_conversation(
                                        group,
                                        parent_pane_id,
                                        request_name.clone(),
                                        parent_conversation_id,
                                        env_vars,
                                        ctx,
                                    ) {
                                        BlocklistAIHistoryModel::handle(ctx).update(
                                            ctx,
                                            |history_model, ctx| {
                                                history_model.assign_run_id_for_conversation(
                                                    conversation_id,
                                                    run_id,
                                                    Some(local_run_id),
                                                    terminal_view_id,
                                                    ctx,
                                                );
                                            },
                                        );

                                        new_terminal_view.update(ctx, |terminal_view, ctx| {
                                            terminal_view.execute_command_or_set_pending(
                                                &command,
                                                ctx,
                                            );
                                            terminal_view.enter_agent_view(
                                                None,
                                                Some(conversation_id),
                                                AgentViewEntryOrigin::ChildAgent,
                                                ctx,
                                            );
                                        });
                                    } else {
                                        create_error_child_agent_conversation(
                                            group,
                                            parent_pane_id,
                                            request_name,
                                            parent_conversation_id,
                                            "Failed to create a hidden pane for the local child harness."
                                                .to_string(),
                                            ctx,
                                        );
                                    }
                                }
                                Err(error_message) => {
                                    create_error_child_agent_conversation(
                                        group,
                                        parent_pane_id,
                                        request_name,
                                        parent_conversation_id,
                                        error_message,
                                        ctx,
                                    );
                                }
                            },
                        );
                    }
                    #[cfg(target_family = "wasm")]
                    StartAgentExecutionMode::Local { .. } => {
                        create_error_child_agent_conversation(
                            group,
                            pane_id,
                            request.name,
                            request.parent_conversation_id,
                            "Local harness child agents are not supported in WASM builds."
                                .to_string(),
                            ctx,
                        );
                    }
                    StartAgentExecutionMode::Remote { .. } => {
                        create_error_child_agent_conversation(
                            group,
                            pane_id,
                            request.name,
                            request.parent_conversation_id,
                            "Hosted remote child agents are unavailable in local-only Warper."
                                .to_string(),
                            ctx,
                        );
                    }
                }
            }
            _ => {}
        }
    } else {
        log::warn!("Session {terminal_pane_id:?} not found");
    }
}

#[cfg(feature = "local_fs")]
fn handle_ai_history_event(
    event: &BlocklistAIHistoryEvent,
    terminal_view_id: EntityId,
    terminal_pane_id: TerminalPaneId,
    model_event_sender: SyncSender<ModelEvent>,
    ctx: &mut ViewContext<PaneGroup>,
) {
    use std::sync::Arc;

    use crate::ai::blocklist::{
        AIQueryHistoryOutputStatus, PersistedAIInput, PersistedAIInputType,
    };

    if event
        .terminal_view_id()
        .is_some_and(|id| id != terminal_view_id)
    {
        return;
    }

    match event {
        BlocklistAIHistoryEvent::AppendedExchange {
            exchange_id,
            conversation_id,
            is_hidden,
            ..
        }
        | BlocklistAIHistoryEvent::UpdatedStreamingExchange {
            exchange_id,
            conversation_id,
            is_hidden,
            ..
        } => {
            // Check if session restoration is enabled.
            if !*GeneralSettings::as_ref(ctx).restore_session
                || !AppExecutionMode::as_ref(ctx).can_save_session()
            {
                return;
            }

            let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
            else {
                log::warn!("Received event with invalid conversation ID: {conversation_id:?}");
                return;
            };

            let Some(exchange) = conversation.exchange_with_id(*exchange_id) else {
                log::warn!("Received event with invalid exchange ID: {exchange_id:?}");
                return;
            };

            // Hidden blocks and passive-only conversations should not be restored, so we skip
            // them.
            if *is_hidden || conversation.is_entirely_passive() {
                return;
            }

            let persisted_query = PersistedAIInput {
                start_ts: exchange.start_time,
                inputs: exchange
                    .input
                    .iter()
                    .filter_map(|input| PersistedAIInputType::try_from(input).ok())
                    .collect(),
                exchange_id: exchange.id,
                conversation_id: *conversation_id,
                output_status: AIQueryHistoryOutputStatus::from(&exchange.output_status),
                working_directory: exchange.working_directory.clone(),
                // TODO(CORE-3546): shell: exchange.shell.clone(),
                model_id: exchange.model_id.clone(),
                coding_model_id: exchange.coding_model_id.clone(),
            };
            let upsert_ai_query_event = ModelEvent::UpsertAIQuery {
                query: Arc::new(persisted_query),
            };
            let _ = ctx.spawn(
                // Sending over a sync sender can block the current thread, so we
                // do this async.
                async move { model_event_sender.send(upsert_ai_query_event) },
                move |_, res, _| {
                    if let Err(err) = res {
                        log::error!(
                            "Error sending upsert AI query event for terminal id {terminal_pane_id:?} {err:?}"
                        );
                    }
                },
            );
        }
        BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
        | BlocklistAIHistoryEvent::ClearedActiveConversation { .. } => {
            ctx.emit(pane_group::Event::InvalidatedActiveConversation);
        }
        BlocklistAIHistoryEvent::RemoveConversation {
            conversation_id, ..
        } => {
            let conversation_id = conversation_id.to_string();
            // On remove, delete all related AI query and multi-agent conversation data for this conversation.
            let _ = ctx.spawn(
                async move {
                    model_event_sender.send(ModelEvent::DeleteAIConversation {
                        conversation_id: conversation_id.clone(),
                    })?;
                    model_event_sender.send(ModelEvent::DeleteMultiAgentConversations {
                        conversation_ids: vec![conversation_id],
                    })
                },
                |_, res, _| {
                    if let Err(err) = res {
                        log::error!("Error sending delete events for conversation: {err:?}");
                    }
                },
            );
        }
        // DeletedConversation SQL cleanup is handled directly in delete_conversation().
        BlocklistAIHistoryEvent::DeletedConversation { .. }
        | BlocklistAIHistoryEvent::StartedNewConversation { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
        | BlocklistAIHistoryEvent::ReassignedExchange { .. }
        | BlocklistAIHistoryEvent::SetActiveConversation { .. }
        | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
        | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
        | BlocklistAIHistoryEvent::SplitConversation { .. }
        | BlocklistAIHistoryEvent::RestoredConversations { .. }
        | BlocklistAIHistoryEvent::CreatedSubtask { .. }
        | BlocklistAIHistoryEvent::UpgradedTask { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
        | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
        | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. } => (),
    }
}
