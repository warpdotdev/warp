//! General-purpose administrative commands in the Warp CLI.

use anyhow::{Context, Result};
use serde::Serialize;
use warp_cli::agent::OutputFormat;
use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity};

use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::user::PrincipalType;
use crate::auth::{AuthStateProvider, UserUid};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

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
                            println!("You are already logged in as {username} ({email}).")
                        }
                        (Some(name), _) | (None, Some(name)) => {
                            println!("You are already logged in as {name}.")
                        }
                        (None, None) => {
                            println!("You are already logged in.")
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                } else {
                    // Device auth succeeded.
                    println!("Logged in successfully");
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
                            format!("Authentication failed: {err:#}")
                        }
                        _ => "Authentication failed".to_string(),
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
                    println!("To log in, open this URL in your browser:\n{url}");
                } else {
                    println!(
                        "To log in, visit {verification_url} and enter this code: {user_code}"
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    team_uids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    team_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_name: Option<String>,
}

impl WhoamiOutput {
    fn set_workspace(&mut self, workspace: Option<&Workspace>, user_uid: UserUid) {
        let Some(workspace) = workspace else {
            return;
        };
        let teams: Vec<_> = workspace
            .teams
            .iter()
            .filter(|team| team.members.iter().any(|member| member.uid == user_uid))
            .collect();

        self.team_uids = teams.iter().map(|team| team.uid.to_string()).collect();
        self.team_names = teams.iter().map(|team| team.name.clone()).collect();
        self.workspace_uid = Some(workspace.uid.into());
        self.workspace_name = (!workspace.name.is_empty()).then(|| workspace.name.clone());
    }

    fn pretty(&self, principal_type: PrincipalType) -> String {
        let mut lines = vec![match principal_type {
            PrincipalType::User => format!("User ID: {}", self.uid),
            PrincipalType::ServiceAccount => format!("Service account ID: {}", self.uid),
        }];

        if let Some(name) = &self.display_name {
            lines.push(format!("Display Name: {name}"));
        }
        if let Some(email) = &self.email {
            lines.push(format!("Email: {email}"));
        }

        if let Some(workspace_uid) = &self.workspace_uid {
            lines.push(format!("Workspace UID: {workspace_uid}"));
        }
        if let Some(workspace_name) = &self.workspace_name {
            lines.push(format!("Workspace Name: {workspace_name}"));
        }
        if self.team_uids.len() > 1 {
            lines.push("Teams:".to_string());
        }

        for (team_uid, team_name) in self.team_uids.iter().zip(&self.team_names) {
            let indent = if self.team_uids.len() > 1 { "  " } else { "" };
            lines.push(format!("{indent}Team ID: {team_uid}"));
            if !team_name.is_empty() {
                lines.push(format!("{indent}Team Name: {team_name}"));
            }
        }

        lines.join("\n")
    }
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

    let user_uid = auth_state
        .user_id()
        .ok_or_else(|| anyhow::anyhow!("Could not determine user ID. Are you logged in?"))?;
    let uid = user_uid.as_string();
    let uid = uid
        .strip_prefix("serviceAccount:")
        .map(String::from)
        .unwrap_or(uid);

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
        team_uids: vec![],
        team_names: vec![],
        workspace_uid: None,
        workspace_name: None,
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

            info.set_workspace(UserWorkspaces::as_ref(ctx).current_workspace(), user_uid);

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
                    println!("{}", info.pretty(principal_type));
                }
                OutputFormat::Text => {
                    println!("{}:{}", info.principal_type, info.uid);
                }
                OutputFormat::Ndjson => {
                    ctx.terminate_app(
                        TerminationMode::ForceTerminate,
                        Some(Err(anyhow::anyhow!(
                            "`whoami` does not support `--output-format ndjson`"
                        ))),
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
        println!("You are not logged in.");
        ctx.terminate_app(TerminationMode::ForceTerminate, None);
        return Ok(());
    }

    crate::auth::log_out(ctx);
    println!("Logged out successfully.");
    ctx.terminate_app(TerminationMode::ForceTerminate, None);
    Ok(())
}

#[cfg(test)]
#[path = "admin_tests.rs"]
mod tests;
