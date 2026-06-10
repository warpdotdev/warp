use core::fmt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warp_core::user_preferences::GetUserPreferences;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use super::{
    AIExecutionProfile, ActionPermission, CloudAIExecutionProfileModel, WriteToPtyPermission,
};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::{CloudModelEvent, UpdateSource};
use crate::cloud_object::{CloudObject as _, GenericStringObjectFormat, JsonObjectType};
use crate::drive::CloudObjectTypeAndId;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::settings::AgentModeCommandExecutionPredicate;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{send_telemetry_from_ctx, CloudModel, LaunchMode, TelemetryEvent};

/// ExecutionProfileId is the identifier that users of the AIExecutionProfilesModel use
/// to refer back to a specific profile. These are unique across the lifespan of the app.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClientProfileId(usize);

impl ClientProfileId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> ClientProfileId {
        static NEXT_PROFILE_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_PROFILE_ID.fetch_add(1, Ordering::Relaxed);
        ClientProfileId(raw)
    }
}

impl fmt::Display for ClientProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

#[derive(Clone, Debug)]
pub struct AIExecutionProfileInfo {
    id: ClientProfileId,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    sync_id: Option<SyncId>,
    data: AIExecutionProfile,
}

impl AIExecutionProfileInfo {
    pub fn id(&self) -> &ClientProfileId {
        &self.id
    }

    /// The Warp Drive sync ID of this profile, if it has been synced.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn sync_id(&self) -> Option<SyncId> {
        self.sync_id
    }

    pub fn data(&self) -> &AIExecutionProfile {
        &self.data
    }
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum DefaultProfileState {
    Unsynced {
        id: ClientProfileId,
        profile: AIExecutionProfile,
    },
    Synced {
        id: ClientProfileId,
    },
    /// Currently, the behavior of the CLI default is that it
    /// cannot be updated and will never be synced.
    #[allow(dead_code)]
    Cli {
        id: ClientProfileId,
        profile: AIExecutionProfile,
    },
}

impl std::fmt::Display for DefaultProfileState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DefaultProfileState::Unsynced { .. } => write!(f, "Unsynced"),
            DefaultProfileState::Synced { .. } => write!(f, "Synced"),
            DefaultProfileState::Cli { .. } => write!(f, "CLI"),
        }
    }
}

impl DefaultProfileState {
    pub fn id(&self) -> ClientProfileId {
        match self {
            DefaultProfileState::Unsynced { id, .. } => *id,
            DefaultProfileState::Synced { id } => *id,
            DefaultProfileState::Cli { id, .. } => *id,
        }
    }
}

pub struct AIExecutionProfilesModel {
    /// The default profile can be in one of three states:
    /// - Unsynced: No cloud object backing the profile. It's purely local read-only data.
    /// - Synced: A cloud object backs the profile, created either when edited locally or received from cloud.
    /// - CLI: When running in CLI mode, a more permissive default profile that doesn't sync to cloud.
    ///
    /// Note that the default_profile_state becomes synced either (1) when an edit happens on
    /// this client or (2) when a default profile is received from the cloud model (say, it was
    /// created for the user on another client). Once the profile is synced, it's never unsynced
    /// again. CLI profiles are currently never synced.
    default_profile_state: DefaultProfileState,
    profile_id_to_sync_id: HashMap<ClientProfileId, SyncId>,
    /// Only contains entries for non-default profiles.
    active_profiles_per_session: HashMap<EntityId, ClientProfileId>,
}

impl AIExecutionProfilesModel {
    #[allow(unused_variables)]
    pub fn new(launch_mode: &LaunchMode, ctx: &mut ModelContext<Self>) -> Self {
        cfg_if::cfg_if! {
            if #[cfg(feature = "agent_mode_evals")] {
                let default_profile_state = DefaultProfileState::Unsynced {
                    id: ClientProfileId::new(),
                    profile: AIExecutionProfile::create_agent_mode_eval_profile(),
                };
                let profile_id_to_sync_id: HashMap<ClientProfileId, SyncId> = HashMap::new();
                let active_profiles_per_session: HashMap<EntityId, ClientProfileId> = HashMap::new();
            } else {
                let cloud_model = CloudModel::handle(ctx).as_ref(ctx);
                let all_profiles_from_cloud: Vec<&super::CloudAIExecutionProfile> = cloud_model
                    .get_all_objects_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>()
                    .filter(|p| Self::is_owned_by_current_user(p, ctx))
                    .collect();

                let default_profile_from_cloud: Option<&super::CloudAIExecutionProfile> = all_profiles_from_cloud
                    .iter()
                    .find(|obj| obj.model().string_model.is_default_profile)
                    .copied();

                let mut profile_id_to_sync_id: HashMap<ClientProfileId, SyncId> = HashMap::new();
                let active_profiles_per_session: HashMap<EntityId, ClientProfileId> = HashMap::new();

                // Insert all non-default profiles from the cloud
                for cloud_profile in all_profiles_from_cloud.iter().filter(|p| !p.model().string_model.is_default_profile) {
                    let profile_id = ClientProfileId::new();
                    profile_id_to_sync_id.insert(profile_id, cloud_profile.id);
                }

                let default_profile_state = match launch_mode {
                    LaunchMode::App { .. } | LaunchMode::Test { .. } => match default_profile_from_cloud {
                        Some(p) => {
                            let execution_profile_id = ClientProfileId::new();
                            profile_id_to_sync_id.insert(execution_profile_id, p.id);
                            DefaultProfileState::Synced {
                                id: execution_profile_id,
                            }
                        }
                        None => DefaultProfileState::Unsynced {
                            id: ClientProfileId::new(),
                            profile: super::create_default_from_legacy_settings(ctx),
                        },
                    },
                    // When running as a CLI, we ignore the GUI default and use a more permissive default.
                    LaunchMode::CommandLine { is_sandboxed, computer_use_override, .. } => {
                        DefaultProfileState::Cli {
                            profile: AIExecutionProfile::create_default_cli_profile(*is_sandboxed, *computer_use_override),
                            id: ClientProfileId::new()
                        }
                    }
                    // RemoteServerProxy and RemoteServerDaemon don't use AI
                    // execution profiles. They never reach this code path
                    // since they don't go through initialize_app, but handle
                    // exhaustively.
                    LaunchMode::RemoteServerProxy | LaunchMode::RemoteServerDaemon { .. } => DefaultProfileState::Unsynced {
                        id: ClientProfileId::new(),
                        profile: super::create_default_from_legacy_settings(ctx),
                    },
                };
            }
        }

