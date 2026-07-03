use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
pub use cloud_object_client::ObjectUpdateMessage;
use futures_util::stream::AbortHandle;
use instant::Instant;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, ModelHandle, RequestState, SingletonEntity};

use super::update_manager::UpdateManager;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::report_error;
use crate::server::retry_strategies::LISTENER_RETRY_STRATEGY;
use crate::server::server_api::object::ObjectClient;
use crate::system::{SystemStats, SystemStatsEvent};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

lazy_static::lazy_static! {
    /// Between successful websocket connections, we ensured at least this amount of time
    /// has elapsed so that we aren't spamming the websocket server (e.g. if connections are being
    /// closed quickly for any reason).
    static ref WAIT_PERIOD_BETWEEN_SUCCESSFUL_RECONNECTS: Duration = Duration::from_secs(30);
}

/// If the websocket reconnects within this duration of the last disconnection, skip the
/// out-of-band refresh of cloud objects. The periodic poll will catch any missed updates.
///
/// This needs to be relatively large, due to the retry policy we use on websocket disconnection,
/// which waits between 10-40s between retries.  At the very least, this should always be slightly
/// larger than the upper end of that range.
const RECONNECTION_REFRESH_THRESHOLD: Duration = Duration::from_secs(60);

/// Maximum random delay added before making an out-of-band refresh after a longer reconnection.
/// Spreading out requests across this window helps avoid a thundering herd when many clients
/// reconnect simultaneously (e.g. after a server release).
const MAX_RECONNECTION_REFRESH_DELAY: Duration = Duration::from_secs(30);

/// Describes the type of websocket connection that was just established.
enum ConnectionEvent {
    /// The very first websocket connection after application startup.
    InitialConnection,
    /// A reconnection after a previous disconnection.
    Reconnection {
        /// The duration since the last websocket disconnection.
        time_since_disconnection: Duration,
    },
}

pub enum ListenerEvent {}

/// The Listener is responsible for listening to updates from
/// the server for cloud-object related things (e.g. a notebook was changed,
/// or edit access was taken for a workflow, etc.)
pub struct Listener {
    cloud_objects_client: Arc<dyn ObjectClient>,
    /// Since we only want to start websocket connections if we know the user is
    /// on a team or has access to cloud objects, we keep track of whether
    /// or not we should be subscribing for updates. Once we start websockets, we don't stop
    /// so that the user gets a snappier experience once they start using Warp Drive.
    should_subscribe_to_updates: bool,
    /// Abort handle for the (retried) future that resolves when the subscription is done.
    current_subscription_abort_handle: Option<AbortHandle>,
    /// Channel that we send a message over each time we've successfully established a subscription.
    subscription_ready_tx: Sender<()>,
    /// The time at which the last websocket disconnection occurred. `None` if no disconnection
    /// has occurred yet (i.e., this is the first connection attempt).
    last_disconnected_at: Option<Instant>,
    /// Abort handle for a pending delayed refresh spawned after a long reconnection. Tracked so
    /// that it can be cancelled if the websocket disconnects again before the refresh fires.
    pending_refresh_abort_handle: Option<AbortHandle>,
}

impl Listener {
    pub fn new(cloud_objects_client: Arc<dyn ObjectClient>, ctx: &mut ModelContext<Self>) -> Self {
        let (subscription_ready_tx, subscription_ready_rx) = async_channel::unbounded();
        let mut listener = Self {
            cloud_objects_client,
            should_subscribe_to_updates: false,
            current_subscription_abort_handle: None,
            subscription_ready_tx,
            last_disconnected_at: None,
            pending_refresh_abort_handle: None,
        };

        // When the websocket signals readiness, decide whether to refresh cloud objects
        // based on how long the connection was down.
        let _ = ctx.spawn_stream_local(
            subscription_ready_rx,
            Self::on_subscription_ready,
            |_, _| {},
        );

        ctx.subscribe_to_model(&SystemStats::handle(ctx), Self::handle_cpu_event);

        ctx.subscribe_to_model(
            &NetworkStatus::handle(ctx),
            Self::handle_network_status_changed_event,
        );

        // To prevent creating unnecessary websockets, we only open a websocket if
        // - a user is known to be part of a team
        // - or a user has access to >= 1 cloud object
        // In either of these cases, it's worth creating a websocket for cloud object updates.
        //
        // Note that we also want a websocket for CloudPreferences, but this is handled via listening
        // to the cloud model for the creation of cloud preferences objects (which happens when settings sync
        // is enabled for the first time).
        ctx.subscribe_to_model(
            &UserWorkspaces::handle(ctx),
            Self::handle_user_workspaces_event,
        );
        ctx.subscribe_to_model(&CloudModel::handle(ctx), Self::handle_cloud_model_event);

        // We need to do a one-time check of cloud objects when starting
        // because the Cloud Model was initialized before this model and we could have populated
        // its object cache with objects from sqlite.
        if listener.has_non_welcome_cloud_objects(ctx) {
            listener.start_listener(ctx);
        }

        listener
    }

    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        use crate::server::server_api::ServerApiProvider;

