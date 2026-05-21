mod changelog;
mod channel_versions;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

use crate::features::FeatureFlag;
use crate::send_telemetry_sync_from_app_ctx;
use crate::server::server_api::ServerApi;
use crate::server::telemetry::TelemetryEvent;
use crate::workspace::Workspace;
use crate::{
    channel::Channel, report_if_error, send_telemetry_from_ctx, server::datetime_ext::DateTimeExt,
    ChannelState,
};
use ::channel_versions::{ParsedVersion, VersionInfo};
use anyhow::{anyhow, Context as _, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use rand::Rng as _;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use warp_core::execution_mode::AppExecutionMode;
use warpui::platform::TerminationMode;
use warpui::r#async::Timer;
use warpui::windowing::state::ApplicationStage;
use warpui::windowing::{self, WindowManager};
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    AppContext,
};
use warpui::{Entity, ModelContext, SingletonEntity, ViewContext};

pub use self::changelog::get_current_changelog;
use self::channel_versions::fetch_channel_versions;

/// A successfully downloaded and unpacked target update.
#[derive(Clone, Debug)]
pub struct DownloadedUpdate {
    pub version: VersionInfo,
    pub update_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum AutoupdateStage {
    /// No update available as of the last check with the server.
    #[default]
    NoUpdateAvailable,
    /// Checking for an update and downloading it if one exists.
    CheckingForUpdate,
    /// The new version is being downloaded.
    DownloadingUpdate,
    /// An update exists but the user does not have authorization to install it.
    UnableToUpdateToNewVersion { new_version: VersionInfo },
    /// An update has been downloaded and is ready for relaunch.
    UpdateReady {
        new_version: VersionInfo,
        update_id: String,
    },
    /// A relaunch has been initiated to use the new, downloaded version.
    Updating {
        new_version: VersionInfo,
        update_id: String,
    },
    /// A relaunch was initiated to use the new version, but failed.
    UnableToLaunchNewVersion { new_version: VersionInfo },
    /// A new version was installed, but Warp hasn't restarted yet.
    ///
    /// This state is only used on macOS, where the update isn't fully applied until right before
    /// restarting.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdatedPendingRestart { new_version: VersionInfo },
}

impl AutoupdateStage {
    /// Returns `true` if we're ready to relaunch and apply an update.
    pub fn ready_for_update(&self) -> bool {
        matches!(
            self,
            AutoupdateStage::UpdateReady { .. } | AutoupdateStage::UpdatedPendingRestart { .. }
        )
    }

    /// Returns the new version's VersionInfo, if available in the current autoupdate stage.
    pub fn available_new_version(&self) -> Option<&VersionInfo> {
        match self {
            AutoupdateStage::UpdateReady { new_version, .. }
            | AutoupdateStage::Updating { new_version, .. }
            | AutoupdateStage::UpdatedPendingRestart { new_version }
            | AutoupdateStage::UnableToLaunchNewVersion { new_version }
            | AutoupdateStage::UnableToUpdateToNewVersion { new_version } => Some(new_version),
            _ => None,
        }
    }
}

pub struct AutoupdateState {
    /// We only want to hit /client_version/daily about once a day. This field is client state for
    /// implementing the logic so that we (mostly) limit requests to that endpoint to once a day,
    /// though we don't persist that across app-restarts.
    last_successful_daily_update_check: Option<DateTime<FixedOffset>>,
    stage: AutoupdateStage,
    /// The most recently downloaded and extracted update. We need this so that if there are
    /// multiple update checks without a relaunch, we only download the update once.
    downloaded_update: Option<DownloadedUpdate>,
    /// Holds requests for update checks that are awaiting to be executed. We need this because we
    /// prevent update checks from starting if there is another already in-flight. The different
    /// RequestTypes have different behavior and side-effects, so it's important not to skip any
    /// but to queue them instead.
    request_queue: VecDeque<RequestType>,
    server_api: Arc<ServerApi>,
    /// Whether the polling loop has been explicitly started. Requests are silently queued but not
    /// executed until `start_polling` is called. This ensures no version-check requests are made
    /// before onboarding completes.
    polling_started: bool,
}

impl AutoupdateState {
    pub fn new(server_api: Arc<ServerApi>) -> Self {
        Self {
            server_api,
            last_successful_daily_update_check: None,
            stage: AutoupdateStage::default(),
            downloaded_update: None,
            request_queue: VecDeque::new(),
            polling_started: false,
        }
    }

    pub fn register(ctx: &mut AppContext, server_api: Arc<ServerApi>) {
        ctx.add_singleton_model(move |_ctx| Self::new(server_api));
    }

