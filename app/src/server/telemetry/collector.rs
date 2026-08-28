use std::fs::remove_file;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::{LocalResult, TimeZone, Utc};
use warp_core::execution_mode::AppExecutionMode;
use warp_errors::{report_error, report_if_error};
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{App, Entity, ModelContext, SingletonEntity};

use super::{RUDDER_TELEMETRY_EVENTS_FILE_NAME, clear_event_queue, rudder_event_file_path};
use crate::auth::AuthStateProvider;
use crate::channel::ChannelState;
use crate::features::FeatureFlag;
use crate::server::server_api::ServerApi;
use crate::settings::{PrivacySettings, PrivacySettingsChangedEvent};

// How often we send Active Usage signals.
const ACTIVE_USAGE_DURATION: Duration = Duration::from_secs(60);

/// Duration to wait before flushing the event queue to Rudderstack.
const TELEMETRY_FLUSH_DURATION: Duration = Duration::from_secs(30);

/// Max telemetry events to write to disk. This is bounded to limit the size of the file as well
/// as latency of writing the file.
const MAX_TELEMETRY_EVENTS_TO_STORE: usize = 20;

/// Maximum time to wait for the telemetry flush network request during shutdown.
/// If the network is unavailable or slow, we don't want the CLI process to hang indefinitely.
const TELEMETRY_SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// App singleton responsible for scheduling periodic background tasks for sending batches of
/// telemetry events to Rudderstack.  This model respects the user's telemetry enablement setting.
pub struct TelemetryCollector {
    server_api: Arc<ServerApi>,
}

impl TelemetryCollector {
    pub fn new(server_api: Arc<ServerApi>) -> Self {
        Self { server_api }
    }

    pub fn initialize_telemetry_collection(&self, ctx: &mut ModelContext<TelemetryCollector>) {
        // Start a background thread to periodically flush events from the telemetry event queue.
        if ChannelState::is_release_bundle() || FeatureFlag::WithSandboxTelemetry.is_enabled() {
            // Flush the events to Rudderstack that were persisted into a file the last time the app was
            // quit.
            self.flush_persisted_events_from_disk(ctx);
        }

        // Send Active App Usage signals
        if FeatureFlag::RecordAppActiveEvents.is_enabled()
            && (ChannelState::is_release_bundle() || FeatureFlag::WithSandboxTelemetry.is_enabled())
        {
            self.schedule_send_active_usage_event(ctx);
        }

        // Start a background thread to periodically flush events from the telemetry event queue.
        if ChannelState::is_release_bundle()
            || FeatureFlag::WithSandboxTelemetry.is_enabled()
            || FeatureFlag::SendTelemetryToFile.is_enabled()
        {
            self.schedule_event_queue_flush(ctx);
        }

        // Clear queued telemetry events when telemetry is enabled or disabled. If telemetry is
        // enabled, we will start sending Rudderstack requests when the event queue is periodically
        // flushed. The initial request should not contain any events recorded when the user was
        // previously opted-out of telemetry. In the case where the user turns the telemetry from
        // on to off, we should not send another request with any telemetry, even if the event was
        // initially recorded prior to the user turning telemetry off.`
        ctx.subscribe_to_model(&PrivacySettings::handle(ctx), |_me, _, event, _ctx| {
            if let PrivacySettingsChangedEvent::UpdateIsTelemetryEnabled { .. } = event {
                clear_event_queue();
            }
        });
    }

    /// Writes all queued but unsent telemetry telemetry events to disk so that they may be sent
    /// on the next app startup.
    pub fn write_telemetry_events_to_disk(&self, ctx: &mut ModelContext<TelemetryCollector>) {
        match self.server_api.persist_telemetry_events(
            MAX_TELEMETRY_EVENTS_TO_STORE,
            PrivacySettings::as_ref(ctx).get_snapshot(ctx),
        ) {
            Ok(()) => {
                log::info!("Successfully wrote telemetry events to disk")
            }
            Err(e) => {
                report_error!(e.context("Failed to write telemetry events to disk"));
            }
        }
    }