        Self::new(ServerApiProvider::new_for_test().get(), ctx)
    }

    fn is_part_of_some_team(&self, ctx: &ModelContext<Self>) -> bool {
        UserWorkspaces::as_ref(ctx).has_teams()
    }

    // If the user is part of a team, we should start subscribing for updates.
    fn handle_user_workspaces_event(
        &mut self,
        _: ModelHandle<UserWorkspaces>,
        event: &UserWorkspacesEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let UserWorkspacesEvent::TeamsChanged = event {
            if self.is_part_of_some_team(ctx) {
                self.start_listener(ctx);
            }
        }
    }

    /// Returns true if the user has any object that is not a welcome object. If the user only has objects
    /// that are welcome objects, returns false.
    fn has_non_welcome_cloud_objects(&self, ctx: &ModelContext<Self>) -> bool {
        CloudModel::as_ref(ctx).has_non_welcome_objects()
    }

    // If the user has access to >= 1 cloud objects, we should subscribe for updates.
    fn handle_cloud_model_event(
        &mut self,
        _: ModelHandle<CloudModel>,
        _event: &CloudModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.has_non_welcome_cloud_objects(ctx) {
            self.start_listener(ctx);
        }
    }

    // This is a workaround for an issue where the future that should resolve when the websocket
    // is finished is _not_ polled when the websocket is closed by the server and the CPU is asleep.
    // To get around this, we manually abort the future (effectively closing the websocket)
    // when the CPU goes to sleep and restart it when it's awakened.
    // https://linear.app/warpdotdev/issue/CLD-172/websocket-hangs-when-closed-during-cpu-sleep
    fn handle_cpu_event(
        &mut self,
        _: ModelHandle<SystemStats>,
        event: &SystemStatsEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            SystemStatsEvent::CpuWasAwakened => {
                if let Some(abort_handle) = self.current_subscription_abort_handle.take() {
                    abort_handle.abort();
                }

                // We intentionally do not update `last_disconnected_at` or cancel pending
                // refreshes here. The paired `CpuWillSleep` event already handled both;
                // this handler just restarts the websocket so that `on_subscription_ready`
                // can decide whether to refresh based on the sleep-time gap.
                if self.should_subscribe_to_updates {
                    self.get_warp_drive_updates(ctx);
                }
            }
            SystemStatsEvent::CpuWillSleep => {
                if let Some(abort_handle) = self.current_subscription_abort_handle.take() {
                    abort_handle.abort();
                    self.last_disconnected_at = Some(Instant::now());
                    self.cancel_pending_refresh();
                }
            }
        }
    }

    fn handle_network_status_changed_event(
        &mut self,
        _: ModelHandle<NetworkStatus>,
        event: &NetworkStatusEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            NetworkStatusEvent::NetworkStatusChanged { new_status } => match new_status {
                // When coming back online, restart a websocket.
                NetworkStatusKind::Online => {
                    if let Some(abort_handle) = self.current_subscription_abort_handle.take() {
                        abort_handle.abort();
                    }

                    if self.should_subscribe_to_updates {
                        self.get_warp_drive_updates(ctx);
                    }
                }

                // When losing connection, abort the current subscription to avoid a lingering future that doesn't resolve.
                NetworkStatusKind::Offline => {
                    if let Some(abort_handle) = self.current_subscription_abort_handle.take() {
                        abort_handle.abort();
                        self.last_disconnected_at = Some(Instant::now());
                        self.cancel_pending_refresh();
                    }
                }
            },
        }
    }

    fn start_listener(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.should_subscribe_to_updates {
            self.should_subscribe_to_updates = true;
            self.get_warp_drive_updates(ctx);
        }
    }

    /// Cancels any pending delayed refresh that was scheduled after a reconnection.
    fn cancel_pending_refresh(&mut self) {
        if let Some(abort_handle) = self.pending_refresh_abort_handle.take() {
            abort_handle.abort();
        }
    }

    /// Called each time the websocket signals readiness. Decides whether to trigger an
    /// out-of-band refresh of cloud objects based on how long the connection was down.
    fn on_subscription_ready(&mut self, _: (), ctx: &mut ModelContext<Self>) {
        // Cancel any pending refresh from a previous reconnection to avoid accumulating
        // stale refresh requests if the websocket is rapidly cycling.
        self.cancel_pending_refresh();

        let connection_event = match self.last_disconnected_at {
            None => ConnectionEvent::InitialConnection,
            Some(disconnected_at) => ConnectionEvent::Reconnection {
                time_since_disconnection: disconnected_at.elapsed(),
            },
        };

        match connection_event {
            ConnectionEvent::InitialConnection => {
                // No out-of-band refresh needed for the initial connection. The periodic
                // poll (started by TeamTesterStatus) already fetches cloud objects at
                // startup, so an additional request here would be duplicative.
                log::info!(
                    "Initial websocket connection established; skipping out-of-band refresh."
                );
            }
            ConnectionEvent::Reconnection {
                time_since_disconnection,
            } if time_since_disconnection < RECONNECTION_REFRESH_THRESHOLD => {
                log::info!(
                    "Websocket reconnected after {time_since_disconnection:?}, within the \
                     {RECONNECTION_REFRESH_THRESHOLD:?} refresh threshold; \
                     skipping out-of-band refresh.",
                );
            }
            ConnectionEvent::Reconnection {
                time_since_disconnection,
            } => {
                // Add a random delay to avoid a thundering herd when many clients reconnect
                // simultaneously (e.g. after a server release).
                let delay = MAX_RECONNECTION_REFRESH_DELAY.mul_f32(rand::random::<f32>());
                log::info!(
                    "Websocket reconnected after {time_since_disconnection:?}; \
                     refreshing objects after {delay:?} delay.",
                );

                let handle = ctx.spawn(async move { Timer::after(delay).await }, |me, _, ctx| {
                    me.pending_refresh_abort_handle = None;
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.refresh_updated_objects(ctx);
                    });
                });
                self.pending_refresh_abort_handle = Some(handle.abort_handle());
            }
        }
    }

    fn get_warp_drive_updates(&mut self, ctx: &mut ModelContext<Self>) {
        let object_client = self.cloud_objects_client.clone();
        let (message_sender, message_receiver) = async_channel::unbounded();
        let subscription_ready_tx = self.subscription_ready_tx.clone();

        // On every message we receive (over the message_receiver), send it
        // to the UpdateManager.
        let _ = ctx.spawn_stream_local(
            message_receiver,
            |_me, item: ObjectUpdateMessage, ctx| {
                log::info!(
                    "Received {} message in CloudObjects::Listener",
                    item.as_str()
                );
                UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                    update_manager.received_message_from_server(item, ctx);
                })
            },
            |_, _| {},
        );

        // Start the future that sends messages over the message_sender stream.
        // TODO: we should investigate having get_warp_drive_updates (and in turn,
        // start_graphql_streaming_operation) return an `impl Stream` so that we don't
        // need to spawn and then spawn_stream_local. For this, we'll need an equivalent
        // spawn_stream (which is like spawn_stream_local but polls the futures in the stream
        // on a background thread).
        let spawn_handle = ctx.spawn_with_retry_on_error(
            move || {
                let object_client = object_client.clone();
                let message_sender = message_sender.clone();
                let subscription_ready_tx = subscription_ready_tx.clone();
                async move {
                    let start_time = Instant::now();
                    log::info!("Attempting to start websocket connection in CloudObjects::Listener");
                    let res = object_client
                        .get_warp_drive_updates(
                            message_sender,
                            subscription_ready_tx,
                        ).await;
                    res.map(|_| start_time.elapsed())
                }
            },
            LISTENER_RETRY_STRATEGY,
            |me, req_state, ctx| {
                match req_state {
                    RequestState::RequestSucceeded(elapsed_time) => {
                        // Record the disconnection time now that a live connection has ended.
                        // Only set this here (not on failed retries) so that the elapsed time
                        // accurately reflects the full duration since the real disconnection.
                        me.last_disconnected_at = Some(Instant::now());
                        me.cancel_pending_refresh();
                        // The future only resolves once the stream is done, so
                        // at that point, we should restart the stream.
                        // In case the websocket was closed quickly by the server,
                        // let's ensure at least some time has passed before we restart the connection.
                        let time_to_wait = (*WAIT_PERIOD_BETWEEN_SUCCESSFUL_RECONNECTS).saturating_sub(elapsed_time);
                        log::info!("Websocket for CloudObjects::Listener is done; restarting after {}s.", time_to_wait.as_secs());
                        ctx.spawn(async move {
                            Timer::after(time_to_wait).await
                        }, |me, _, ctx| {
                            me.get_warp_drive_updates(ctx);
                        });
                    }
                    RequestState::RequestFailedRetryPending(e) => {
                        log::warn!("CloudObjects::Listener: websocket connection failed to connect or finished with an error; trying again: {e:#}");
                    }
                    RequestState::RequestFailed(e) => {
                        report_error!(e.context("CloudObjects::Listener websocket connection failed"));
                    }
                }
            },
        );

        self.current_subscription_abort_handle = Some(spawn_handle.abort_handle());
    }

    #[allow(dead_code)]
    pub fn has_current_subscription_abort_handle(&self) -> bool {
        self.current_subscription_abort_handle.is_some()
    }
}

impl Entity for Listener {
    type Event = ListenerEvent;
}

impl SingletonEntity for Listener {}