    /// Start the autoupdate polling loop. Idempotent: subsequent calls are no-ops.
    ///
    /// Must be called explicitly once onboarding (if any) has completed. For returning users
    /// who bypass onboarding, this should be called during app startup.
    pub fn start_polling(&mut self, ctx: &mut ModelContext<Self>) {
        if self.polling_started {
            return;
        }
        if FeatureFlag::Autoupdate.is_enabled() && AppExecutionMode::as_ref(ctx).can_autoupdate() {
            log::info!("Starting autoupdate polling loop");
            self.polling_started = true;
            // Initiate the polling loop.
            self.poll_for_update(ctx);
            // Queue a possible update check when the app gets activated, i.e. focused.
            let state_handle = WindowManager::handle(ctx);
            ctx.subscribe_to_model(&state_handle, |me, event, ctx| {
                let windowing::StateEvent::ValueChanged { current, previous } = event;
                if previous.stage == ApplicationStage::Inactive
                    && current.stage == ApplicationStage::Active
                {
                    me.enqueue_request(RequestType::DailyCheck, ctx);
                }
            });
        }
    }

    /// Check if any requests are pending. If there are and we're ready to submit a new request,
    /// run it.
    fn try_execute_request(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(next_request) = self.get_next_request(ctx) {
            self.check_for_update(next_request, ctx);
        }
    }

    /// Check if there are any requests in the queue. Return the next one, but only if there isn't
    /// already a request in-flight.
    fn get_next_request(&mut self, ctx: &mut ModelContext<Self>) -> Option<RequestType> {
        // WASM cannot apply updates, so no request type should ever contact the server.
        if cfg!(target_family = "wasm") {
            return None;
        }

        // Don't execute any requests until polling has been explicitly started (i.e. onboarding
        // has completed). Requests enqueued before that point are silently deferred.
        if !self.polling_started {
            return None;
        }

        if !self.should_start_update_check() {
            return None;
        }

        while let Some(request) = self.request_queue.pop_front() {
            match request {
                RequestType::ManualCheck => return Some(request),
                RequestType::Poll | RequestType::DailyCheck => {
                    if AppExecutionMode::as_ref(ctx).can_autoupdate() {
                        return Some(request);
                    }
                }
            }
        }

        None
    }

    /// After queueing the request, immediately try executing it.
    fn enqueue_request(&mut self, request_type: RequestType, ctx: &mut ModelContext<Self>) {
        // WASM cannot execute any update requests; skip enqueuing entirely so the
        // queue never grows with work that can never be consumed.
        if cfg!(target_family = "wasm") {
            return;
        }
        self.request_queue.push_back(request_type);
        self.try_execute_request(ctx);
    }

    // Poll for updates once per 10 minutes.
    const AUTOUPDATE_POLL: Duration = Duration::from_secs(10 * 60);

    /// This method recursively calls itself after a delay. Call it once and only once to start the
    /// loop.
    fn poll_for_update(&mut self, ctx: &mut ModelContext<Self>) {
        self.enqueue_request(RequestType::Poll, ctx);
        ctx.spawn(
            async {
                Timer::after(Self::AUTOUPDATE_POLL).await;
            },
            |me, _, ctx| me.poll_for_update(ctx),
        );
    }

    /// User-initiated check for updates.
    pub fn manually_check_for_update(&mut self, ctx: &mut ModelContext<Self>) {
        self.enqueue_request(RequestType::ManualCheck, ctx);
    }

    fn should_start_update_check(&self) -> bool {
        !matches!(
            self.stage,
            AutoupdateStage::CheckingForUpdate
                | AutoupdateStage::DownloadingUpdate
                | AutoupdateStage::UpdatedPendingRestart { .. }
        )
    }

    /// Trigger the update check to /client_version/daily, but only go through with sending the
    /// request if we haven't done that today.
    pub fn maybe_daily_check_for_update(&mut self, ctx: &mut ModelContext<Self>) {
        self.enqueue_request(RequestType::DailyCheck, ctx)
    }

    /// Check if an update is available.
    ///
    /// The caller is responsible for checking that we _should_ check for an update. Generally, the
    /// only caller should be [`Self::try_execute_request`].
    fn check_for_update(&mut self, request_type: RequestType, ctx: &mut ModelContext<Self>) {
        let current_date = DateTime::now().date_naive();
        let is_daily = self.should_make_daily_request(
            request_type,
            &current_date,
            ctx.windows().app_is_active(),
        );

        // Other RequestTypes will fallback to hitting `/client_version`, but DailyCheck will not.
        if request_type == RequestType::DailyCheck && !is_daily {
            return;
        }

        self.stage = AutoupdateStage::CheckingForUpdate;
        ctx.notify();

        let server_api = self.server_api.clone();
        ctx.spawn(
            async move {
                let update_id = new_update_id();
                let channel = ChannelState::channel();
                log::info!("Checking for update on channel {channel}. Update id is {update_id}");
                let version = fetch_version(&channel, is_daily, &update_id, server_api)
                    .await
                    .context("Error checking for new version");
                report_if_error!(version);
                (update_id, version)
            },
            move |me, (update_id, version), ctx| {
                me.on_update_check_complete(request_type, update_id, version, is_daily, ctx);
            },
        );
    }

