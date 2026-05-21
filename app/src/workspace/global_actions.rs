use crate::auth;
use crate::network::NetworkStatus;
use crate::persistence::ModelEvent;
use crate::server::server_api::auth::AuthClient;
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::general_settings::GeneralSettings;
use crate::workspace::cross_window_tab_drag::CrossWindowTabDrag;
use crate::{app_state::get_app_state, server::server_api::ServerApiProvider};
use ::settings::ToggleableSetting;
use warp_core::execution_mode::AppExecutionMode;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentExchangeId;
use crate::root_view::OpenPath;
use crate::undo_close::UndoCloseStack;
use crate::workspace::{Workspace, WorkspaceAction};
use crate::GlobalResourceHandlesProvider;
use std::path::PathBuf;
use warp_graphql::mutations::create_anonymous_user::AnonymousUserType;
use warpui::windowing::WindowManager;
use warpui::{AppContext, SingletonEntity, TypedActionView};

/// Specifies where a forked conversation should be opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ForkedConversationDestination {
    /// Open the forked conversation in a new pane (split to the right).
    #[default]
    SplitPane,
    /// Open the forked conversation in the current pane, replacing the current view.
    CurrentPane,
    /// Open the forked conversation in a new tab.
    NewTab,
}

impl ForkedConversationDestination {
    pub fn is_new_tab(&self) -> bool {
        matches!(self, Self::NewTab)
    }

    pub fn is_split_pane(&self) -> bool {
        matches!(self, Self::SplitPane)
    }

    pub fn is_current_pane(&self) -> bool {
        matches!(self, Self::CurrentPane)
    }
}

/// Specifies the exchange at which to fork an AI conversation.
#[derive(Debug, Clone, Copy)]
pub struct ForkFromExchange {
    pub exchange_id: AIAgentExchangeId,
    /// When true, the fork stops immediately after this exchange without extending
    /// to the next user query boundary.
    pub fork_from_exact_exchange: bool,
}

/// Parameters for forking an AI conversation.
pub struct ForkAIConversationParams {
    pub conversation_id: AIConversationId,
    /// When Some, fork from the given response (or exchange if `fork_from_exact_exchange` is true).
    pub fork_from_exchange: Option<ForkFromExchange>,
    pub summarize_after_fork: bool,
    pub summarization_prompt: Option<String>,
    pub initial_prompt: Option<String>,
    pub destination: ForkedConversationDestination,
}

/// DEPRECATED. Global actions are being phased out.
/// Do not add any more global actions; use typed actions instead.
pub fn init_global_actions(app: &mut AppContext) {
    app.add_global_action("workspace:toggle_mouse_reporting", toggle_mouse_reporting);
    app.add_global_action("workspace:toggle_scroll_reporting", toggle_scroll_reporting);
    app.add_global_action("workspace:toggle_focus_reporting", toggle_focus_reporting);
    app.add_global_action("workspace:save_app", save_app);
    app.add_global_action("workspace:fork_ai_conversation", fork_ai_conversation);
    app.add_global_action(
        "workspace:summarize_ai_conversation",
        summarize_ai_conversation,
    );
    app.add_global_action(
        "workspace:toggle_debug_network_status",
        toggle_debug_network_status,
    );
    app.add_global_action(
        "workspace:debug_create_anonymous_user",
        create_anonymous_user,
    );
    app.add_global_action("workspace:open_repository", open_repository);
    app.add_global_action("app:undo_close", undo_close);
    app.add_global_action("app:maybe_log_out", trigger_maybe_log_out);
    app.add_global_action("app:log_out", trigger_log_out);
}

fn toggle_mouse_reporting(_: &(), ctx: &mut AppContext) {
    AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
        reporting
            .mouse_reporting_enabled
            .toggle_and_save_value(ctx)
            .expect("MouseReportingEnabled failed to serialize");
    });
}

fn toggle_scroll_reporting(_: &(), ctx: &mut AppContext) {
    AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
        reporting
            .scroll_reporting_enabled
            .toggle_and_save_value(ctx)
            .expect("ScrollReportingEnabled failed to serialize");
    });
}

fn toggle_focus_reporting(_: &(), ctx: &mut AppContext) {
    AltScreenReporting::handle(ctx).update(ctx, |reporting, ctx| {
        reporting
            .focus_reporting_enabled
            .toggle_and_save_value(ctx)
            .expect("FocusReportingEnabled failed to serialize");
    });
}

