use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::channel::oneshot::{self, Receiver};
use futures::stream::AbortHandle;
use warpui::r#async::Timer;
use warpui::{
    duration_with_jitter, Entity, ModelContext, ModelHandle, RequestState, SingletonEntity,
};

use super::team_tester::{TeamTesterStatus, TeamTesterStatusEvent};
use super::user_workspaces::{
    CreateTeamResponse, UserWorkspaces, WorkspacesMetadataResponse, WorkspacesMetadataWithPricing,
};
use super::workspace::WorkspaceUid;
use crate::ai::llms::LLMPreferences;
use crate::auth::AuthStateProvider;
use crate::cloud_object::CloudObjectEventEntrypoint;
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::persistence::ModelEvent;
use crate::pricing::PricingInfoModel;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::ServerId;
use crate::server::retry_strategies::{
    OUT_OF_BAND_REQUEST_RETRY_STRATEGY, PERIODIC_POLL, PERIODIC_POLL_RETRY_STRATEGY,
};
use crate::server::server_api::team::TeamClient;
use crate::server::server_api::ServerApiProvider;
use crate::{report_error, report_if_error};

pub enum TeamUpdateManagerEvent {
    LeaveSuccess,
    LeaveError,
    RenameTeamSuccess,
    RenameTeamError,
}

/// TeamUpdateManager is a singleton model responsible for communicating with the server and local
/// database regarding teams' metadata.
/// It emits events that are later processed by UserWorkspaces model (which is an in-memory store for
/// the workspace metadata).
/// TeamUpdateManager is used when sending a team-related request to the server and processing the
/// response, but also controls the periodic polling from the server (also controlled by calling
/// `force_refresh` method).
pub struct TeamUpdateManager {
    team_client: Arc<dyn TeamClient>,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    should_poll_for_workspace_metadata_updates: bool,

    /// The abort handle for the timer that waits a fixed duration
    /// before making an outbound request for workspace metadata, if any.
    next_poll_abort_handle: Option<AbortHandle>,

    /// The abort handle for the in flight request of workspace metadata,
    /// if any.
    in_flight_request_abort_handle: Option<AbortHandle>,
}

impl TeamUpdateManager {
    pub fn new(
        team_client: Arc<dyn TeamClient>,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, Self::handle_network_status_changed);

        let team_tester_status = TeamTesterStatus::handle(ctx);
        ctx.subscribe_to_model(&team_tester_status, Self::handle_team_tester_status_changed);

