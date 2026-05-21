mod collector;
mod context;
pub mod context_provider;
mod events;
mod macros;
pub mod rudder_message;
pub mod secret_redaction;

use chrono::Utc;
pub use collector::*;
pub use context::telemetry_context;
pub use events::*;

use crate::auth::UserUid;
use crate::features::FeatureFlag;
use crate::server::telemetry::context::AttachContext;
use crate::server::telemetry_ext::TelemetryExt;
use crate::settings::PrivacySettingsSnapshot;
use crate::ChannelState;
use anyhow::Result;
use futures::FutureExt;
use rudder_message::{
    Batch as RudderBatch, BatchMessage as RudderBatchMessageWithMetadata,
    BatchMessageItem as RudderBatchMessage, Message as RudderMessage,
};
use std::fs::File;
#[cfg(not(target_family = "wasm"))]
use std::fs::OpenOptions;
use std::future::Future;
use std::path::{Path, PathBuf};
use warp_core::channel::RudderStackDestination;
use warpui::telemetry::Event;

/// Filename for file where telemetry events are written on app quit.
const RUDDER_TELEMETRY_EVENTS_FILE_NAME: &str = "rudder_telemetry_events.json";

/// Filepath where the Rudder events should be written on app quit.
fn rudder_event_file_path() -> PathBuf {
    warp_core::paths::secure_state_dir()
        .unwrap_or_else(warp_core::paths::state_dir)
        .join(RUDDER_TELEMETRY_EVENTS_FILE_NAME)
}

/// Removes all telemetry events from the app telemetry event queue.
pub fn clear_event_queue() {
    let _ = warpui::telemetry::flush_events();
}

pub struct TelemetryApi {
    pub(super) client: http_client::Client,
}

