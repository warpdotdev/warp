use std::sync::{Arc, Mutex};
use std::time::Duration;

use instant::Instant;
use warp_server_client::iap::{IapManager, IapManagerEvent};
use warpui::{AppContext, SingletonEntity, ViewHandle, WindowId};

use crate::workspace::{Workspace, WorkspaceRegistry, WorkspaceRegistryEvent};

const COOLDOWN: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
struct State {
    cooldown_started_at: Option<Instant>,
    pending_message: Option<String>,
}

pub fn init(ctx: &mut AppContext) {
    let state = Arc::new(Mutex::new(State::default()));
    let state_for_failure = state.clone();
    ctx.subscribe_to_model(&IapManager::handle(ctx), move |_, event, ctx| {
        let IapManagerEvent::RefreshFailed {
            message,
            is_first_failure_of_streak: true,
        } = event
        else {
            return;
        };

        if state_for_failure
            .lock()
            .expect("IAP modal state lock poisoned")
            .cooldown_started_at
            .is_some_and(|started_at| started_at.elapsed() < COOLDOWN)
        {
            return;
        }

        if !show_dialog(message.clone(), state_for_failure.clone(), None, ctx) {
            state_for_failure
                .lock()
                .expect("IAP modal state lock poisoned")
                .pending_message = Some(message.clone());
        }
    });

    ctx.subscribe_to_model(&WorkspaceRegistry::handle(ctx), move |_, event, ctx| {
        let WorkspaceRegistryEvent::Registered(window_id) = event;
        let message = state
            .lock()
            .expect("IAP modal state lock poisoned")
            .pending_message
            .take();
        if let Some(message) = message
            && !show_dialog(message.clone(), state.clone(), Some(*window_id), ctx)
        {
            state
                .lock()
                .expect("IAP modal state lock poisoned")
                .pending_message = Some(message);
        }
    });
}

fn show_dialog(
    message: String,
    state: Arc<Mutex<State>>,
    window_id: Option<WindowId>,
    ctx: &mut AppContext,
) -> bool {
    let Some(workspace) = find_workspace(window_id, ctx) else {
        return false;
    };

    workspace.update(ctx, |workspace, ctx| {
        workspace.show_iap_refresh_failure_modal(
            message,
            move || {
                if let Ok(mut state) = state.lock() {
                    state.cooldown_started_at = Some(Instant::now());
                }
            },
            ctx,
        );
    });
    true
}

fn find_workspace(window_id: Option<WindowId>, ctx: &AppContext) -> Option<ViewHandle<Workspace>> {
    let window_id = window_id.or_else(|| {
        ctx.windows()
            .active_window()
            .or_else(|| ctx.windows().ordered_window_ids().first().copied())
    })?;
    WorkspaceRegistry::as_ref(ctx).get(window_id, ctx)
}