        Self {
            team_client,
            model_event_sender,
            should_poll_for_workspace_metadata_updates: false,
            next_poll_abort_handle: None,
            in_flight_request_abort_handle: None,
        }
    }

    fn handle_network_status_changed(
        &mut self,
        _: ModelHandle<NetworkStatus>,
        network_status: &NetworkStatusEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match network_status {
            NetworkStatusEvent::NetworkStatusChanged { new_status } => match new_status {
                NetworkStatusKind::Online => {
                    // TODO: this will cause us to reset our polling very frequently
                    // if the client's network conn is repeatedly flipping between on and off.
                    self.start_polling_for_workspace_metadata_updates(ctx);
                }
                NetworkStatusKind::Offline => self.stop_polling_for_workspace_metadata_updates(),
            },
        }
    }

    fn handle_team_tester_status_changed(
        &mut self,
        _: ModelHandle<TeamTesterStatus>,
        event: &TeamTesterStatusEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let TeamTesterStatusEvent::InitiateDataPollers { force_refresh } = event;
        if *force_refresh {
            std::mem::drop(self.refresh_workspace_metadata(ctx));
        }

        self.start_polling_for_workspace_metadata_updates(ctx);
    }

    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        use crate::server::server_api::team::MockTeamClient;

        // This mock API is used in test contexts where we don't care which teams the user is on.
        // Since the mocked `TeamClient` is inaccessible to tests, stub the metadata polling to
        // avoid noisy `No matching expectation found` errors.
        let mut team_client = MockTeamClient::new();
        team_client.expect_workspaces_metadata().returning(|| {
            Ok(WorkspacesMetadataWithPricing {
                metadata: WorkspacesMetadataResponse {
                    workspaces: vec![],
                    joinable_teams: vec![],
                    experiments: None,
                    feature_model_choices: None,
                },
                pricing_info: None,
            })
        });

        Self::new(Arc::new(team_client), Default::default(), ctx)
    }

    /// Starts a periodic poll for workspace metadata changes, if there isn't already
    /// an existing poll queued up.
    pub fn start_polling_for_workspace_metadata_updates(&mut self, ctx: &mut ModelContext<Self>) {
        let is_online = NetworkStatus::as_ref(ctx).is_online();
        if !self.should_poll_for_workspace_metadata_updates && is_online {
            self.should_poll_for_workspace_metadata_updates = true;
            self.poll_for_workspace_metadata_changes(ctx);
        }
    }

    pub fn stop_polling_for_workspace_metadata_updates(&mut self) {
        self.should_poll_for_workspace_metadata_updates = false;
        self.abort_existing_poll();
    }

    /// Out-of-band (from the regular poll) refresh of workspace metadata.
    /// Returns a oneshot Receiver that resolves when the refresh completes (success or final failure).
    pub fn refresh_workspace_metadata(&mut self, ctx: &mut ModelContext<Self>) -> Receiver<()> {
        // Skip the refresh when logged out to avoid noisy auth errors.
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            let (tx, rx) = oneshot::channel::<()>();
            let _ = tx.send(());
            return rx;
        }

        let team_client = self.team_client.clone();
        let (tx, rx) = oneshot::channel::<()>();
        let mut tx = Some(tx);
        ctx.spawn_with_retry_on_error(
            move || {
                let team_client = team_client.clone();
                async move { team_client.workspaces_metadata().await }
            },
            OUT_OF_BAND_REQUEST_RETRY_STRATEGY,
            move |update_manager, request_state, ctx| {
                // Only signal once there are no more retries left.
                let is_final = !request_state.has_pending_retries();
                update_manager.handle_workspace_metadata_with_request_state(request_state, ctx);
                if is_final {
                    if let Some(sender) = tx.take() {
                        let _ = sender.send(());
                    }
                }
            },
        );
        rx
    }

    fn abort_existing_poll(&mut self) {
        if let Some(abort_handle) = self.in_flight_request_abort_handle.take() {
            abort_handle.abort();
        }

        if let Some(abort_handle) = self.next_poll_abort_handle.take() {
            abort_handle.abort();
        }
    }

    /// Only call this method if you need to restart the poll and force a refresh.
    /// Currently called when
    /// - we decide that a feature flag needs to change
    /// - find out that a user is a team tester
    /// - a network status changes (we go from offline to online state)
    ///
    /// Note: the gql query for this poll also pulls in experiment state. If we change
    /// the behaviour for polling workspace metadata, we should consider what ramifications
    /// that has on querying experiment state.
    fn poll_for_workspace_metadata_changes(&mut self, ctx: &mut ModelContext<Self>) {
        self.abort_existing_poll();

        if !self.should_poll_for_workspace_metadata_updates {
            return;
        }

        // Don't poll when the user is logged out to avoid spamming auth errors in the logs.
        // Polling will be restarted when the user logs in via `initiate_data_pollers`.
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            self.should_poll_for_workspace_metadata_updates = false;
            return;
        }

        let team_client = self.team_client.clone();
        // We retry a few times here in case there are any transient network errors.
        let spawn_handle = ctx.spawn_with_retry_on_error(
            move || {
                let team_client = team_client.clone();
                async move {
                    team_client
                        .workspaces_metadata()
                        .await
                        .context("Error polling for workspace metadata changes")
                }
            },
            PERIODIC_POLL_RETRY_STRATEGY,
            |update_manager, res, ctx| {
                // Only poll if `spawn_with_retry_on_error` is not going to retry again so we don't end up with multiple
                // polls running simultaneously.
                let should_poll_again = !res.has_pending_retries();
                update_manager.handle_workspace_metadata_with_request_state(res, ctx);

                if should_poll_again {
                    let next_poll_handle = ctx.spawn(
                        async move {
                            Timer::after(duration_with_jitter(
                                PERIODIC_POLL,
                                0.2, /* max_jitter_multiplier */
                            ))
                            .await
                        },
                        |update_manager, _, ctx| {
                            update_manager.poll_for_workspace_metadata_changes(ctx);
                        },
                    );
                    update_manager.next_poll_abort_handle = Some(next_poll_handle.abort_handle());
                }
            },
        );

        self.in_flight_request_abort_handle = Some(spawn_handle.abort_handle());
    }

    fn save_to_db(&self, events: impl IntoIterator<Item = ModelEvent>) {
        let model_event_sender = self.model_event_sender.clone();
        if let Some(model_event_sender) = &model_event_sender {
            for event in events {
                report_if_error!(model_event_sender
                    .send(event)
                    .context("Unable to save teams metadata to sqlite"));
            }
        }
    }

    pub fn create_team(
        &mut self,
        team_name: String,
        entrypoint: CloudObjectEventEntrypoint,
        discoverable: Option<bool>,
        ctx: &mut ModelContext<Self>,
    ) {
        let team_client = self.team_client.clone();
        let _ = ctx.spawn(
            async move {
                team_client
                    .create_team(team_name, entrypoint, discoverable)
                    .await
                    .context("Error creating team")
            },
            Self::on_team_created,
        );
    }

    fn on_team_created(
        &mut self,
        create_team_response: Result<CreateTeamResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        // TODO we should implement a similar mechanism to cloud objects with local team id
        report_if_error!(create_team_response);
        let Ok(create_team_response) = create_team_response else {
            return;
        };

        // Update sqlite
        self.save_to_db([ModelEvent::UpsertWorkspace {
            workspace: Box::new(create_team_response.workspace.clone()),
        }]);

        // Update UserWorkspaces
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.team_created(&create_team_response, ctx);
        });
    }

    pub fn leave_team(
        &mut self,
        team_uid: ServerId,
        entrypoint: CloudObjectEventEntrypoint,
        ctx: &mut ModelContext<Self>,
    ) {
        // Handle server update
        let user_uid = AuthStateProvider::as_ref(ctx).get().user_id();
        if let Some(user_uid) = user_uid {
            let team_client = self.team_client.clone();
            let _ = ctx.spawn(
                async move {
                    team_client
                        .leave_team(user_uid, team_uid, entrypoint)
                        .await
                        .context("Error leaving team")
                },
                move |me, result, ctx| {
                    me.on_team_left(team_uid, result, ctx);
                },
            );
        } else {
            log::warn!("User is not authenticated, cannot leave team");
            ctx.emit(TeamUpdateManagerEvent::LeaveError);
        }
    }

    fn on_team_left(
        &mut self,
        left_team_uid: ServerId,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(response) => {
                if let Some(pricing_info) = response.pricing_info {
                    PricingInfoModel::handle(ctx).update(ctx, |model, ctx| {
                        model.update_pricing_info(pricing_info, ctx);
                    });
                }

                let workspaces = response.metadata.workspaces;
                let joinable_teams = response.metadata.joinable_teams;

                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.update_workspaces(workspaces.clone(), ctx);
                    user_workspaces.update_joinable_teams(joinable_teams, ctx);
                });

                // Check if the current workspace is still in the list of workspaces.
                // If it's not, then set the current workspace to the first workspace in the list.
                if let Some(current_workspace) = UserWorkspaces::as_ref(ctx).current_workspace() {
                    if !workspaces.iter().any(|w| w.uid == current_workspace.uid) {
                        if let Some(workspace_uid) = workspaces.first().map(|w| w.uid) {
                            self.set_current_workspace_uid(workspace_uid, ctx);
                        };
                    }
                } else if let Some(workspace_uid) = workspaces.first().map(|w| w.uid) {
                    self.set_current_workspace_uid(workspace_uid, ctx);
                }

                // Update sqlite
                self.save_to_db([ModelEvent::UpsertWorkspaces { workspaces }]);

                // Remove objects owned by the team that was left.
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    // We first remove team objects from local state so that they're not shown to the user.
                    // Then, refresh all objects to fetch any that were independently shared.
                    update_manager.remove_team_objects(left_team_uid, ctx);
                    update_manager.refresh_updated_objects(ctx);
                });

                ctx.emit(TeamUpdateManagerEvent::LeaveSuccess);
            }
            Err(e) => {
                report_error!(e);

                ctx.emit(TeamUpdateManagerEvent::LeaveError);
            }
        }
    }

    pub fn rename_team(&mut self, new_name: String, ctx: &mut ModelContext<Self>) {
        let team_client = self.team_client.clone();
        let team_uid = UserWorkspaces::handle(ctx).read(ctx, |user_workspaces, _| {
            user_workspaces.current_team().map(|team| team.uid)
        });
        if let Some(team_uid) = team_uid {
            let _ = ctx.spawn(
                async move { team_client.rename_team(new_name, team_uid).await },
                Self::on_team_renamed,
            );
        }
    }

    fn on_team_renamed(
        &mut self,
        result: Result<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Err(_) => ctx.emit(TeamUpdateManagerEvent::RenameTeamError),
            Ok(response) => {
                if let Some(pricing_info) = response.pricing_info.clone() {
                    PricingInfoModel::handle(ctx).update(ctx, |model, ctx| {
                        model.update_pricing_info(pricing_info, ctx);
                    });
                }

                self.on_workspaces_updated(Ok(response.metadata.clone()), ctx);

                // Update sqlite
                self.save_to_db([ModelEvent::UpsertWorkspaces {
                    workspaces: response.metadata.workspaces,
                }]);

                ctx.emit(TeamUpdateManagerEvent::RenameTeamSuccess);
            }
        };
        ctx.notify();
    }

    fn handle_workspace_metadata_with_request_state(
        &mut self,
        request_state: RequestState<WorkspacesMetadataWithPricing>,
        ctx: &mut ModelContext<Self>,
    ) {
        match request_state {
            RequestState::RequestSucceeded(response) => {
                if let Some(pricing_info) = response.pricing_info.clone() {
                    PricingInfoModel::handle(ctx).update(ctx, |model, ctx| {
                        model.update_pricing_info(pricing_info, ctx);
                    });
                }

                // Right now, this function is coupled with how we handle leaving a team.
                // TODO(zheng) refactor so we can separate these two cases and have clearer logic.
                self.on_workspaces_updated(Ok(response.metadata), ctx);
            }
            RequestState::RequestFailedRetryPending(err) => {
                log::info!(
                    "get_workspaces_metadata_for_user: request failed with error {err:#}. Trying again."
                );
            }
            RequestState::RequestFailed(err) => {
                log::info!("get_workspaces_metadata_for_user: request failed with error {err:#}. Retries exhausted.");
            }
        }
    }

    fn on_workspaces_updated(
        &mut self,
        result: Result<WorkspacesMetadataResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(user_workspaces_access) => {
                let workspaces = user_workspaces_access.workspaces;
                let joinable_teams = user_workspaces_access.joinable_teams;
                let experiments = user_workspaces_access.experiments;

                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.update_workspaces(workspaces.clone(), ctx);
                    user_workspaces.update_joinable_teams(joinable_teams.clone(), ctx);
                });

                // Check if the current workspace is still in the list of workspaces.
                // If it's not, then set the current workspace to the first workspace in the list.
                if let Some(current_workspace) = UserWorkspaces::as_ref(ctx).current_workspace() {
                    if !workspaces.iter().any(|w| w.uid == current_workspace.uid) {
                        if let Some(workspace_uid) = workspaces.first().map(|w| w.uid) {
                            self.set_current_workspace_uid(workspace_uid, ctx);
                        };
                    }
                } else if let Some(workspace_uid) = workspaces.first().map(|w| w.uid) {
                    self.set_current_workspace_uid(workspace_uid, ctx);
                }

                if let Some(experiments) = experiments {
                    ServerApiProvider::handle(ctx).update(ctx, |provider, ctx| {
                        provider.handle_experiments_fetched(experiments, ctx);
                    });
                }

                if let Some(feature_model_choices) = user_workspaces_access.feature_model_choices {
                    LLMPreferences::handle(ctx).update(ctx, |llm_preferences, ctx| {
                        llm_preferences
                            .update_feature_model_choices(feature_model_choices.try_into(), ctx);
                    });
                }

                // Update sqlite
                self.save_to_db([ModelEvent::UpsertWorkspaces { workspaces }]);
            }
            Err(e) => {
                report_error!(e);
            }
        }
    }

    pub fn set_current_workspace_uid(
        &mut self,
        workspace_uid: WorkspaceUid,
        ctx: &mut ModelContext<Self>,
    ) {
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
        });

        // Update sqlite
        self.save_to_db([ModelEvent::SetCurrentWorkspace { workspace_uid }]);
    }
}

impl Entity for TeamUpdateManager {
    type Event = TeamUpdateManagerEvent;
}

impl SingletonEntity for TeamUpdateManager {}

#[cfg(test)]
#[path = "update_manager_tests.rs"]
mod tests;