    /// Flushes telemetry events when the app is shutting down.
    ///
    /// Depending on the app's execution mode, this will either:
    /// * Write events to disk, for sending on the next app startup
    /// * Synchronously send events to rudderstack
    pub fn flush_telemetry_events_for_shutdown(&self, ctx: &mut ModelContext<TelemetryCollector>) {
        let execution_mode = AppExecutionMode::as_ref(ctx);

        if execution_mode.send_telemetry_at_shutdown() {
            let privacy_settings_snapshot = PrivacySettings::as_ref(ctx).get_snapshot(ctx);
            let server_api = self.server_api.clone();
            match warpui::r#async::block_on(async move {
                server_api
                    .flush_telemetry_events(privacy_settings_snapshot)
                    .with_timeout(TELEMETRY_SHUTDOWN_FLUSH_TIMEOUT)
                    .await
            }) {
                Ok(Ok(count)) => {
                    if count > 0 {
                        log::info!("Successfully flushed telemetry events before shutdown");
                    }
                }
                Ok(Err(e)) => {
                    report_error!(e.context("Error flushing telemetry events before shutdown"));
                }
                Err(_) => {
                    log::warn!(
                        "Telemetry flush timed out after {}s during shutdown, skipping",
                        TELEMETRY_SHUTDOWN_FLUSH_TIMEOUT.as_secs()
                    );
                }
            }
        } else {
            self.write_telemetry_events_to_disk(ctx);
        }
    }

    /// Sends rudderstack requests containing events persisted to disk (if telemetry is enabled).
    /// Events may be written to disk at the end of a session prior to app termination; this
    /// function should be called on startup to track events that were recorded at the end of the
    /// last session and were not flushed.
    fn flush_persisted_events_from_disk(&self, ctx: &mut ModelContext<TelemetryCollector>) {
        let privacy_settings_snapshot = PrivacySettings::as_ref(ctx).get_snapshot(ctx);
        let server_api = self.server_api.clone();
        let _ = ctx.spawn(
            async move {
                let new_path = rudder_event_file_path();
                let old_path =
                    warp_core::paths::state_dir().join(RUDDER_TELEMETRY_EVENTS_FILE_NAME);

                // Try flushing from both new and legacy locations.
                for path in [new_path, old_path] {
                    report_if_error!(
                        server_api
                            .flush_persisted_events_to_rudder(&path, privacy_settings_snapshot)
                            .await
                            .context("Failed to flush rudder events from disk")
                    );
                    // Remove the file regardless of outcome  of flushing the events to avoid the
                    // case where we accidentally try to re-flush the events on the next app startup.
                    if let Err(e) = remove_file(&path)
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        report_error!(
                            anyhow::anyhow!(e).context("Failed to remove persisted event file")
                        );
                    }
                }
            },
            |_, _, _| (),
        );
    }

    /// Schedules a background task to send an active usage event in a rudderstack request if
    /// telemetry is enabled. The scheduled task once again schedules itself after
    /// `ACTIVE_USAGE_DURATION`.
    fn schedule_send_active_usage_event(&self, ctx: &mut ModelContext<TelemetryCollector>) {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let is_telemetry_enabled = PrivacySettings::as_ref(ctx).is_telemetry_enabled;
        let _ = ctx.spawn(
            async move {
                // Record app active if there was any activity now or right after the previous check
                let last_active_timestamp = App::last_active_timestamp();
                if is_telemetry_enabled
                    && last_active_timestamp + ACTIVE_USAGE_DURATION.as_secs() as i64
                        > Utc::now().timestamp()
                    && let LocalResult::Single(timestamp) =
                        Utc.timestamp_opt(last_active_timestamp, 0)
                {
                    warpui::telemetry::record_app_active_event(
                        auth_state.user_id().map(|uid| uid.as_string()),
                        auth_state.anonymous_id(),
                        timestamp,
                    );
                }
                Timer::after(ACTIVE_USAGE_DURATION).await;
            },
            |me, _, ctx| me.schedule_send_active_usage_event(ctx),
        );
    }

    /// Flushes events from the in-memory event queue and schedules a background task to send
    /// them in rudderstack request if telemetry is enabled. The scheduled task once again schedules
    /// itself after `TELEMETRY_FLUSH_DURATION`.
    fn schedule_event_queue_flush(&self, ctx: &mut ModelContext<TelemetryCollector>) {
        let server_api = self.server_api.clone();
        let privacy_settings_snapshot = PrivacySettings::as_ref(ctx).get_snapshot(ctx);
        let _ = ctx.spawn(
            async move {
                match server_api
                    .flush_telemetry_events(privacy_settings_snapshot)
                    .await
                {
                    Ok(count) => {
                        if count > 0 {
                            log::debug!("Flushed telemetry events.");
                        }
                    }
                    Err(e) => {
                        log::info!("Failed to flush events from Telemetry queue: {e}");
                    }
                }
                Timer::after(TELEMETRY_FLUSH_DURATION).await;
            },
            |me, _, ctx| me.schedule_event_queue_flush(ctx),
        );
    }
}

impl Entity for TelemetryCollector {
    type Event = ();
}

impl SingletonEntity for TelemetryCollector {}
