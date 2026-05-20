use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::{
    ai::{
        agent::conversation::AIConversationId,
        execution_profiles::{
            profiles::{AIExecutionProfilesModel, ClientProfileId},
            AIExecutionProfile, ActionPermission, AskUserQuestionPermission, WriteToPtyPermission,
        },
    },
    report_if_error,
    settings::{AISettings, AgentModeCodingPermissionsType, AgentModeCommandExecutionPredicate},
    workspaces::{user_workspaces::UserWorkspaces, workspace::AiAutonomySettings},
};
use warp_core::execution_mode::AppExecutionMode;

use crate::ai::mcp::mcp_provider_from_file_path;
#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::TemplatableMCPServerManager;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use warp_completer::parsers::simple::decompose_command;
use warp_core::user_preferences::GetUserPreferences;
use warp_core::{features::FeatureFlag, settings::Setting};
use warp_util::path::EscapeChar;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use super::BlocklistAIHistoryModel;

/// Whether or not a command can be auto-executed, along with a detailed reason.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum CommandExecutionPermission {
    Allowed(CommandExecutionPermissionAllowedReason),
    Denied(CommandExecutionPermissionDeniedReason),
}

/// Why a command can be auto-executed.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum CommandExecutionPermissionAllowedReason {
    Dispatched,
    ExplicitlyAllowlisted,
    IsReadOnlyAndSettingEnabled,
    AgentDecided,
    AlwaysAllowed,
    RunToCompletion,
}

/// Why a command can't be auto-executed.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum CommandExecutionPermissionDeniedReason {
    AutonomyForceDisabled,
    AlwaysAskEnabled,
    ExplicitlyDenylisted,
    ContainsRedirection,
    Inconclusive,
    AgentDecided,
}

impl CommandExecutionPermission {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed(..))
    }
}

/// Whether or not a file can be auto-read, along with a detailed reason.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum FileReadPermission {
    Allowed(FileReadPermissionAllowedReason),
    Denied(FileReadPermissionDeniedReason),
}

/// Why a file can be auto-read.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum FileReadPermissionAllowedReason {
    Dispatched,
    AlreadyReadInConvo,
    ExplicitlyAllowlisted,
    AutoreadSettingEnabled,
    AgentDecided,
    RunToCompletion,
}

/// Why a file can't be auto-read.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum FileReadPermissionDeniedReason {
    AutonomyForceDisabled,
    AlwaysAskEnabled,
    Inconclusive,
    AgentDecided,
}

impl FileReadPermission {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed(..))
    }
}

/// Whether or not a file can be auto-written, along with a detailed reason.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum FileWritePermission {
    Allowed(FileWritePermissionAllowedReason),
    Denied(FileWritePermissionDeniedReason),
}

impl FileWritePermission {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed(..))
    }
}

/// Why a file can be written automatically.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum FileWritePermissionAllowedReason {
    Dispatched,
    AgentDecided,
    AutowriteSettingEnabled,
    RunToCompletion,
}

/// Why a file can't be written automatically.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum FileWritePermissionDeniedReason {
    AutonomyForceDisabled,
    AlwaysAskEnabled,
    Inconclusive,
    AgentDecided,
    /// The path is a system-protected file (e.g. an MCP config) that must never
    /// be auto-written regardless of user autonomy settings.
    ProtectedPath,
}

/// Describes permissions that Agent Mode has, backed by [`AISettings`].
pub struct BlocklistAIPermissions {
    /// A set of one-off files that the user has allowed Agent Mode
    /// to read for the duration of a given conversation.
    ///
    /// TODO: remove this once AM doesn't re-request access to the same file in a given convo.
    temporary_file_permissions: HashMap<AIConversationId, HashSet<PathBuf>>,
}

