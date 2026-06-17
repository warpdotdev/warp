use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cloud_object_models::JsonSerializer;
use lazy_static::lazy_static;
use settings::{Setting as _, SyncToCloud};
use warp_core::execution_mode::AppExecutionMode;
use warp_core::r#async::debounce;
use warp_core::settings::ChangeEventReason;
use warp_core::user_preferences::GetUserPreferences;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};
use warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences;

use super::cloud_preferences::{CloudPreferencesSettings, CloudPreferencesSettingsChangedEvent};
use super::manager::SettingsEvent;
use super::PrivacySettings;
use crate::auth::auth_state::AuthState;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectEventEntrypoint, GenericStringObjectFormat, JsonObjectType};
use crate::drive::CloudObjectTypeAndId;
use crate::report_if_error;
use crate::server::cloud_objects::update_manager::{
    GenericStringObjectInput, InitiatedBy, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::{ClientId, SyncId};
use crate::server::sync_queue::{SyncQueue, SyncQueueEvent};
use crate::settings::cloud_preferences::{
    CloudPreference, CloudPreferenceModel, Platform, Preference,
};
use crate::settings::manager::SettingsManager;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Provides client ids for creating cloud preferences.
/// We define this as a trait so tests can track what client ids are created and use
/// them for mocking server responses.
pub trait ClientIdProvider {
    fn next_client_id(&self) -> ClientId;
}

struct DefaultClientIdProvider;
impl ClientIdProvider for DefaultClientIdProvider {
    fn next_client_id(&self) -> ClientId {
        ClientId::new()
    }
}

/// Key used to persist the hash of the settings file content as of the
/// last successful cloud sync reconciliation. Used on next startup to
/// detect whether the user made local changes (via file edit or offline
/// UI change) that cloud sync doesn't know about yet.
pub(super) const SETTINGS_FILE_LAST_SYNCED_HASH_KEY: &str = "SettingsFileLastSyncedHash";

/// Constructs the cloud preferences syncer, computing the
/// `force_local_wins_on_startup` flag by comparing the current settings
/// file hash against the last-synced hash stored in private preferences.
///
/// This is the only entry point used to construct the syncer at app
/// startup; production code in `lib.rs` and end-to-end tests both call
/// it so they exercise the same code path.
pub fn initialize_cloud_preferences_syncer(
    toml_file_path: PathBuf,
    startup_toml_parse_error: Option<&str>,
    ctx: &mut ModelContext<CloudPreferencesSyncer>,
) -> CloudPreferencesSyncer {
    let current_hash = TomlBackedUserPreferences::file_content_hash(&toml_file_path);
    let stored_hash = ctx
        .private_user_preferences()
        .read_value(SETTINGS_FILE_LAST_SYNCED_HASH_KEY)
        .unwrap_or_default();

    let file_has_unsynced_changes = match (current_hash, stored_hash) {
        // File present, stored hash present: trust the comparison.
        (Some(current), Some(stored)) => current != stored,
        // File present, no stored hash (first launch, fresh install, or
        // the stored hash was cleared): cloud wins, consistent with
        // today's behavior.
        (Some(_), None) => false,
        // File missing/empty, stored hash present (user deleted or
        // emptied the file): cloud wins. If we treated this as "local
        // differs" we'd upload defaults and wipe the user's cloud
        // settings — exactly what they likely don't want.
        (None, Some(_)) => false,
        // File missing/empty, no stored hash (fresh install with no
        // file yet): cloud wins.
        (None, None) => false,
    };

    // Broken-file guard: when the file can't be parsed, there are no
    // meaningful local values to preserve. Cloud sync restores settings
    // in memory while flush suppression protects the broken file on
    // disk.
    let force_local_wins_on_startup =
        file_has_unsynced_changes && startup_toml_parse_error.is_none();

    CloudPreferencesSyncer::new(force_local_wins_on_startup, toml_file_path, ctx)
}

/// Handles syncing CloudPreferences (the Warp Drive objects) and local Settings models that
/// have been created using the define_settings_group macro.
pub struct CloudPreferencesSyncer {
    // A channel used for debouncing local settings updates so that we don't spam the
    // server with requests.  Most important for settings that continuously update
    // like ones that are driven by sliders.
    #[allow(dead_code)]
    update_tx: async_channel::Sender<()>,

    // Local prefs awaiting syncing to the cloud after a debounce period.
    dirty_local_prefs: HashSet<String>,

    // Provides the next ClientId to use in creating cloud preferences.
    client_id_provider: Arc<dyn ClientIdProvider>,

    has_completed_initial_load: bool,

    /// When `true`, the first `handle_initial_load` will force local
    /// values to be uploaded to cloud rather than accepting cloud
    /// values — equivalent to `ForceCloudToMatchLocal::Yes`. Only
    /// consulted on the first initial load; subsequent `sync()` calls
    /// use their own flag.
    force_local_wins_on_startup: bool,

    /// Path to the user's `settings.toml` file, used by
    /// `update_stored_settings_hash` to compute the hash persisted
    /// after every successful cloud sync reconciliation.
    toml_file_path: PathBuf,
}

/// Event fired by the CloudPreferencesSyncer when a cloud preference has changed.
#[derive(Debug)]
pub enum CloudPreferencesSyncerEvent {
    /// Emitted when the local preferences are updated to match values from cloud upon initial load.
    InitialLoadCompleted,

    /// Event variant indicating there's a new value for the preference with
    /// a specific storage key
    Updated { key: String, value: String },
}

/// Whether to force the cloud to match the local settings or not.
/// Used to force a resync of the cloud state to the current local state when a
/// user manually re-enables settings sync.
#[derive(Debug)]
pub enum ForceCloudToMatchLocal {
    Yes,
    No,
}

struct PreferenceToCreate {
    value: String,
    syncing_mode: SyncToCloud,
}

lazy_static! {
    static ref LEGACY_CLOUD_SETTINGS_STORAGE_KEYS: Vec<&'static str> = vec![
        super::privacy::TELEMETRY_ENABLED_DEFAULTS_KEY,
        super::privacy::CRASH_REPORTING_ENABLED_DEFAULTS_KEY,
        super::privacy::CLOUD_CONVERSATION_STORAGE_ENABLED_DEFAULTS_KEY,
    ];
}

