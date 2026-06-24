mod auth_secret_ftux_dropdown;
mod auth_secret_ftux_view;
pub(crate) mod auth_secret_selector;
mod block;
mod delete_auth_secret_confirmation_dialog;
mod first_time_setup;
mod footer;
mod harness_selector;
mod host_selector;
mod loading_screen;
mod model;
mod model_selector;
mod progress;
mod progress_ui_state;
mod tips;
mod view_impl;

pub use auth_secret_ftux_view::{
    AuthSecretFtuxAction, AuthSecretFtuxView, AuthSecretFtuxViewEvent,
};
pub use auth_secret_selector::{
    AuthSecretSelector, AuthSecretSelectorAction, AuthSecretSelectorEvent,
};
pub use block::*;
pub use first_time_setup::{FirstTimeCloudAgentSetupView, FirstTimeCloudAgentSetupViewEvent};
pub use footer::{render_error_footer, render_loading_footer};
pub use harness_selector::{HarnessSelector, HarnessSelectorAction, HarnessSelectorEvent};
pub use host_selector::{
    Host, HostSelector, HostSelectorAction, HostSelectorEvent, NakedHeaderButtonTheme,
};
pub use loading_screen::{render_cloud_mode_error_screen, render_cloud_mode_loading_screen};
pub(crate) use model::should_disable_snapshot;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
pub(crate) use model::PendingHandoff;
pub use model::{AgentProgress, AmbientAgentViewModel, AmbientAgentViewModelEvent, Status};
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
pub(crate) use model::{HandoffSubmissionState, SnapshotUploadStatus};
pub use model_selector::{
    HarnessSelection, ModelSelection, ModelSelector, ModelSelectorAction, ModelSelectorEvent,
};
pub use progress::{render_progress, ProgressProps, ProgressStep, ProgressStepState};
pub use progress_ui_state::AmbientAgentProgressUIState;
pub use tips::{get_cloud_mode_tips, CloudModeTip};
use warp_core::features::FeatureFlag;
use warpui::geometry::vector::Vector2F;
use warpui::{AppContext, ModelHandle, ViewHandle, WindowId};

use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewState};
use crate::pane_group::TerminalViewResources;
use crate::terminal::{shared_session, TerminalManager, TerminalModel, TerminalView};

