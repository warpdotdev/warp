pub mod auth_manager;
mod auth_override_warning_body;
pub mod auth_override_warning_modal;
mod auth_view_body;
pub mod auth_view_modal;
mod auth_view_shared_helpers;
mod login_error_modal;
mod login_failure_notification;
pub mod login_slide;
pub mod needs_sso_link_view;
pub mod paste_auth_token_modal;
mod user_properties;
pub use warp_server_auth::{auth_state, credentials, user, user_uid};
#[cfg(target_family = "wasm")]
pub mod web_handoff;

use ::settings::{Setting, SettingsManager, ToggleableSetting};
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
pub use auth_manager::AuthManager;
pub use auth_state::AuthStateProvider;
use itertools::Itertools;
pub use login_failure_notification::LoginFailureReason;
#[cfg(feature = "tui")]
use url::Url;
pub use user_uid::UserUid;
use warp_core::channel::ChannelState;
use warp_core::user_preferences::GetUserPreferences as _;
use warp_errors::{report_error, report_if_error};
use warpui::modals::{AlertDialogWithCallbacks, ModalButton};
use warpui::{AppContext, SingletonEntity};

use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::orchestration_pill_bar_model::OrchestrationPillBarModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::ai_assistant::requests::REQUEST_LIMIT_INFO_CACHE_KEY;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor_management::{CodeEditorStatus, CodeEditorSummary};
use crate::env_vars::manager::EnvVarCollectionManager;
use crate::notebooks::manager::NotebookManager;
use crate::palette::PaletteMode;
use crate::root_view::RootView;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::sync_queue::SyncQueue;
use crate::server::telemetry::{PaletteSource, TelemetryEvent};
use crate::session_management::{RunningSessionSummary, SessionNavigationData};
use crate::settings::{
    AISettings, CRASH_REPORTING_ENABLED_DEFAULTS_KEY, CloudPreferencesSettings, PrivacySettings,
    TELEMETRY_ENABLED_DEFAULTS_KEY,
};
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::shared_session::manager::Manager as SharedSessionManager;
use crate::workflows::manager::WorkflowManager;
use crate::workspace::{Workspace, WorkspaceAction};
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::{
    GlobalResourceHandlesProvider, focus_running_window_and_show_native_modal, persistence,
    send_telemetry_sync_from_app_ctx,
};

pub fn init(app: &mut AppContext) {
    auth_view_modal::init(app);
    auth_view_body::init(app);
    auth_override_warning_body::init(app);
    login_slide::init(app);
    paste_auth_token_modal::init(app);
}

/// Returns the configured Warp web logout URL.
///
/// Keep this derived from the channel's server root so local and non-production
/// builds log out of the same web session they use for authentication.
pub fn web_logout_url() -> String {
    format!(
        "{}/logout",
        ChannelState::server_root_url().trim_end_matches('/')
    )
}

/// Returns the configured Warp web logout URL with a validated browser continuation.
///
/// TUI logout only continues to the same Warp web origin's device page. This
/// keeps the logout endpoint from becoming an open redirect if an unexpected
/// device-authorization response reaches the client.
#[cfg(feature = "tui")]
pub fn web_logout_url_with_continue(continue_url: &str) -> Option<String> {
    let mut logout_url =
        Url::parse(&web_logout_url()).expect("configured Warp web logout URL must be valid");
    let continue_url = Url::parse(continue_url).ok()?;
    let has_required_query = continue_url
        .query_pairs()
        .any(|(key, value)| key == "user_code" && !value.is_empty())
        && continue_url
            .query_pairs()
            .any(|(key, value)| key == "source" && value == "warp-agent-cli");
    if continue_url.origin() != logout_url.origin()
        || continue_url.path() != "/device"
        || !continue_url.username().is_empty()
        || continue_url.password().is_some()
        || continue_url.fragment().is_some()
        || !has_required_query
    {
        return None;
    }
    logout_url
        .query_pairs_mut()
        .append_pair("continue", continue_url.as_str());
    Some(logout_url.into())
}

