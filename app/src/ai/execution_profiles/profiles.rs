use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context as _;
use indexmap::IndexMap;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences;
use warp_errors::report_error;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity};

use super::{
    AIExecutionProfile, ActionPermission, CloudAIExecutionProfileModel, ExecutionProfileId,
    ExecutionProfilesConfig, WriteToPtyPermission,
};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManagerEvent;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::{CloudModelEvent, UpdateSource};
use crate::cloud_object::{CloudObject as _, GenericStringObjectFormat, JsonObjectType};
use crate::drive::CloudObjectTypeAndId;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::settings::cloud_preferences::CloudPreferencesSettings;
use crate::settings::cloud_preferences_syncer::{
    CloudPreferencesSyncer, CloudPreferencesSyncerEvent,
};
use crate::settings::{
    AISettings, AISettingsChangedEvent, AgentModeCommandExecutionPredicate, ExecutionProfiles,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{CloudModel, LaunchMode, TelemetryEvent, send_telemetry_from_ctx};

#[derive(Clone, Debug)]
pub struct AIExecutionProfileInfo {
    id: ExecutionProfileId,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    sync_id: Option<SyncId>,
    data: AIExecutionProfile,
}

impl AIExecutionProfileInfo {
    pub fn id(&self) -> &ExecutionProfileId {
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

/// Enables file-backed profiles for every TUI build and for flagged GUI builds.
///
/// CLI and remote-server modes retain their dedicated in-memory behavior.
fn file_backed_execution_profiles_enabled(launch_mode: &LaunchMode) -> bool {
    match launch_mode {
        LaunchMode::Tui { .. } => true,
        LaunchMode::App { .. } | LaunchMode::Test { .. } => {
            FeatureFlag::FileBackedExecutionProfiles.is_enabled()
        }
        LaunchMode::CommandLine { .. }
        | LaunchMode::RemoteServerProxy
        | LaunchMode::RemoteServerDaemon { .. } => false,
    }
}

/// Selects the authoritative persistence backend for execution profiles.
#[derive(Clone, Copy, Debug)]
enum ProfileSource {
    /// Existing per-profile Warp Drive objects used by GUI rollback builds.
    LegacyCloudObjects,
    /// One file-backed collection stored in [`AISettings`].
    SettingsCollection {
        /// Whether this launch imports account-owned legacy cloud objects.
        migrates_legacy_cloud_profiles: bool,
    },
}

impl ProfileSource {
    /// Resolves the persistence backend for this launch.
    fn for_launch_mode(launch_mode: &LaunchMode) -> Self {
        if !file_backed_execution_profiles_enabled(launch_mode) {
            return Self::LegacyCloudObjects;
        }

        Self::SettingsCollection {
            migrates_legacy_cloud_profiles: matches!(
                launch_mode,
                LaunchMode::App { .. } | LaunchMode::Test { .. }
            ),
        }
    }

    /// Returns whether this source stores profiles in the settings collection.
    fn is_settings_collection(self) -> bool {
        matches!(self, Self::SettingsCollection { .. })
    }

    /// Returns whether this launch performs the one-way legacy import.
    fn imports_legacy_profiles(self) -> bool {
        matches!(
            self,
            Self::SettingsCollection {
                migrates_legacy_cloud_profiles: true
            }
        )
    }

    /// Returns the stable key for a default profile read from the selected source.
    fn default_profile_id(self) -> ExecutionProfileId {
        if self.imports_legacy_profiles() {
            ExecutionProfileId::default_profile()
        } else {
            ExecutionProfileId::new()
        }
    }

    /// Derives the stable collection key assigned to a legacy cloud object.
    ///
    /// Synced non-default profiles derive their key from the server ID so independent clients
    /// produce the same mapping. A profile that still has only a client ID receives a generated
    /// key until the server ID arrives.
    fn legacy_profile_id(self, sync_id: SyncId, is_default_profile: bool) -> ExecutionProfileId {
        if is_default_profile {
            return self.default_profile_id();
        }
        if !self.imports_legacy_profiles() {
            return ExecutionProfileId::new();
        }
        sync_id
            .into_server()
            .map(ExecutionProfileId::from_legacy_server_id)
            .unwrap_or_else(ExecutionProfileId::new)
    }
}

/// Tracks which profile source is authoritative while the settings collection is initialized.
///
/// This state is process-local. An explicit [`ExecutionProfiles`] value is the durable signal
/// that a collection was materialized on a previous launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsMigrationState {
    /// No explicit collection is available, so reads continue using owned legacy cloud objects.
    PendingLegacyImport,
    /// The settings collection is authoritative, but its initial cloud reconciliation is pending.
    PendingExplicitSync,
    /// The collection needs no further migration work during this process.
    Complete,
}

impl SettingsMigrationState {
    /// Selects the initial authority state for a profile source.
    fn for_launch(source: ProfileSource, settings_profiles_are_explicit: bool) -> Self {
        match (
            source.imports_legacy_profiles(),
            settings_profiles_are_explicit,
        ) {
            (true, true) => Self::PendingExplicitSync,
            (true, false) => Self::PendingLegacyImport,
            (false, _) => Self::Complete,
        }
    }

    /// Returns whether this state permits reads and writes through [`AISettings`].
    fn settings_are_authoritative(self) -> bool {
        !matches!(self, Self::PendingLegacyImport)
    }
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum DefaultProfileState {
    Unsynced {
        id: ExecutionProfileId,
        profile: AIExecutionProfile,
    },
    Synced {
        id: ExecutionProfileId,
    },
    /// Currently, the behavior of the CLI default is that it
    /// cannot be updated and will never be synced.
    #[allow(dead_code)]
    Cli {
        id: ExecutionProfileId,
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
    pub fn id(&self) -> ExecutionProfileId {
        match self {
            DefaultProfileState::Unsynced { id, .. } => id.clone(),
            DefaultProfileState::Synced { id } => id.clone(),
            DefaultProfileState::Cli { id, .. } => id.clone(),
        }
    }
}

pub struct AIExecutionProfilesModel {
    /// Authoritative persistence backend selected for this launch.
    source: ProfileSource,
    /// State of the one-time import into the settings collection.
    settings_migration_state: SettingsMigrationState,
    /// Whether post-auth onboarding must preserve the materialized default profile.
    preserve_profile_onboarding_overrides: bool,
    /// Previous settings snapshot used only to classify collection change events.
    last_settings_profiles: ExecutionProfilesConfig,
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
    /// Maps stable profile IDs to legacy cloud-object IDs for the rollback backend.
    profile_id_to_sync_id: HashMap<ExecutionProfileId, SyncId>,
    /// Only contains entries for non-default profiles.
    active_profiles_per_session: HashMap<EntityId, ExecutionProfileId>,
}

impl AIExecutionProfilesModel {
    #[allow(unused_variables)]
    pub(crate) fn new(launch_mode: &LaunchMode, ctx: &mut ModelContext<Self>) -> Self {
        // Resolve the persistence backend before constructing any source-specific state.
        let source = ProfileSource::for_launch_mode(launch_mode);
        let uses_file_backed_profiles = source.is_settings_collection();
        let imports_legacy_profiles = source.imports_legacy_profiles();

        // A TUI with no explicit collection seeds its default from the existing
        // local scalar settings, then uses only the collection.
        let mut last_settings_profiles = AISettings::as_ref(ctx).execution_profiles.value().clone();
        if matches!(launch_mode, LaunchMode::Tui { .. })
            && !AISettings::as_ref(ctx)
                .execution_profiles
                .is_value_explicitly_set()
        {
            let mut profiles = ExecutionProfilesConfig::default();
            profiles.insert(
                ExecutionProfileId::default_profile(),
                super::create_default_for_tui_from_legacy_settings(ctx),
            );
            if let Err(error) = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.execution_profiles.set_value(profiles.clone(), ctx)
            }) {
                report_error!(error.context("Failed to initialize TUI execution profiles"));
            } else {
                last_settings_profiles = profiles;
            }
        }

        let settings_profiles_are_explicit = AISettings::as_ref(ctx)
            .execution_profiles
            .is_value_explicitly_set();
        // Existing collections are authoritative immediately. A migration-capable launch without
        // one keeps reading legacy objects until the import has materialized the settings value.
        let settings_migration_state =
            SettingsMigrationState::for_launch(source, settings_profiles_are_explicit);
        let settings_are_authoritative =
            uses_file_backed_profiles && settings_migration_state.settings_are_authoritative();

        // Build the source-specific legacy state. The settings backend does not
        // need cloud IDs; all effective reads still come directly from AISettings.
        cfg_if::cfg_if! {
            if #[cfg(feature = "agent_mode_evals")] {
                let default_profile_state = DefaultProfileState::Unsynced {
                    id: ExecutionProfileId::new(),
                    profile: AIExecutionProfile::create_agent_mode_eval_profile(),
                };
                let profile_id_to_sync_id: HashMap<ExecutionProfileId, SyncId> = HashMap::new();
                let active_profiles_per_session: HashMap<EntityId, ExecutionProfileId> = HashMap::new();
            } else {
                let (
                    default_profile_state,
                    profile_id_to_sync_id,
                    active_profiles_per_session,
                ) = if settings_are_authoritative {
                    (
                        DefaultProfileState::Unsynced {
                            id: ExecutionProfileId::default_profile(),
                            profile: AIExecutionProfile::default(),
                        },
                        HashMap::new(),
                        HashMap::new(),
                    )
                } else {
                    // Legacy GUI/CLI/remote launches reconstruct their local ID
                    // mappings from the currently loaded cloud objects.
                    let cloud_model = CloudModel::handle(ctx).as_ref(ctx);
                    let all_profiles_from_cloud: Vec<&super::CloudAIExecutionProfile> = cloud_model
                        .get_all_objects_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>()
                        .filter(|p| Self::is_owned_by_current_user(p, ctx))
                        .collect();

                    let default_profile_from_cloud: Option<&super::CloudAIExecutionProfile> = all_profiles_from_cloud
                        .iter()
                        .find(|obj| obj.model().string_model.is_default_profile)
                        .copied();

                    let mut profile_id_to_sync_id: HashMap<ExecutionProfileId, SyncId> = HashMap::new();
                    let active_profiles_per_session: HashMap<EntityId, ExecutionProfileId> = HashMap::new();

                    // Insert all non-default profiles from the cloud
                    for cloud_profile in all_profiles_from_cloud.iter().filter(|p| !p.model().string_model.is_default_profile) {
                        let profile_id = source.legacy_profile_id(cloud_profile.id, false);
                        profile_id_to_sync_id.insert(profile_id, cloud_profile.id);
                    }

                    let default_profile_state = match launch_mode {
                        LaunchMode::App { .. }
                        | LaunchMode::Test { .. } => {
                            match default_profile_from_cloud {
                                Some(p) => {
                                    let execution_profile_id =
                                        source.legacy_profile_id(p.id, true);
                                    profile_id_to_sync_id.insert(execution_profile_id.clone(), p.id);
                                    DefaultProfileState::Synced {
                                        id: execution_profile_id,
                                    }
                                }
                                None => DefaultProfileState::Unsynced {
                                    id: source.default_profile_id(),
                                    profile: super::create_default_from_legacy_settings(ctx),
                                },
                            }
                        }
                        // When running as a CLI, we ignore the GUI default and use a more permissive default.
                        LaunchMode::CommandLine { is_sandboxed, computer_use_override, .. } => {
                            DefaultProfileState::Cli {
                                profile: AIExecutionProfile::create_default_cli_profile(*is_sandboxed, *computer_use_override),
                                id: ExecutionProfileId::new(),
                            }
                        }
                        // RemoteServerProxy and RemoteServerDaemon don't use AI
                        // execution profiles. They never reach this code path
                        // since they don't go through initialize_app, but handle
                        // exhaustively.
                        LaunchMode::RemoteServerProxy | LaunchMode::RemoteServerDaemon { .. } => DefaultProfileState::Unsynced {
                            id: ExecutionProfileId::new(),
                            profile: super::create_default_from_legacy_settings(ctx),
                        },
                        // Settings-backed TUI initialization is handled before the
                        // legacy cloud-object branch.
                        LaunchMode::Tui { .. } => unreachable!("TUI profiles use settings"),
                    };
                    (
                        default_profile_state,
                        profile_id_to_sync_id,
                        active_profiles_per_session,
                    )
                };
            }
        }

        // We have to listen for changes to AIExecutionProfiles for a few reasons:
        // (1) In case the default profile is unsynced AND a default profile arrives from the cloud
        // (2) Let views subscribed to us know whenever a backing profile changes.
        // (3) Keep profile_id_to_sync_id map up to date when profiles are created/deleted remotely
        if !cfg!(feature = "agent_mode_evals") && !uses_file_backed_profiles {
            ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, event, ctx| {
                me.handle_cloud_model_event(event, ctx);
            });
        }

        if uses_file_backed_profiles {
            ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
                if matches!(event, AISettingsChangedEvent::ExecutionProfiles { .. }) {
                    me.handle_settings_profiles_changed(ctx);
                }
            });

            if imports_legacy_profiles {
                if ctx.has_singleton_model::<AuthManager>() {
                    ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| {
                        if matches!(event, AuthManagerEvent::AuthComplete) {
                            me.migrate_settings_profiles(ctx);
                        }
                    });
                }

                if ctx.has_singleton_model::<CloudPreferencesSyncer>() {
                    ctx.subscribe_to_model(
                        &CloudPreferencesSyncer::handle(ctx),
                        |me, _, event, ctx| {
                            if matches!(event, CloudPreferencesSyncerEvent::InitialLoadCompleted) {
                                me.migrate_settings_profiles(ctx);
                            }
                        },
                    );
                }
                ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, event, ctx| {
                    if !me.settings_are_authoritative() {
                        me.handle_cloud_model_event(event, ctx);
                    }
                    if let CloudModelEvent::ObjectSynced {
                        type_and_id:
                            CloudObjectTypeAndId::GenericStringObject {
                                object_type:
                                    GenericStringObjectFormat::Json(
                                        JsonObjectType::AIExecutionProfile,
                                    ),
                                ..
                            },
                        client_id,
                        server_id,
                    } = event
                    {
                        me.replace_client_id_with_server_id(
                            SyncId::ServerId(*server_id),
                            SyncId::ClientId(*client_id),
                            ctx,
                        );
                        me.migrate_settings_profiles(ctx);
                    } else if matches!(event, CloudModelEvent::InitialLoadCompleted) {
                        me.migrate_settings_profiles(ctx);
                    }
                });
            }
        }

        ctx.subscribe_to_model(
            &TemplatableMCPServerManager::handle(ctx),
            |me, _, event, ctx| {
                me.handle_templatable_mcp_server_manager_event(event, ctx);
            },
        );

        // In dev, it's possible the SQLite data read in for the default profile actually comes from a different environment
        // (say, we switch between local and staging servers). When that happens the default profile starts as synced but
        // then the profile is deleted when initial load returns. To fix that, we listen for the deletion of the default
        // profile and reset the model state when that happens.
        if !uses_file_backed_profiles
            && ChannelState::channel().is_dogfood()
            && let DefaultProfileState::Synced { id } = &default_profile_state
        {
            let sync_id_of_default_profile = *profile_id_to_sync_id
                .get(id)
                .expect("default profile is synced but no sync id found");
            ctx.subscribe_to_model(&CloudModel::handle(ctx), move |me, _, event, _| {
                if let CloudModelEvent::ObjectDeleted {
                    type_and_id:
                        CloudObjectTypeAndId::GenericStringObject {
                            id: deleted_sync_id,
                            ..
                        },
                    ..
                } = event
                    && *deleted_sync_id == sync_id_of_default_profile
                {
                    log::info!(
                        "Resetting execution profile model because default profile was deleted."
                    );
                    me.reset(false);
                }
            });
        }

        log::info!(
            "Initialized execution profile model with state: {default_profile_state}, file_backed_profiles: {uses_file_backed_profiles}",
        );

        let mut model = Self {
            source,
            settings_migration_state,
            preserve_profile_onboarding_overrides: settings_profiles_are_explicit,
            last_settings_profiles,
            default_profile_state,
            profile_id_to_sync_id,
            active_profiles_per_session,
        };

        if !uses_file_backed_profiles {
            model.maybe_inherit_from_legacy_settings(ctx);
        } else if imports_legacy_profiles
            && ctx.has_singleton_model::<CloudPreferencesSyncer>()
            && CloudPreferencesSyncer::as_ref(ctx).has_completed_initial_load()
        {
            // The syncer may finish before this model is registered. In that
            // case its one-shot event cannot reach this subscription, so run
            // migration from the already-completed state.
            model.migrate_settings_profiles(ctx);
        }
        model
    }

    /// Returns whether a legacy cloud profile belongs to the current personal drive.
    fn is_owned_by_current_user(
        profile: &super::CloudAIExecutionProfile,
        ctx: &AppContext,
    ) -> bool {
        UserWorkspaces::as_ref(ctx)
            .personal_drive(ctx)
            .is_some_and(|owner| profile.permissions().owner == owner)
    }

    /// Returns whether the settings collection is currently authoritative for profile operations.
    fn settings_are_authoritative(&self) -> bool {
        self.source.is_settings_collection()
            && self.settings_migration_state.settings_are_authoritative()
    }

    /// Snapshots the currently visible legacy-backed profiles into a settings collection.
    ///
    /// Local edits use this snapshot to become file-backed without exposing the empty implicit
    /// settings default.
    fn pending_legacy_profiles(&self, ctx: &AppContext) -> ExecutionProfilesConfig {
        let mut profiles = ExecutionProfilesConfig::default();
        for profile_id in self.get_all_profile_ids() {
            if let Some(profile) = self.get_profile_by_id(&profile_id, ctx) {
                profiles.insert(profile_id, profile.data);
            }
        }
        profiles
    }

    fn cloud_collection_exists(ctx: &AppContext) -> bool {
        CloudModel::as_ref(ctx)
            .get_all_cloud_preferences_by_storage_key()
            .contains_key(ExecutionProfiles::storage_key())
    }

    /// Returns whether settings sync must apply an existing cloud collection before local changes.
    ///
    /// Deferring in this state prevents stale legacy or local values from overwriting a newer
    /// collection received from another client.
    fn cloud_collection_awaiting_reconciliation(ctx: &AppContext) -> bool {
        *CloudPreferencesSettings::as_ref(ctx)
            .settings_sync_enabled
            .value()
            && Self::cloud_collection_exists(ctx)
            && ctx.has_singleton_model::<CloudPreferencesSyncer>()
            && !CloudPreferencesSyncer::as_ref(ctx).has_completed_initial_load()
    }

    /// Makes a locally edited pending collection authoritative in [`AISettings`].
    ///
    /// Returns `false` without changing authority when cloud reconciliation must run first or when
    /// the collection cannot be persisted.
    fn activate_pending_settings_collection(
        &mut self,
        profiles: ExecutionProfilesConfig,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if AuthStateProvider::as_ref(ctx).get().user_id().is_some()
            && !UpdateManager::as_ref(ctx).has_completed_initial_load()
        {
            return false;
        }

        if Self::cloud_collection_awaiting_reconciliation(ctx) {
            return false;
        }
        let update_result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.execution_profiles.set_value(profiles, ctx)
        });
        match update_result {
            Ok(()) => {
                self.settings_migration_state = SettingsMigrationState::PendingExplicitSync;
                self.preserve_profile_onboarding_overrides = true;
                true
            }
            Err(error) => {
                report_error!(
                    error.context("Failed to materialize pending execution profiles in settings")
                );
                false
            }
        }
    }

    /// Classifies a collection update into profile events and removes stale selections.
    fn handle_settings_profiles_changed(&mut self, ctx: &mut ModelContext<Self>) {
        let current = AISettings::as_ref(ctx).execution_profiles.value().clone();
        let previous_ids = self
            .last_settings_profiles
            .profile_ids()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let current_ids = current
            .profile_ids()
            .cloned()
            .collect::<std::collections::HashSet<_>>();

        if current_ids.iter().any(|id| !previous_ids.contains(id)) {
            ctx.emit(AIExecutionProfilesModelEvent::ProfileCreated);
        }
        if previous_ids.iter().any(|id| !current_ids.contains(id)) {
            ctx.emit(AIExecutionProfilesModelEvent::ProfileDeleted);
        }
        for id in current_ids.intersection(&previous_ids) {
            if current.profile(id) != self.last_settings_profiles.profile(id) {
                ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated((*id).clone()));
            }
        }
        self.active_profiles_per_session
            .retain(|_, id| current_ids.contains(id));
        self.last_settings_profiles = current;
        if self.settings_migration_state == SettingsMigrationState::PendingLegacyImport
            && AISettings::as_ref(ctx)
                .execution_profiles
                .is_value_explicitly_set()
        {
            self.settings_migration_state = SettingsMigrationState::PendingExplicitSync;
            self.preserve_profile_onboarding_overrides = true;
        }
    }

    /// Attempts to make the account's file-backed execution-profile collection authoritative.
    ///
    /// An explicit collection is reconciled with cloud preferences after their initial-load
    /// direction is known. Otherwise, owned legacy cloud objects are imported once all have server
    /// IDs. Missing prerequisites leave the migration pending so a later readiness event can retry.
    pub(crate) fn migrate_settings_profiles(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.source.imports_legacy_profiles()
            || self.settings_migration_state == SettingsMigrationState::Complete
        {
            return;
        }
        if AuthStateProvider::as_ref(ctx).get().user_id().is_none() {
            return;
        }

        if !UpdateManager::as_ref(ctx).has_completed_initial_load() {
            return;
        }

        if AISettings::as_ref(ctx)
            .execution_profiles
            .is_value_explicitly_set()
        {
            if ctx.has_singleton_model::<CloudPreferencesSyncer>()
                && !CloudPreferencesSyncer::as_ref(ctx).has_completed_initial_load()
            {
                return;
            }
            self.preserve_profile_onboarding_overrides = true;
            Self::sync_explicit_settings_collection(ctx);
            self.settings_migration_state = SettingsMigrationState::Complete;
            return;
        }
        if Self::cloud_collection_awaiting_reconciliation(ctx) {
            return;
        }

        if *CloudPreferencesSettings::as_ref(ctx)
            .settings_sync_enabled
            .value()
            && Self::cloud_collection_exists(ctx)
        {
            log::error!(
                "Failed to apply cloud execution profiles; recovering from legacy profiles"
            );
        }

        let owned_legacy_profiles = CloudModel::as_ref(ctx)
            .get_all_objects_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>()
            .filter(|profile| Self::is_owned_by_current_user(profile, ctx))
            .collect::<Vec<_>>();
        if owned_legacy_profiles
            .iter()
            .any(|profile| profile.id.into_server().is_none())
        {
            log::info!(
                "Waiting to migrate execution profiles until pending legacy profiles have server IDs"
            );
            return;
        }

        let mut legacy_profiles = owned_legacy_profiles
            .into_iter()
            .map(|profile| {
                let id = if profile.model().string_model.is_default_profile {
                    ExecutionProfileId::default_profile()
                } else {
                    ExecutionProfileId::from_legacy_server_id(
                        profile
                            .id
                            .into_server()
                            .expect("pending legacy profiles were filtered above"),
                    )
                };
                (id, profile.model().string_model.clone())
            })
            .collect::<Vec<_>>();
        legacy_profiles.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));

        let (profiles, preserve_profile_onboarding_overrides) = if legacy_profiles.is_empty() {
            let mut profile = super::create_default_from_legacy_settings(ctx);
            if let Some(base_llm_id) = ctx
                .private_user_preferences()
                .read_value("PreferredAgentModeLLMId")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str::<Option<LLMId>>(&value).ok())
                .flatten()
            {
                profile.base_model = Some(base_llm_id);
            }
            let mut profiles = ExecutionProfilesConfig::default();
            profiles.insert(ExecutionProfileId::default_profile(), profile);
            (profiles, false)
        } else {
            if !legacy_profiles.iter().any(|(id, _)| id.is_default()) {
                legacy_profiles.insert(
                    0,
                    (
                        ExecutionProfileId::default_profile(),
                        super::create_default_from_legacy_settings(ctx),
                    ),
                );
            }
            (
                ExecutionProfilesConfig::from_profiles(IndexMap::from_iter(legacy_profiles))
                    .expect("legacy migration inserts a default profile"),
                true,
            )
        };

        let update_result = AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.execution_profiles.set_value(profiles, ctx)
        });
        match update_result {
            Ok(()) => {
                self.preserve_profile_onboarding_overrides = preserve_profile_onboarding_overrides;
                self.settings_migration_state = SettingsMigrationState::Complete;
                log::info!("Migrated legacy execution profiles to the settings collection");
            }
            Err(error) => {
                report_error!(error.context("Failed to migrate execution profiles to settings"));
            }
        }
    }
    /// Uploads an explicit local collection after the deferred migration check.
    fn sync_explicit_settings_collection(ctx: &mut ModelContext<Self>) {
        if !ctx.has_singleton_model::<CloudPreferencesSyncer>() {
            return;
        }
        CloudPreferencesSyncer::handle(ctx).update(ctx, |syncer, ctx| {
            syncer.maybe_sync_local_prefs_to_cloud(
                vec![ExecutionProfiles::storage_key().to_string()],
                ctx,
            );
        });
    }

    /// Returns whether onboarding must leave the existing default profile unchanged.
    pub fn should_preserve_onboarding_profile(&self, ctx: &AppContext) -> bool {
        if self.settings_are_authoritative() {
            self.preserve_profile_onboarding_overrides
        } else {
            self.default_profile(ctx).sync_id().is_some()
        }
    }

    /// This function performs one-time migrations from legacy settings into the default profile.
    /// The issue this solves is that, whenever we migrate an existing setting into the profile object,
    /// users will initialize the new field to its default value. We need to manually check to see if
    /// the legacy setting hasn't been migrated and, if it hasn't, do a one-time overwrite on the new profile
    /// field.
    fn maybe_inherit_from_legacy_settings(&mut self, ctx: &mut ModelContext<Self>) {
        let DefaultProfileState::Synced {
            id: default_profile_id,
        } = &self.default_profile_state
        else {
            return;
        };
        let default_profile_id = default_profile_id.clone();

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
                .context("Failed to remove old PreferredAgentModeLLMId user pref")
            {
                report_error!(e);
            }
            self.set_base_model(&default_profile_id, Some(base_llm_id.clone()), ctx);
            log::info!("Overwrote default profile with legacy setting for base llm: {base_llm_id}");
        }
    }

    pub fn create_profile(&mut self, ctx: &mut ModelContext<Self>) -> Option<ExecutionProfileId> {
        let profile_id = ExecutionProfileId::new();
        if self.settings_migration_state == SettingsMigrationState::PendingLegacyImport {
            let mut profiles = self.pending_legacy_profiles(ctx);
            let mut new_profile = self.default_profile(ctx).data().clone();
            new_profile.name = String::new();
            new_profile.is_default_profile = false;
            new_profile.autosync_plans_to_warp_drive = true;
            profiles.insert(profile_id.clone(), new_profile);
            if !self.activate_pending_settings_collection(profiles, ctx) {
                return None;
            }
            send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileCreated, ctx);
            return Some(profile_id);
        }

        if self.settings_are_authoritative() {
            let mut profiles = AISettings::as_ref(ctx).execution_profiles.value().clone();
            let mut new_profile = self.default_profile(ctx).data().clone();
            new_profile.name = String::new();
            new_profile.is_default_profile = false;
            new_profile.autosync_plans_to_warp_drive = true;
            profiles.insert(profile_id.clone(), new_profile);
            if let Err(error) = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.execution_profiles.set_value(profiles, ctx)
            }) {
                report_error!(error.context("Failed to create execution profile in settings"));
                return None;
            }
            send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileCreated, ctx);
            return Some(profile_id);
        }

        let Some(owner) = UserWorkspaces::as_ref(ctx).personal_drive(ctx) else {
            report_error!("Failed to create AI execution profile: personal drive not available");
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
            .insert(profile_id.clone(), SyncId::ClientId(client_id));

        send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileCreated, ctx);

        ctx.emit(AIExecutionProfilesModelEvent::ProfileCreated);

        Some(profile_id)
    }

    pub fn delete_profile(
        &mut self,
        profile_id: &ExecutionProfileId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.settings_migration_state == SettingsMigrationState::PendingLegacyImport {
            if profile_id == &self.default_profile_state.id() {
                log::warn!("Attempted to delete default profile (id: {profile_id})");
                return;
            }
            let mut profiles = self.pending_legacy_profiles(ctx);
            if profiles.remove(profile_id).is_none() {
                return;
            }
            if !self.activate_pending_settings_collection(profiles, ctx) {
                return;
            }
            self.active_profiles_per_session
                .retain(|_, active_profile_id| active_profile_id != profile_id);
            send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileDeleted, ctx);
            return;
        }
        if self.settings_are_authoritative() {
            if profile_id.is_default() {
                log::warn!("Attempted to delete default profile (id: {profile_id})");
                return;
            }
            let mut profiles = AISettings::as_ref(ctx).execution_profiles.value().clone();
            if profiles.remove(profile_id).is_none() {
                return;
            }
            if let Err(error) = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.execution_profiles.set_value(profiles, ctx)
            }) {
                report_error!(error.context("Failed to delete execution profile from settings"));
                return;
            }
            self.active_profiles_per_session
                .retain(|_, active_profile_id| active_profile_id != profile_id);
            send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileDeleted, ctx);
            return;
        }

        let id = self.default_profile_state.id();
        if id == *profile_id {
            log::warn!("Attempted to delete default profile (id: {profile_id})");
            return;
        }

        let Some(sync_id) = self.profile_id_to_sync_id.get(profile_id).cloned() else {
            return;
        };

        self.active_profiles_per_session
            .retain(|_, active_profile_id| active_profile_id != profile_id);

        self.profile_id_to_sync_id.remove(profile_id);

        let update_manager = UpdateManager::handle(ctx);
        update_manager.update(ctx, |update_manager, ctx| {
            update_manager.delete_ai_execution_profile(sync_id, ctx);
        });

        send_telemetry_from_ctx!(TelemetryEvent::AIExecutionProfileDeleted, ctx);
        ctx.emit(AIExecutionProfilesModelEvent::ProfileDeleted);
    }

    // On logout, we need to clear any existing profile state.
    pub fn reset(&mut self, settings_profiles_are_explicit: bool) {
        if self.source.is_settings_collection() {
            if self.source.imports_legacy_profiles() {
                self.settings_migration_state =
                    SettingsMigrationState::for_launch(self.source, settings_profiles_are_explicit);
                self.preserve_profile_onboarding_overrides = settings_profiles_are_explicit;
                self.default_profile_state = DefaultProfileState::Unsynced {
                    id: ExecutionProfileId::default_profile(),
                    profile: AIExecutionProfile {
                        name: "Default".to_string(),
                        is_default_profile: true,
                        ..Default::default()
                    },
                };
                self.profile_id_to_sync_id.clear();
            }
            self.active_profiles_per_session.clear();
            return;
        }
        self.default_profile_state = DefaultProfileState::Unsynced {
            id: ExecutionProfileId::new(),
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
            .and_then(|profile_id| self.get_profile_by_id(profile_id, ctx))
            .unwrap_or_else(|| self.default_profile(ctx))
    }

    pub fn default_profile_id(&self) -> ExecutionProfileId {
        if self.settings_are_authoritative() {
            return ExecutionProfileId::default_profile();
        }
        self.default_profile_state.id()
    }

    pub fn default_profile(&self, ctx: &AppContext) -> AIExecutionProfileInfo {
        if self.settings_are_authoritative() {
            let id = ExecutionProfileId::default_profile();
            let data = AISettings::as_ref(ctx)
                .execution_profiles
                .value()
                .profile(&id)
                .cloned()
                .unwrap_or_else(|| {
                    report_error!("Execution profile settings are missing the default profile");
                    AIExecutionProfile {
                        name: "Default".to_string(),
                        is_default_profile: true,
                        ..Default::default()
                    }
                });
            return AIExecutionProfileInfo {
                id,
                sync_id: None,
                data,
            };
        }

        match &self.default_profile_state {
            DefaultProfileState::Unsynced { id, profile } => AIExecutionProfileInfo {
                id: id.clone(),
                sync_id: None,
                data: profile.clone(),
            },
            DefaultProfileState::Synced { id } => {
                let Some(sync_id) = self.profile_id_to_sync_id.get(id) else {
                    report_error!(
                        "Default profile is synced but no sync_id found in profile_id_to_sync_id map."
                    );
                    return AIExecutionProfileInfo {
                        id: id.clone(),
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
                    id: id.clone(),
                    sync_id: Some(*sync_id),
                    data,
                }
            }
            DefaultProfileState::Cli { id, profile } => AIExecutionProfileInfo {
                id: id.clone(),
                sync_id: None,
                data: profile.clone(),
            },
        }
    }

    /// Sets the active profile for a specific terminal view.
    pub fn set_active_profile(
        &mut self,
        terminal_view_id: EntityId,
        profile_id: ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
        ctx: &AppContext,
    ) -> Option<AIExecutionProfileInfo> {
        if self.settings_are_authoritative() {
            return AISettings::as_ref(ctx)
                .execution_profiles
                .value()
                .profile(profile_id)
                .cloned()
                .map(|data| AIExecutionProfileInfo {
                    id: profile_id.clone(),
                    sync_id: None,
                    data,
                });
        }
        // Handle an unsynced default profile (including CLI)
        match &self.default_profile_state {
            DefaultProfileState::Unsynced { id, profile }
            | DefaultProfileState::Cli { id, profile } => {
                if profile_id == id {
                    return Some(AIExecutionProfileInfo {
                        id: id.clone(),
                        sync_id: None,
                        data: profile.clone(),
                    });
                }
            }
            DefaultProfileState::Synced { .. } => {}
        }

        // Handle all synced profiles (default and non-default)
        let sync_id = self.profile_id_to_sync_id.get(profile_id)?;
        let cloud_model = CloudModel::as_ref(ctx);
        let data = cloud_model
            .get_object_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>(sync_id)
            .map(|o| o.model().string_model.clone())
            .unwrap_or_default();

        Some(AIExecutionProfileInfo {
            id: profile_id.clone(),
            sync_id: Some(*sync_id),
            data,
        })
    }

    pub fn get_all_profile_ids(&self) -> Vec<ExecutionProfileId> {
        if self.settings_are_authoritative() {
            return self.last_settings_profiles.profile_ids().cloned().collect();
        }
        let default_profile_id = self.default_profile_state.id();

        // Default profile is always first in the list
        std::iter::once(default_profile_id.clone())
            .chain(
                self.profile_id_to_sync_id
                    .keys()
                    .filter(|id| *id != &default_profile_id)
                    .cloned(),
            )
            .collect()
    }

    /// Resolves a legacy cloud sync ID to the profile key used by the active backend.
    ///
    /// Legacy backends consult their in-memory ID map. Migration-capable settings backends retain
    /// mappings from the current process, then derive deterministic keys for profiles restored on
    /// a later launch. Non-migrating settings backends, such as the TUI, have no legacy sync-ID
    /// mapping.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn get_profile_id_by_sync_id(
        &self,
        sync_id: &SyncId,
        ctx: &AppContext,
    ) -> Option<ExecutionProfileId> {
        if self.settings_are_authoritative() {
            if !self.source.imports_legacy_profiles() {
                return None;
            }
            let profiles = AISettings::as_ref(ctx).execution_profiles.value();
            if let Some(profile_id) =
                self.profile_id_to_sync_id
                    .iter()
                    .find_map(|(profile_id, candidate_sync_id)| {
                        (candidate_sync_id == sync_id).then(|| profile_id.clone())
                    })
                && profiles.profile(&profile_id).is_some()
            {
                return Some(profile_id);
            }
            let server_id = sync_id.into_server()?;
            let legacy_profile = CloudModel::as_ref(ctx)
                .get_object_of_type::<GenericStringObjectId, CloudAIExecutionProfileModel>(sync_id);
            let profile_id = if legacy_profile
                .is_some_and(|profile| profile.model().string_model.is_default_profile)
            {
                ExecutionProfileId::default_profile()
            } else {
                ExecutionProfileId::from_legacy_server_id(server_id)
            };
            return profiles
                .profile(&profile_id)
                .is_some()
                .then_some(profile_id);
        }
        self.profile_id_to_sync_id
            .iter()
            .find_map(|(client_id, id)| {
                if id == sync_id {
                    Some(client_id.clone())
                } else {
                    None
                }
            })
    }

    pub fn has_multiple_profiles(&self) -> bool {
        if self.settings_are_authoritative() {
            return self.last_settings_profiles.profile_ids().nth(1).is_some();
        }
        let default_profile_id = self.default_profile_state.id();

        self.profile_id_to_sync_id
            .keys()
            .any(|id| id != &default_profile_id)
    }

    pub fn set_base_model(
        &mut self,
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
                .unwrap_or_else(|| llm_preferences.get_default_base_model(ctx));
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
        permission: super::RunAgentsPermission,
        ctx: &mut ModelContext<Self>,
    ) {
        let current_value = self
            .get_profile_by_id(&profile_id.clone(), ctx)
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
        profile_id: &ExecutionProfileId,
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

    pub fn set_autosync_plans_to_warp_drive(
        &mut self,
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
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
        profile_id: &ExecutionProfileId,
        edit_fn: impl FnOnce(&mut AIExecutionProfile) -> bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.settings_migration_state == SettingsMigrationState::PendingLegacyImport {
            let mut profiles = self.pending_legacy_profiles(ctx);
            let Some(profile) = profiles.profile_mut(profile_id) else {
                return false;
            };
            if !edit_fn(profile) {
                return false;
            }
            return self.activate_pending_settings_collection(profiles, ctx);
        }
        if self.settings_are_authoritative() {
            let mut profiles = AISettings::as_ref(ctx).execution_profiles.value().clone();
            let Some(profile) = profiles.profile_mut(profile_id) else {
                return false;
            };
            if !edit_fn(profile) {
                return false;
            }
            if let Err(error) = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.execution_profiles.set_value(profiles, ctx)
            }) {
                report_error!(error.context("Failed to persist execution profile settings edit"));
                return false;
            }
            return true;
        }
        // We don't yet support editing the default profile for the CLI.
        if let DefaultProfileState::Cli { id, .. } = &self.default_profile_state
            && id == profile_id
        {
            log::warn!("Attempted to edit CLI default profile, which is not yet supported.");
            return false;
        }

        // Case: this might be an edit to a not-yet-created default profile object. If so, we need to create
        // a cloud object to back the default profile.
        if let DefaultProfileState::Unsynced { id, profile } = &self.default_profile_state
            && id == profile_id
        {
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
                    update_manager.create_ai_execution_profile(new_profile, client_id, owner, ctx);
                });

                // For forever on, the default profile state is synced.
                let sync_id = SyncId::ClientId(client_id);
                self.default_profile_state = DefaultProfileState::Synced {
                    id: profile_id.clone(),
                };
                self.profile_id_to_sync_id
                    .insert(profile_id.clone(), sync_id);

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
                    id: profile_id.clone(),
                    profile: new_profile,
                };

                log::info!(
                    "Updated local unsynced default execution profile (no personal drive yet): {profile_id:?}"
                );
            }
            ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(
                profile_id.clone(),
            ));
            return true;
        }

        let mut value_changed = false;
        if let Some(sync_id) = self.profile_id_to_sync_id.get(profile_id) {
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
                report_error!(
                    "Profile id is mapped but no object found",
                    extra: { "profile_id" => ?profile_id }
                );
            }
        }
        ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(
            profile_id.clone(),
        ));
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
        if let DefaultProfileState::Unsynced { id, .. } = &self.default_profile_state
            && let Some((sync_id, _)) = all_profiles.iter().find(|(_, is_default)| *is_default)
        {
            let id = id.clone();
            self.default_profile_state = DefaultProfileState::Synced { id: id.clone() };
            self.profile_id_to_sync_id.insert(id.clone(), *sync_id);
            log::info!(
                "Reconciled default execution profile with cloud after initial load: \
                     profile_id={id:?}, sync_id={sync_id:?}"
            );
            ctx.emit(AIExecutionProfilesModelEvent::ProfileUpdated(id));
        }

        // Register non-default profiles from cloud that we aren't
        // already tracking so later edits find their backing sync_id.
        let mut added_non_default = false;
        for (sync_id, is_default) in all_profiles {
            if is_default {
                continue;
            }
            if !self.profile_id_to_sync_id.values().any(|s| *s == sync_id) {
                let profile_id = self.source.legacy_profile_id(sync_id, false);
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
            | TemplatableMCPServerManagerEvent::AuthenticationRequired { uuid: _ }
            | TemplatableMCPServerManagerEvent::CredentialsChanged { uuid: _ }
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
            log::warn!(
                "Received ObjectCreated event for AI execution profile but object not found in CloudModel: {sync_id:?}"
            );
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
            if let DefaultProfileState::Unsynced { id, .. } = &self.default_profile_state {
                let id = id.clone();
                self.default_profile_state = DefaultProfileState::Synced { id: id.clone() };
                self.profile_id_to_sync_id.insert(id.clone(), sync_id);
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
            let profile_id = self.source.legacy_profile_id(sync_id, false);
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
                    Some(client_id.clone())
                } else {
                    None
                }
            });

        if let Some(profile_id) = profile_id {
            self.profile_id_to_sync_id.remove(&profile_id);

            // Also remove from active profiles per session
            self.active_profiles_per_session
                .retain(|_, active_id| active_id != &profile_id);

            // If the default profile was deleted, transition back to unsynced state
            let is_default = matches!(&self.default_profile_state, DefaultProfileState::Synced { id } if id == &profile_id);
            if is_default {
                log::warn!(
                    "Default execution profile was deleted from cloud. Transitioning to unsynced state: {sync_id:?}"
                );
                self.default_profile_state = DefaultProfileState::Unsynced {
                    id: profile_id.clone(),
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
        let profile_id = self.get_profile_id_by_sync_id(&sync_id, ctx);

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
                &profile_id,
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

    /// Replaces a temporary client sync ID with the server ID assigned after object creation.
    ///
    /// Migration-capable sources also replace generated profile keys with their deterministic
    /// migrated keys in settings and active/default references.
    pub fn replace_client_id_with_server_id(
        &mut self,
        server_id: SyncId,
        client_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(profile_id) = self
            .profile_id_to_sync_id
            .iter()
            .find_map(|(profile_id, sync_id)| (*sync_id == client_id).then(|| profile_id.clone()))
        else {
            return;
        };
        let is_default_profile = profile_id == self.default_profile_state.id();
        let migrated_profile_id = if self.source.imports_legacy_profiles() {
            self.source.legacy_profile_id(server_id, is_default_profile)
        } else {
            profile_id.clone()
        };

        self.profile_id_to_sync_id.remove(&profile_id);
        self.profile_id_to_sync_id
            .insert(migrated_profile_id.clone(), server_id);
        if migrated_profile_id != profile_id {
            if self.settings_are_authoritative() {
                let mut profiles = AISettings::as_ref(ctx).execution_profiles.value().clone();
                if let Some(profile) = profiles.remove(&profile_id) {
                    profiles.insert(migrated_profile_id.clone(), profile);
                    if let Err(error) = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                        settings.execution_profiles.set_value(profiles, ctx)
                    }) {
                        report_error!(
                            error.context("Failed to re-key migrated execution profile settings")
                        );
                    }
                }
            }
            for active_profile_id in self.active_profiles_per_session.values_mut() {
                if *active_profile_id == profile_id {
                    active_profile_id.clone_from(&migrated_profile_id);
                }
            }
            match &mut self.default_profile_state {
                DefaultProfileState::Unsynced { id, .. }
                | DefaultProfileState::Synced { id }
                | DefaultProfileState::Cli { id, .. }
                    if *id == profile_id =>
                {
                    id.clone_from(&migrated_profile_id);
                }
                DefaultProfileState::Unsynced { .. }
                | DefaultProfileState::Synced { .. }
                | DefaultProfileState::Cli { .. } => {}
            }
        }
        log::info!("Updated profile id mapping after creating a new execution profile");
    }

    /// Replaces the given profile's data with CLI defaults for the given sandboxed state.
    /// Use in tests to simulate the profile configuration used by the sandboxed CLI agent.
    #[cfg(test)]
    pub fn apply_cli_profile_defaults_for_test(
        &mut self,
        profile_id: &ExecutionProfileId,
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
    ProfileUpdated(ExecutionProfileId),
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