    // Only make this the `/client_version/daily` request if:
    //   1. We haven't yet done it today.
    //   2. The app is currently active during this polling period, or if we've explicitly
    //      triggered the daily request rather than waiting for the poll interval. Note that
    //      the app will not be considered active if the system is sleeping.
    fn should_make_daily_request(
        &self,
        request_type: RequestType,
        current_date: &NaiveDate,
        app_is_active: bool,
    ) -> bool {
        let daily_check_still_todo =
            self.last_successful_daily_update_check
                .is_none_or(|last_check_datetime| {
                    let last_check_date = last_check_datetime.date_naive();
                    // Is today's date incremented from the date of the last check?
                    *current_date > last_check_date
                });

        daily_check_still_todo
            && ((request_type == RequestType::Poll && app_is_active)
                || request_type == RequestType::ManualCheck
                || request_type == RequestType::DailyCheck)
    }

    /// Given a newly-available version, check if we should update.
    fn should_update(&mut self, version: VersionInfo, update_id: String) -> UpdateReady {
        let current_version = match ChannelState::app_version() {
            Some(version) => version,
            None => {
                log::info!("No version tag set, cannot autoupdate");
                return UpdateReady::No;
            }
        };

        // In case the version didn't update, but the soft cutoff did,
        // let's make sure that's reflected in the last downloaded update that we continue to use.
        if let Some(download) = self.downloaded_update.as_mut() {
            download
                .version
                .soft_cutoff
                .clone_from(&version.soft_cutoff);
            download.version.update_by = version.update_by;
        }

        if version.version == current_version {
            log::info!("Already up to date with {}", version.version);
            UpdateReady::No
        } else {
            if let Ok(true) =
                self.is_current_version_ahead_of_latest_version(&version, current_version)
            {
                let is_rollback = version.is_rollback.unwrap_or(false);
                if !is_rollback {
                    log::info!(
                        "Current version ({}) is ahead of version in channel versions({}), not updating",
                        current_version,
                        version.version
                    );
                    return UpdateReady::No;
                }
            }

            // We should update - the only thing left to do is check if this version is already
            // downloaded.
            match &self.downloaded_update {
                Some(downloaded_update) if downloaded_update.version.version == version.version => {
                    // This case occurs if a check runs after an update has already been
                    // downloaded, but not yet applied, i.e. when opening a new window.
                    log::info!(
                        "Already downloaded {} in update_id {}",
                        version.version,
                        downloaded_update.update_id
                    );
                    UpdateReady::Yes {
                        new_version: downloaded_update.version.clone(),
                        update_id: downloaded_update.update_id.clone(),
                    }
                }
                _ => {
                    // Either we haven't downloaded any updates or we've downloaded a different
                    // version.
                    UpdateReady::CanDownload {
                        new_version: version,
                        update_id,
                    }
                }
            }
        }
    }

    /// Returns whether the current version is ahead of the version reported by the server as the "latest" version
    /// in channel versions.
    fn is_current_version_ahead_of_latest_version(
        &self,
        new_version: &VersionInfo,
        current_version: &str,
    ) -> Result<bool> {
        let current_version = ParsedVersion::try_from(current_version)?;
        let new_version = ParsedVersion::try_from(new_version.version.as_str())?;
        Ok(current_version > new_version)
    }

    fn on_update_check_complete(
        &mut self,
        request_type: RequestType,
        update_id: String,
        version: Result<VersionInfo>,
        is_daily: bool,
        ctx: &mut ModelContext<AutoupdateState>,
    ) {
        if is_daily && version.is_ok() {
            self.last_successful_daily_update_check = Some(DateTime::now());
        }

        // If one update was already applied, we cannot apply another.
        if matches!(self.stage, AutoupdateStage::UpdatedPendingRestart { .. }) {
            return;
        }

        let update_available = version.map(|version| self.should_update(version, update_id));
        match &update_available {
            Ok(UpdateReady::CanDownload {
                new_version,
                update_id,
            }) => {
                self.download_new_update(update_id.clone(), request_type, new_version.clone(), ctx);
                // We report the update status after attempting to download the update.
                return;
            }
            Ok(UpdateReady::Yes {
                new_version,
                update_id,
            }) => {
                // UpdateReady::Yes means the update has already been downloaded.
                //
                // If so, and we're already in AutoupdateStage::UpdateReady for this version, that
                // means we'd previously downloaded and unpacked the update but did not restart.
                // We can use the existing download, which will have its own update_id.
                //
                // If we're in a different state, then we downloaded the update but haven't
                // reported it as ready yet.
                // TODO(ben): I'm not sure this state is reachable, try simplifying along with
                // removing DOWNLOADED_UPDATE.
                let already_checked_for_update = matches!(&self.stage,
                    AutoupdateStage::UpdateReady {
                        new_version: existing_new_version,
                        ..
                    } if new_version == existing_new_version);
                if already_checked_for_update {
                    log::info!("Already downloaded update for {}", new_version.version);
                } else {
                    self.stage = AutoupdateStage::UpdateReady {
                        new_version: new_version.clone(),
                        update_id: update_id.clone(),
                    };
                }
                ctx.emit(AutoupdateStateEvent::UpdateAvailable);
            }
            Ok(UpdateReady::No) => {
                self.stage = AutoupdateStage::NoUpdateAvailable;
                log::info!("No update available");
            }
            Err(ref e) => {
                // We commonly get errors as the autoupdate code runs when a laptop wakes up
                // briefly while asleep, but the network call to check for updates gets cancelled
                // when returning to sleep. So we fail silently and wait for the next update poll.
                self.stage = AutoupdateStage::NoUpdateAvailable;
                log::warn!("Error checking for update {e:#}");
            }
        };

        self.on_check_complete(update_available, request_type, ctx);
    }