/// If the app has running processes or dirty objects, we'll show a confirmation modal before logging out.
/// If the user aborts, the user will not be logged out.
pub fn maybe_log_out(app: &mut AppContext) {
    send_telemetry_sync_from_app_ctx!(TelemetryEvent::UserInitiatedLogOut, app);

    let sessions = SessionNavigationData::all_sessions(app).collect_vec();
    let num_long_running_commands = RunningSessionSummary::new(&sessions)
        .long_running_cmds
        .len();
    let num_shared_sessions = crate::session_management::num_shared_sessions(app);
    let num_unsaved_objects =
        CloudModel::as_ref(app).num_unsaved_objects_to_warn_about_before_quitting();

    let code_editors = CodeEditorStatus::all_editors(app).collect_vec();
    let code_editor_summary = CodeEditorSummary::new(&code_editors);

    let num_unsaved_files = code_editor_summary.unsaved_changes.len();

    let show_warning_before_log_out = *GeneralSettings::as_ref(app)
        .show_warning_before_quitting
        .value();
    if show_warning_before_log_out
        && (num_long_running_commands > 0
            || num_shared_sessions > 0
            || num_unsaved_objects > 0
            || num_unsaved_files > 0)
    {
        send_telemetry_sync_from_app_ctx!(TelemetryEvent::LogOutModalShown, app);
        let mut button_data = vec![ModalButton::for_app("Yes, log out", |ctx| {
            log_out_and_open_web(ctx);
        })];

        let mut info_text_vec: Vec<String> = vec![];
        if num_long_running_commands > 0 {
            let plural = if num_long_running_commands > 1 {
                "processes"
            } else {
                "process"
            };
            info_text_vec.push(format!(
                "You have {num_long_running_commands} {plural} running."
            ));

            button_data.push(ModalButton::for_app("Show running processes", move |ctx| {
                send_telemetry_sync_from_app_ctx!(
                    TelemetryEvent::LogOutModalCancel { nav_palette: true },
                    ctx
                );
                let windowing_model = ctx.windows();
                let window_id = if let Some(active_window_id) = windowing_model.active_window() {
                    active_window_id
                } else if let Some(window_id) = ctx.window_ids().collect_vec().first() {
                    let window_id = *window_id;
                    windowing_model.show_window_and_focus_app(window_id);
                    window_id
                } else {
                    return;
                };

                if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id)
                    && let Some(handle) = workspaces.first()
                {
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        handle.id(),
                        &WorkspaceAction::OpenPalette {
                            mode: PaletteMode::Navigation,
                            source: PaletteSource::LogOutModal,
                            query: Some("running".to_owned()),
                        },
                    );
                }
            }))
        }

        if num_shared_sessions > 0 {
            let plural = if num_shared_sessions > 1 {
                "sessions"
            } else {
                "session"
            };
            info_text_vec.push(format!("You have {num_shared_sessions} shared {plural}."));
        }

        if num_unsaved_objects > 0 {
            let plural = if num_unsaved_objects > 1 {
                "objects"
            } else {
                "object"
            };
            info_text_vec.push(format!(
                "You have {num_unsaved_objects} unsynced Warp Drive {plural}. \
            Logging out will cause you to lose the {plural}."
            ));
        }

        if num_unsaved_files > 0 {
            let plural = if num_unsaved_files > 1 {
                "files"
            } else {
                "file"
            };
            info_text_vec.push(format!(
                "You have {num_unsaved_files} unsaved {plural}. \
            Logging out will cause you to lose the {plural}."
            ));
        }

        button_data.push(ModalButton::for_app("Cancel", move |ctx| {
            send_telemetry_sync_from_app_ctx!(
                TelemetryEvent::LogOutModalCancel { nav_palette: false },
                ctx
            );
        }));

        let alert_data = AlertDialogWithCallbacks::for_app(
            "Log out?",
            info_text_vec.join("\n"),
            button_data,
            move |ctx| {
                GeneralSettings::handle(ctx).update(ctx, |general_settings, ctx| {
                    report_if_error!(
                        general_settings
                            .show_warning_before_quitting
                            .toggle_and_save_value(ctx)
                    );
                });
            },
        );

        // On mac, we show the native platform modal. On platforms that don't support a native modal,
        // we show the custom warp modal.
        if cfg!(all(not(target_family = "wasm"), target_os = "macos")) {
            app.show_native_platform_modal(alert_data);
        } else {
            let sessions = SessionNavigationData::all_sessions(app).collect_vec();
            let sessions_summary = RunningSessionSummary::new(&sessions);
            focus_running_window_and_show_native_modal(sessions_summary, alert_data, app);
        }
    } else {
        log_out_and_open_web(app);
    }
}

/// Logs out locally and sends the user to Warp web's logout endpoint.
///
/// This is intentionally separate from [`log_out`], which is also used for
/// non-user-initiated auth recovery paths where opening a browser would be
/// surprising.
pub fn log_out_and_open_web(app: &mut AppContext) {
    log_out(app);
    let logout_url = web_logout_url();
    app.open_url(&logout_url);
}