impl BlocklistAIPermissions {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Migrate the old `AgentModeAutoReadFiles` setting to the new [`AgentModeCodingPermissionsType`].
        if let Some(can_read_files) = ctx
            .private_user_preferences()
            .read_value("AgentModeAutoReadFiles")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
        {
            if let Err(e) = ctx
                .private_user_preferences()
                .remove_value("AgentModeAutoReadFiles")
            {
                log::error!("Failed to remove old AgentModeAutoReadFiles user pref: {e}");
            }
            if can_read_files {
                report_if_error!(AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .agent_mode_coding_permissions
                        .set_value(AgentModeCodingPermissionsType::AlwaysAllowReading, ctx)
                }));
            }
        }

        Self {
            temporary_file_permissions: Default::default(),
        }
    }

    /// Returns the active permissions profile, accounting for any enterprise overrides.
    pub fn permissions_profile_for_id(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> AIExecutionProfile {
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        let profile = profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx));
        let profile_data = profile.data();

        AIExecutionProfile {
            // Some fields may have an enterprise override.
            apply_code_diffs: self.get_apply_code_diffs_setting_for_profile(ctx, profile_id),
            read_files: self.get_read_files_setting_for_profile(ctx, profile_id),
            execute_commands: self.get_execute_commands_setting_for_profile(ctx, profile_id),
            mcp_permissions: self.get_mcp_permissions_setting_for_profile(ctx, profile_id),
            write_to_pty: self.get_write_to_pty_setting_for_profile(ctx, profile_id),
            command_allowlist: self.get_execute_commands_allowlist_for_profile(ctx, profile_id),
            command_denylist: self.get_execute_commands_denylist_for_profile(ctx, profile_id),
            directory_allowlist: self.get_read_files_allowlist_for_profile(ctx, profile_id),
            mcp_allowlist: self.get_mcp_allowlist_for_profile(ctx, profile_id),
            mcp_denylist: self.get_mcp_denylist_for_profile(ctx, profile_id),
            computer_use: self.get_computer_use_setting_for_profile(ctx, profile_id),
            ask_user_question: self.get_ask_user_question_setting_for_profile(ctx, profile_id),

            // Some fields are read directly from the profile.
            name: profile_data.name.clone(),
            is_default_profile: profile_data.is_default_profile,
            base_model: profile_data.base_model.clone(),
            coding_model: profile_data.coding_model.clone(),
            cli_agent_model: profile_data.cli_agent_model.clone(),
            computer_use_model: profile_data.computer_use_model.clone(),
            context_window_limit: profile_data.context_window_limit,
            autosync_plans_to_warp_drive: profile_data.autosync_plans_to_warp_drive,
            web_search_enabled: profile_data.web_search_enabled,
        }
    }

    pub fn active_permissions_profile(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> AIExecutionProfile {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.permissions_profile_for_id(ctx, *active_profile.id())
    }

    /// Returns the applicable workspace autonomy settings based on execution mode.
    /// In sandboxed mode, returns settings derived from the sandboxed agent config.
    /// In unsandboxed mode, returns the standard AI autonomy settings.
    fn workspace_autonomy_settings(ctx: &AppContext) -> AiAutonomySettings {
        if AppExecutionMode::as_ref(ctx).is_sandboxed() {
            let sandboxed = UserWorkspaces::as_ref(ctx).sandboxed_agent_settings();
            AiAutonomySettings {
                execute_commands_denylist: sandboxed.and_then(|s| s.execute_commands_denylist),
                ..Default::default()
            }
        } else {
            UserWorkspaces::as_ref(ctx).ai_autonomy_settings()
        }
    }

    pub fn get_apply_code_diffs_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> ActionPermission {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let apply_code_diffs_workspace_setting = autonomy_settings.apply_code_diffs_setting;

        apply_code_diffs_workspace_setting.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .apply_code_diffs
        })
    }

    /// Returns what the current setting is for applying code diffs,
    /// based on the workspace setting and the active profile.
    pub fn get_apply_code_diffs_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> ActionPermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);

        self.get_apply_code_diffs_setting_for_profile(ctx, *active_profile.id())
    }

    pub fn get_read_files_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> ActionPermission {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let read_files_workspace_setting = autonomy_settings.read_files_setting;

        read_files_workspace_setting.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .read_files
        })
    }

    /// Returns what the current setting is for reading files,
    /// based on the workspace setting and the active profile.
    pub fn get_read_files_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> ActionPermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_read_files_setting_for_profile(ctx, *active_profile.id())
    }

    pub fn get_read_files_allowlist_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> Vec<PathBuf> {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let read_files_workspace_allowlist = autonomy_settings.read_files_allowlist;

        read_files_workspace_allowlist.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .directory_allowlist
                .clone()
        })
    }

    /// Returns an allowlist of paths that AM should be able to auto-read.
    /// Note that the caller is responsible for deciding how the workspace's/user's settings
    /// should affect how this gets used, if at all.
    pub fn get_read_files_allowlist(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> Vec<PathBuf> {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_read_files_allowlist_for_profile(ctx, *active_profile.id())
    }

    pub fn get_execute_commands_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> ActionPermission {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let execute_commands_workspace_setting = autonomy_settings.execute_commands_setting;

        execute_commands_workspace_setting.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .execute_commands
        })
    }

    /// Returns what the current setting is for executing commands,
    /// based on the workspace setting and the active profile.
    pub fn get_execute_commands_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> ActionPermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_execute_commands_setting_for_profile(ctx, *active_profile.id())
    }

    pub fn get_execute_commands_allowlist_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> Vec<AgentModeCommandExecutionPredicate> {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let execute_commands_workspace_allowlist = autonomy_settings.execute_commands_allowlist;

        execute_commands_workspace_allowlist.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .command_allowlist
                .clone()
        })
    }

    /// Returns an allowlist of command regexes that AM should be able to auto-execute.
    /// Note that the caller is responsible for deciding how the workspace's/user's settings
    /// should affect how this gets used, if at all.
    pub fn get_execute_commands_allowlist(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> Vec<AgentModeCommandExecutionPredicate> {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_execute_commands_allowlist_for_profile(ctx, *active_profile.id())
    }

    pub fn get_execute_commands_denylist_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> Vec<AgentModeCommandExecutionPredicate> {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        let user_denylist = profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx))
            .data()
            .command_denylist
            .clone();

        match autonomy_settings.execute_commands_denylist {
            Some(org_denylist) => {
                let mut merged = org_denylist;
                for item in user_denylist {
                    if !merged.contains(&item) {
                        merged.push(item);
                    }
                }
                merged
            }
            None => user_denylist,
        }
    }

    pub fn get_org_execute_commands_denylist(
        ctx: &AppContext,
    ) -> Vec<AgentModeCommandExecutionPredicate> {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        autonomy_settings
            .execute_commands_denylist
            .unwrap_or_default()
    }

    /// Returns a denylist of command regexes that AM should not auto-execute.
    /// Note that the caller is responsible for deciding how the workspace's/user's settings
    /// should affect how this gets used, if at all.
    pub fn get_execute_commands_denylist(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> Vec<AgentModeCommandExecutionPredicate> {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_execute_commands_denylist_for_profile(ctx, *active_profile.id())
    }

    pub fn get_write_to_pty_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> WriteToPtyPermission {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let write_to_pty_workspace_setting = autonomy_settings.write_to_pty_setting;

        write_to_pty_workspace_setting.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .write_to_pty
        })
    }

    pub fn get_write_to_pty_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> WriteToPtyPermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_write_to_pty_setting_for_profile(ctx, *active_profile.id())
    }

    pub fn can_write_to_pty(
        &self,
        conversation_id: &AIConversationId,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> WriteToPtyPermission {
        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .is_some_and(|convo| convo.autoexecute_any_action())
        {
            return WriteToPtyPermission::AlwaysAllow;
        }
        self.get_write_to_pty_setting(ctx, terminal_view_id)
    }

    pub fn get_mcp_permissions_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> ActionPermission {
        // TODO: allow a workspace override on MCP permissions.

        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx))
            .data()
            .mcp_permissions
    }

    pub fn get_mcp_permissions_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> ActionPermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_mcp_permissions_setting_for_profile(ctx, *active_profile.id())
    }

    pub fn get_mcp_allowlist_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> Vec<uuid::Uuid> {
        // TODO: allow a workspace override on MCP allowlist.

        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx))
            .data()
            .mcp_allowlist
            .clone()
    }

    pub fn get_mcp_allowlist(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> Vec<uuid::Uuid> {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_mcp_allowlist_for_profile(ctx, *active_profile.id())
    }

    pub fn get_mcp_denylist_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> Vec<uuid::Uuid> {
        // TODO: allow a workspace override on MCP denylist.

        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx))
            .data()
            .mcp_denylist
            .clone()
    }

    pub fn get_mcp_denylist(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> Vec<uuid::Uuid> {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_mcp_denylist_for_profile(ctx, *active_profile.id())
    }

    pub fn get_web_search_enabled_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> bool {
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx))
            .data()
            .web_search_enabled
    }

    pub fn get_web_search_enabled(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> bool {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_web_search_enabled_for_profile(ctx, *active_profile.id())
    }

    pub fn get_computer_use_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> crate::ai::execution_profiles::ComputerUsePermission {
        let autonomy_settings = Self::workspace_autonomy_settings(ctx);
        let computer_use_workspace_setting = autonomy_settings.computer_use_setting;

        computer_use_workspace_setting.unwrap_or_else(|| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
            profiles_model
                .get_profile_by_id(profile_id, ctx)
                .unwrap_or_else(|| profiles_model.default_profile(ctx))
                .data()
                .computer_use
        })
    }

    pub fn get_computer_use_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> crate::ai::execution_profiles::ComputerUsePermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_computer_use_setting_for_profile(ctx, *active_profile.id())
    }

    pub fn get_ask_user_question_setting_for_profile(
        &self,
        ctx: &AppContext,
        profile_id: ClientProfileId,
    ) -> AskUserQuestionPermission {
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        profiles_model
            .get_profile_by_id(profile_id, ctx)
            .unwrap_or_else(|| profiles_model.default_profile(ctx))
            .data()
            .ask_user_question
    }

    pub fn get_ask_user_question_setting(
        &self,
        ctx: &AppContext,
        terminal_view_id: Option<EntityId>,
    ) -> AskUserQuestionPermission {
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(terminal_view_id, ctx);
        self.get_ask_user_question_setting_for_profile(ctx, *active_profile.id())
    }

    /// Returns whether or not Agent Mode can auto-read the given files.
    pub fn can_read_files_with_conversation(
        &self,
        conversation_id: &AIConversationId,
        paths: Vec<PathBuf>,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> FileReadPermission {
        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .is_some_and(|convo| convo.autoexecute_any_action())
        {
            return FileReadPermission::Allowed(FileReadPermissionAllowedReason::RunToCompletion);
        }

        self.can_read_files(Some(conversation_id), paths, terminal_view_id, ctx)
    }

    /// Returns whether or not Warp can auto-read the given files (e.g. for codebase indexing).
    pub fn can_read_files(
        &self,
        conversation_id: Option<&AIConversationId>,
        paths: Vec<PathBuf>,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> FileReadPermission {
        if paths.is_empty() {
            // We can vacuously read 0 files.
            return FileReadPermission::Allowed(
                FileReadPermissionAllowedReason::ExplicitlyAllowlisted,
            );
        }

        // Check if we've already been given permission to read these files in this conversation.
        if let Some(temp_permissions) =
            conversation_id.and_then(|id| self.temporary_file_permissions.get(id))
        {
            if paths.iter().all(|path| {
                temp_permissions
                    .iter()
                    .any(|allowed| path.starts_with(allowed))
            }) {
                return FileReadPermission::Allowed(
                    FileReadPermissionAllowedReason::AlreadyReadInConvo,
                );
            }
        }

        match self.get_read_files_setting(ctx, terminal_view_id) {
            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                // For now, we always read files. We don't ask the user for permission.
                FileReadPermission::Allowed(FileReadPermissionAllowedReason::AgentDecided)
            }
            ActionPermission::AlwaysAllow => {
                FileReadPermission::Allowed(FileReadPermissionAllowedReason::AutoreadSettingEnabled)
            }
            ActionPermission::AlwaysAsk => {
                let allowlisted_paths = self.get_read_files_allowlist(ctx, terminal_view_id);
                if paths
                    .iter()
                    .all(|p| allowlisted_paths.iter().any(|dir| p.starts_with(dir)))
                {
                    FileReadPermission::Allowed(
                        FileReadPermissionAllowedReason::ExplicitlyAllowlisted,
                    )
                } else {
                    FileReadPermission::Denied(FileReadPermissionDeniedReason::AlwaysAskEnabled)
                }
            }
        }
    }

    /// Returns whether or not Agent Mode can automatically write to files.
    pub fn can_write_files(
        &self,
        conversation_id: &AIConversationId,
        paths: &[PathBuf],
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> FileWritePermission {
        // Protected paths are always denied, regardless of autonomy settings.
        if let Some(denied) = check_protected_write_paths(paths) {
            return denied;
        }

        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .is_some_and(|convo| convo.autoexecute_any_action())
        {
            return FileWritePermission::Allowed(FileWritePermissionAllowedReason::RunToCompletion);
        }

        self.determine_write_permissions_from_active_profile(terminal_view_id, ctx)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn can_call_mcp_tool(
        &self,
        server_id: Option<&uuid::Uuid>,
        name: &str,
        conversation_id: &AIConversationId,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> bool {
        let templatable_manager = TemplatableMCPServerManager::as_ref(ctx);

        // Try resolving via server UUID first, then fall back to tool-name lookup.
        // On recent clients, the server UUID should always be set - we should eventually
        // require the server UUID.
        let mut uuid_of_mcp_server =
            server_id.and_then(|id| templatable_manager.get_template_uuid(*id));

        // Prefer templatable MCP servers over legacy when a tool name exists in both.
        // Fall back to legacy behavior if templatable lookup fails or is disabled.
        if uuid_of_mcp_server.is_none() {
            uuid_of_mcp_server = templatable_manager
                .server_from_tool(name.to_string())
                .copied()
                .and_then(|installation_uuid| {
                    templatable_manager.get_template_uuid(installation_uuid)
                });
        }

        self.can_use_mcp_server(conversation_id, uuid_of_mcp_server, terminal_view_id, ctx)
    }

    /// Returns whether or not Agent Mode can automatically read the given MCP resource.
    #[cfg(not(target_family = "wasm"))]
    pub fn can_read_mcp_resource(
        &self,
        server_id: Option<&uuid::Uuid>,
        name: &str,
        uri: Option<&str>,
        conversation_id: &AIConversationId,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> bool {
        let templatable_manager = TemplatableMCPServerManager::as_ref(ctx);

        // Try resolving via server UUID first, then fall back to resource name/URI lookup.
        // On recent clients, the server UUID should always be set - we should eventually
        // require the server UUID.
        let mut uuid_of_mcp_server =
            server_id.and_then(|id| templatable_manager.get_template_uuid(*id));

        // Prefer templatable MCP servers over legacy when a resource name exists in both.
        // Fall back to legacy behavior if templatable lookup fails or is disabled.
        if uuid_of_mcp_server.is_none() {
            uuid_of_mcp_server = templatable_manager
                .server_from_resource(name, uri)
                .copied()
                .and_then(|installation_uuid| {
                    templatable_manager.get_template_uuid(installation_uuid)
                });
        }

        self.can_use_mcp_server(conversation_id, uuid_of_mcp_server, terminal_view_id, ctx)
    }

    /// Checks whether the given MCP server (identified by its template UUID) is permitted
    /// to be used based on the current MCP permission setting and allowlist/denylist.
    #[cfg(not(target_family = "wasm"))]
    fn can_use_mcp_server(
        &self,
        conversation_id: &AIConversationId,
        uuid_of_mcp_server: Option<uuid::Uuid>,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> bool {
        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .is_some_and(|convo| convo.autoexecute_any_action())
        {
            return true;
        }

        let allowlisted = uuid_of_mcp_server
            .is_some_and(|uid| self.get_mcp_allowlist(ctx, terminal_view_id).contains(&uid));
        let denylisted = uuid_of_mcp_server
            .is_some_and(|uid| self.get_mcp_denylist(ctx, terminal_view_id).contains(&uid));

        match self.get_mcp_permissions_setting(ctx, terminal_view_id) {
            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                allowlisted && !denylisted
            }
            ActionPermission::AlwaysAllow => !denylisted,
            ActionPermission::AlwaysAsk => allowlisted && !denylisted,
        }
    }

    // Helper function to evaluate the active profile + workspace settings.
    fn determine_write_permissions_from_active_profile(
        &self,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> FileWritePermission {
        match self.get_apply_code_diffs_setting(ctx, terminal_view_id) {
            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                FileWritePermission::Denied(FileWritePermissionDeniedReason::AgentDecided)
            }
            ActionPermission::AlwaysAllow => FileWritePermission::Allowed(
                FileWritePermissionAllowedReason::AutowriteSettingEnabled,
            ),
            ActionPermission::AlwaysAsk => {
                FileWritePermission::Denied(FileWritePermissionDeniedReason::AlwaysAskEnabled)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Returns whether or not Agent Mode can auto-execute the given command.
    pub fn can_autoexecute_command(
        &self,
        conversation_id: &AIConversationId,
        command: &str,
        escape_char: EscapeChar,
        is_read_only: bool,
        is_risky: Option<bool>,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> CommandExecutionPermission {
        // Normalize line continuations based on shell type.
        // POSIX shells (bash/zsh/fish) use backslash, PowerShell uses backtick.
        let normalized_command = match escape_char {
            EscapeChar::Backslash => command.replace("\\\n", " "),
            EscapeChar::Backtick => command.replace("`\n", " "),
        };

        // The command string might be composed of multiple commands so let's
        // break it up first.
        let (commands, contains_redirection) = decompose_command(&normalized_command, escape_char);

        // The denylist takes precedence over all other conditions.
        let denylist = self.get_execute_commands_denylist(ctx, terminal_view_id);
        if commands
            .iter()
            .any(|c| denylist.iter().any(|d| d.matches(c)))
        {
            return CommandExecutionPermission::Denied(
                CommandExecutionPermissionDeniedReason::ExplicitlyDenylisted,
            );
        }

        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .is_some_and(|convo| convo.autoexecute_any_action())
        {
            return CommandExecutionPermission::Allowed(
                CommandExecutionPermissionAllowedReason::RunToCompletion,
            );
        }

        match self.get_execute_commands_setting(ctx, terminal_view_id) {
            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                if FeatureFlag::AgentDecidesCommandExecution.is_enabled() && is_risky == Some(false)
                {
                    return CommandExecutionPermission::Allowed(
                        CommandExecutionPermissionAllowedReason::AgentDecided,
                    );
                }

                if contains_redirection {
                    return CommandExecutionPermission::Denied(
                        CommandExecutionPermissionDeniedReason::ContainsRedirection,
                    );
                }

                let allowlist = self.get_execute_commands_allowlist(ctx, terminal_view_id);
                if commands.iter().all(|command| {
                    allowlist
                        .iter()
                        .any(|allowlist_item| allowlist_item.matches(command))
                }) {
                    return CommandExecutionPermission::Allowed(
                        CommandExecutionPermissionAllowedReason::ExplicitlyAllowlisted,
                    );
                }

                // For now, the heuristic is if the command is read only or if we're executing
                // a plan. Otherwise, we don't want to autoexecute.
                if is_read_only {
                    CommandExecutionPermission::Allowed(
                        CommandExecutionPermissionAllowedReason::AgentDecided,
                    )
                } else {
                    CommandExecutionPermission::Denied(
                        CommandExecutionPermissionDeniedReason::AgentDecided,
                    )
                }
            }
            ActionPermission::AlwaysAllow => CommandExecutionPermission::Allowed(
                CommandExecutionPermissionAllowedReason::AlwaysAllowed,
            ),
            ActionPermission::AlwaysAsk => {
                let allowlist = self.get_execute_commands_allowlist(ctx, terminal_view_id);

                if commands.iter().all(|command| {
                    allowlist
                        .iter()
                        .any(|allowlist_item| allowlist_item.matches(command))
                }) {
                    CommandExecutionPermission::Allowed(
                        CommandExecutionPermissionAllowedReason::ExplicitlyAllowlisted,
                    )
                } else {
                    CommandExecutionPermission::Denied(
                        CommandExecutionPermissionDeniedReason::AlwaysAskEnabled,
                    )
                }
            }
        }
    }

    /// Allows Agent Mode to auto-execute commands that match `command`.
    ///
    /// The denylist (see [`Self::add_command_to_autoexecution_denylist`])
    /// takes precedence over the allowlist.
    pub fn add_command_to_autoexecution_allowlist(
        &mut self,
        command: AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let mut allowlist = AISettings::as_ref(ctx)
            .agent_mode_command_execution_allowlist
            .clone();
        allowlist.push(command);
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_command_execution_allowlist
                .set_value(allowlist, ctx)
        })
    }

    /// Removes `command` from the auto-execution allowlist.
    ///
    /// See [`Self::add_command_to_autoexecution_allowlist`] for more about the allowlist.
    pub fn remove_command_from_autoexecution_allowlist(
        &mut self,
        command: &AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let mut allowlist = AISettings::as_ref(ctx)
            .agent_mode_command_execution_allowlist
            .clone();
        allowlist.retain(|c| c != command);
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_command_execution_allowlist
                .set_value(allowlist, ctx)
        })
    }

    /// Forces Agent Mode to ask for user consent before executing commands that match `command`.
    pub fn add_command_to_autoexecution_denylist(
        &mut self,
        command: AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let mut denylist = AISettings::as_ref(ctx)
            .agent_mode_command_execution_denylist
            .clone();
        denylist.push(command);
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_command_execution_denylist
                .set_value(denylist, ctx)
        })
    }

    /// Removes `command` from the auto-execution denylist.
    ///
    /// See [`Self::add_command_to_autoexecution_denylist`] for more about the denylist.
    pub fn remove_command_from_denylist(
        &mut self,
        command: &AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let mut denylist = AISettings::as_ref(ctx)
            .agent_mode_command_execution_denylist
            .clone();
        denylist.retain(|c| c != command);
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_command_execution_denylist
                .set_value(denylist, ctx)
        })
    }

    /// Sets whether or not readonly commands can be auto-executed by Agent Mode.
    pub fn set_should_autoexecute_readonly_commands(
        &mut self,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_execute_read_only_commands
                .set_value(enabled, ctx)
                .map(|_| ())?;

            // If enabling, no need to show the file speedbump since
            // that setting will be superseded by this setting.
            if enabled {
                settings
                    .should_show_agent_mode_autoread_files_speedbump
                    .set_value(false, ctx)?;
            }

            settings
                .should_show_agent_mode_autoexecute_readonly_commands_speedbump
                .set_value(false, ctx)
        })
    }

    /// Sets whether or not we should always allow writing to the PTY.
    pub fn set_always_allow_write_to_pty(
        &mut self,
        enabled: bool,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let permission = if enabled {
            WriteToPtyPermission::AlwaysAllow
        } else {
            WriteToPtyPermission::AlwaysAsk
        };
        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(Some(terminal_view_id), ctx);
        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
            profiles_model.set_write_to_pty(*active_profile.id(), &permission, ctx);
        });
        Ok(())
    }

    /// Sets whether or not we should always allow reading files.
    pub fn set_always_allow_read_files(
        &mut self,
        enabled: bool,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let permissions = if enabled {
            ActionPermission::AlwaysAllow
        } else {
            ActionPermission::AlwaysAsk
        };

        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(Some(terminal_view_id), ctx);
        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
            profiles_model.set_read_files(*active_profile.id(), &permissions, ctx);
        });
        Ok(())
    }

    /// Sets permissions that Agent Mode has for coding tasks.
    pub fn set_coding_permissions(
        &mut self,
        permissions: AgentModeCodingPermissionsType,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_coding_permissions
                .set_value(permissions, ctx)
                .map(|_| ())?;

            settings
                .should_show_agent_mode_autoread_files_speedbump
                .set_value(false, ctx)
        })
    }

    /// Adds a filepath that Agent Mode can read for coding tasks without additional permissions.
    /// Used in conjunction with [`AgentModeCodingPermissionsType::AllowReadingSpecificFiles`].
    ///
    /// This does not do any validation on the filepath; callers should ensure the filepath is valid.
    pub fn add_filepath_to_code_read_allowlist(
        &mut self,
        filepath: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let mut allowlist = AISettings::as_ref(ctx)
            .agent_mode_coding_file_read_allowlist
            .clone();
        allowlist.push(filepath);
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_coding_file_read_allowlist
                .set_value(allowlist, ctx)
        })
    }

    /// Counterpart to [`Self::add_filepath_to_code_read_allowlist`].
    pub fn remove_filepath_from_code_read_allowlist(
        &mut self,
        filepath: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        let mut allowlist = AISettings::as_ref(ctx)
            .agent_mode_coding_file_read_allowlist
            .clone();
        allowlist.retain(|p| p != &filepath);
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .agent_mode_coding_file_read_allowlist
                .set_value(allowlist, ctx)
        })
    }

    /// Gives Agent Mode temporary access to the provided `files`.
    /// The permissions are scoped to the given conversation.
    pub fn add_temporary_file_read_permissions<P: Into<PathBuf>>(
        &mut self,
        conversation_id: AIConversationId,
        files: impl IntoIterator<Item = P>,
    ) {
        self.temporary_file_permissions
            .entry(conversation_id)
            .or_default()
            .extend(files.into_iter().map(Into::into));
    }

    /// Returns whether the agent can ask the user a question in the given conversation.
    pub fn can_ask_user_question(
        &self,
        conversation_id: &AIConversationId,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> bool {
        match self.get_ask_user_question_setting(ctx, terminal_view_id) {
            AskUserQuestionPermission::Never => false,
            AskUserQuestionPermission::AskExceptInAutoApprove
            | AskUserQuestionPermission::Unknown => !BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(conversation_id)
                .is_some_and(|convo| convo.autoexecute_any_action()),
            AskUserQuestionPermission::AlwaysAsk => true,
        }
    }
}

/// Returns `Some(Denied(ProtectedPath))` if any of the given paths are system-protected
/// and must never be auto-written regardless of user autonomy settings.
/// Returns `None` if no paths are protected.
fn check_protected_write_paths(paths: &[PathBuf]) -> Option<FileWritePermission> {
    // MCP config files are always protected from auto-write to prevent security risks
    // from injecting arbitrary context into the agent.
    if paths
        .iter()
        .any(|p| mcp_provider_from_file_path(p).is_some())
    {
        Some(FileWritePermission::Denied(
            FileWritePermissionDeniedReason::ProtectedPath,
        ))
    } else {
        None
    }
}

impl Entity for BlocklistAIPermissions {
    type Event = ();
}

impl SingletonEntity for BlocklistAIPermissions {}

/// Returns true iff Agent Mode autonomy features are allowed on this client.
/// Granular permissions still need to be checked for specific autonomy features
/// (e.g. whether a command is auto-executable).
pub fn is_agent_mode_autonomy_allowed(ctx: &AppContext) -> bool {
    crate::UserWorkspaces::as_ref(ctx).is_ai_autonomy_allowed()
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