    fn download_new_update(
        &mut self,
        update_id: String,
        request_type: RequestType,
        new_version: VersionInfo,
        ctx: &mut ModelContext<AutoupdateState>,
    ) {
        self.stage = AutoupdateStage::DownloadingUpdate;
        ctx.notify();

        // We're downloading `new_version` as update `update_id`.
        //
        // In case we previously downloaded an update and haven't applied it yet,
        // make sure not to clean up the last successful download as part of this one.
        // If the user applies that update while we're downloading this one, we don't
        // want to have cleaned it up.
        let last_successful_update_id =
            self.downloaded_update.as_ref().map(|d| d.update_id.clone());
        let _ = ctx.spawn(
            download_update(
                new_version.clone(),
                update_id.clone(),
                last_successful_update_id,
                self.server_api.clone(),
            ),
            move |autoupdate_state, download_ready, ctx| {
                autoupdate_state.on_download_update_complete(
                    request_type,
                    new_version,
                    update_id,
                    download_ready,
                    ctx,
                )
            },
        );
    }

    fn on_download_update_complete(
        &mut self,
        request_type: RequestType,
        new_version: VersionInfo,
        update_id: String,
        download_ready: Result<DownloadReady>,
        ctx: &mut ModelContext<AutoupdateState>,
    ) {
        let was_update_available = match download_ready {
            Ok(DownloadReady::Yes) => {
                self.clear_old_autoupdate_dirs(&update_id, ctx);
                self.downloaded_update = Some(DownloadedUpdate {
                    version: new_version.clone(),
                    update_id: update_id.clone(),
                });
                self.stage = AutoupdateStage::UpdateReady {
                    new_version: new_version.clone(),
                    update_id: update_id.clone(),
                };
                log::info!(
                    "Downloaded update to {} at update ID {update_id}",
                    new_version.version
                );
                Ok(UpdateReady::Yes {
                    new_version,
                    update_id,
                })
            }
            Ok(DownloadReady::NeedsAuthorization) => {
                send_telemetry_from_ctx!(TelemetryEvent::UnableToAutoUpdateToNewVersion, ctx);
                self.stage = AutoupdateStage::UnableToUpdateToNewVersion { new_version };
                Ok(UpdateReady::No)
            }
            Ok(DownloadReady::No) => {
                log::info!("Could not download a newer version");
                self.stage = AutoupdateStage::NoUpdateAvailable;
                Ok(UpdateReady::No)
            }
            Err(e) => {
                log::warn!("Error downloading update {e:#}");
                // We commonly get errors as the autoupdate code runs when a laptop wakes up
                // briefly while asleep, but the network call to download gets cancelled when
                // returning to sleep. So we fail silently and wait for the next update poll.
                self.stage = AutoupdateStage::NoUpdateAvailable;
                Err(e)
            }
        };

        ctx.emit(AutoupdateStateEvent::UpdateAvailable);
        self.on_check_complete(was_update_available, request_type, ctx);
    }

    /// Clean up all old autoupdate directories except the current one.
    #[cfg_attr(not(target_os = "macos"), expect(unused_variables))]
    fn clear_old_autoupdate_dirs(&self, update_id: &str, ctx: &mut ModelContext<Self>) {
        #[cfg(target_os = "macos")]
        {
            let update_id_owned = update_id.to_owned();
            ctx.spawn(
                async move {
                    // Clean up all autoupdate directories except any current ones
                    mac::cleanup_all_except(Some(&update_id_owned)).await;
                },
                |_, _, _| {},
            );
        }
    }

    fn on_check_complete(
        &mut self,
        update_available: Result<UpdateReady>,
        request_type: RequestType,
        ctx: &mut ModelContext<AutoupdateState>,
    ) {
        if let Some(content) = accessibility_content(&update_available, request_type) {
            ctx.emit_a11y_content(content);
        }

        ctx.emit(AutoupdateStateEvent::CheckComplete {
            result: update_available,
            request_type,
        });
        ctx.notify();

        // A request might've gotten queued while this last one was in-flight. This point is when
        // we'd begin the next one.
        self.try_execute_request(ctx);
    }