// Log out the user, clears workspace state, stops running processes, and deletes database.
pub fn log_out(app: &mut AppContext) {
    send_telemetry_sync_from_app_ctx!(TelemetryEvent::LogOut, app);

    CodebaseIndexManager::handle(app).update(app, |index_manager, ctx| {
        index_manager.reset_codebase_indexing(ctx);
    });

    let global_resource_handles = GlobalResourceHandlesProvider::as_ref(app).get();

    // As part of Logout v0, we remove sqlite3 so sessions and cloud objects don't persist between accounts.
    // TODO: Implement per-user scoping of sqlite3.
    persistence::remove(&global_resource_handles.model_event_sender);

    AuthManager::handle(app).update(app, |auth_manager, ctx| {
        auth_manager.log_out(ctx);
    });
    // Detach built-in Warp-hosted MCP servers; they authenticate with the
    // credentials that were just cleared.
    #[cfg(not(target_family = "wasm"))]
    TemplatableMCPServerManager::handle(app).update(app, |manager, ctx| {
        manager.sync_builtin_servers(false, ctx);
    });
    AIRequestUsageModel::handle(app).update(app, |usage_model, ctx| {
        usage_model.reset_server_availability(ctx);
    });
    BlocklistAIHistoryModel::handle(app).update(app, |history_model, _| {
        history_model.reset();
    });
    OrchestrationPillBarModel::handle(app).update(app, |pill_bar_model, _| {
        pill_bar_model.reset();
    });
    AgentConversationsModel::handle(app).update(app, |agent_conversations_model, _| {
        agent_conversations_model.reset();
    });
    CloudModel::handle(app).update(app, |cloud_model, _| {
        cloud_model.reset();
    });
    // Clear the sync queue so that we don't try to sync the old user's objects to the new user.
    SyncQueue::handle(app).update(app, |sync_queue, _| {
        sync_queue.clear();
    });

    // Stop the cloud object and workspace metadata polling loops that were started on login.
    UpdateManager::handle(app).update(app, |manager, _| {
        manager.stop_polling_for_updated_objects();
    });
    TeamUpdateManager::handle(app).update(app, |manager, _| {
        manager.stop_polling_for_workspace_metadata_updates();
    });
    remove_cloud_persisted_settings(app);

    let settings_profiles_are_explicit = AISettings::as_ref(app)
        .execution_profiles
        .is_value_explicitly_set();
    AIExecutionProfilesModel::handle(app).update(app, |profiles, _| {
        profiles.reset(settings_profiles_are_explicit);
    });

    NotebookManager::handle(app).update(app, |manager, _| manager.reset());
    EnvVarCollectionManager::handle(app).update(app, |manager, _| manager.reset());
    WorkflowManager::handle(app).update(app, |manager, _| manager.reset());

    // Stop and leave all shared sessions
    SharedSessionManager::handle(app).update(app, |manager, ctx| {
        manager.stop_all_shared_sessions(ctx);
        manager.clear_joined();
    });

    // Dispatch the GUI root-view action on every open GUI window so its state can be updated
    // correctly. Other front-ends, such as the TUI, manage their logout transition separately.
    let window_ids = app.window_ids().collect_vec();
    for window_id in window_ids {
        if let Some(root_views) = app.views_of_type::<RootView>(window_id) {
            let Some(root_view) = root_views.first() else {
                continue;
            };
            app.dispatch_action(
                window_id,
                &[root_view.id()],
                "root_view:log_out",
                &(),
                log::Level::Info,
            );
        }
    }

    #[cfg(target_family = "wasm")]
    crate::platform::wasm::emit_event(crate::platform::wasm::WarpEvent::LoggedOut);
}

// Remove the cloud persisted settings from user defaults.
// When a user signs out, we remove cloud persisted settings of their account.
// This is so they do not experience the old settings when they log in with a different account.
// Partial deletion of user defaults is a stopgap for Logout v0. The correct solution is:
fn remove_cloud_persisted_settings(app: &mut AppContext) {
    let is_settings_sync_enabled = *CloudPreferencesSettings::as_ref(app).settings_sync_enabled;
    if is_settings_sync_enabled {
        SettingsManager::handle(app).update(app, |settings_manager, ctx| {
            let errors = settings_manager.clear_cloud_settings_local_state(ctx);
            for e in errors {
                report_error!(
                    e.context("Failed to remove cloud synced setting from user defaults")
                );
            }
        });
    }

    if let Err(e) = app
        .private_user_preferences()
        .remove_value(TELEMETRY_ENABLED_DEFAULTS_KEY)
    {
        report_error!(
            anyhow::Error::new(e)
                .context("Failed to remove Telemetry Enabled Defaults Key from user defaults")
        );
    }

    if let Err(e) = app
        .private_user_preferences()
        .remove_value(CRASH_REPORTING_ENABLED_DEFAULTS_KEY)
    {
        report_error!(
            anyhow::Error::new(e).context(
                "Failed to remove Crash Reporting Enabled Defaults Key from user defaults"
            )
        );
    }

    if let Err(e) = app
        .private_user_preferences()
        .remove_value(REQUEST_LIMIT_INFO_CACHE_KEY)
    {
        report_error!(
            anyhow::Error::new(e)
                .context("Failed to remove Request Limit Defaults Key from user defaults")
        );
    }

    // Reset the Privacy Settings in the login screen to default values.
    PrivacySettings::handle(app).update(app, |privacy_settings, _| {
        privacy_settings.refresh_to_default();
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