impl Default for TelemetryApi {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryApi {
    pub fn new() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(test)] {
                let client = http_client::Client::new_for_test();
            } else if #[cfg(target_family = "wasm")] {
                let client = http_client::Client::default();
            } else {
                use std::time::Duration;

                let client = http_client::Client::from_client_builder(
                    // We use our own http client directly instead of the Rudderstack SDK's because using
                    // our own client gives us the ability to have universal hooks for pre/post
                    // request/response logic.
                    reqwest::Client::builder()
                        // Don't allow insecure connections; they will be rejected by
                        // the server with a 403 Forbidden.
                        .https_only(true)
                        // Keep idle connections in the pool for up to 55s. AWS
                        // Application Load Balancers will drop idle connections after
                        // 60s and the default pool idle timeout is 90s; a pool idle
                        // timeout longer than the server timeout can lead to errors
                        // upon trying to use an idle connection.
                        .pool_idle_timeout(Duration::from_secs(55))
                        .connect_timeout(Duration::from_secs(10)),
                ).expect("Client should be constructed since we use a compatibility layer to use reqwest::Client");
            }
        }

        Self { client }
    }

    // Batches up telemetry events from the global queue and sends a Message to the Rudderstack API.
    // Returns the number of events that were flushed.
    pub async fn flush_events(&self, settings_snapshot: PrivacySettingsSnapshot) -> Result<usize> {
        let events = warpui::telemetry::flush_events();
        let event_count = events.len();

        #[cfg(not(target_family = "wasm"))]
        if FeatureFlag::SendTelemetryToFile.is_enabled() {
            self.persist_events_to_telemetry_log_file(events.clone())?;
        }

        if ChannelState::is_release_bundle() || FeatureFlag::WithSandboxTelemetry.is_enabled() {
            self.send_batch_messages_to_rudder(
                events
                    .into_iter()
                    .map(Event::to_rudder_batch_message)
                    .collect(),
                settings_snapshot,
            )
            .await?;
        }

        Ok(event_count)
    }

    /// Flushes events directly to Rudder that were previously written into a file at `path`
    /// (likely via a call to `write_events_to_disk`).
    pub async fn flush_persisted_events_to_rudder(
        &self,
        path: &Path,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        if path.exists() {
            let file = File::open(path)?;
            let events: Vec<RudderBatchMessage> = serde_json::from_reader(file)?;
            if !events.is_empty() {
                let rudder_batch_messages = events
                    .into_iter()
                    .map(|message| RudderBatchMessageWithMetadata {
                        message,
                        // We don't persist any events that contain sensitive user data.
                        contains_ugc: false,
                    })
                    .collect();
                self.send_batch_messages_to_rudder(rudder_batch_messages, settings_snapshot)
                    .await?;
                log::info!("Successfully flushed events to rudder from disk");
            }
        }
        Ok(())
    }

    /// Writes the last `max_event_count` events into disk. This is useful for persisting events
    /// where we can't make a network call to Rudder (such as when the app quits). To flush these
    /// events to Rudder, call `flush_events_to_rudder_from_disk`.
    pub fn flush_and_persist_events(
        &self,
        max_event_count: usize,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        self.flush_and_persist_events_at_path(
            max_event_count,
            settings_snapshot,
            rudder_event_file_path(),
        )
    }

    fn flush_and_persist_events_at_path(
        &self,
        max_event_count: usize,
        settings_snapshot: PrivacySettingsSnapshot,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        if settings_snapshot.should_disable_telemetry() {
            log::info!("Not writing queued events to disk because telemetry is disabled.");
            return Result::Ok(());
        }
        log::info!("Writing queued events to disk because telemetry is enabled.");

        let file = File::create(path)?;

        let events = warpui::telemetry::flush_events();
        if events.len() > max_event_count {
            log::error!("More telemetry events in queue than the limit to persist")
        }

        self.persist_events_at_path(&file, max_event_count, events)?;

        Ok(())
    }

    fn persist_events_at_path(
        &self,
        file: &File,
        max_event_count: usize,
        events: Vec<Event>,
    ) -> Result<()> {
        let rudder_events_to_persist: Vec<_> = events
            .into_iter()
            .rev()
            .take(max_event_count)
            .map(TelemetryExt::to_rudder_batch_message)
            .filter_map(|message| (!message.contains_ugc).then_some(message.message))
            .collect();
        serde_json::to_writer(file, &rudder_events_to_persist)?;
        Ok(())
    }

    #[cfg(not(target_family = "wasm"))]
    fn persist_events_to_telemetry_log_file(&self, events: Vec<Event>) -> Result<()> {
        let log_directory = warp_logging::log_directory()?;
        let telemetry_file_path = log_directory.join(&*ChannelState::telemetry_file_name());

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&telemetry_file_path)?;

        self.persist_events_at_path(&file, events.len(), events)
    }

    /// Sends a `TelemetryEvent` to the Rudderstack API.
    pub async fn send_telemetry_event(
        &self,
        user_id: Option<UserUid>,
        anonymous_id: String,
        event: impl warp_core::telemetry::TelemetryEvent,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        let event = warpui::telemetry::create_event(
            user_id.map(|uid| uid.as_string()),
            anonymous_id,
            event.name().into(),
            event.payload(),
            event.contains_ugc(),
            warpui::time::get_current_time(),
        );

        self.send_telemetry_event_internal(event, settings_snapshot)
            .await
    }

    /// Internal implementation for sending telemetry events. This reduces code size, since
    // we:
    // 1. Return a boxed future, so calling `async` functions don't need to inline this one.
    // 2. Don't have to monomorphize for each telemetry event implementation.
    fn send_telemetry_event_internal(
        &self,
        event: Event,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> impl Future<Output = Result<()>> + '_ {
        let work = async move {
            if settings_snapshot.should_disable_telemetry() {
                log::info!("Not sending telemetry event because telemetry is disabled.");
                return Result::Ok(());
            }

            #[cfg(not(target_family = "wasm"))]
            if FeatureFlag::SendTelemetryToFile.is_enabled() {
                self.persist_events_to_telemetry_log_file(vec![event.clone()])?;
            }

            if !(ChannelState::is_release_bundle()
                || FeatureFlag::WithSandboxTelemetry.is_enabled())
            {
                return Result::Ok(());
            }

            let rudder_batch = vec![event.to_rudder_batch_message()];

            let result = self
                .send_batch_messages_to_rudder(rudder_batch, settings_snapshot)
                .await;

            // This is only conditionally compiled because `is_connect` is not
            // available on wasm.  If additional checks are made against the
            // `reqwest::Error`, this condition should be performed specifically
            // against `is_connect` and not the whole loop.
            #[cfg(not(target_family = "wasm"))]
            if let Err(error) = &result {
                for cause in error.chain() {
                    if let Some(err) = cause.downcast_ref::<reqwest::Error>() {
                        if err.is_connect() {
                            log::warn!("Failed to send telemetry event: {error}");
                            return Ok(());
                        }
                    }
                }
            }

            result
        };

        // On WASM, the work future is non-Send, because the HTTP request future contains a reference to a JS
        // value (which is fine, since our WASM executor is single-threaded). On all other platforms, we must
        // return a Send future in order to use the background executor.
        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                work.boxed_local()
            } else {
                work.boxed()
            }
        }
    }

    /// Send a batch of RudderStack messages to their HTTP API.
    /// Note that the rudderanalytics SDK provides a client, but we don't
    /// use it for a few reasons:
    /// 1. It only supports a blocking HTTP client instead of an async one
    /// 2. We want to use our own HTTP client which has before/after request logging hooks
    #[cfg_attr(target_family = "wasm", allow(clippy::question_mark))]
    async fn send_batch_messages_to_rudder(
        &self,
        messages: Vec<RudderBatchMessageWithMetadata>,
        settings_snapshot: PrivacySettingsSnapshot,
    ) -> Result<()> {
        if messages.is_empty() {
            log::debug!("Dropping empty RudderStack telemetry batch");
            return Ok(());
        }

        if settings_snapshot.should_disable_telemetry() {
            log::info!("Not sending batched messages because telemetry is disabled.");
            return Ok(());
        }

        log::info!("Start to send telemetry events to RudderStack");

        let (mut messages_with_ugc, messages_without_ugc): (Vec<_>, Vec<_>) = messages
            .into_iter()
            .partition(|message| message.contains_ugc);

        // If we shouldn't collect UGC telemetry, forcibly clear any messages with UGC before trying to send.
        if !settings_snapshot.should_collect_ai_ugc_telemetry() {
            messages_with_ugc.clear();
        }

        for (messages, rudder_stack_destination) in [
            (
                messages_with_ugc,
                ChannelState::rudderstack_ugc_destination(),
            ),
            (
                messages_without_ugc,
                ChannelState::rudderstack_non_ugc_destination(),
            ),
        ] {
            if messages.is_empty() {
                continue;
            }

            // Note that timestamp and context are already included in the individual RudderBatchMessages
            // and these are the most important ones,
            // but we also add them to the RudderMessage::Batch wrapper.
            let rudder_message = RudderMessage::Batch(RudderBatch {
                batch: messages
                    .into_iter()
                    .map(|message| message.message)
                    .collect(),
                original_timestamp: Some(Utc::now()),
                ..Default::default()
            });
            if let Err(e) = self
                .send_rudder_request(rudder_message, rudder_stack_destination)
                .await
            {
                // Don't treat a connection issue as an error as these are outside of our control.
                //
                // This is only conditionally compiled because `is_connect` is not
                // available on wasm.  If additional checks are made against the
                // `reqwest::Error`, this condition should be performed specifically
                // against `is_connect` and not the whole loop.
                #[cfg(not(target_family = "wasm"))]
                for cause in e.chain() {
                    if let Some(err) = cause.downcast_ref::<reqwest::Error>() {
                        if err.is_connect() {
                            log::warn!("Failed to send event to RudderStack: {e}");
                            return Ok(());
                        }
                    }
                }
                return Err(e);
            }
        }
        Ok(())
    }

    /// Sends a POST request to the RudderStack HTTP API.
    async fn send_rudder_request(
        &self,
        mut msg: RudderMessage,
        rudder_stack_destination: RudderStackDestination,
    ) -> Result<()> {
        msg.attach_context();

        let path = match msg {
            RudderMessage::Identify(_) => "/v1/identify",
            RudderMessage::Track(_) => "/v1/track",
            RudderMessage::Page(_) => "/v1/page",
            RudderMessage::Screen(_) => "/v1/screen",
            RudderMessage::Group(_) => "/v1/group",
            RudderMessage::Alias(_) => "/v1/alias",
            RudderMessage::Batch(_) => "/v1/batch",
        };

        self.client
            .post(&format!("{}{}", rudder_stack_destination.root_url, path))
            .basic_auth(rudder_stack_destination.write_key, Some(""))
            .json(&msg)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
