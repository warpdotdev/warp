//! [`TerminalView`]-specific implementation for ambient agent functionality.

use std::cell::Cell;
use std::rc::Rc;
use warp_cli::agent::Harness;
use warp_terminal::model::BlockId;

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::AIRequestUsageModel;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warpui::prelude::{Empty, Vector2F};

use crate::ai::ambient_agents::telemetry::{CloudAgentTelemetryEvent, CloudModeEntryPoint};
use crate::ai::blocklist::{agent_view::AgentViewEntryOrigin, BlocklistAIHistoryModel};
use crate::ai::conversation_details_panel::ConversationDetailsData;
use crate::pane_group::TerminalViewResources;
use crate::server::server_api::ai::SpawnAgentRequest;
use crate::terminal::view::ambient_agent::CloudModeInitialUserQuery;
use crate::terminal::view::rich_content::{RichContentInsertionPosition, RichContentMetadata};
use crate::terminal::view::TerminalView;
use crate::terminal::CLIAgent;
use crate::workspace::view::cloud_agent_capacity_modal::CloudAgentCapacityModalVariant;
use crate::workspaces::user_workspaces::UserWorkspaces;
use warp_core::ui::appearance::Appearance;
use warpui::elements::Align;
use warpui::{AppContext, Element, EntityId, SingletonEntity, ViewContext};

use super::loading_screen::{
    render_cloud_mode_cancelled_screen, render_cloud_mode_error_screen,
    render_cloud_mode_github_auth_required_screen, render_cloud_mode_loading_screen,
};
use super::{
    is_cloud_agent_pre_first_exchange, AmbientAgentEntryBlock, AmbientAgentViewModelEvent,
};
const CHILD_AGENT_GITHUB_AUTH_REQUIRED_BLOCKED_ACTION: &str =
    "GitHub authentication required before starting the child agent.";

impl TerminalView {
    fn active_ambient_agent_conversation_id(&self, ctx: &AppContext) -> Option<AIConversationId> {
        self.agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()
    }

