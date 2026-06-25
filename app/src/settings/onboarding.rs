use onboarding::slides::{AgentAutonomy, AgentDevelopmentSettings};
use onboarding::{SelectedSettings, SessionDefault, UICustomizationSettings};
use settings::Setting as _;
use warpui::{AppContext, SingletonEntity as _};

use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::execution_profiles::{ActionPermission, WriteToPtyPermission};
use crate::drive::settings::WarpDriveSettings;
use crate::report_if_error;
use crate::settings::ai::DefaultSessionMode;
use crate::settings::{AISettings, CodeSettings};
use crate::workspace::tab_settings::TabSettings;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Applies onboarding settings based on the user's selected mode.
pub fn apply_onboarding_settings(selected_settings: &SelectedSettings, app: &mut AppContext) {
    let is_ai_enabled = match selected_settings {
        SelectedSettings::AgentDrivenDevelopment {
            agent_settings,
            ui_customization,
            ..
        } => {
            apply_agent_settings(agent_settings, app);
            let is_ai_enabled = !agent_settings.disable_oz;
            if let Some(ui) = ui_customization {
                apply_ui_customization_settings(ui, true, app);
            }
            is_ai_enabled
        }
        SelectedSettings::Terminal {
            ui_customization,
            cli_agent_toolbar_enabled,
            show_agent_notifications,
        } => {
            if let Some(ui) = ui_customization {
                apply_ui_customization_settings(ui, false, app);
            }
            AISettings::handle(app).update(app, |settings, ctx| {
                report_if_error!(settings
                    .should_render_cli_agent_footer
                    .set_value(*cli_agent_toolbar_enabled, ctx));
                report_if_error!(settings
                    .show_agent_notifications
                    .set_value(*show_agent_notifications, ctx));
            });
            false
        }
    };

    AISettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings.is_any_ai_enabled.set_value(is_ai_enabled, ctx));
    });
}

/// Applies the explicit UI customization settings chosen during the
/// "Customize your UI" onboarding slide.
fn apply_ui_customization_settings(
    ui: &UICustomizationSettings,
    is_agent_intent: bool,
    app: &mut AppContext,
) {
    TabSettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings
            .use_vertical_tabs
            .set_value(ui.use_vertical_tabs, ctx));
        report_if_error!(settings
            .show_code_review_button
            .set_value(ui.show_code_review_button, ctx));
    });

    WarpDriveSettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings
            .enable_warp_drive
            .set_value(ui.show_warp_drive, ctx));
    });

    CodeSettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings
            .show_project_explorer
            .set_value(ui.show_project_explorer, ctx));
        report_if_error!(settings
            .show_global_search
            .set_value(ui.show_global_search, ctx));
    });

    // For agent intent, configure showing conversation history.
    // For terminal intent, this option was not surfaced in onboarding, so leave the default.
    // It will be hidden anyway because AI is off, but we want to keep the default in case they enable AI later.
    if is_agent_intent {
        AISettings::handle(app).update(app, |settings, ctx| {
            report_if_error!(settings
                .show_conversation_history
                .set_value(ui.show_conversation_history, ctx));
        });
    }
}

