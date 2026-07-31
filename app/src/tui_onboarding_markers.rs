use std::sync::Arc;
use std::time::Duration;

use cloud_object_models::JsonSerializer;
use settings::{RespectUserSyncSetting, SyncToCloud};
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::cloud_object::CloudObjectEventEntrypoint;
use crate::cloud_object::model::persistence::CloudModel;
use crate::server::cloud_objects::update_manager::{
    GenericStringObjectInput, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::ClientId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::tui_onboarding::{
    TuiOnboardingClient, TuiOnboardingMarkersSnapshot,
};
use crate::settings::cloud_preferences::{CloudPreference, CloudPreferenceModel, Preference};
use crate::workspaces::user_workspaces::UserWorkspaces;

const FIRST_ZERO_STATE_STORAGE_KEY: &str = "TuiFirstZeroStateShown";
const FIRST_CREDIT_GATE_STORAGE_KEY: &str = "TuiFirstCreditGateShown";
const MARKER_LOAD_TIMEOUT: Duration = Duration::from_secs(3);

/// Account-scoped, monotonic TUI onboarding markers stored as separate global
/// cloud preferences so concurrent devices cannot overwrite unrelated state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOnboardingMarker {
    FirstZeroState,
    FirstCreditGate,
}

impl TuiOnboardingMarker {
    fn storage_key(self) -> &'static str {
        match self {
            Self::FirstZeroState => FIRST_ZERO_STATE_STORAGE_KEY,
            Self::FirstCreditGate => FIRST_CREDIT_GATE_STORAGE_KEY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiOnboardingMarkersState {
    Loading,
    Ready {
        first_zero_state_available: bool,
        first_credit_gate_available: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MarkerPersistenceAction {
    None,
    Create,
    Update,
}

fn marker_persistence_action(existing_value: Option<bool>) -> MarkerPersistenceAction {
    match existing_value {
        Some(true) => MarkerPersistenceAction::None,
        Some(false) => MarkerPersistenceAction::Update,
        None => MarkerPersistenceAction::Create,
    }
}

/// Events emitted as the current account's marker snapshot is invalidated and resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOnboardingMarkersEvent {
    Loading,
    Ready,
}

/// Dedicated cloud-preference model for the TUI's once-per-account surfaces.
pub struct TuiOnboardingMarkers {
    state: TuiOnboardingMarkersState,
    load_generation: u64,
    persist_markers: bool,
    onboarding_client: Option<Arc<dyn TuiOnboardingClient>>,
    load_timeout: Duration,
}

impl TuiOnboardingMarkers {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let onboarding_client = ServerApiProvider::as_ref(ctx).get_tui_onboarding_client();
        ctx.subscribe_to_model(&UpdateManager::handle(ctx), |markers, _, event, ctx| {
            let UpdateManagerEvent::CloudPreferencesUpdated { updated } = event else {
                return;
            };
            let mut changed = false;
            for preference in updated {
                if preference.value.as_bool() == Some(true) {
                    changed |= markers.mark_consumed_by_storage_key(&preference.storage_key);
                }
            }
            if changed {
                ctx.notify();
            }
        });
        Self {
            state: TuiOnboardingMarkersState::Loading,
            load_generation: 0,
            persist_markers: true,
            onboarding_client: Some(onboarding_client),
            load_timeout: MARKER_LOAD_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn new_loading_for_test(
        onboarding_client: Arc<dyn TuiOnboardingClient>,
        load_timeout: Duration,
    ) -> Self {
        Self {
            state: TuiOnboardingMarkersState::Loading,
            load_generation: 0,
            persist_markers: false,
            onboarding_client: Some(onboarding_client),
            load_timeout,
        }
    }

    /// Starts a fresh, account-scoped load. Terminal creation never waits for
    /// this request; consumers reconcile provisional one-time UI on
    /// [`TuiOnboardingMarkersEvent::Ready`].
    pub fn load_current_account(&mut self, ctx: &mut ModelContext<Self>) {
        self.state = TuiOnboardingMarkersState::Loading;
        self.load_generation = self.load_generation.wrapping_add(1);
        ctx.emit(TuiOnboardingMarkersEvent::Loading);
        let load_generation = self.load_generation;
        let onboarding_client = self
            .onboarding_client
            .clone()
            .expect("production TUI onboarding marker model must have an API client");
        let load_timeout = self.load_timeout;
        ctx.spawn(
            load_markers_with_timeout(onboarding_client, load_timeout),
            move |markers, result, ctx| {
                markers.handle_load_result(load_generation, result, ctx);
            },
        );
        ctx.notify();
    }

    /// Invalidates the previous account snapshot before a signed-out terminal
    /// can be created for the next browser authentication flow.
    pub fn reset_for_account_transition(&mut self, ctx: &mut ModelContext<Self>) {
        self.state = TuiOnboardingMarkersState::Loading;
        self.load_generation = self.load_generation.wrapping_add(1);
        ctx.emit(TuiOnboardingMarkersEvent::Loading);
        ctx.notify();
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, TuiOnboardingMarkersState::Ready { .. })
    }

    /// Consumes a marker immediately in memory, then queues the monotonic
    /// cloud write. Returning `false` means the marker was loading or had
    /// already been consumed in this process/account snapshot.
    pub fn consume(&mut self, marker: TuiOnboardingMarker, ctx: &mut ModelContext<Self>) -> bool {
        if !self.take_available(marker) {
            return false;
        }
        if self.persist_markers {
            self.persist_consumed_marker(marker, ctx);
        }
        ctx.notify();
        true
    }

    fn resolve_from_snapshot(
        &mut self,
        snapshot: TuiOnboardingMarkersSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        self.state = TuiOnboardingMarkersState::Ready {
            first_zero_state_available: !snapshot.first_zero_state_shown,
            first_credit_gate_available: !snapshot.first_credit_gate_shown,
        };
        ctx.emit(TuiOnboardingMarkersEvent::Ready);
        ctx.notify();
    }
    fn resolve_unavailable(&mut self, ctx: &mut ModelContext<Self>) {
        self.state = TuiOnboardingMarkersState::Ready {
            first_zero_state_available: false,
            first_credit_gate_available: false,
        };
        ctx.emit(TuiOnboardingMarkersEvent::Ready);
        ctx.notify();
    }
    fn handle_load_result(
        &mut self,
        load_generation: u64,
        result: MarkerLoadResult,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.load_generation != load_generation {
            return;
        }
        match result {
            MarkerLoadResult::Loaded(Ok(snapshot)) => {
                self.resolve_from_snapshot(snapshot, ctx);
            }
            MarkerLoadResult::Loaded(Err(error)) => {
                log::warn!(
                    "Unable to load TUI onboarding markers; continuing without one-time onboarding surfaces: {error:#}"
                );
                self.resolve_unavailable(ctx);
            }
            MarkerLoadResult::TimedOut => {
                log::warn!(
                    "Timed out loading TUI onboarding markers; continuing without one-time onboarding surfaces"
                );
                self.resolve_unavailable(ctx);
            }
        }
    }

    fn take_available(&mut self, marker: TuiOnboardingMarker) -> bool {
        let TuiOnboardingMarkersState::Ready {
            first_zero_state_available,
            first_credit_gate_available,
        } = &mut self.state
        else {
            return false;
        };
        let available = match marker {
            TuiOnboardingMarker::FirstZeroState => first_zero_state_available,
            TuiOnboardingMarker::FirstCreditGate => first_credit_gate_available,
        };
        std::mem::take(available)
    }

    fn mark_consumed_by_storage_key(&mut self, storage_key: &str) -> bool {
        let TuiOnboardingMarkersState::Ready {
            first_zero_state_available,
            first_credit_gate_available,
        } = &mut self.state
        else {
            return false;
        };
        match storage_key {
            FIRST_ZERO_STATE_STORAGE_KEY => std::mem::take(first_zero_state_available),
            FIRST_CREDIT_GATE_STORAGE_KEY => std::mem::take(first_credit_gate_available),
            _ => false,
        }
    }

    fn persist_consumed_marker(&self, marker: TuiOnboardingMarker, ctx: &mut ModelContext<Self>) {
        let storage_key = marker.storage_key();
        let existing = CloudModel::as_ref(ctx)
            .get_all_cloud_preferences_by_storage_key()
            .get(storage_key)
            .map(|preference| (*preference).clone());
        let existing_value = existing
            .as_ref()
            .and_then(|preference| preference.model().string_model.value.as_bool());
        match marker_persistence_action(existing_value) {
            MarkerPersistenceAction::None => {}
            MarkerPersistenceAction::Update => {
                let Some(preference) = existing else {
                    return;
                };
                self.update_marker(storage_key, preference, ctx);
            }
            MarkerPersistenceAction::Create => self.create_marker(storage_key, ctx),
        }
    }

    fn update_marker(
        &self,
        storage_key: &str,
        cloud_preference: CloudPreference,
        ctx: &mut ModelContext<Self>,
    ) {
        let Ok(preference) = Preference::new(
            storage_key.to_owned(),
            "true",
            SyncToCloud::Globally(RespectUserSyncSetting::No),
        ) else {
            log::warn!("Unable to serialize consumed TUI onboarding marker {storage_key}");
            return;
        };
        let mut model = cloud_preference.model().clone();
        model.string_model = preference;
        let revision = CloudModel::as_ref(ctx)
            .current_revision(&cloud_preference.id)
            .cloned();
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.update_object(model, cloud_preference.id, revision, ctx);
        });
    }

    fn create_marker(&self, storage_key: &str, ctx: &mut ModelContext<Self>) {
        let Some(personal_drive) = UserWorkspaces::as_ref(ctx).personal_drive(ctx) else {
            log::warn!(
                "Unable to create consumed TUI onboarding marker {storage_key}: personal drive is unavailable"
            );
            return;
        };
        let Ok(preference) = Preference::new(
            storage_key.to_owned(),
            "true",
            SyncToCloud::Globally(RespectUserSyncSetting::No),
        ) else {
            log::warn!("Unable to serialize consumed TUI onboarding marker {storage_key}");
            return;
        };
        let input = GenericStringObjectInput::<Preference, JsonSerializer> {
            id: ClientId::new(),
            model: CloudPreferenceModel::new(preference),
            initial_folder_id: None,
            entrypoint: CloudObjectEventEntrypoint::Unknown,
        };
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.bulk_create_generic_string_objects(personal_drive, vec![input], ctx);
        });
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn new_ready_for_test(
        first_zero_state_available: bool,
        first_credit_gate_available: bool,
    ) -> Self {
        Self {
            state: TuiOnboardingMarkersState::Ready {
                first_zero_state_available,
                first_credit_gate_available,
            },
            load_generation: 0,
            persist_markers: false,
            onboarding_client: None,
            load_timeout: MARKER_LOAD_TIMEOUT,
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn set_ready_for_test(
        &mut self,
        first_zero_state_available: bool,
        first_credit_gate_available: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.state = TuiOnboardingMarkersState::Ready {
            first_zero_state_available,
            first_credit_gate_available,
        };
        ctx.emit(TuiOnboardingMarkersEvent::Ready);
        ctx.notify();
    }
}

impl Entity for TuiOnboardingMarkers {
    type Event = TuiOnboardingMarkersEvent;
}
impl SingletonEntity for TuiOnboardingMarkers {}

enum MarkerLoadResult {
    Loaded(anyhow::Result<TuiOnboardingMarkersSnapshot>),
    TimedOut,
}

async fn load_markers_with_timeout(
    onboarding_client: Arc<dyn TuiOnboardingClient>,
    timeout: Duration,
) -> MarkerLoadResult {
    let load = onboarding_client.get_tui_onboarding_markers();
    let timeout = Timer::after(timeout);
    futures::pin_mut!(load);
    futures::pin_mut!(timeout);
    match futures::future::select(load, timeout).await {
        futures::future::Either::Left((result, _)) => MarkerLoadResult::Loaded(result),
        futures::future::Either::Right(_) => MarkerLoadResult::TimedOut,
    }
}

#[cfg(test)]
#[path = "tui_onboarding_markers_tests.rs"]
mod tests;