    // Reset the most-recently-downloaded update.
    #[cfg_attr(not(target_os = "macos"), expect(dead_code))]
    fn clear_downloaded_update(&mut self, update_id: &str, ctx: &mut ModelContext<Self>) {
        if self
            .downloaded_update
            .as_ref()
            .is_some_and(|update| update.update_id == update_id)
        {
            self.downloaded_update = None;
            ctx.notify();
        }
    }

    /// Set the current autoupdate stage. This must *only* be called from within the [`autoupdate`]
    /// module to correctly maintain the update state machine.
    fn set_autoupdate_stage(&mut self, stage: AutoupdateStage, ctx: &mut ModelContext<Self>) {
        self.stage = stage;
        ctx.notify();
    }

    /// Record that we did not successfully relaunch to update.
    fn set_unable_to_launch_state(
        &mut self,
        get_next_stage: fn(VersionInfo, String) -> AutoupdateStage,
        ctx: &mut ModelContext<Self>,
    ) {
        match &self.stage {
            // If we were relaunching into an already-applied update, we should stay in that state
            // for the *next* relaunch to open it.
            AutoupdateStage::UpdatedPendingRestart { .. } => (),
            // If we were relaunching in the middle of an update, and that failed, use the callback to
            // decide what to do next.
            AutoupdateStage::Updating {
                new_version,
                update_id,
            } => {
                let next_stage = get_next_stage(new_version.clone(), update_id.clone());
                self.set_autoupdate_stage(next_stage, ctx);
            }
            _ => {
                log::warn!(
                    "Tried to set the autoupdate state after a relaunch was cancelled, but the previous state was not Updating."
                );
            }
        }
    }

    /// Mark both the autoupdate stage and the relaunch status as failed.
    fn relaunch_failed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_unable_to_launch_state(
            |new_version, _update_id| AutoupdateStage::UnableToLaunchNewVersion { new_version },
            ctx,
        );
        RelaunchModel::handle(ctx).update(ctx, |me, ctx| {
            me.relaunch_status = RelaunchStatus::Failed;
            ctx.notify();
        });
    }
}

/// The set of events that are emitted from the AutoupdateState model.
pub enum AutoupdateStateEvent {
    /// Emitted when an update check has finished.
    CheckComplete {
        /// Result of the check of whether there is an update available.
        result: Result<UpdateReady>,
        /// Type of request that this check references.
        request_type: RequestType,
    },
    /// Emitted when an update is available.
    UpdateAvailable,
}

impl Entity for AutoupdateState {
    type Event = AutoupdateStateEvent;
}

impl SingletonEntity for AutoupdateState {}

/// Set of results from an update check.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateReady {
    /// An update is available and has been downloaded.
    Yes {
        /// The version that has been downloaded.
        new_version: VersionInfo,
        /// Nonce used to identify this update check.
        update_id: String,
    },
    /// An update is available but not yet downloaded.
    CanDownload {
        /// The available version to update to.
        new_version: VersionInfo,
        /// Nonce used to identify this update check.
        update_id: String,
    },
    /// There is no update available.
    No,
}

/// Set of results from downloading an update.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum DownloadReady {
    /// The update was downloaded successfully.
    Yes,
    /// There were insufficient permissions to download the update.
    #[cfg_attr(windows, allow(dead_code))]
    NeedsAuthorization,
    /// A newer version could not be downloaded.
    No,
}

/// Whether or not we're ready to relaunch the app after the user requests that
/// we apply an update.
///
/// This exists for Linux, when the app is installed via a package manager.
/// Instead of immediately applying the update, we open a new tab and populate
/// the input field with the command the user needs to run to install the
/// update via their package manager.  After the update completes, we send
/// ourselves a signal (via a DCS hook) that the update has completed and we're
/// ready to relaunch.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum ReadyForRelaunch {
    Yes,
    #[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestType {
    /// For when the user triggers the check manually in the settings page.
    ManualCheck,
    /// We automatically poll for updates every AUTOUPDATE_POLL. This can also trigger the daily
    /// request to /client_version/daily if it hasn't been done yet today.
    Poll,
    /// Only go through with the check to /client_version/daily if it hasn't been done yet today.
    /// Otherwise, abort the check. This is useful if we want to eagerly send the check b/c we
    /// don't want to wait for the next polling interval. Relying on the polling interval alone to
    /// send the daily checks could lead to under-counting.
    DailyCheck,
}