        // We have to listen for changes to AIExecutionProfiles for a few reasons:
        // (1) In case the default profile is unsynced AND a default profile arrives from the cloud
        // (2) Let views subscribed to us know whenever a backing profile changes.
        // (3) Keep profile_id_to_sync_id map up to date when profiles are created/deleted remotely
        if !cfg!(feature = "agent_mode_evals") {
            ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, event, ctx| {
                me.handle_cloud_model_event(event, ctx);
            });
        }

        ctx.subscribe_to_model(
            &TemplatableMCPServerManager::handle(ctx),
            |me, event, ctx| {
                me.handle_templatable_mcp_server_manager_event(event, ctx);
            },
        );

        // In dev, it's possible the SQLite data read in for the default profile actually comes from a different environment
        // (say, we switch between local and staging servers). When that happens the default profile starts as synced but
        // then the profile is deleted when initial load returns. To fix that, we listen for the deletion of the default
        // profile and reset the model state when that happens.
        if ChannelState::channel().is_dogfood() {
            if let DefaultProfileState::Synced { id } = &default_profile_state {
                let sync_id_of_default_profile = *profile_id_to_sync_id
                    .get(id)
                    .expect("default profile is synced but no sync id found");
                ctx.subscribe_to_model(&CloudModel::handle(ctx), move |me, event, _| {
                if let CloudModelEvent::ObjectDeleted {
                    type_and_id: CloudObjectTypeAndId::GenericStringObject {
                        id: deleted_sync_id,
                        ..
                    },
                    ..
                } = event {
                    if *deleted_sync_id == sync_id_of_default_profile {
                        log::info!("Resetting execution profile model because default profile was deleted.");
                        me.reset();
                    }
                }
            });
            }
        }

        log::info!("Initialized execution profile model with state: {default_profile_state}",);

        let mut model = Self {
            default_profile_state,
            profile_id_to_sync_id,
            active_profiles_per_session,
        };

        model.maybe_inherit_from_legacy_settings(ctx);
        model
    }

    fn is_owned_by_current_user(
        profile: &super::CloudAIExecutionProfile,
        ctx: &AppContext,
    ) -> bool {
        UserWorkspaces::as_ref(ctx)
            .personal_drive(ctx)
            .is_some_and(|owner| profile.permissions().owner == owner)
    }

    /// This function performs one-time migrations from legacy settings into the default profile.
    /// The issue this solves is that, whenever we migrate an existing setting into the profile object,
    /// users will initialize the new field to its default value. We need to manually check to see if
    /// the legacy setting hasn't been migrated and, if it hasn't, do a one-time overwrite on the new profile
    /// field.
    fn maybe_inherit_from_legacy_settings(&mut self, ctx: &mut ModelContext<Self>) {
        let DefaultProfileState::Synced {
            id: default_profile_id,
        } = self.default_profile_state
        else {
            return;
        };

        if let Some(base_llm_id) = ctx
            .private_user_preferences()
            .read_value("PreferredAgentModeLLMId")
            .ok()
            .flatten()
            .map(|s| serde_json::from_str::<Option<LLMId>>(&s))
            .and_then(|res| res.ok())
            .flatten()
        {
            if let Err(e) = ctx
                .private_user_preferences()
                .remove_value("PreferredAgentModeLLMId")
            {
                log::error!("Failed to remove old PreferredAgentModeLLMId user pref: {e}");
            }
            self.set_base_model(default_profile_id, Some(base_llm_id.clone()), ctx);
            log::info!("Overwrote default profile with legacy setting for base llm: {base_llm_id}");
        }
    }

    pub fn create_profile(&mut self, ctx: &mut ModelContext<Self>) -> Option<ClientProfileId> {
        let profile_id = ClientProfileId::new();

        let Some(owner) = UserWorkspaces::as_ref(ctx).personal_drive(ctx) else {
            log::error!("Failed to create AI execution profile: personal drive not available");
            return None;
        };

        let mut new_profile = self.default_profile(ctx).data().clone();
        new_profile.name = "".to_string();
        new_profile.is_default_profile = false;
        new_profile.autosync_plans_to_warp_drive = true;

        let update_manager = UpdateManager::handle(ctx);
        let client_id = ClientId::default();
        update_manager.update(ctx, |update_manager, ctx| {
            update_manager.create_ai_execution_profile(new_profile, client_id, owner, ctx);
        });

        self.profile_id_to_sync_id
            .insert(profile_id, SyncId::ClientId(client_id));

        send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileCreated, ctx);

        ctx.emit(AIExecutionProfilesModelEvent::ProfileCreated);

        Some(profile_id)
    }

    pub fn delete_profile(&mut self, profile_id: ClientProfileId, ctx: &mut ModelContext<Self>) {
        let id = self.default_profile_state.id();
        if id == profile_id {
            log::warn!("Attempted to delete default profile (id: {profile_id})");
            return;
        }

        let Some(sync_id) = self.profile_id_to_sync_id.get(&profile_id).cloned() else {
            return;
        };

        self.active_profiles_per_session
            .retain(|_, active_profile_id| *active_profile_id != profile_id);

        self.profile_id_to_sync_id.remove(&profile_id);

        let update_manager = UpdateManager::handle(ctx);
        update_manager.update(ctx, |update_manager, ctx| {
            update_manager.delete_ai_execution_profile(sync_id, ctx);
        });

        send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileDeleted, ctx);
        ctx.emit(AIExecutionProfilesModelEvent::ProfileDeleted);
    }

    // On logout, we need to clear any existing profile state.
    pub fn reset(&mut self) {
        self.default_profile_state = DefaultProfileState::Unsynced {
            id: ClientProfileId::new(),
            profile: AIExecutionProfile {
                is_default_profile: true,
                ..Default::default()
            },
        };
        self.profile_id_to_sync_id.clear();
        self.active_profiles_per_session.clear();
    }

    /// Returns the active permissions profile for a specific terminal view.
    /// If no terminal_view is provided, returns the default profile.
    ///
    /// If you need to account for enterprise overrides, call `BlocklistAIPermissions::active_permissions_profile` instead.
    pub fn active_profile(
        &self,
        terminal_view_id: Option<EntityId>,
        ctx: &AppContext,
    ) -> AIExecutionProfileInfo {
        terminal_view_id
            .and_then(|id| self.active_profiles_per_session.get(&id))
            .and_then(|profile_id| self.get_profile_by_id(*profile_id, ctx))
            .unwrap_or_else(|| self.default_profile(ctx))
    }

    pub fn default_profile_id(&self) -> ClientProfileId {
        self.default_profile_state.id()
    }

    pub fn default_profile(&self, ctx: &AppContext) -> AIExecutionProfileInfo {
        match &self.default_profile_state {
            DefaultProfileState::Unsynced { id, profile } => AIExecutionProfileInfo {
                id: *id,
                sync_id: None,
                data: profile.clone(),
            },
            DefaultProfileState::Synced { id } => {
                let Some(sync_id) = self.profile_id_to_sync_id.get(id) else {
                    log::error!(
                        "Default profile is synced but no sync_id found in profile_id_to_sync_id map."
                    );
                    return AIExecutionProfileInfo {
                        id: *id,
                        sync_id: None,
                        data: AIExecutionProfile::default(),
                    };
                };
                let cloud_model = CloudModel::as_ref(ctx);
                let data = cloud_model
                    .get_object_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>(
                        sync_id,
                    )
                    .map(|o| o.model().string_model.clone())
                    .unwrap_or_default();

                AIExecutionProfileInfo {
                    id: *id,
                    sync_id: Some(*sync_id),
                    data,
                }
            }
            DefaultProfileState::Cli { id, profile } => AIExecutionProfileInfo {
                id: *id,
                sync_id: None,
                data: profile.clone(),
            },
        }
    }

    /// Sets the active profile for a specific terminal view.
    pub fn set_active_profile(
        &mut self,
        terminal_view_id: EntityId,
        profile_id: ClientProfileId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.active_profiles_per_session
            .insert(terminal_view_id, profile_id);
        ctx.emit(AIExecutionProfilesModelEvent::UpdatedActiveProfile { terminal_view_id });
    }

    /// Returns a profile by its client ID.
    /// Returns None if the profile is not found.
    pub fn get_profile_by_id(
        &self,
        profile_id: ClientProfileId,
        ctx: &AppContext,
    ) -> Option<AIExecutionProfileInfo> {
        // Handle an unsynced default profile (including CLI)
        match &self.default_profile_state {
            DefaultProfileState::Unsynced { id, profile }
            | DefaultProfileState::Cli { id, profile } => {
                if profile_id == *id {
                    return Some(AIExecutionProfileInfo {
                        id: *id,
                        sync_id: None,
                        data: profile.clone(),
                    });
                }
            }
            DefaultProfileState::Synced { .. } => {}
        }

        // Handle all synced profiles (default and non-default)
        let sync_id = self.profile_id_to_sync_id.get(&profile_id)?;
        let cloud_model = CloudModel::as_ref(ctx);
        let data = cloud_model
            .get_object_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>(sync_id)
            .map(|o| o.model().string_model.clone())
            .unwrap_or_default();

        Some(AIExecutionProfileInfo {
            id: profile_id,
            sync_id: Some(*sync_id),
            data,
        })
    }

    pub fn get_all_profile_ids(&self) -> Vec<ClientProfileId> {
        let default_profile_id = self.default_profile_state.id();

        // Default profile is always first in the list
        std::iter::once(default_profile_id)
            .chain(
                self.profile_id_to_sync_id
                    .keys()
                    .filter(|&&id| id != default_profile_id)
                    .cloned(),
            )
            .collect()
    }

    /// Look up a local client profile ID from its cloud sync ID.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn get_profile_id_by_sync_id(&self, sync_id: &SyncId) -> Option<ClientProfileId> {
        self.profile_id_to_sync_id
            .iter()
            .find_map(|(client_id, id)| {
                if id == sync_id {
                    Some(*client_id)
                } else {
                    None
                }
            })
    }

    pub fn has_multiple_profiles(&self) -> bool {
        let default_profile_id = self.default_profile_state.id();

        self.profile_id_to_sync_id
            .keys()
            .any(|&id| id != default_profile_id)
    }

    pub fn set_base_model(
        &mut self,
        profile_id: ClientProfileId,
        llm_id: Option<LLMId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.base_model != llm_id {
                    profile.base_model = llm_id.clone();
                    return true;
                }
                false
            },
            ctx,
        );

        if let Some(model_id) = &llm_id {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileModelSelected {
                    model_type: "base".to_string(),
                    model_value: model_id.to_string(),
                },
                ctx
            );
        }
    }

    pub fn set_coding_model(
        &mut self,
        profile_id: ClientProfileId,
        model_id: Option<LLMId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.coding_model != model_id {
                    profile.coding_model = model_id.clone();
                    return true;
                }
                false
            },
            ctx,
        );

        if let Some(model_id) = &model_id {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileModelSelected {
                    model_type: "coding".to_string(),
                    model_value: model_id.to_string(),
                },
                ctx
            );
        }
    }

    pub fn set_cli_agent_model(
        &mut self,
        profile_id: ClientProfileId,
        model_id: Option<LLMId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.cli_agent_model != model_id {
                    profile.cli_agent_model = model_id.clone();
                    return true;
                }
                false
            },
            ctx,
        );

        if let Some(model_id) = &model_id {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileModelSelected {
                    model_type: "cli_agent".to_string(),
                    model_value: model_id.to_string(),
                },
                ctx
            );
        }
    }

    pub fn set_computer_use_model(
        &mut self,
        profile_id: ClientProfileId,
        model_id: Option<LLMId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.computer_use_model != model_id {
                    profile.computer_use_model = model_id.clone();
                    return true;
                }
                false
            },
            ctx,
        );

        if let Some(model_id) = &model_id {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileModelSelected {
                    model_type: "computer_use".to_string(),
                    model_value: model_id.to_string(),
                },
                ctx
            );
        }
    }

    pub fn set_context_window_limit(
        &mut self,
        profile_id: ClientProfileId,
        limit: Option<u32>,
        ctx: &mut ModelContext<Self>,
    ) {
        let changed = self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.context_window_limit != limit {
                    profile.context_window_limit = limit;
                    return true;
                }
                false
            },
            ctx,
        );

        // Gate on the limit being non-empty. The limit is cleared during
        // reconciliation, which runs inside an `LLMPreferences` update where the
        // `LLMPreferences::as_ref` read below would panic.
        if changed && limit.is_some() {
            let Some(profile) = self.get_profile_by_id(profile_id, ctx) else {
                return;
            };
            let llm_preferences = LLMPreferences::as_ref(ctx);
            let model_info = profile
                .data()
                .base_model
                .as_ref()
                .and_then(|id| llm_preferences.get_llm_info(id))
                .unwrap_or_else(|| llm_preferences.get_default_base_model());
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileContextWindowSelected {
                    tokens: limit,
                    model_id: model_info.id.to_string(),
                },
                ctx
            );
        }
    }

    pub fn set_apply_code_diffs(
        &mut self,
        profile_id: ClientProfileId,
        apply_code_diffs: &ActionPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.apply_code_diffs != *apply_code_diffs {
                    profile.apply_code_diffs = *apply_code_diffs;
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "apply_code_diffs".to_string(),
                setting_value: format!("{apply_code_diffs:?}"),
            },
            ctx
        );
    }

    pub fn set_read_files(
        &mut self,
        profile_id: ClientProfileId,
        read_files: &ActionPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.read_files != *read_files {
                    profile.read_files = *read_files;
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "read_files".to_string(),
                setting_value: format!("{read_files:?}"),
            },
            ctx
        );
    }

    pub fn set_execute_commands(
        &mut self,
        profile_id: ClientProfileId,
        execute_commands: &ActionPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.execute_commands != *execute_commands {
                    profile.execute_commands = *execute_commands;
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "execute_commands".to_string(),
                setting_value: format!("{execute_commands:?}"),
            },
            ctx
        );
    }

    pub fn set_write_to_pty(
        &mut self,
        profile_id: ClientProfileId,
        write_to_pty: &WriteToPtyPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.write_to_pty != *write_to_pty {
                    profile.write_to_pty = *write_to_pty;
                    return true;
                }
                false
            },
            ctx,
        );
        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "write_to_pty".to_string(),
                setting_value: format!("{write_to_pty:?}"),
            },
            ctx
        );
    }

    pub fn set_mcp_permissions(
        &mut self,
        profile_id: ClientProfileId,
        mcp_permissions: &ActionPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.mcp_permissions == *mcp_permissions {
                    return false;
                }

                if mcp_permissions == &ActionPermission::AlwaysAllow {
                    profile.mcp_allowlist.clear();
                } else if mcp_permissions == &ActionPermission::AlwaysAsk {
                    profile.mcp_denylist.clear();
                }
                profile.mcp_permissions = *mcp_permissions;
                true
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "mcp_permissions".to_string(),
                setting_value: format!("{mcp_permissions:?}"),
            },
            ctx
        );
    }

    pub fn set_computer_use(
        &mut self,
        profile_id: ClientProfileId,
        permission: &super::ComputerUsePermission,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_value = self
            .get_profile_by_id(profile_id, ctx)
            .map(|p| p.data().computer_use);

        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.computer_use != *permission {
                    profile.computer_use = *permission;
                    return true;
                }
                false
            },
            ctx,
        );

        if current_value != Some(*permission) {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileSettingUpdated {
                    setting_type: "computer_use".to_string(),
                    setting_value: format!("{permission:?}"),
                },
                ctx
            );
        }
    }

    pub fn set_ask_user_question(
        &mut self,
        profile_id: ClientProfileId,
        permission: super::AskUserQuestionPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_value = self
            .get_profile_by_id(profile_id, ctx)
            .map(|p| p.data().ask_user_question);

        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.ask_user_question != permission {
                    profile.ask_user_question = permission;
                    return true;
                }
                false
            },
            ctx,
        );

        if current_value != Some(permission) {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileSettingUpdated {
                    setting_type: "ask_user_question".to_string(),
                    setting_value: format!("{permission:?}"),
                },
                ctx
            );
        }
    }

    pub fn set_run_agents(
        &mut self,
        profile_id: ClientProfileId,
        permission: super::RunAgentsPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_value = self
            .get_profile_by_id(profile_id, ctx)
            .map(|p| p.data().run_agents);

        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.run_agents != permission {
                    profile.run_agents = permission;
                    return true;
                }
                false
            },
            ctx,
        );

        if current_value != Some(permission) {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileSettingUpdated {
                    setting_type: "run_agents".to_string(),
                    setting_value: format!("{permission:?}"),
                },
                ctx
            );
        }
    }

    pub fn set_web_search_enabled(
        &mut self,
        profile_id: ClientProfileId,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.web_search_enabled != enabled {
                    profile.web_search_enabled = enabled;
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "web_search_enabled".to_string(),
                setting_value: format!("{enabled}"),
            },
            ctx
        );
    }

    /// Bulk-applies the most permissive "full control" settings to every
    /// per-action permission on the profile (analogous to Claude Code's
    /// `--dangerously-skip-permissions`). See
    /// [`AIExecutionProfile::apply_full_control`].
    pub fn grant_full_control(
        &mut self,
        profile_id: ClientProfileId,
        include_computer_use: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let changed = self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.has_full_control(include_computer_use) {
                    return false;
                }
                profile.apply_full_control(include_computer_use);
                true
            },
            ctx,
        );

        if changed {
            send_telemetry_from_ctx!(
                TelemetryEvent::AIExecutionProfileSettingUpdated {
                    setting_type: "grant_full_control".to_string(),
                    setting_value: format!("include_computer_use={include_computer_use}"),
                },
                ctx
            );
        }
    }

    pub fn set_autosync_plans_to_warp_drive(
        &mut self,
        profile_id: ClientProfileId,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.autosync_plans_to_warp_drive != enabled {
                    profile.autosync_plans_to_warp_drive = enabled;
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "plan_auto_sync".to_string(),
                setting_value: format!("{enabled}"),
            },
            ctx
        );
    }

    pub fn set_profile_name(
        &mut self,
        profile_id: ClientProfileId,
        name: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if profile.name != name {
                    profile.name = name.to_string();
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileSettingUpdated {
                setting_type: "name".to_string(),
                setting_value: name.to_string(),
            },
            ctx
        );
    }

    pub fn add_to_command_allowlist(
        &mut self,
        profile_id: ClientProfileId,
        predicate: &AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if !profile.command_allowlist.contains(predicate) {
                    profile.command_allowlist.push(predicate.clone());
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileAddedToAllowlist {
                list_type: "command".to_string(),
                value: predicate.to_string(),
            },
            ctx
        );
    }

    pub fn remove_from_command_allowlist(
        &mut self,
        profile_id: ClientProfileId,
        predicate: &AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                let original_len = profile.command_allowlist.len();
                profile.command_allowlist.retain(|p| p != predicate);
                profile.command_allowlist.len() != original_len
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileRemovedFromAllowlist {
                list_type: "command".to_string(),
                value: predicate.to_string(),
            },
            ctx
        );
    }

    pub fn add_to_directory_allowlist(
        &mut self,
        profile_id: ClientProfileId,
        path: &PathBuf,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if !profile.directory_allowlist.contains(path) {
                    profile.directory_allowlist.push(path.clone());
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileAddedToAllowlist {
                list_type: "directory".to_string(),
                value: path.to_string_lossy().to_string(),
            },
            ctx
        );
    }

    pub fn remove_from_directory_allowlist(
        &mut self,
        profile_id: ClientProfileId,
        path: &PathBuf,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                let original_len = profile.directory_allowlist.len();
                profile.directory_allowlist.retain(|p| p != path);
                profile.directory_allowlist.len() != original_len
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileRemovedFromAllowlist {
                list_type: "directory".to_string(),
                value: path.to_string_lossy().to_string(),
            },
            ctx
        );
    }

    pub fn add_to_command_denylist(
        &mut self,
        profile_id: ClientProfileId,
        predicate: &AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if !profile.command_denylist.contains(predicate) {
                    profile.command_denylist.push(predicate.clone());
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileAddedToDenylist {
                list_type: "command".to_string(),
                value: predicate.to_string(),
            },
            ctx
        );
    }

    pub fn remove_from_command_denylist(
        &mut self,
        profile_id: ClientProfileId,
        predicate: &AgentModeCommandExecutionPredicate,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                let original_len = profile.command_denylist.len();
                profile.command_denylist.retain(|p| p != predicate);
                profile.command_denylist.len() != original_len
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileRemovedFromDenylist {
                list_type: "command".to_string(),
                value: predicate.to_string(),
            },
            ctx
        );
    }

    pub fn add_to_mcp_allowlist(
        &mut self,
        profile_id: ClientProfileId,
        id: &Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if !profile.mcp_allowlist.contains(id) {
                    profile.mcp_allowlist.push(*id);
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileAddedToAllowlist {
                list_type: "mcp".to_string(),
                value: id.to_string(),
            },
            ctx
        );
    }

    pub fn remove_from_mcp_allowlist(
        &mut self,
        profile_id: ClientProfileId,
        id: &Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                let original_len = profile.mcp_allowlist.len();
                profile.mcp_allowlist.retain(|p| p != id);
                profile.mcp_allowlist.len() != original_len
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileRemovedFromAllowlist {
                list_type: "mcp".to_string(),
                value: id.to_string(),
            },
            ctx
        );
    }

    pub fn add_to_mcp_denylist(
        &mut self,
        profile_id: ClientProfileId,
        id: &Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                if !profile.mcp_denylist.contains(id) {
                    profile.mcp_denylist.push(*id);
                    return true;
                }
                false
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileAddedToDenylist {
                list_type: "mcp".to_string(),
                value: id.to_string(),
            },
            ctx
        );
    }

    pub fn remove_from_mcp_denylist(
        &mut self,
        profile_id: ClientProfileId,
        id: &Uuid,
        ctx: &mut ModelContext<Self>,
    ) {
        self.edit_profile_internal(
            profile_id,
            |profile| {
                let original_len = profile.mcp_denylist.len();
                profile.mcp_denylist.retain(|p| p != id);
                profile.mcp_denylist.len() != original_len
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::AIExecutionProfileRemovedFromDenylist {
                list_type: "mcp".to_string(),
                value: id.to_string(),
            },
            ctx
        );
    }

    /// `edit_profile_internal` edits an AIExecutionProfile and upserts the changed profile to the cloud
    /// Parameters:
    /// * `profile_id`: The id of the profile to edit
    /// * `edit_fn`: a closure that safely modifies the AIExecutionProfile. It should return `true` if the profile was changed, `false` otherwise. When `true`, it syncs the changes to the cloud, and otherwise exits early to prevent excessive cloud operations if no changes occurred.
    /// * `ctx`: The model context
    ///
    /// Returns `true` if the profile was actually changed (and synced),
    /// `false` otherwise. Callers can use this to gate side effects such as
    /// telemetry on real changes.
    fn edit_profile_internal(
        &mut self,
        profile_id: ClientProfileId,
        edit_fn: impl FnOnce(&mut AIExecutionProfile) -> bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        // We don't yet support editing the default profile for the CLI.
        if let DefaultProfileState::Cli { id, .. } = &self.default_profile_state {
            if *id == profile_id {
                log::warn!("Attempted to edit CLI default profile, which is not yet supported.");
                return false;
            }
        }

        // Case: this might be an edit to a not-yet-created default profile object. If so, we need to create
        // a cloud object to back the default profile.
        if let DefaultProfileState::Unsynced { id, profile } = &self.default_profile_state {
            if *id == profile_id {
                let mut new_profile = profile.clone();
                // If the edit function didn't make any changes to the profile, it's still the default profile, so we don't need to sync it
                let value_changed = edit_fn(&mut new_profile);
                if !value_changed {
                    return false;
                }

                if let Some(owner) = UserWorkspaces::as_ref(ctx).personal_drive(ctx) {
                    let update_manager = UpdateManager::handle(ctx);
                    let client_id = ClientId::default();
                    update_manager.update(ctx, |update_manager, ctx| {
                        update_manager.create_ai_execution_profile(
                            new_profile,
                            client_id,
                            owner,
                            ctx,
                        );
                    });

                    // For forever on, the default profile state is synced.
                    let sync_id = SyncId::ClientId(client_id);
                    self.default_profile_state = DefaultProfileState::Synced { id: profile_id };
                    self.profile_id_to_sync_id.insert(profile_id, sync_id);

                    log::info!(
                        "Creating a cloud object for the default execution profile: {profile_id:?}"
                    );
                } else {
                    // The user isn't logged in yet (or personal drive isn't available),
                    // so we can't create a cloud object. Persist the edit locally on the
                    // Unsynced profile so it isn't silently dropped; it will be promoted
                    // to a Synced cloud object the next time an edit runs after login.
                    // Without this, onboarding-driven edits (e.g. autonomy permissions
                    // written by `apply_agent_settings`) disappear when onboarding is
                    // completed before login.
                    self.default_profile_state = DefaultProfileState::Unsynced {
                        id: profile_id,
                        profile: new_profile,
                    };

                    log::info!(
                        "Updated local unsynced default execution profile (no personal drive yet): {profile_id:?}"
                    );
                }
                ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(profile_id));
                return true;
            }
        }

        let mut value_changed = false;
        if let Some(sync_id) = self.profile_id_to_sync_id.get(&profile_id) {
            let cloud_model = CloudModel::as_ref(ctx);
            if let Some(object) = cloud_model
                .get_object_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>(sync_id)
            {
                let mut data = object.model().string_model.clone();
                // If the edit function didn't make any changes to the profile, we should exit early
                value_changed = edit_fn(&mut data);
                if !value_changed {
                    return false;
                }
                let update_manager = UpdateManager::handle(ctx);
                update_manager.update(ctx, |update_manager, ctx| {
                    update_manager.update_ai_execution_profile(data, *sync_id, None, ctx);
                });

                log::info!("Edited execution profile with id: {profile_id:?}");
            } else {
                log::error!("Profile id is mapped but no object found: {profile_id:?}");
            }
        }
        ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(profile_id));
        value_changed
    }

    /// Handle CloudModel events to keep the profile_id_to_sync_id map and default profile state up to date.
    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            CloudModelEvent::ObjectCreated {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile),
                        id,
                    },
            } => {
                self.handle_ai_execution_profile_created(*id, ctx);
            }
            CloudModelEvent::ObjectDeleted {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile),
                        id,
                    },
                folder_id: _,
            } => {
                self.handle_ai_execution_profile_deleted(*id, ctx);
            }
            CloudModelEvent::ObjectDeleted {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
                folder_id: _,
            } => {
                // Legacy MCP servers are converted to templatable on startup;
                // no action needed when a legacy cloud object is deleted.
            }
            CloudModelEvent::ObjectUpdated {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type:
                            GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile),
                        id,
                    },
                source,
            } => {
                self.handle_ai_execution_profile_updated(*id, *source, ctx);
            }
            CloudModelEvent::InitialLoadCompleted => {
                self.reconcile_with_cloud_state_after_initial_load(ctx);
            }
            _ => {}
        }
    }

    /// Reconcile model state with `CloudModel` once an initial bulk load
    /// completes.
    ///
    /// The initial load path (`update_objects_from_initial_load`) inserts
    /// cloud objects into `CloudModel` *without* emitting per-object
    /// `ObjectCreated` events — it emits a single
    /// `CloudModelEvent::InitialLoadCompleted` afterward instead. That means
    /// our normal `handle_ai_execution_profile_created` handler never fires
    /// for execution profiles that arrived via initial load, and the model
    /// stays in `Unsynced` even though the user already has a cloud default
    /// profile.
    ///
    /// Without this reconciliation, a subsequent edit from `apply_agent_settings`
    /// (onboarding) would hit the `Unsynced` branch of `edit_profile_internal`
    /// and *create a duplicate* cloud default profile rather than editing the
    /// existing one. That manifests as the default profile showing neither
    /// the user's prior cloud values nor the onboarding choices — because the
    /// UI ends up reading a fresh client-side default with only a few fields
    /// touched.
    fn reconcile_with_cloud_state_after_initial_load(&mut self, ctx: &mut ModelContext<Self>) {
        let cloud_model = CloudModel::as_ref(ctx);
        let all_profiles: Vec<(SyncId, bool)> = cloud_model
            .get_all_objects_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>()
            .filter(|o| Self::is_owned_by_current_user(o, ctx))
            .map(|o| (o.id, o.model().string_model.is_default_profile))
            .collect();

        // Transition Unsynced -> Synced if cloud has a default profile.
        if let DefaultProfileState::Unsynced { id, .. } = self.default_profile_state {
            if let Some((sync_id, _)) = all_profiles.iter().find(|(_, is_default)| *is_default) {
                self.default_profile_state = DefaultProfileState::Synced { id };
                self.profile_id_to_sync_id.insert(id, *sync_id);
                log::info!(
                    "Reconciled default execution profile with cloud after initial load: \
                     profile_id={id:?}, sync_id={sync_id:?}"
                );
                ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(id));
            }
        }

        // Register non-default profiles from cloud that we aren't
        // already tracking so later edits find their backing sync_id.
        let mut added_non_default = false;
        for (sync_id, is_default) in all_profiles {
            if is_default {
                continue;
            }
            if !self.profile_id_to_sync_id.values().any(|s| *s == sync_id) {
                let profile_id = ClientProfileId::new();
                self.profile_id_to_sync_id.insert(profile_id, sync_id);
                log::info!(
                    "Registered existing cloud execution profile after initial load: {sync_id:?}"
                );
                added_non_default = true;
            }
        }
        if added_non_default {
            ctx.emit(AIExecutionProfilesModelEvent::ProfileCreated);
        }
    }

    fn handle_templatable_mcp_server_manager_event(
        &mut self,
        event: &TemplatableMCPServerManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            TemplatableMCPServerManagerEvent::TemplatableMCPServersUpdated => {
                self.remove_deleted_mcp_servers(ctx);
            }
            TemplatableMCPServerManagerEvent::LegacyServerConverted
            | TemplatableMCPServerManagerEvent::StateChanged { uuid: _, state: _ }
            | TemplatableMCPServerManagerEvent::ServerInstallationAdded(_)
            | TemplatableMCPServerManagerEvent::ServerInstallationDeleted(_) => {}
        }
    }

    /// Handle a newly created AI execution profile from the cloud.
    fn handle_ai_execution_profile_created(
        &mut self,
        sync_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_model = CloudModel::as_ref(ctx);
        let Some(object) = cloud_model
            .get_object_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>(&sync_id)
        else {
            log::warn!("Received ObjectCreated event for AI execution profile but object not found in CloudModel: {sync_id:?}");
            return;
        };

        if !Self::is_owned_by_current_user(object, ctx) {
            log::info!("Ignoring non-owned execution profile from cloud: {sync_id:?}");
            return;
        }

        // Check if this is the default profile
        if object.model().string_model.is_default_profile {
            // Don't add the cloud default profile if we're in CLI mode
            if matches!(self.default_profile_state, DefaultProfileState::Cli { .. }) {
                log::info!("Ignoring cloud default profile in CLI mode: {sync_id:?}");
                return;
            }

            // If we're in an unsynced state, transition to synced
            if let DefaultProfileState::Unsynced { id, .. } = self.default_profile_state {
                self.default_profile_state = DefaultProfileState::Synced { id };
                self.profile_id_to_sync_id.insert(id, sync_id);
                log::info!(
                    "Received default execution profile from cloud. Marking profile as synced: {sync_id:?}"
                );
                ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(id));
            }

            return;
        }

        // For non-default profiles, add to the map if not already present
        let profile_exists = self.profile_id_to_sync_id.values().any(|id| *id == sync_id);
        if !profile_exists {
            let profile_id = ClientProfileId::new();
            self.profile_id_to_sync_id.insert(profile_id, sync_id);
            log::info!("Added new execution profile to map: {sync_id:?}");
            ctx.emit(AIExecutionProfilesModelEvent::ProfileCreated);
        }
    }

    /// Handle a deleted AI execution profile from the cloud.
    fn handle_ai_execution_profile_deleted(
        &mut self,
        sync_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Find and remove the profile from our map
        let profile_id = self
            .profile_id_to_sync_id
            .iter()
            .find_map(|(client_id, id)| {
                if *id == sync_id {
                    Some(*client_id)
                } else {
                    None
                }
            });

        if let Some(profile_id) = profile_id {
            self.profile_id_to_sync_id.remove(&profile_id);

            // Also remove from active profiles per session
            self.active_profiles_per_session
                .retain(|_, active_id| *active_id != profile_id);

            // If the default profile was deleted, transition back to unsynced state
            let is_default = matches!(&self.default_profile_state, DefaultProfileState::Synced { id } if *id == profile_id);
            if is_default {
                log::warn!("Default execution profile was deleted from cloud. Transitioning to unsynced state: {sync_id:?}");
                self.default_profile_state = DefaultProfileState::Unsynced {
                    id: profile_id,
                    profile: AIExecutionProfile {
                        is_default_profile: true,
                        ..Default::default()
                    },
                };
            }

            log::info!("Removed execution profile from map: {sync_id:?}");
            ctx.emit(AIExecutionProfilesModelEvent::ProfileDeleted);
        }
    }

    /// Handle an updated AI execution profile from the cloud.
    fn handle_ai_execution_profile_updated(
        &mut self,
        sync_id: SyncId,
        source: UpdateSource,
        ctx: &mut ModelContext<Self>,
    ) {
        // Only notify about updates from the server (not local updates, which we already handle)
        if source != UpdateSource::Server {
            return;
        }

        // Find the client profile ID for this sync ID
        let profile_id = self.get_profile_id_by_sync_id(&sync_id);

        if let Some(profile_id) = profile_id {
            log::info!("Execution profile updated from server: {sync_id:?}");
            ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(profile_id));
        }
    }

    /// Handle deleted MCP servers by deleting its uuid from all profiles.
    fn remove_deleted_mcp_servers(&mut self, ctx: &mut ModelContext<Self>) {
        let all_valid_uuids = TemplatableMCPServerManager::get_all_cloud_synced_mcp_servers(ctx);
        for profile_id in self.get_all_profile_ids() {
            self.edit_profile_internal(
                profile_id,
                |profile| {
                    let original_allowlist_len = profile.mcp_allowlist.len();
                    let original_denylist_len = profile.mcp_denylist.len();
                    profile
                        .mcp_allowlist
                        .retain(|uuid| all_valid_uuids.contains_key(uuid));
                    profile
                        .mcp_denylist
                        .retain(|uuid| all_valid_uuids.contains_key(uuid));
                    profile.mcp_allowlist.len() != original_allowlist_len
                        || profile.mcp_denylist.len() != original_denylist_len
                },
                ctx,
            );
        }
    }

    // We don't want stale client ids in our map. We won't be able to find the backing cloud object when
    // an edit occurs.
    pub fn replace_client_id_with_server_id(&mut self, server_id: SyncId, client_id: SyncId) {
        for (_, sync_id) in self.profile_id_to_sync_id.iter_mut() {
            if *sync_id == client_id {
                *sync_id = server_id;
                log::info!("Updated profile id mapping after creating a new execution profile");
            }
        }
    }

    /// Replaces the given profile's data with CLI defaults for the given sandboxed state.
    /// Use in tests to simulate the profile configuration used by the sandboxed CLI agent.
    #[cfg(test)]
    pub fn apply_cli_profile_defaults_for_test(
        &mut self,
        profile_id: ClientProfileId,
        is_sandboxed: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let cli_profile = AIExecutionProfile::create_default_cli_profile(is_sandboxed, None);
        self.edit_profile_internal(
            profile_id,
            move |profile| {
                *profile = cli_profile;
                true
            },
            ctx,
        );
    }
}

#[allow(clippy::enum_variant_names)]
pub enum AIExecutionProfilesModelEvent {
    ProfileUpdated(ClientProfileId),
    ProfileCreated,
    ProfileDeleted,
    UpdatedActiveProfile { terminal_view_id: EntityId },
}

impl Entity for AIExecutionProfilesModel {
    type Event = AIExecutionProfilesModelEvent;
}

impl SingletonEntity for AIExecutionProfilesModel {}

#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