fn save_app(_: &(), ctx: &mut AppContext) {
    if !AppExecutionMode::as_ref(ctx).can_save_session() {
        return;
    }

    if !*GeneralSettings::as_ref(ctx).restore_session {
        return;
    }

    // While a cross-window tab drag is active, the dragged tab's pane group
    // is in flight between source and preview windows and `get_app_state`
    // would produce a snapshot with zero windows. Persisting that snapshot
    // wipes the on-disk session via `save_app_state`'s delete-then-insert
    // transaction. `save_app` fires from window move / focus / resize /
    // close callbacks (see `app_callbacks` in `lib.rs`), all of which run
    // during a drag, so we have to short-circuit at this boundary. The
    // first save after the drag finalizes will rewrite the snapshot.
    if CrossWindowTabDrag::as_ref(ctx).is_active() {
        return;
    }

    let Some(model_event_sender) = GlobalResourceHandlesProvider::as_ref(ctx)
        .get()
        .model_event_sender
        .clone()
    else {
        return;
    };

    // Only compute the app state if we're definitely going to use it.
    let app_state = get_app_state(ctx);
    let event = ModelEvent::Snapshot(app_state);

    if let Err(err) = model_event_sender.send(event) {
        log::error!("Error trying to send model event {err:?}");
    }
}

fn toggle_debug_network_status(_: &(), ctx: &mut AppContext) {
    NetworkStatus::handle(ctx).update(ctx, move |me, ctx| {
        let is_reachable = me.is_online();
        let new_is_reachable = !is_reachable;
        if new_is_reachable {
            log::info!("Manually toggled network status to be reachable");
        } else {
            log::info!("Manually toggled network status to be not reachable");
        }
        me.reachability_changed(new_is_reachable, ctx)
    });
}

fn create_anonymous_user(_: &(), ctx: &mut AppContext) {
    log::info!("Creating anonymous user");
    let anonymous_user_type = AnonymousUserType::NativeClientAnonymousUser;
    let server_api = ServerApiProvider::handle(ctx).read(ctx, |provider, _ctx| provider.get());
    let result =
        warpui::r#async::block_on(server_api.create_anonymous_user(None, anonymous_user_type));
    match result {
        Ok(user) => log::info!("Successfully created anonymous user {user:?}"),
        Err(err) => log::error!("Failed to create anonymous user: {err:?}"),
    }
}

/// Reopens the last closed item (window or tab).
fn undo_close(_: &(), ctx: &mut AppContext) {
    UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| {
        stack.undo_close(ctx);
    });
}

fn trigger_maybe_log_out(_: &(), ctx: &mut AppContext) {
    auth::maybe_log_out(ctx)
}

/// Dispatches an action to the active workspace, if one exists.
fn dispatch_to_active_workspace(ctx: &mut AppContext, action: WorkspaceAction) {
    if let Some(window_id) = WindowManager::as_ref(ctx).active_window() {
        if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) {
            if let Some(workspace) = workspaces.into_iter().next() {
                workspace.update(ctx, |workspace, ctx| {
                    workspace.handle_action(&action, ctx);
                });
            }
        }
    }
}

fn open_repository(path: &String, ctx: &mut AppContext) {
    if WindowManager::as_ref(ctx).active_window().is_some() {
        // There's an active window, dispatch to its workspace
        dispatch_to_active_workspace(
            ctx,
            WorkspaceAction::OpenRepository {
                path: Some(path.clone()),
            },
        );
    } else {
        // No active window, create a new one with the repository path
        let path_buf = PathBuf::from(path);
        ctx.dispatch_global_action("root_view:open_new_from_path", &OpenPath { path: path_buf });
    }
}

fn fork_ai_conversation(params: &ForkAIConversationParams, ctx: &mut AppContext) {
    dispatch_to_active_workspace(
        ctx,
        WorkspaceAction::ForkAIConversation {
            conversation_id: params.conversation_id,
            fork_from_exchange: params.fork_from_exchange,
            summarize_after_fork: params.summarize_after_fork,
            summarization_prompt: params.summarization_prompt.clone(),
            initial_prompt: params.initial_prompt.clone(),
            destination: params.destination,
        },
    );
}

fn summarize_ai_conversation(prompt: &Option<String>, ctx: &mut AppContext) {
    dispatch_to_active_workspace(
        ctx,
        WorkspaceAction::SummarizeAIConversation {
            prompt: prompt.clone(),
            initial_prompt: None,
        },
    );
}

fn trigger_log_out(_: &(), ctx: &mut AppContext) {
    auth::log_out(ctx)
}