fn apply_agent_settings(agent_settings: &AgentDevelopmentSettings, app: &mut AppContext) {
    // Apply session default mode.
    let default_mode = match agent_settings.session_default {
        SessionDefault::Agent => DefaultSessionMode::Agent,
        SessionDefault::Terminal => DefaultSessionMode::Terminal,
    };
    AISettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings
            .default_session_mode_internal
            .set_value(default_mode, ctx));
    });

    let workspace_autonomy_settings = UserWorkspaces::as_ref(app).ai_autonomy_settings();

    AISettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings
            .should_render_cli_agent_footer
            .set_value(agent_settings.cli_agent_toolbar_enabled, ctx));
        report_if_error!(settings
            .show_agent_notifications
            .set_value(agent_settings.show_agent_notifications, ctx));
    });

    AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
        let default_profile_info = profiles.default_profile(ctx);
        let default_profile_id = *default_profile_info.id();

        // Preserve the existing cloud default profile for users who are
        // already logged in (or who log in at the end of onboarding). A
        // `Some` sync_id means the profile is backed by a cloud object that
        // was either loaded at startup or reconciled during the post-login
        // initial load, and its values represent what the user has stored
        // previously. Overwriting those with the onboarding-selected
        // base_model / autonomy would silently discard their prior
        // customizations. Fresh `Unsynced` default profiles (brand-new
        // users, or users without any cloud default yet) still receive the
        // onboarding values.
        if default_profile_info.sync_id().is_some() {
            log::info!(
                "Preserving existing cloud default execution profile; skipping \
                 onboarding-driven overrides for profile {default_profile_id:?}"
            );
            return;
        }

        profiles.set_base_model(
            default_profile_id,
            Some(agent_settings.selected_model_id.clone()),
            ctx,
        );

        // If autonomy is None, the workspace enforces autonomy settings, so skip setting them.
        let Some(autonomy) = agent_settings.autonomy else {
            return;
        };

        let permissions = action_permissions_for_onboarding_autonomy(autonomy);

        // Only set permissions that are not enforced by the workspace
        if !workspace_autonomy_settings.has_override_for_code_diffs() {
            profiles.set_apply_code_diffs(default_profile_id, &permissions.apply_code_diffs, ctx);
        }
        if !workspace_autonomy_settings.has_override_for_read_files() {
            profiles.set_read_files(default_profile_id, &permissions.read_files, ctx);
        }
        if !workspace_autonomy_settings.has_override_for_execute_commands() {
            profiles.set_execute_commands(default_profile_id, &permissions.execute_commands, ctx);
        }
        // Note: MCP permissions don't have a workspace-level override, so always set them
        profiles.set_mcp_permissions(default_profile_id, &permissions.mcp_permissions, ctx);
        if !workspace_autonomy_settings.has_override_for_write_to_pty() {
            profiles.set_write_to_pty(default_profile_id, &permissions.write_to_pty, ctx);
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OnboardingAutonomyPermissions {
    apply_code_diffs: ActionPermission,
    read_files: ActionPermission,
    execute_commands: ActionPermission,
    mcp_permissions: ActionPermission,
    write_to_pty: WriteToPtyPermission,
}

fn action_permissions_for_onboarding_autonomy(
    autonomy: AgentAutonomy,
) -> OnboardingAutonomyPermissions {
    match autonomy {
        // Full autonomy promises "Runs commands, writes code, and reads files
        // without asking," so every permission is `AlwaysAllow`. The command
        // denylist still takes precedence at runtime when a specific command
        // is considered unsafe.
        AgentAutonomy::Full => OnboardingAutonomyPermissions {
            apply_code_diffs: ActionPermission::AlwaysAllow,
            read_files: ActionPermission::AlwaysAllow,
            execute_commands: ActionPermission::AlwaysAllow,
            mcp_permissions: ActionPermission::AlwaysAllow,
            write_to_pty: WriteToPtyPermission::AlwaysAllow,
        },
        // Partial autonomy: reads are always allowed, applying code diffs
        // always asks, and the agent decides on command / MCP execution
        // (asking only for sensitive actions).
        AgentAutonomy::Partial => OnboardingAutonomyPermissions {
            apply_code_diffs: ActionPermission::AlwaysAsk,
            read_files: ActionPermission::AlwaysAllow,
            execute_commands: ActionPermission::AgentDecides,
            mcp_permissions: ActionPermission::AgentDecides,
            write_to_pty: WriteToPtyPermission::AlwaysAsk,
        },
        AgentAutonomy::None => OnboardingAutonomyPermissions {
            apply_code_diffs: ActionPermission::AlwaysAsk,
            read_files: ActionPermission::AlwaysAsk,
            execute_commands: ActionPermission::AlwaysAsk,
            mcp_permissions: ActionPermission::AlwaysAsk,
            write_to_pty: WriteToPtyPermission::AlwaysAsk,
        },
    }
}

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