/// Creates a cloud mode terminal view and manager for ambient agent sessions.
/// See `viewer::TerminalManager::enable_orchestration_polling` for the flag.
pub fn create_cloud_mode_view(
    resources: TerminalViewResources,
    view_bounds_size: Vector2F,
    window_id: WindowId,
    enable_orchestration_polling: bool,
    ctx: &mut AppContext,
) -> (
    ViewHandle<TerminalView>,
    ModelHandle<Box<dyn TerminalManager>>,
) {
    // In Cloud Mode, ambient agent prompts are composed in an uninitialized session-sharing
    // viewer pane. This lets us reuse the terminal input without a backing session, and
    // then join the ambient agent session once it's ready.
    // The viewer manager builds its own `TerminalView` during construction and
    // exposes it via `view()`, so grab the view before boxing the manager behind
    // the object-safe `TerminalManager` trait (which has no `view()`).
    let manager = shared_session::viewer::TerminalManager::new_deferred(
        resources,
        view_bounds_size,
        window_id,
        enable_orchestration_polling,
        ctx,
    );
    let terminal_view = manager.view();
    let terminal_manager: ModelHandle<Box<dyn TerminalManager>> =
        ctx.add_model(move |_| Box::new(manager) as Box<dyn TerminalManager>);

    // Subscribe to the ambient agent view model to join the session once it's ready.
    // This ensures that we use the manager corresponding to this specific view.
    let Some(view_model) = terminal_view
        .as_ref(ctx)
        .ambient_agent_view_model()
        .cloned()
    else {
        log::warn!("Cloud mode view was created without an ambient agent view model");
        return (terminal_view, terminal_manager);
    };
    terminal_manager.update(ctx, |_, ctx| {
        ctx.subscribe_to_model(&view_model, move |manager, view_model, event, ctx| {
            let Some(manager) = manager
                .as_any_mut()
                .downcast_mut::<shared_session::viewer::TerminalManager>()
            else {
                return;
            };
            match event {
                AmbientAgentViewModelEvent::SessionReady { session_id } => {
                    // Local-to-cloud handoff panes pre-populate the forked
                    // conversation on chip click. Use append-mode scrollback
                    // + replay suppression so the cloud agent's replay doesn't
                    // duplicate the blocks we already have.
                    let append_followup_scrollback =
                        view_model.as_ref(ctx).is_local_to_cloud_handoff();
                    if manager.connect_to_session(*session_id, append_followup_scrollback, ctx) {
                        manager.start_cloud_mode_setup_command_tracking();
                    }
                }
                AmbientAgentViewModelEvent::ExecutionSessionReady { session_id } => {
                    manager.attach_execution_session(*session_id, ctx);
                }
                AmbientAgentViewModelEvent::EnteredSetupState
                | AmbientAgentViewModelEvent::EnteredComposingState
                | AmbientAgentViewModelEvent::DispatchedAgent
                | AmbientAgentViewModelEvent::FollowupDispatched
                | AmbientAgentViewModelEvent::ProgressUpdated
                | AmbientAgentViewModelEvent::EnvironmentSelected
                | AmbientAgentViewModelEvent::Failed { .. }
                | AmbientAgentViewModelEvent::ShowCloudAgentCapacityModal
                | AmbientAgentViewModelEvent::ShowAICreditModal
                | AmbientAgentViewModelEvent::NeedsGithubAuth
                | AmbientAgentViewModelEvent::Cancelled
                | AmbientAgentViewModelEvent::HarnessSelected
                | AmbientAgentViewModelEvent::ViewerHarnessResolved
                | AmbientAgentViewModelEvent::HostSelected
                | AmbientAgentViewModelEvent::HarnessModelSelected
                | AmbientAgentViewModelEvent::HarnessCommandStarted { .. }
                | AmbientAgentViewModelEvent::PendingHandoffChanged
                | AmbientAgentViewModelEvent::HandoffSnapshotUploadFailed { .. }
                | AmbientAgentViewModelEvent::UpdatedSetupCommandVisibility
                | AmbientAgentViewModelEvent::AuthSecretSelected
                | AmbientAgentViewModelEvent::RunLifecycleChanged => {}
            }
        });
    });

    (terminal_view, terminal_manager)
}

/// Returns `true` when a cloud agent shared session is in any pre-first-exchange phase —
/// either still spawning (loading screen) or running setup commands before the first
/// agent turn. In this state, we hide the interactive input and render a loading footer.
pub fn is_cloud_agent_pre_first_exchange(
    ambient_agent_view_model: Option<&ModelHandle<AmbientAgentViewModel>>,
    agent_view_controller: &ModelHandle<AgentViewController>,
    terminal_model: &TerminalModel,
    app: &AppContext,
) -> bool {
    if !(FeatureFlag::CloudMode.is_enabled() && FeatureFlag::AgentView.is_enabled()) {
        return false;
    }

    let Some(ambient_agent_view_model) = ambient_agent_view_model else {
        return false;
    };

    let view_model = ambient_agent_view_model.as_ref(app);

    let is_in_pre_first_exchange_status = matches!(
        view_model.status(),
        Status::WaitingForSession { .. } | Status::AgentRunning
    );
    if !is_in_pre_first_exchange_status {
        return false;
    }

    let agent_view_state = agent_view_controller.as_ref(app).agent_view_state().clone();
    let AgentViewState::Active { origin, .. } = agent_view_state else {
        return false;
    };

    // Handoff panes enter agent view with `RestoreExistingConversation` because they restore the
    // forked conversation, not `CloudAgent`. The `is_local_to_cloud_handoff` flag is the
    // authoritative "this is a cloud agent pane" signal for that path, so accept either.
    if !origin.is_cloud_agent() && !view_model.is_local_to_cloud_handoff() {
        return false;
    }

    // For non-oz harness runs, there is no Oz `AppendedExchange` to key off of, so we also
    // exit the pre-first-exchange phase when the harness CLI (e.g. `claude`, `gemini`) has
    // been detected. See `mark_harness_command_started`.
    if view_model.harness_command_started() {
        return false;
    }

    // Loading phase (`WaitingForSession`): no setup commands have started yet, but we're
    // still pre-first-exchange. Skip the block-list flag check.
    if matches!(view_model.status(), Status::WaitingForSession { .. }) {
        return true;
    }

    terminal_model
        .block_list()
        .is_executing_oz_environment_startup_commands()
}