// We only want to announce autoupdates when there's manual check. Otherwise, the autoupdate check
// may clash with other announcements, such as log in form or referral form.
// Users will still get the autoupdate on the next relaunch anyways, so for now it's ok.
pub fn accessibility_content(
    update_available: &Result<UpdateReady>,
    request_type: RequestType,
) -> Option<AccessibilityContent> {
    match (request_type, update_available) {
        // Found autoupdate
        (RequestType::ManualCheck, Ok(UpdateReady::Yes { .. })) => Some(AccessibilityContent::new(
            "Update available.",
            "Use the command palette to install and relaunch Warp",
            WarpA11yRole::HelpRole,
        )),
        // Any non-successful autoupdate check
        (RequestType::ManualCheck, _) => Some(AccessibilityContent::new_without_help(
            "No updates available",
            WarpA11yRole::HelpRole,
        )),
        _ => None,
    }
}

pub fn get_update_state(app: &AppContext) -> AutoupdateStage {
    AutoupdateState::as_ref(app).stage.clone()
}

fn get_curr_parsed_version() -> Option<ParsedVersion> {
    let curr_version = ChannelState::app_version();
    curr_version.and_then(|v| ParsedVersion::try_from(v).ok())
}

/// Generate a new random update ID.
fn new_update_id() -> String {
    let mut rng = rand::thread_rng();
    std::iter::repeat(())
        .map(|()| rng.sample(rand::distributions::Alphanumeric))
        .map(char::from)
        .take(7)
        .collect()
}

/// Fetch the current version on the given channel.
async fn fetch_version(
    channel: &Channel,
    is_daily: bool,
    update_id: &str,
    server_api: Arc<ServerApi>,
) -> Result<VersionInfo> {
    let versions = fetch_channel_versions(update_id, server_api.clone(), false, is_daily).await?;

    let channel_version = match channel {
        Channel::Stable => versions.stable,
        Channel::Preview => versions.preview,
        Channel::Dev => versions.dev,
        Channel::Integration | Channel::Local | Channel::Oss => {
            // These channels don't ship release artifacts, so there's no
            // version to fetch. This branch is normally unreachable because
            // `AutoupdateState::register` gates the poll loop on the
            // `Autoupdate` feature flag, but builds (e.g. local wasm bundles)
            // can end up with `Autoupdate` enabled while running on one of
            // these channels. Return an error rather than panicking so the
            // poll loop just logs and bails.
            anyhow::bail!(
                "Local, integration, and open-source channel binaries don't support autoupdate"
            );
        }
    };
    let version_info = channel_version.version_info();
    Ok(version_info)
}

// This method is unimplemented on wasm, so we allow unused variables.
#[cfg_attr(target_family = "wasm", allow(unused_variables))]
async fn download_update(
    version_info: VersionInfo,
    update_id: String,
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    last_successful_update_id: Option<String>,
    server_api: Arc<ServerApi>,
) -> Result<DownloadReady> {
    if ChannelState::app_version().is_none() {
        log::info!("No tag set, not performing autoupdate.");
        return Ok(DownloadReady::No);
    }

    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            mac::download_update_and_cleanup(&version_info, &update_id, last_successful_update_id.as_deref(), server_api.http_client()).await
        } else if #[cfg(target_os = "linux")] {
            linux::download_update_and_cleanup(&version_info, &update_id, server_api.http_client()).await
        } else if #[cfg(windows)] {
            windows::download_update_and_cleanup(&version_info, &update_id, server_api.http_client()).await
        } else {
            Err(anyhow::anyhow!("Not implemented"))
        }
    }
}

/// Apply a downloaded update. If this returns `Ok(ReadyForRelaunch::Yes)`, then the app should be
/// relaunched to complete the update. If this returns `Ok(ReadyForRelaunch::No)`, then more user
/// action is needed to apply the update, and the app shouldn't relaunch automatically.
///
/// The timing of how updates are applied is very platform-specific:
/// * On macOS, updates are applied asynchronously, _immediately_ before relaunching. This always
///   returns [`ReadyForRelaunch::Yes`].
/// * On Windows, updates are applied by a separate installer process, which is spawned
///   [just before the app terminates](spawn_child_if_necessary).
/// * On Linux, if using a package manager, we ask the user to install the update via their package
///   manager, and do not relaunch until that's complete. This returns [`ReadyForRelaunch::No`].
pub fn apply_update(
    _initiating_workspace: &mut Workspace,
    _ctx: &mut ViewContext<Workspace>,
) -> Result<ReadyForRelaunch> {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "macos", windows))] {
            // macOS applies the update during the download step. Windows does it during
            // `spawn_child_if_necessary`. In either case, simply continue relaunching the app.
            Ok(ReadyForRelaunch::Yes)
        } else if #[cfg(target_os = "linux")] {
            let AutoupdateStage::UpdateReady { update_id, .. } = &AutoupdateState::handle(_ctx).as_ref(_ctx).stage else {
                anyhow::bail!("Trying to apply an update without AutoupdateState being UpdateReady!");
            };
            let update_id = update_id.clone();
            linux::apply_update(_initiating_workspace, &update_id, _ctx)
        } else {
            anyhow::bail!("Not implemented")
        }
    }
}

