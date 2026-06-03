//! General-purpose administrative commands in the Warp CLI.

use anyhow::{Context, Result};
use serde::Serialize;
use warp_cli::agent::OutputFormat;
use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity};

use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::user::PrincipalType;
use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Kick off a device authorization login flow and handle auth events.
pub fn login(ctx: &mut AppContext) -> Result<()> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    let has_cached_credentials = auth_state.is_logged_in();

    // If the user is already logged in, we require that the user log out before logging
    // back in to ensure their existing state isn't replaced (especially if using both the CLI
    // and the desktop app). In this case, try refreshing their credentials first. If the user
    // is trying to log in because the cached credentials are invalid, we should let them do so.
    // Track whether we've started the device auth flow. Failure events
    // that arrive before device auth has started are leftover refresh
    // errors and should be ignored rather than treated as terminal.
    let mut started_device_auth = !has_cached_credentials;
    ctx.subscribe_to_model(
        &AuthManager::handle(ctx),
        move |_, event, ctx| match event {
            AuthManagerEvent::AuthComplete => {
                if !started_device_auth {
                    // Refresh succeeded - credentials are still valid.
                    let auth_state = AuthStateProvider::as_ref(ctx).get();
                    match (auth_state.username_for_display(), auth_state.user_email()) {
                        (Some(username), Some(email)) if username != email => {
                            println!(
                                "{}",
                                i18n::t("ai.agent_sdk.admin.already_logged_in_as_username_email")
                                    .replace("{username}", &username)
                                    .replace("{email}", &email)
                            )
                        }
                        (Some(name), _) | (None, Some(name)) => {
                            println!(
                                "{}",
                                i18n::t("ai.agent_sdk.admin.already_logged_in_as_name")
                                    .replace("{name}", &name)
                            )
                        }
                        (None, None) => {
                            println!("{}", i18n::t("ai.agent_sdk.admin.already_logged_in"))
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                } else {
                    // Device auth succeeded.
                    println!("{}", i18n::t("ai.agent_sdk.admin.logged_in_successfully"));
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
            }
            AuthManagerEvent::AuthFailed(_) => {
                if !started_device_auth {
                    // Refresh failed - start a fresh device auth flow.
                    started_device_auth = true;
                    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                        auth_manager.authorize_device(ctx);
                    });
                } else {
                    // Device auth failed.
                    let err_msg = match event {
                        AuthManagerEvent::AuthFailed(err) => {
                            i18n::t("ai.agent_sdk.admin.authentication_failed_with_error")
                                .replace("{error}", &format!("{err:#}"))
                        }
                        _ => i18n::t("ai.agent_sdk.admin.authentication_failed"),
                    };
                    ctx.terminate_app(
                        TerminationMode::ForceTerminate,
                        Some(Err(anyhow::anyhow!(err_msg))),
                    );
                }
            }
            AuthManagerEvent::ReceivedDeviceAuthorizationCode {
                verification_url,
                verification_url_complete,
                user_code,
            } => {
                if let Some(url) = verification_url_complete {
                    println!(
                        "{}",
                        i18n::t("ai.agent_sdk.admin.login_open_url").replace("{url}", &url)
                    );
                } else {
                    println!(
                        "{}",
                        i18n::t("ai.agent_sdk.admin.login_visit_and_enter_code")
                            .replace("{url}", &verification_url)
                            .replace("{code}", &user_code)
                    );
                }
            }
            _ => {}
        },
    );

    // Either refresh existing credentials or start device auth from scratch.
    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        if has_cached_credentials {
            auth_manager.refresh_user(ctx);
        } else {
            auth_manager.authorize_device(ctx);
        }
    });

    Ok(())
}

#[derive(Serialize)]
struct WhoamiOutput {
    uid: String,
    #[serde(rename = "type")]
    principal_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_name: Option<String>,
}

/// Singleton model that provides a `ModelContext` for the `whoami` command's async work.
struct WhoamiRunner;

impl warpui::Entity for WhoamiRunner {
    type Event = ();
}

impl SingletonEntity for WhoamiRunner {}