const PREFERENCES_DEBOUNCE_PERIOD: Duration = Duration::from_millis(500);

impl CloudPreferencesSyncer {
    // Retry preferences every five minutes until they are successfully synced.
    // Only enabled for users in the warp drive preferences experiment.
    const RETRY_POLL: Duration = Duration::from_secs(60 * 5);

    #[cfg(test)]
    pub fn new_for_test(
        ctx: &mut ModelContext<Self>,
        client_id_provider: Arc<dyn ClientIdProvider>,
    ) -> Self {
        // Existing tests do not exercise the hash-update path; the
        // default empty path causes `file_content_hash` to return
        // `None` and `update_stored_settings_hash` becomes a no-op.
        // End-to-end tests that care about the hash path construct
        // the syncer via `initialize_cloud_preferences_syncer`.
        Self::new_internal(ctx, client_id_provider, PathBuf::new())
    }

    pub fn new(
        force_local_wins_on_startup: bool,
        toml_file_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut me = Self::new_internal(ctx, Arc::new(DefaultClientIdProvider), toml_file_path);
        me.force_local_wins_on_startup = force_local_wins_on_startup;
        me.retry_failed_settings(ctx);
        me
    }

    fn new_internal(
        ctx: &mut ModelContext<Self>,
        client_id_provider: Arc<dyn ClientIdProvider>,
        toml_file_path: PathBuf,
    ) -> Self {
        // Set up event syncing in both directions (local -> cloud and cloud -> local).
        // We only apply cloud->local updates AFTER the initial load has been processed by
        // handle_initial_load. This prevents the CloudPreferencesUpdated event (which fires
        // synchronously in on_changed_objects_fetched) from overwriting local settings
        // before handle_initial_load has a chance to determine sync direction.
        ctx.subscribe_to_model(&UpdateManager::handle(ctx), |syncer, event, ctx| {
            if let UpdateManagerEvent::CloudPreferencesUpdated { updated } = event {
                // Defer cloud→local updates until `handle_initial_load`
                // has determined the correct sync direction. The
                // `CloudPreferencesUpdated` event fires synchronously
                // during `on_changed_objects_fetched` and would
                // overwrite local settings before `handle_initial_load`
                // gets a chance to decide whether local or cloud wins.
                if !syncer.has_completed_initial_load {
                    return;
                }
                for preference in updated {
                    syncer.maybe_sync_cloud_pref_to_local(&preference.storage_key, ctx);
                }
            }
        });
        let (update_tx, update_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(
            debounce(PREFERENCES_DEBOUNCE_PERIOD, update_rx),
            |me, _, ctx| {
                let prefs_to_sync = me.dirty_local_prefs.drain().collect();
                me.maybe_sync_local_prefs_to_cloud(prefs_to_sync, ctx);
            },
            |_, _| {},
        );
        ctx.subscribe_to_model(
            &SettingsManager::handle(ctx),
            |me, event, ctx| match event {
                SettingsEvent::LocalPreferencesUpdated { storage_key, .. } => {
                    me.handle_local_preference_updated(storage_key, ctx);
                }
            },
        );
        // Update the stored settings file hash whenever a preference is
        // successfully created or updated on the server. This ensures the
        // hash only moves forward when the cloud has actually accepted
        // local changes — if the upload fails (e.g. offline), the hash
        // stays stale and the next startup will correctly detect
        // divergence.
        ctx.subscribe_to_model(&SyncQueue::handle(ctx), Self::handle_sync_queue_event);
        ctx.subscribe_to_model(
            &CloudPreferencesSettings::handle(ctx),
            |me, event, ctx| match event {
                CloudPreferencesSettingsChangedEvent::IsSettingsSyncEnabled {
                    change_event_reason,
                } => {
                    let force_cloud_to_match_local = match change_event_reason {
                        ChangeEventReason::CloudSync => ForceCloudToMatchLocal::No,
                        ChangeEventReason::LocalChange => ForceCloudToMatchLocal::Yes,
                        ChangeEventReason::Clear => {
                            log::info!(
                                "Not resyncing cloud preferences because the setting was cleared \
                                (typically on logout)"
                            );
                            return;
                        }
                    };
                    log::info!(
                        "Settings sync enabled setting changed. Resyncing cloud preferences. Force \
                        cloud to match local: {force_cloud_to_match_local:?}"
                    );
                    // Always resync from the local client when the setting changes,
                    // but only force cloud to match this client's local settings if the change in the setting
                    // was initiated in this client.
                    me.sync(force_cloud_to_match_local, ctx);
                }
            },
        );

        Self {
            update_tx,
            dirty_local_prefs: HashSet::new(),
            client_id_provider,
            has_completed_initial_load: false,
            force_local_wins_on_startup: false,
            toml_file_path,
        }
    }

    /// Handles SyncQueue success events by updating the stored
    /// settings file hash when a cloud preference is successfully
    /// created or updated on the server.
    fn handle_sync_queue_event(&mut self, event: &SyncQueueEvent, ctx: &mut ModelContext<Self>) {
        let server_id = match event {
            SyncQueueEvent::ObjectCreationSuccessful {
                server_creation_info,
                ..
            } => Some(server_creation_info.server_id_and_type.id),
            SyncQueueEvent::ObjectUpdateSuccessful { server_id, .. } => Some(*server_id),
            _ => None,
        };
        if let Some(server_id) = server_id {
            // Check whether this object is a cloud preference.
            // GenericStringObject is a superset that also includes
            // env var collections, workflow enums, MCP servers, etc.
            // Only preference changes should update the stored hash.
            let sync_id = SyncId::ServerId(server_id);
            let is_preference = CloudModel::as_ref(ctx)
                .get_all_cloud_preferences_by_storage_key()
                .values()
                .any(|pref| pref.id == sync_id);
            if is_preference {
                self.update_stored_settings_hash(ctx);
            }
        }
    }

    /// Reads the current settings file hash from disk and persists it
    /// as the last-synced hash in private preferences. Called at every
    /// sync reconciliation point so that on the next startup, the
    /// stored hash accurately reflects what the cloud last saw.
    ///
    /// This is a no-op when the file is missing, empty, or unreadable
    /// (`file_content_hash` returns `None`).
    fn update_stored_settings_hash(&self, ctx: &mut ModelContext<Self>) {
        let Some(hash) = TomlBackedUserPreferences::file_content_hash(&self.toml_file_path) else {
            return;
        };
        if let Err(err) = ctx
            .private_user_preferences()
            .write_value(SETTINGS_FILE_LAST_SYNCED_HASH_KEY, hash)
        {
            log::warn!("Failed to persist settings file hash after sync: {err}");
        }
    }

    #[cfg(not(test))]
    fn handle_local_preference_updated(&mut self, storage_key: &str, _: &mut ModelContext<Self>) {
        self.dirty_local_prefs.insert(storage_key.to_string());
        let _ = self.update_tx.try_send(());
    }

    #[cfg(test)]
    fn handle_local_preference_updated(&mut self, storage_key: &str, ctx: &mut ModelContext<Self>) {
        // Don't debounce in tests - they have enough async stuff going
        self.maybe_sync_local_prefs_to_cloud(vec![storage_key.to_string()], ctx);
    }

    /// This method recursively calls itself after a delay. Call it once and only once to start the
    /// loop. It ensures failed preferences are retried until they are successfully synced.
    fn retry_failed_settings(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async {
                Timer::after(Self::RETRY_POLL).await;
            },
            |me, _, ctx| {
                let ids_to_retry = CloudModel::handle(ctx).update(ctx, |cloud_model, _ctx| {
                    cloud_model
                        .cloud_objects()
                        .filter_map(move |object| {
                            if !object.metadata().is_errored() {
                                return None;
                            }

                            let settings_object: Option<&CloudPreference> = object.into();
                            settings_object.map(|object| object.id)
                        })
                        .collect::<Vec<_>>()
                });
                if !ids_to_retry.is_empty() {
                    log::info!(
                        "Retrying {} failed preference objects...",
                        ids_to_retry.len()
                    );
                }
                for sync_id in ids_to_retry {
                    log::debug!("Retrying failed preference object with sync_id {sync_id:?}");
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.resync_object(
                            &CloudObjectTypeAndId::GenericStringObject {
                                object_type: GenericStringObjectFormat::Json(
                                    JsonObjectType::Preference,
                                ),
                                id: sync_id,
                            },
                            ctx,
                        );
                    });
                }
                me.retry_failed_settings(ctx);
            },
        );
    }

    /// Handler for when the user has been fetched. Potentially kicks off a sync.
    pub fn handle_user_fetched(
        &mut self,
        auth_state: Arc<AuthState>,
        ctx: &mut ModelContext<Self>,
    ) {
        let is_onboarded = auth_state.is_onboarded();

        // Reset the initial load flag so that we re-evaluate sync direction
        // based on the new user's fresh cloud data rather than stale data from
        // a previous session (e.g. anonymous user's cloud prefs).
        self.has_completed_initial_load = false;

        // The startup hash-based override was computed for the app launch
        // and consumed on the first initial load. Clear it so it doesn't
        // re-trigger for the new user session.
        self.force_local_wins_on_startup = false;

        if is_onboarded == Some(false) {
            log::info!("Opting first-time user into cloud preferences");
            CloudPreferencesSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings.settings_sync_enabled.set_value(true, ctx));
            });
        } else {
            log::info!("Not opting existing user into cloud preferences");
        }

        // Always trigger a sync explicitly. For not-yet-onboarded users the
        // set_value call above may be a no-op if settings_sync_enabled was
        // already true (e.g. from a prior anonymous session), so no change
        // event would fire. We need the sync to happen regardless so that
        // handle_initial_load can examine the *new* user's cloud state and
        // decide whether to preserve local settings (brand-new user, no cloud
        // prefs) or apply cloud settings (existing user with cloud prefs).
        self.sync(ForceCloudToMatchLocal::No, ctx);
    }

    /// Performs a settings sync. Checks internally to confirm that the correct settings are
    /// synced based on whether the user has opted in to settings sync.
    ///
    /// Specifically, this call spawns a future waiting for cloud preferences to load and then
    /// 1) If no cloud preferences exist yet, creates and stores them from the local prefs.
    /// 2) If cloud prefs do exist, they are merged into the local preferences, with the cloud
    ///    value overwriting any local values for the same keys. This is only true if force_cloud_to_match_local
    ///    is false. If force_cloud_to_match_local is true, then the local values will overwrite the cloud value.
    ///    This is the behavior we want when a user is enabling settings sync manually in the UI -
    ///    we should disregard any potentially stale cloud values and overwrite them with the current
    ///    local settings.
    pub fn sync(
        &self,
        force_cloud_to_match_local: ForceCloudToMatchLocal,
        ctx: &mut ModelContext<Self>,
    ) {
        let update_manager = UpdateManager::as_ref(ctx);

        // We wait for the cloud objects to load because we need to know if there are any cloud preferences
        // to sync.
        ctx.spawn(update_manager.initial_load_complete(), move |me, _, ctx| {
            me.handle_initial_load(force_cloud_to_match_local, ctx);
        });

        PrivacySettings::handle(ctx).update(ctx, |privacy_settings, ctx| {
            // Note that this also blocks on update_manager.initial_load_complete()
            privacy_settings.maybe_sync_with_warp_drive_prefs(ctx);
        });
    }

    /// Fixes https://linear.app/warpdotdev/issue/CLD-2629/duplicate-prefs-for-users
    fn ensure_no_duplicate_cloud_prefs(&mut self, ctx: &mut ModelContext<Self>) {
        log::info!("Ensuring no duplicate cloud prefs");
        let ids_to_delete = CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            let cloud_prefs = cloud_model
                .get_all_objects_of_type::<GenericStringObjectId, CloudPreferenceModel>();

            // First group all the cloud prefs by storage key.
            let mut prefs_by_storage_key = HashMap::new();
            for pref in cloud_prefs {
                let storage_key = pref.model().string_model.storage_key.clone();
                prefs_by_storage_key
                    .entry(storage_key)
                    .or_insert_with(Vec::new)
                    .push(pref);
            }

            // Then, for any storage key that has multiple prefs (which is an error introduced in the linked issue
            // mentioned in the function comment), we delete all but whichever one is set as the current pref value on this client.
            // If none of the prefs have the current value, we delete all of them, and the local value will end
            // up being the value of the preference when we sync.
           let pref_ids = prefs_by_storage_key
                .iter()
                .filter_map(|(storage_key, prefs)| {
                    if prefs.len() == 1 {
                        return None;
                    }

                    let current_value = SettingsManager::as_ref(ctx)
                        .read_local_setting_value(storage_key, ctx)
                        .unwrap_or_default()
                        .unwrap_or_default();

                    let current_pref_id = prefs
                        .iter()
                        .find(|pref| {
                            let pref_value = pref.model().string_model.value.to_string();
                            pref_value == current_value
                        })
                        .map(|pref| pref.id);
                    log::debug!(
                        "Cleaning up duplicate prefs for storage key {storage_key} and current pref value: {current_value:?}"
                    );
                    let pref_ids = prefs.iter().filter_map(|pref| {
                        let should_delete = current_pref_id != Some(pref.id);
                        if should_delete {
                            log::debug!(
                                "Deleting duplicate pref with id {} for storage key {} with value {}",
                                pref.id,
                                storage_key,
                                pref.model().string_model.value
                            );
                            Some(pref.id)
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>();
                    Some(pref_ids)
                }) .collect::<Vec<_>>();
                pref_ids.iter().flatten().cloned().collect::<Vec<_>>()
        });

        for pref_id in ids_to_delete {
            UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                update_manager.delete_object_with_initiated_by(
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::Preference),
                        id: pref_id,
                    },
                    InitiatedBy::System,
                    ctx,
                );
            });
        }
    }

    fn handle_initial_load(
        &mut self,
        force_cloud_to_match_local: ForceCloudToMatchLocal,
        ctx: &mut ModelContext<Self>,
    ) {
        self.ensure_no_duplicate_cloud_prefs(ctx);

        // First-load override: if the startup hash check detected
        // local changes that cloud sync doesn't know about, force
        // local to win on this one call. The `!has_completed_initial_load`
        // guard makes this a genuine one-time override — robust
        // against any future code path that calls `sync()` again.
        let force_cloud_to_match_local =
            if !self.has_completed_initial_load && self.force_local_wins_on_startup {
                log::info!(
                    "Local has unsynced changes on startup; forcing cloud to match local \
                     on initial load"
                );
                ForceCloudToMatchLocal::Yes
            } else {
                force_cloud_to_match_local
            };

        log::info!(
            "Initial load complete, syncing cloud preferences. \
            Force cloud to match local: {force_cloud_to_match_local:?}"
        );

        // These are the preferences that the cloud model currently knows about (i.e. the
        // preferences that have been synced to the cloud)
        let prefs_in_cloud_model = CloudModel::as_ref(ctx)
            .get_all_cloud_preferences_by_storage_key()
            .keys()
            .cloned()
            .collect::<HashSet<_>>();

        // Keys to sync is a list of all of the preferences that *should* be "cloud synced"
        // We identify these preferences by their storage key (which is the unique key they are
        // stored under in the local preferences store)
        //
        // Sort so that settings with `RespectUserSyncSetting::No` (i.e.
        // settings that sync regardless of the user's sync-enabled
        // toggle) are processed first. This ensures that
        // `IsSettingsSyncEnabled` is restored from cloud before other
        // settings check `settings_sync_enabled`. Without this,
        // HashMap iteration order could cause the sync-enabled flag to
        // still be at its default (false) when other settings are
        // processed, silently skipping them.
        let settings_manager = SettingsManager::as_ref(ctx);
        let mut keys_to_sync = settings_manager
            .all_storage_keys()
            .cloned()
            .collect::<Vec<_>>();
        keys_to_sync.sort_by_key(|key| {
            if settings_manager.sync_regardless_of_users_syncing_setting(key) {
                0
            } else {
                1
            }
        });

        let mut keys_to_sync_to_cloud = Vec::new();
        for storage_key in keys_to_sync {
            if prefs_in_cloud_model.contains(&storage_key)
                && matches!(force_cloud_to_match_local, ForceCloudToMatchLocal::No)
            {
                // Update local pref to match cloud pref unless we are doing a forced preferences sync.
                self.maybe_sync_cloud_pref_to_local(&storage_key, ctx)
            } else if !LEGACY_CLOUD_SETTINGS_STORAGE_KEYS.contains(&storage_key.as_str()) {
                // For all settings except legacy cloud-synced settings, we sync them immediately to warp drive on
                // initial load.
                keys_to_sync_to_cloud.push(storage_key);
            } else {
                // This is one of the two legacy settings stored in the user_settings table and
                // it has not yet been saved to warp drive. In this case we want to wait for
                // these settings to load from the server, and then sync them to warp drive.
                // The logic for this is in privacy.rs.
                log::info!(
                    "Waiting to sync legacy cloud preference with storage key {storage_key} until it is explicitly set"
                );
            }
        }
        // Create a new cloud setting with the local value.
        self.maybe_sync_local_prefs_to_cloud(keys_to_sync_to_cloud, ctx);

        if !self.has_completed_initial_load {
            self.has_completed_initial_load = true;
            ctx.emit(CloudPreferencesSyncerEvent::InitialLoadCompleted);
        }

        // Reconciliation is complete (or a no-op). Persist the current
        // file hash so the next startup can detect further divergence.
        self.update_stored_settings_hash(ctx);
    }

    /// Syncs the local preferences with the given storage keys to the cloud.
    /// For each storage key, if there is an existing cloud preference for that key, it updates it.
    /// Otherwise, it creates a new one. All creations happen in a single bulk request.
    pub(crate) fn maybe_sync_local_prefs_to_cloud(
        &mut self,
        keys_to_sync: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !AppExecutionMode::as_ref(ctx).can_sync_preferences() {
            // Early exit if the app can't sync preferences.
            return;
        }

        let mut cloud_prefs_to_create = HashMap::new();
        let cloud_prefs_by_storage_key = CloudModel::as_ref(ctx)
            .get_all_cloud_preferences_by_storage_key()
            .iter()
            .map(|(storage_key, cloud_pref)| (storage_key.clone(), (*cloud_pref).clone()))
            .collect::<HashMap<_, _>>();
        let settings_sync_enabled = *CloudPreferencesSettings::as_ref(ctx)
            .settings_sync_enabled
            .value();
        for storage_key in &keys_to_sync {
            let settings_manager = SettingsManager::as_ref(ctx);
            if !settings_sync_enabled
                && !settings_manager.sync_regardless_of_users_syncing_setting(storage_key)
            {
                // Skip syncing if settings sync is disabled and this particular cloud pref is not always synced.
                log::debug!("Not syncing cloud preference with storage key {storage_key} because settings sync is disabled for it");
                continue;
            }

            let syncing_mode = settings_manager
                .cloud_syncing_mode_for_storage_key(storage_key)
                .unwrap_or(SyncToCloud::Never);
            if syncing_mode == SyncToCloud::Never {
                // Skip non-cloud-synced prefs
                continue;
            }

            let is_current_value_syncable = settings_manager
                .is_current_value_syncable(storage_key, ctx)
                .unwrap_or(false);
            if !is_current_value_syncable {
                // Don't sync this preference if the current value is not syncable.
                log::debug!("Not syncing cloud preference with storage key {storage_key} because the current value is not syncable");
                continue;
            }

            let Ok(Some(local_value)) = settings_manager.read_local_setting_value(storage_key, ctx)
            else {
                log::debug!(
                    "No local value set for preference with storage key {storage_key}. Skipping cloud sync."
                );
                continue;
            };

            let Some(supported_platforms) =
                SettingsManager::as_ref(ctx).supported_platforms_for_storage_key(storage_key)
            else {
                log::warn!(
                    "No supported platforms found for preference with storage key {storage_key}. Skipping cloud sync."
                );
                continue;
            };

            if !supported_platforms.matches_current_platform() {
                log::debug!(
                    "Preference with storage key {storage_key} is not supported on the current platform. Skipping cloud sync."
                );
                continue;
            }

            if let Some(cloud_pref) = cloud_prefs_by_storage_key.get(storage_key) {
                self.maybe_update_cloud_pref_to_match_local(
                    storage_key,
                    syncing_mode,
                    cloud_pref,
                    &local_value,
                    ctx,
                );
            } else {
                cloud_prefs_to_create.insert(
                    storage_key.clone(),
                    PreferenceToCreate {
                        value: local_value,
                        syncing_mode,
                    },
                );
            }
        }
        self.bulk_create_cloud_prefs_from_local(cloud_prefs_to_create, ctx);
    }

    fn maybe_update_cloud_pref_to_match_local(
        &self,
        storage_key: &str,
        syncing_mode: SyncToCloud,
        cloud_pref: &CloudPreference,
        local_value: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        // Preference has already been synced to the cloud, so update it to the new value if it's different
        // than the current value.
        let cloud_value = &cloud_pref.model().string_model.value.to_string();
        let settings_manager = SettingsManager::as_ref(ctx);

        let local_and_cloud_values_are_equal =
            match settings_manager.are_equal_settings(storage_key, local_value, cloud_value) {
                Ok(equal) => equal,
                Err(e) => {
                    log::warn!(
                        "Error {e} comparing local value {local_value} for cloud preference with \
                        storage key {storage_key}",
                    );
                    return;
                }
            };
        let model_revision_and_id = if local_and_cloud_values_are_equal {
            None
        } else {
            // Create a new instance of the cloud model with the new preference.
            let mut model = cloud_pref.model().clone();
            match Preference::new(storage_key.to_owned(), local_value, syncing_mode) {
                Ok(updated_pref) => model.string_model = updated_pref,
                Err(e) => {
                    log::warn!(
                        "Error updating cloud preference with storage key {storage_key} from \
                        local value {local_value}: {e}"
                    );
                }
            }
            let revision = CloudModel::as_ref(ctx)
                .current_revision(&cloud_pref.id)
                .cloned();
            Some((model, revision, cloud_pref.id))
        };

        if let Some((model, revision, id)) = model_revision_and_id {
            // Save the update.
            UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                log::info!(
                    "Updating cloud preference with storage key {storage_key} to value \
                    {local_value}"
                );
                update_manager.update_object(model, id, revision, ctx);
            });
        }
    }

    fn bulk_create_cloud_prefs_from_local(
        &self,
        cloud_prefs_to_create: HashMap<String, PreferenceToCreate>,
        ctx: &mut ModelContext<Self>,
    ) {
        let inputs = cloud_prefs_to_create
            .into_iter()
            .filter_map(|(storage_key, preference_to_create)| {
                // Create a new instance of the cloud model with the new preference.
                match Preference::new(
                    storage_key.to_owned(),
                    &preference_to_create.value,
                    preference_to_create.syncing_mode,
                ) {
                    Ok(new_pref) => Some(GenericStringObjectInput::<Preference, JsonSerializer> {
                        id: self.client_id_provider.next_client_id(),
                        model: CloudPreferenceModel::new(new_pref),
                        initial_folder_id: None,
                        entrypoint: CloudObjectEventEntrypoint::Unknown,
                    }),
                    Err(e) => {
                        log::warn!("Error {e} creating cloud preference with {storage_key}");
                        None
                    }
                }
            })
            .collect::<Vec<_>>();

        let Some(personal_drive) = UserWorkspaces::as_ref(ctx).personal_drive(ctx) else {
            log::warn!("Unable to create cloud preferences due to unset personal drive");
            return;
        };

        // Preferences don't yet exist in the cloud, so create them.
        // Note that there is a potential race condition here with the same storage key being created
        // on different clients at the same time. The server handles this and will only accept the first
        // create request for each storage key.
        if !inputs.is_empty() {
            log::debug!(
                "Bulk creating {} generic string objects with storage keys {:?}",
                inputs.len(),
                inputs
                    .iter()
                    .map(|input| input.model.string_model.storage_key.clone())
                    .collect::<Vec<_>>()
            );
            UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                update_manager.bulk_create_generic_string_objects(personal_drive, inputs, ctx);
            });
        }
    }

    // Syncs the given cloud pref to local, if cloud syncing is enabled for the pref on this client.
    // Returns early if the pref with the given storage key isn't actually synced to the cloud.
    fn maybe_sync_cloud_pref_to_local(&self, storage_key: &str, ctx: &mut ModelContext<Self>) {
        let Some(model) = CloudModel::as_ref(ctx)
            .get_all_cloud_preferences_by_storage_key()
            .get(storage_key)
            .filter(|object| !object.metadata.pending_changes_statuses.pending_delete)
            .map(|cloud_pref| cloud_pref.model().clone())
        else {
            // No cloud pref to sync
            return;
        };
        let settings_sync_enabled = *CloudPreferencesSettings::as_ref(ctx).settings_sync_enabled;
        SettingsManager::handle(ctx).update(ctx, |manager, ctx| {
            let always_sync = manager.sync_regardless_of_users_syncing_setting(storage_key);
            if !settings_sync_enabled && !always_sync {
                // Early exit in the case where settings sync is disabled, unless this particular cloud pref is always synced.
                return;
            }

            let syncing_mode = manager
                .cloud_syncing_mode_for_storage_key(storage_key)
                .unwrap_or(SyncToCloud::Never);
            if syncing_mode == SyncToCloud::Never {
                // Early exit if this isn't a cloud synced key. Could happen if the current client
                // is on a different build from another one which happened to sync this key. We
                // always honor the syncability setting on the current client.
                return;
            }

            let Some(supported_platforms) =
                manager.supported_platforms_for_storage_key(storage_key)
            else {
                log::warn!(
                    "No supported platforms for storage key {storage_key}. Not updating local \
                    pref to match cloud pref"
                );
                return;
            };

            if !supported_platforms.matches_current_platform() {
                log::debug!(
                    "Preference with storage key {storage_key} is not supported on the current \
                    platform. Not updating local pref to match cloud pref"
                );
                return;
            }

            let platform = model.string_model.platform;
            if matches!(syncing_mode, SyncToCloud::PerPlatform(_))
                && !platform.applies_to_current_platform()
            {
                log::debug!(
                    "Not applying platform-specific preference for {platform:?} with storage key \
                    {storage_key} on current platform {:?}",
                    Platform::current_platform()
                );
                return;
            }

            let is_current_value_syncable = manager
                .is_current_value_syncable(storage_key, ctx)
                .unwrap_or(false);
            if !is_current_value_syncable {
                log::info!(
                    "Not syncing cloud preference with storage key {storage_key} to local because \
                    the current value is not syncable and we don't want to overwrite it with a \
                    cloud value"
                );
                return;
            }

            let value = &model.string_model.value;
            let value_str = value.to_string();

            // Get current local value to compare if it's changing
            let current_value = manager
                .read_local_setting_value(storage_key, ctx)
                .ok()
                .flatten();
            let is_changing = current_value
                .as_ref()
                .and_then(|current| {
                    manager
                        .are_equal_settings(storage_key, current, &value_str)
                        .ok()
                })
                .map(|are_equal| !are_equal)
                .unwrap_or(true); // If we can't determine, assume it's changing

            if is_changing {
                log::info!(
                    "Updating local preference with storage key {storage_key} and value {value} to \
                    match cloud preference"
                );

                if let Err(e) = manager.update_setting_with_storage_key(
                    storage_key,
                    value.to_string(),
                    true, /* from_cloud_sync */
                    ctx,
                ) {
                    log::warn!(
                        "Error updating setting with storage key {storage_key} while merging cloud \
                        prefs to local: {e}"
                    );
                }
            } else {
                log::debug!(
                    "Local preference with storage key {storage_key} already matches cloud value \
                    {value}"
                );
            }
        })
    }
}

impl Entity for CloudPreferencesSyncer {
    type Event = CloudPreferencesSyncerEvent;
}

/// Mark CloudPreferencesSyncer as global application state.
impl SingletonEntity for CloudPreferencesSyncer {}

#[cfg(test)]
#[path = "cloud_preferences_syncer_tests.rs"]
mod tests;