/// Relaunch Warp to apply an update.
///
/// This will:
/// 1. Perform any last update steps.
/// 2. Request a relaunch.
/// 3. Terminate the app.
pub fn initiate_relaunch_for_update(app: &mut AppContext) {
    let autoupdate_stage = &AutoupdateState::as_ref(app).stage;

    match autoupdate_stage {
        AutoupdateStage::UpdatedPendingRestart { .. } => {
            // The update was already fully applied, so all that's left to do is relaunch.
            log::info!("Relaunching to apply update");
            RelaunchModel::handle(app).update(app, RelaunchModel::request_relaunch);
            app.terminate_app(TerminationMode::Cancellable, None);
        }
        AutoupdateStage::UpdateReady {
            new_version,
            update_id,
        } => {
            // There's a pending update, and we haven't finished applying it.
            let new_version = new_version.clone();
            let new_version_string = new_version.version.clone();
            let update_id = update_id.clone();

            // First, record that we're applying an update.
            AutoupdateState::handle(app).update(app, |autoupdate_state, ctx| {
                autoupdate_state.set_autoupdate_stage(
                    AutoupdateStage::Updating {
                        new_version,
                        update_id,
                    },
                    ctx,
                );
            });

            // Record that we should relaunch the app after terminating.
            RelaunchModel::handle(app).update(app, RelaunchModel::request_relaunch);

            // If there are any update steps that are deferred until relaunching, perform them now
            // (this is only true on macOS). We do this *before* requesting termination so that, if
            // the update fails, we can display a workspace banner to the user.
            finalize_update(app, move |result, app| {
                if result.is_err() {
                    // finalize_update reports the error itself.
                    return;
                }

                // Report that we're attempting to relaunch for an update, so that we can track failed
                // relaunches (e.g. if the update got corrupted). This is sent synchronously because
                // the app is about to quit.
                let event = TelemetryEvent::AutoupdateRelaunchAttempt {
                    new_version: new_version_string,
                };
                send_telemetry_sync_from_app_ctx!(event, app);

                // Request termination of the app.
                app.terminate_app(TerminationMode::Cancellable, None);
            });
        }
        _ => {
            log::info!("No update ready to install, not relaunching");
        }
    }
}

/// Apply a pending update without relaunching. This is called at shutdown in case the user quit
/// without updating. Returns `true` if there was a pending update to apply.
///
/// The callback is invoked once the update is complete (whether or not it was successful). It is
/// *not* called if there was no update.
pub fn apply_pending_update<F>(app: &mut AppContext, on_update_complete: F) -> bool
where
    F: FnOnce(&mut AppContext) + Send + 'static,
{
    let has_update = AutoupdateState::handle(app).update(app, |autoupdate_state, ctx| {
        if let AutoupdateStage::UpdateReady {
            new_version,
            update_id,
        } = &autoupdate_state.stage
        {
            let new_stage = AutoupdateStage::Updating {
                new_version: new_version.clone(),
                update_id: update_id.clone(),
            };
            autoupdate_state.set_autoupdate_stage(new_stage, ctx);
            true
        } else {
            false
        }
    });

    if !has_update {
        return false;
    }

    finalize_update(app, move |_result, app| on_update_complete(app));
    true
}

/// Perform any autoupdate steps that must be deferred until we're about to relaunch.
///
/// Returns `true` if there was an autoupdate to apply; `false` otherwise.
///
/// These steps may involve expensive operations and async work, so the caller must provide a
/// completion callback.
fn finalize_update<F>(app: &mut AppContext, callback: F)
where
    F: FnOnce(Result<()>, &mut AppContext) + Send + 'static,
{
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            mac::apply_update_async(app, |autoupdate_state, result, ctx| {
                match result {
                    Ok(maybe_new_version) => {
                        if let Some(new_version) = maybe_new_version {
                            log::info!("Pending update applied successfully");
                            // Record that this update was applied, so we don't reattempt it.
                            autoupdate_state.set_autoupdate_stage(AutoupdateStage::UpdatedPendingRestart { new_version }, ctx);
                        }
                        callback(Ok(()), ctx);
                    },
                    Err(err) => {
                        autoupdate_state.relaunch_failed(ctx);

                        let err = anyhow!(err).context("Error applying installed update");
                        crate::report_error!(&err);
                        callback(Err(err), ctx);
                    }
                }
            });
        } else {
            callback(Ok(()), app);
        }
    }
}

pub fn cancel_relaunch(app: &mut AppContext) {
    let previous_status = RelaunchModel::handle(app).update(app, RelaunchModel::cancel_relaunch);

    if previous_status == RelaunchStatus::Requested {
        AutoupdateState::handle(app).update(app, |autoupdate_state, ctx| {
            autoupdate_state.set_unable_to_launch_state(
                |new_version: VersionInfo, update_id: String| AutoupdateStage::UpdateReady {
                    new_version,
                    update_id,
                },
                ctx,
            );
        });
    }
}