    fn active_ambient_agent_conversation_is_child(&self, ctx: &AppContext) -> bool {
        let Some(conversation_id) = self.active_ambient_agent_conversation_id(ctx) else {
            return false;
        };

        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.is_child_agent_conversation())
    }

    fn update_active_ambient_agent_conversation_status(
        &self,
        status: ConversationStatus,
        error_message: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(conversation_id) = self.active_ambient_agent_conversation_id(ctx) else {
            return;
        };

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            history_model.update_conversation_status_with_error_message(
                self.id(),
                conversation_id,
                status,
                error_message,
                ctx,
            );
        });
    }

    pub(in crate::terminal::view) fn show_out_of_credits_modal(&self, ctx: &mut ViewContext<Self>) {
        let is_on_paid_plan = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .is_some_and(|workspace| workspace.billing_metadata.is_user_on_paid_plan());

        if is_on_paid_plan {
            ctx.emit(crate::terminal::view::Event::ShowCloudAgentCapacityModal {
                variant: CloudAgentCapacityModalVariant::OutOfCredits,
            });
        } else {
            AIRequestUsageModel::handle(ctx).update(ctx, |model, ctx| {
                model.refresh_request_usage_async(ctx);
            });
        }
    }

    /// Handles ambient agent view model events.
    pub(in crate::terminal::view) fn handle_ambient_agent_event(
        &mut self,
        event: &AmbientAgentViewModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.clone() else {
            return;
        };

        // Tear down the non-oz cloud-mode queued-prompt block on terminal / transition
        // events that replace it. `Failed`, `NeedsGithubAuth`, and `Cancelled` hand off
        // to the existing error / auth / cancelled UI; `HarnessCommandStarted` hands
        // off to the live harness CLI block. Idempotent and cheap when no block exists.
        if matches!(
            event,
            AmbientAgentViewModelEvent::Failed { .. }
                | AmbientAgentViewModelEvent::NeedsGithubAuth
                | AmbientAgentViewModelEvent::Cancelled
                | AmbientAgentViewModelEvent::HarnessCommandStarted
        ) {
            self.remove_pending_user_query_block(ctx);
        }

        match event {
            AmbientAgentViewModelEvent::EnteredSetupState => {
                // Re-render to show the setup view.
                self.update_pane_configuration(ctx);
                ctx.notify();
            }
            AmbientAgentViewModelEvent::EnteredComposingState => {
                // Update pane configuration to show cloud indicator.
                self.update_pane_configuration(ctx);
            }
            AmbientAgentViewModelEvent::DispatchedAgent => {
                // Pane chrome (e.g. cloud indicator, task id) must update on viewer surfaces
                // too, so this runs above the viewer short-circuit below.
                self.update_pane_configuration(ctx);
                // Only the spawner's view handles `DispatchedAgent`. Viewer surfaces (shared
                // ambient agent session or transcript viewer) have no submitted prompt to render
                // and should not insert cloud-mode rich content here.
                let is_viewer = self.is_shared_ambient_agent_session()
                    || self.model.lock().is_conversation_transcript_viewer();
                if is_viewer {
                    ctx.notify();
                    return;
                }
                if FeatureFlag::CloudModeSetupV2.is_enabled() {
                    if ambient_agent_view_model
                        .as_ref(ctx)
                        .is_third_party_harness()
                    {
                        // Non-oz runs: render the submitted prompt via the queued-prompt UI.
                        // The block is removed later by `HarnessCommandStarted` / failure /
                        // cancel / auth handlers.
                        let prompt = ambient_agent_view_model
                            .as_ref(ctx)
                            .request()
                            .map(|request| request.prompt.clone())
                            .unwrap_or_default();
                        if !prompt.is_empty() {
                            self.insert_cloud_mode_queued_user_query_block(prompt, ctx);
                        }
                    } else {
                        let initial_user_query = ctx.add_view(|ctx| {
                            CloudModeInitialUserQuery::new(ambient_agent_view_model.clone(), ctx)
                        });
                        self.insert_rich_content(
                            None,
                            initial_user_query,
                            None,
                            RichContentInsertionPosition::Append {
                                insert_below_long_running_block: true,
                            },
                            ctx,
                        );
                        ambient_agent_view_model.update(ctx, |model, _| {
                            model.set_has_inserted_cloud_mode_user_query_block(true);
                        });
                    }
                } else {
                    // Reset tip cooldown so the first tip shows for 60 seconds
                    let tip_model = ambient_agent_view_model
                        .as_ref(ctx)
                        .ui_state
                        .tip_model
                        .clone();
                    tip_model.update(ctx, |model, model_ctx| {
                        model.reset_cooldown(model_ctx);
                    });
                }
                // Re-render to show loading state.
                ctx.notify();
            }
            AmbientAgentViewModelEvent::SessionReady { .. }
            | AmbientAgentViewModelEvent::FollowupSessionReady { .. } => {
                // Auto-open details panel for local cloud mode once the session is ready.
                self.maybe_auto_open_cloud_mode_details_panel(ctx);
                // Re-render to hide the loading screen now that the session is ready.
                ctx.notify();
            }
            AmbientAgentViewModelEvent::EnvironmentSelected => {}
            AmbientAgentViewModelEvent::ProgressUpdated => {
                // Refresh the tip (respects 60s cooldown internally)
                let tip_model = ambient_agent_view_model
                    .as_ref(ctx)
                    .ui_state
                    .tip_model
                    .clone();
                tip_model.update(ctx, |model, model_ctx| {
                    model.maybe_refresh_tip(model_ctx);
                });
                // Update pane header to reflect any changes (e.g., task_id being set)
                self.update_pane_configuration(ctx);
                ctx.notify();
            }
            AmbientAgentViewModelEvent::Failed { error_message } => {
                self.update_active_ambient_agent_conversation_status(
                    ConversationStatus::Error,
                    Some(error_message.clone()),
                    ctx,
                );
                // Refresh the details panel to show failed status
                if self.is_cloud_mode_details_panel_open {
                    self.fetch_and_update_cloud_mode_details_panel(ctx);
                }
                // Re-render to show the error state in the footer.
                ctx.notify();
            }
            AmbientAgentViewModelEvent::ShowCloudAgentCapacityModal => {
                if FeatureFlag::CloudMode.is_enabled()
                    && ambient_agent_view_model.as_ref(ctx).is_ambient_agent()
                    && !self.model.lock().is_shared_ambient_agent_session()
                {
                    ctx.emit(crate::terminal::view::Event::ShowCloudAgentCapacityModal {
                        variant: CloudAgentCapacityModalVariant::ConcurrentLimit,
                    });
                }

                ctx.notify();
            }
            AmbientAgentViewModelEvent::ShowAICreditModal => {
                if FeatureFlag::CloudMode.is_enabled()
                    && ambient_agent_view_model.as_ref(ctx).is_ambient_agent()
                    && !self.model.lock().is_shared_ambient_agent_session()
                {
                    self.show_out_of_credits_modal(ctx);
                }

                ctx.notify();
            }
            AmbientAgentViewModelEvent::NeedsGithubAuth => {
                if self.active_ambient_agent_conversation_is_child(ctx) {
                    self.update_active_ambient_agent_conversation_status(
                        ConversationStatus::Blocked {
                            blocked_action: CHILD_AGENT_GITHUB_AUTH_REQUIRED_BLOCKED_ACTION
                                .to_string(),
                        },
                        None,
                        ctx,
                    );
                }
                // Re-render to show the GitHub auth required state in the footer.
                ctx.notify();
            }
            AmbientAgentViewModelEvent::Cancelled => {
                self.update_active_ambient_agent_conversation_status(
                    ConversationStatus::Cancelled,
                    None,
                    ctx,
                );
                // Refresh the details panel to show cancelled status
                if self.is_cloud_mode_details_panel_open {
                    self.fetch_and_update_cloud_mode_details_panel(ctx);
                }
                // Re-render to show the cancelled state in the footer.
                ctx.notify();
            }
            AmbientAgentViewModelEvent::HarnessSelected => {
                self.maybe_enter_agent_view_for_shared_third_party_viewer(ctx);
                ctx.notify();
            }
            AmbientAgentViewModelEvent::HarnessCommandStarted => {
                // Stop classifying new blocks as environment setup commands, mirroring the
                // Oz path in the `AppendedExchange` handler. Flipping this flag to `false`
                // also un-hides and un-marks the active block so it renders like a normal
                // CLI-agent session.
                {
                    let mut model = self.model.lock();
                    if model
                        .block_list()
                        .is_executing_oz_environment_startup_commands()
                    {
                        model
                            .block_list_mut()
                            .set_is_executing_oz_environment_startup_commands(false);
                    }
                }
                // Collapse the setup-commands summary, matching the oz first-exchange behavior.
                ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_setup_command_visibility(false, ctx);
                });
                // Force a fresh viewer size report to the sharer so the harness CLI (e.g.
                // the claude TUI) starts at our terminal's actual dimensions instead of
                // whatever the sandbox PTY was sized to during setup.
                self.force_report_viewer_terminal_size(ctx);
                ctx.notify();
            }
            AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility => (),
        }
    }

    pub(in crate::terminal::view) fn maybe_insert_setup_command_blocks(
        &mut self,
        block_id: &BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::CloudModeSetupV2.is_enabled() {
            return;
        }

        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.clone() else {
            return;
        };

        if !is_cloud_agent_pre_first_exchange(
            self.ambient_agent_view_model.as_ref(),
            &self.agent_view_controller,
            ctx,
        ) {
            return;
        }

        // For non-oz harness runs, transition out of the setup phase when the harness CLI
        // starts (e.g. `claude --session-id …`). The block is the actual harness session
        // and should NOT be classified as a setup command; the `HarnessCommandStarted`
        // handler flips the block-list flag so the block renders like a normal CLI-agent
        // session.
        if ambient_agent_view_model
            .as_ref(ctx)
            .is_third_party_harness()
            && self.active_block_matches_run_harness(ctx)
        {
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.mark_harness_command_started(ctx);
            });
            return;
        }

        let Some(block_index) = self.model.lock().block_list().block_index_for_id(block_id) else {
            return;
        };

        if !ambient_agent_view_model
            .as_ref(ctx)
            .setup_command_state()
            .did_execute_a_setup_command()
        {
            ambient_agent_view_model.update(ctx, |model, _| {
                model
                    .setup_command_state_mut()
                    .set_did_execute_a_setup_command(true);
            });

            let setup_command_text = ctx.add_typed_action_view(|ctx| {
                super::CloudModeSetupTextBlock::new(
                    ambient_agent_view_model.clone(),
                    self.agent_view_controller.clone(),
                    ctx,
                )
            });
            self.insert_rich_content(
                None,
                setup_command_text,
                None,
                RichContentInsertionPosition::BeforeBlockIndex(block_index),
                ctx,
            );
        }

        let setup_command_block = ctx.add_typed_action_view(|ctx| {
            super::CloudModeSetupCommandBlock::new(
                block_id.clone(),
                ambient_agent_view_model.clone(),
                &self.model_events_handle,
                self.model.clone(),
                ctx,
            )
        });
        ctx.subscribe_to_view(&setup_command_block, |me, _, event, _| {
            let super::CloudModeSetupCommandBlockEvent::ToggleBlockVisibility(block_id) = event;
            me.model
                .lock()
                .block_list_mut()
                .toggle_visibility_of_block(block_id);
        });
        self.insert_rich_content(
            None,
            setup_command_block,
            None,
            RichContentInsertionPosition::BeforeBlockIndex(block_index),
            ctx,
        );
    }

    /// Enters agent view for a live shared-session viewer of a non-oz cloud run, so every
    /// viewer lands in the same agent-view chrome regardless of which entry point opened the
    /// conversation. Called from the `HarnessSelected` handler once the viewer has resolved
    /// the run's harness asynchronously.
    ///
    /// Transcript viewer entry is handled directly in `load_data_into_transcript_viewer` so
    /// the snapshot block exists before we retag — we intentionally do not trigger that path
    /// here.
    ///
    /// The viewer-context guard is load-bearing: `HarnessSelected` also fires when the local
    /// spawner picks a harness from the dropdown, and in that case the cloud-mode setup flow
    /// handles agent view entry instead.
    fn maybe_enter_agent_view_for_shared_third_party_viewer(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .is_active()
        {
            return;
        }
        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.as_ref() else {
            return;
        };
        if !ambient_agent_view_model
            .as_ref(ctx)
            .is_third_party_harness()
        {
            return;
        }
        if !self.is_shared_ambient_agent_session() {
            return;
        }

        self.enter_agent_view_for_new_conversation(
            None,
            AgentViewEntryOrigin::ThirdPartyCloudAgent,
            ctx,
        );

        let Some(vehicle_conversation_id) = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()
        else {
            return;
        };

        // Retag existing non-setup blocks so the harness content passes the agent view filter.
        self.model
            .lock()
            .block_list_mut()
            .attach_non_startup_blocks_to_conversation(vehicle_conversation_id);

        // Retag rich content inserted in terminal mode (setup-commands summary, tombstone, …)
        // so it stays visible under the vehicle conversation. Rich content with
        // `agent_view_conversation_id == None` is hidden in full-screen agent view by
        // `RichContentItem::should_hide_for_agent_view_state`.
        let ids_to_retag: Vec<EntityId> = self
            .rich_content_views
            .iter()
            .filter(|rc| rc.agent_view_conversation_id().is_none())
            .map(|rc| rc.view_id())
            .collect();
        for view_id in ids_to_retag {
            self.set_rich_content_agent_view_conversation_id(view_id, vehicle_conversation_id);
        }
    }

    /// Returns `true` when the active block's command is the CLI for the run's configured
    /// non-oz harness (e.g. `claude …` for [`Harness::Claude`]).
    /// Used to detect the harness-start transition at `AfterBlockStarted` time. Unlike
    /// `detect_cli_agent_from_model`, this does NOT gate on `is_active_and_long_running` —
    /// we want to classify the block as the harness session as soon as it starts, before the
    /// long-running timer would otherwise elapse.
    fn active_block_matches_run_harness(&self, ctx: &AppContext) -> bool {
        let command = self
            .model
            .lock()
            .block_list()
            .active_block()
            .command_with_secrets_obfuscated(false);
        let Some(cli_agent) = CLIAgent::detect(&command, None, None, ctx) else {
            return false;
        };
        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.as_ref() else {
            return false;
        };
        match ambient_agent_view_model.as_ref(ctx).selected_harness() {
            Harness::Oz => false,
            Harness::Claude => matches!(cli_agent, CLIAgent::Claude),
            Harness::OpenCode => matches!(cli_agent, CLIAgent::OpenCode),
            Harness::Gemini => matches!(cli_agent, CLIAgent::Gemini),
            Harness::Unknown => false,
        }
    }

    /// Enter cloud agent view from this existing session. Behavior depends on the current terminal state:
    ///
    /// 1. Already in nested cloud mode with empty convo (setup/composing): ignore.
    /// 2. Already in nested cloud mode with convo started: pop to parent terminal and start a
    ///    new cloud mode session there (siblings).
    /// 3. Not in nested cloud mode: enter cloud mode from this terminal session.
    pub(in crate::terminal::view) fn enter_cloud_agent_view(
        &mut self,
        initial_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let is_nested_cloud_mode = self.is_nested_cloud_mode(ctx);

        // (1) If we're currently in an empty cloud mode session (setup/composing; no
        // dispatched query yet), do not allow creating a new cloud mode session.
        if is_nested_cloud_mode
            && self.ambient_agent_view_model.as_ref().is_some_and(|model| {
                let model = model.as_ref(ctx);
                model.is_in_setup() || model.is_configuring_ambient_agent()
            })
        {
            return;
        }

        if is_nested_cloud_mode {
            // (2) Start a sibling cloud mode session at the terminal level.
            let Some(pane_stack) = self
                .pane_stack
                .as_ref()
                .and_then(|handle| handle.upgrade(ctx))
            else {
                log::warn!(
                    "Nested cloud mode has no pane stack; cannot pop to start sibling cloud mode session"
                );
                return;
            };

            if pane_stack.as_ref(ctx).depth() <= 1 {
                log::warn!(
                    "Nested cloud mode pane stack depth <= 1; cannot pop to start sibling cloud mode session"
                );
                return;
            }

            pane_stack.update(ctx, |stack, ctx| {
                stack.pop(ctx);
            });

            let active_view = pane_stack.as_ref(ctx).active_view().clone();
            active_view.update(ctx, |view, ctx| {
                view.enter_cloud_mode_from_session(initial_prompt, ctx);
            });

            ctx.notify();
            return;
        }

        // (3) Enter cloud mode from this terminal session.
        self.enter_cloud_mode_from_session(initial_prompt, ctx);
    }

    /// Enter cloud mode from this existing session with the given initial prompt.
    ///
    /// If called from fullscreen agent view, this defers the cloud mode start until after the
    /// agent view has exited so the resulting rich content is scoped to the terminal-level.
    pub(in crate::terminal::view) fn enter_cloud_mode_from_session(
        &mut self,
        initial_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !(FeatureFlag::CloudMode.is_enabled()
            && FeatureFlag::CloudModeFromLocalSession.is_enabled())
        {
            return;
        }

        // If cloud mode is started from fullscreen agent view, we must ensure the resulting
        // rich content (ambient agent entry block) is scoped to the terminal-level.
        if FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_fullscreen()
        {
            let prompt = initial_prompt.clone();
            self.set_pending_cloud_mode_start_callback(
                Box::new(move |view, ctx| {
                    view.start_cloud_mode(None, prompt, ctx);
                }),
                ctx,
            );

            // Starting cloud mode from agent view is analogous to starting a new agent
            // conversation: we exit without confirmation and continue after ExitedAgentView.
            self.agent_view_controller.update(ctx, |controller, ctx| {
                controller.exit_agent_view_without_confirmation(ctx);
            });

            return;
        }

        self.start_cloud_mode(None, initial_prompt, ctx);
    }

    /// Start a cloud mode session nested under this one.
    ///
    /// If `spawn_request` is `Some`, the agent is immediately started. Otherwise, it can
    /// further configured in the cloud mode session.
    fn start_cloud_mode(
        &mut self,
        spawn_request: Option<SpawnAgentRequest>,
        initial_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };

        // TODO: Use self.size_info
        let (terminal_view, terminal_manager) =
            super::create_cloud_mode_view(resources, Vector2F::zero(), ctx.window_id(), ctx);

        // Only insert an ambient agent entry block once the agent is actually dispatched.
        // This avoids persisting an empty "New cloud agent" entry when the user enters cloud mode
        // but exits without sending anything.
        let Some(ambient_agent_view_model) = terminal_view
            .as_ref(ctx)
            .ambient_agent_view_model()
            .cloned()
        else {
            log::warn!("Cloud mode view was created without an ambient agent view model");
            return;
        };
        let terminal_view_weak = terminal_view.downgrade();
        let terminal_manager_weak = terminal_manager.downgrade();
        let pane_stack = self.pane_stack.clone();
        let has_inserted_entry_block = Rc::new(Cell::new(false));

        ctx.subscribe_to_model(&ambient_agent_view_model, move |me, _, event, ctx| {
            if !matches!(event, AmbientAgentViewModelEvent::DispatchedAgent) {
                return;
            }

            if has_inserted_entry_block.get() {
                return;
            }
            has_inserted_entry_block.set(true);

            let Some(pane_stack) = pane_stack.clone() else {
                log::warn!(
                    "Pane stack not available; cannot insert ambient agent entry block for cloud mode"
                );
                return;
            };

            let Some(terminal_view) = terminal_view_weak.upgrade(ctx) else {
                return;
            };
            let Some(terminal_manager) = terminal_manager_weak.upgrade(ctx) else {
                return;
            };

            let block_terminal_view = terminal_view.clone();
            let block_terminal_manager = terminal_manager.clone();
            let block_handle = ctx.add_typed_action_view(|ctx| {
                AmbientAgentEntryBlock::new(
                    block_terminal_view,
                    block_terminal_manager,
                    pane_stack.clone(),
                    ctx,
                )
            });

            me.insert_rich_content(
                None,
                block_handle.clone(),
                Some(RichContentMetadata::AmbientAgentBlock { block_handle }),
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
        });

        let pane_config = self.pane_configuration.clone();
        let ambient_agent_view_model_for_update = ambient_agent_view_model.clone();
        terminal_view.update(ctx, |view, ctx| {
            view.set_pane_configuration(pane_config);

            if let Some(request) = spawn_request {
                // Spawn the agent immediately with the provided request.
                view.enter_agent_view_for_new_conversation(
                    None,
                    AgentViewEntryOrigin::CloudAgent,
                    ctx,
                );
                ambient_agent_view_model_for_update.update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(request, ctx);
                });
            } else {
                // Enter setup mode for composing a prompt
                view.enter_ambient_agent_setup(initial_prompt, ctx);
            }
        });

        if let Some(pane_stack) = self.pane_stack.clone() {
            if let Some(stack) = pane_stack.upgrade(ctx) {
                stack.update(ctx, |stack, ctx| {
                    stack.push(terminal_manager, terminal_view, ctx);
                });
            } else {
                log::warn!("Pane stack deallocated, cannot enter cloud mode");
            }
        } else {
            log::warn!("Pane stack not available, cannot enter cloud mode");
        }

        send_telemetry_from_ctx!(
            CloudAgentTelemetryEvent::EnteredCloudMode {
                entry_point: CloudModeEntryPoint::LocalSession
            },
            ctx
        );
    }

    /// Renders the ambient agent progress view based on agent progress.
    pub(in crate::terminal::view) fn render_ambient_agent_progress(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.as_ref() else {
            return Empty::new().finish();
        };
        let ambient_agent_model = ambient_agent_view_model.as_ref(app);
        let Some(progress) = ambient_agent_model.agent_progress() else {
            return Empty::new().finish();
        };

        // Show appropriate screen based on agent status
        let ui_state = &ambient_agent_model.ui_state;
        let screen = if ambient_agent_model.is_cancelled() {
            // Show cancelled screen
            render_cloud_mode_cancelled_screen(appearance)
        } else if let Some(auth_url) = ambient_agent_model.github_auth_url() {
            // Show GitHub auth required screen
            render_cloud_mode_github_auth_required_screen(
                auth_url,
                appearance,
                &ui_state.auth_button_mouse_state,
                app,
            )
        } else if let Some(error_message) = ambient_agent_model.error_message() {
            // Show error screen
            render_cloud_mode_error_screen(
                error_message,
                appearance,
                &ui_state.error_selection_handle,
                &ui_state.error_selected_text,
                app,
            )
        } else {
            // Show loading screen - determine the message based on progress state
            let message = if progress.harness_started_at.is_some() {
                "Starting Environment (Step 3/3)"
            } else if progress.claimed_at.is_some() {
                "Creating Environment (Step 2/3)"
            } else {
                "Connecting to Host (Step 1/3)"
            };

            render_cloud_mode_loading_screen(
                message,
                appearance,
                &ui_state.loading_shimmer_handle,
                &ui_state.tip_model,
                app,
            )
        };

        // Center the screen within the terminal view
        Align::new(screen).finish()
    }

    /// Handles events from the first-time cloud agent setup view.
    pub(in crate::terminal::view) fn handle_first_time_cloud_agent_setup_event(
        &mut self,
        event: &super::FirstTimeCloudAgentSetupViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            super::FirstTimeCloudAgentSetupViewEvent::Cancelled => {
                // Exit agent view (pops from nav stack)
                self.exit_agent_view(ctx);
            }
            super::FirstTimeCloudAgentSetupViewEvent::EnvironmentCreated => {
                // Set the environment on the ambient agent view model
                if let Some(ambient_agent_view_model) = self.ambient_agent_view_model.as_ref() {
                    ambient_agent_view_model.update(ctx, |model, ctx| {
                        // Transition from Setup to Composing
                        model.enter_composing_from_setup(ctx);
                    });
                }

                // Focus the input box so user can start typing
                self.focus_input_box(ctx);
            }
        }
    }

    /// Fetches task data and updates the cloud mode details panel.
    pub(in crate::terminal::view) fn fetch_and_update_cloud_mode_details_panel(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(task_id) = self.ambient_agent_task_id_for_details_panel(ctx) else {
            log::warn!("fetch_and_update_cloud_mode_details_panel called without task_id");
            return;
        };

        let task = crate::ai::agent_conversations_model::AgentConversationsModel::handle(ctx)
            .update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(&task_id, ctx)
            });

        let data = task
            .as_ref()
            .map(|task| ConversationDetailsData::from_task(task, None, None, ctx))
            .unwrap_or_else(|| ConversationDetailsData::from_task_id(task_id));
        self.cloud_mode_details_panel.update(ctx, |panel, ctx| {
            panel.set_conversation_details(data, ctx);
        });
    }

    /// Auto-opens the cloud mode details panel once.
    /// This is used for local cloud mode sessions (after SessionReady) and shared ambient sessions (after join).
    pub(in crate::terminal::view) fn maybe_auto_open_cloud_mode_details_panel(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.has_auto_opened_cloud_mode_details_panel {
            return;
        }
        self.is_cloud_mode_details_panel_open = true;
        self.has_auto_opened_cloud_mode_details_panel = true;
        self.fetch_and_update_cloud_mode_details_panel(ctx);
        ctx.notify();
    }
}