/// Print information about the currently authenticated principal.
pub fn whoami(ctx: &mut AppContext, output_format: OutputFormat) -> Result<()> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    let principal_type = auth_state.principal_type().unwrap_or_default();

    let uid = auth_state
        .user_id()
        .map(|id| {
            let s = id.as_string();
            s.strip_prefix("serviceAccount:")
                .map(String::from)
                .unwrap_or(s)
        })
        .ok_or_else(|| anyhow::anyhow!(i18n::t("ai.agent_sdk.admin.user_id_missing")))?;

    let mut info = WhoamiOutput {
        uid,
        principal_type: match principal_type {
            PrincipalType::User => "user",
            PrincipalType::ServiceAccount => "service_account",
        },
        display_name: auth_state.display_name(),
        email: match principal_type {
            PrincipalType::User => auth_state.user_email().filter(|e| !e.is_empty()),
            PrincipalType::ServiceAccount => None,
        },
        team_uid: None,
        team_name: None,
    };

    // Refresh workspace metadata before reading team info, so we don't print
    // stale or missing team data if the metadata hasn't been fetched yet.
    let runner = ctx.add_singleton_model(|_| WhoamiRunner);
    runner.update(ctx, move |_, ctx| {
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        ctx.spawn(refresh_future, move |_, result, ctx| {
            if let Err(err) = result {
                // Do not prevent showing user info if fetching team metadata fails.
                log::warn!("Failed to refresh team metadata for whoami: {err:#}");
            }

            let current_team = UserWorkspaces::as_ref(ctx).current_team();
            info.team_uid = current_team.map(|t| t.uid.to_string());
            info.team_name = current_team
                .map(|t| t.name.clone())
                .filter(|n| !n.is_empty());

            match output_format {
                OutputFormat::Json => {
                    match serde_json::to_string(&info).context("whoami output should serialize") {
                        Ok(json) => println!("{json}"),
                        Err(err) => {
                            ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                            return;
                        }
                    }
                }
                OutputFormat::Pretty => {
                    match principal_type {
                        PrincipalType::User => {
                            println!(
                                "{}",
                                i18n::t("ai.agent_sdk.admin.user_id").replace("{uid}", &info.uid)
                            )
                        }
                        PrincipalType::ServiceAccount => {
                            println!(
                                "{}",
                                i18n::t("ai.agent_sdk.admin.service_account_id")
                                    .replace("{uid}", &info.uid)
                            )
                        }
                    }
                    if let Some(name) = &info.display_name {
                        println!(
                            "{}",
                            i18n::t("ai.agent_sdk.admin.display_name").replace("{name}", name)
                        );
                    }
                    if let Some(email) = &info.email {
                        println!(
                            "{}",
                            i18n::t("ai.agent_sdk.admin.email").replace("{email}", email)
                        );
                    }
                    if let Some(team_uid) = &info.team_uid {
                        println!(
                            "{}",
                            i18n::t("ai.agent_sdk.admin.team_id").replace("{team_uid}", team_uid)
                        );
                    }
                    if let Some(team_name) = &info.team_name {
                        println!(
                            "{}",
                            i18n::t("ai.agent_sdk.admin.team_name")
                                .replace("{team_name}", team_name)
                        );
                    }
                }
                OutputFormat::Text => {
                    println!("{}:{}", info.principal_type, info.uid);
                }
                OutputFormat::Ndjson => {
                    ctx.terminate_app(
                        TerminationMode::ForceTerminate,
                        Some(Err(anyhow::anyhow!(i18n::t(
                            "ai.agent_sdk.admin.whoami_ndjson_unsupported"
                        )))),
                    );
                    return;
                }
            }

            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        });
    });

    Ok(())
}

/// Log out of Warp using the same logic as the app.
pub fn logout(ctx: &mut AppContext) -> Result<()> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if !auth_state.is_logged_in() {
        println!("{}", i18n::t("ai.agent_sdk.admin.not_logged_in"));
        ctx.terminate_app(TerminationMode::ForceTerminate, None);
        return Ok(());
    }

    crate::auth::log_out(ctx);
    println!("{}", i18n::t("ai.agent_sdk.admin.logged_out_successfully"));
    ctx.terminate_app(TerminationMode::ForceTerminate, None);
    Ok(())
}