pub fn spawn_child_if_necessary(app: &mut AppContext) {
    let relaunch_handle = RelaunchModel::handle(app);
    let status = relaunch_handle.as_ref(app).relaunch_status;

    // TODO: We'd ideally call finalize_update here. Otherwise, if an update was downloaded and the
    // user restarts normally, the update won't be applied. However, finalize_update is async and
    // we don't necessarily want to block termination on it.

    if status == RelaunchStatus::Requested {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "macos")] {
                let relaunch_status = mac::relaunch();
            } else if #[cfg(target_os = "linux")] {
                let relaunch_status = linux::relaunch();
            } else if #[cfg(windows)] {
                let relaunch_status = windows::relaunch();
            } else {
                let relaunch_status: Result<()> = Err(anyhow!("No autoupdate support on this platform!"));
            }
        }
        match relaunch_status {
            Ok(_) => {
                log::info!("Terminating app for relaunch. Bye!");
            }
            Err(e) => {
                log::error!("Error relaunching app after autoupdate: {e:?}");
                AutoupdateState::handle(app).update(app, |autoupdate_state, ctx| {
                    autoupdate_state.relaunch_failed(ctx);
                });
            }
        }
    }
}

pub fn manually_download_new_version(ctx: &mut AppContext) {
    match get_update_state(ctx) {
        AutoupdateStage::UnableToUpdateToNewVersion { new_version }
        | AutoupdateStage::UnableToLaunchNewVersion { new_version } => {
            manually_download_version(&ChannelState::channel(), &new_version, ctx)
        }
        _ => {
            log::warn!(
                "Tried to manually download update in the wrong autoupdate state, skipping."
            );
        }
    }
}

#[allow(unused_variables)]
fn manually_download_version(channel: &Channel, version: &VersionInfo, ctx: &mut AppContext) {
    #[cfg(target_os = "macos")]
    mac::manually_download_version(channel, version, ctx);
}

pub(crate) fn check_and_report_update_errors(_ctx: &mut AppContext) {
    #[cfg(windows)]
    windows::check_and_report_update_errors(_ctx);
}

pub fn remove_old_executable() -> Result<()> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            mac::remove_old_executable()
        } else if #[cfg(any(target_os = "linux", windows))] {
            // Nothing to do on Linux or Windows; we don't leave anything behind to clean up after
            // a relaunch.
            Ok(())
        } else if #[cfg(target_family = "wasm")] {
            // Nothing to do on web. There's no executables stored somewhere.
            Ok(())
        } else {
            Err(anyhow::anyhow!("Not implemented"))
        }
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum RelaunchStatus {
    #[default]
    None,
    Requested,
    Failed,
}

#[derive(Clone, Copy, Default)]
pub struct RelaunchModel {
    relaunch_status: RelaunchStatus,
}

impl RelaunchModel {
    pub fn new() -> Self {
        Default::default()
    }

    /// Request relaunching the app to apply an update.
    ///
    /// When terminating the app, we check this state to know whether to launch the updated version
    /// or quit the app normally.
    fn request_relaunch(&mut self, ctx: &mut ModelContext<Self>) {
        self.relaunch_status = RelaunchStatus::Requested;
        ctx.notify();
    }

    /// Cancels a requested relaunch, if there was one. Returns the previous relaunch status.
    fn cancel_relaunch(&mut self, ctx: &mut ModelContext<Self>) -> RelaunchStatus {
        let previous_status = self.relaunch_status;
        self.relaunch_status = RelaunchStatus::None;
        ctx.notify();
        previous_status
    }
}

impl Entity for RelaunchModel {
    type Event = ();
}

impl SingletonEntity for RelaunchModel {}

pub fn is_incoming_version_past_current(version: Option<&str>) -> bool {
    let installed_version = get_curr_parsed_version();

    let Ok(incoming_version): Result<ParsedVersion> = version
        .ok_or(anyhow!("version is None"))
        .and_then(|cutoff| cutoff.try_into())
    else {
        return false;
    };

    installed_version.is_some_and(|curr_version| incoming_version > curr_version)
}

/// Returns the base URL that contains release assets for the given version
/// of this app bundle.
fn release_assets_directory_url(channel: Channel, version: &str) -> String {
    let releases_base_url = ChannelState::releases_base_url();
    match channel {
        Channel::Stable => {
            format!("{releases_base_url}/stable/{version}")
        }
        Channel::Preview => {
            format!("{releases_base_url}/preview/{version}")
        }
        Channel::Dev => format!("{releases_base_url}/dev/{version}"),
        Channel::Local | Channel::Integration | Channel::Oss => {
            unreachable!("local/integration/oss autoupdate not supported");
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
