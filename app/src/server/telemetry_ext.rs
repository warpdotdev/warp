use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use warp_core::channel::{Channel, ChannelState};
use warp_core::execution_mode;
use warpui::telemetry::EventPayload;

use super::telemetry::rudder_message::{
    BatchMessage as RudderBatchMessage, BatchMessageItem as RudderBatchMessageItem,
    Identify as RudderIdentify, Track as RudderTrack,
};
use super::telemetry::secret_redaction::redact_secrets_in_value;
use super::telemetry::telemetry_context;
use crate::auth::UserUid;

pub trait TelemetryExt {
    fn to_rudder_batch_message(self) -> RudderBatchMessage;
}

impl TelemetryExt for warpui::telemetry::Event {
    fn to_rudder_batch_message(self) -> RudderBatchMessage {
        let message = match self.payload {
            EventPayload::IdentifyUser {
                user_id,
                anonymous_id,
            } => RudderBatchMessageItem::Identify(RudderIdentify {
                user_id: Some(UserUid::new(user_id.as_str())),
                anonymous_id: Some(anonymous_id),
                original_timestamp: Some(self.timestamp),
                integrations: Some(json!({
                    "Amplitude": {
                      "session_id": self.session_created_at.timestamp(),
                    }
                })),
                context: Some(telemetry_context().as_value()),
                ..Default::default()
            }),
            EventPayload::AppActive {
                user_id,
                anonymous_id,
            } => form_rudder_track_message(
                user_id.map(|uid| UserUid::new(uid.as_str())),
                anonymous_id,
                "Active App Usage".to_string(),
                None,
                self.timestamp,
                self.session_created_at,
            ),
            EventPayload::NamedEvent {
                user_id,
                anonymous_id,
                name,
                mut value,
            } => {
                // For events that may contain user-generated content, run a
                // best-effort secret-redaction pass on the payload before
                // sending. This is independent of the user's safe-mode setting:
                // visual obfuscation is a UX preference, while telemetry-side
                // redaction is a defence-in-depth measure for data leaving the
                // device. See `secret_redaction.rs` for details.
                if self.contains_ugc
                    && let Some(value) = value.as_mut()
                {
                    redact_secrets_in_value(value);
                }
                form_rudder_track_message(
                    user_id.map(|uid| UserUid::new(uid.as_str())),
                    anonymous_id,
                    name.to_string(),
                    value,
                    self.timestamp,
                    self.session_created_at,
                )
            }
        };

        RudderBatchMessage {
            message,
            contains_ugc: self.contains_ugc,
        }
    }
}

fn form_rudder_track_message(
    user_id: Option<UserUid>,
    anonymous_id: String,
    name: String,
    payload: Option<Value>,
    timestamp: DateTime<Utc>,
    session_created_at: DateTime<Utc>,
) -> RudderBatchMessageItem {
    RudderBatchMessageItem::Track(RudderTrack {
        user_id,
        anonymous_id: Some(anonymous_id),
        event: name,
        properties: Some(json!({
            "release_mode": release_mode(ChannelState::channel()),
            "tag": ChannelState::app_version().unwrap_or("<no tag>"),
            "client_id": execution_mode::current_client_id(),
            "payload": payload
        })),
        original_timestamp: Some(timestamp),
        integrations: Some(json!({
            "Amplitude": {
              "session_id": session_created_at.timestamp(),
            }
        })),
        context: Some(telemetry_context().as_value()),
    })
}

fn release_mode(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "stable_release",
        Channel::Preview => "preview_release",
        Channel::Local => "local",
        Channel::Integration => "integration_test",
        Channel::Dev => "dev_release",
        // We don't ever expect to send telemetry for the OSS build, but
        // until we have some time to clean things up here, we'll set a valid
        // value that we never intend to receive.
        Channel::Oss => "oss_release",
    }
}

#[cfg(test)]
#[path = "telemetry_ext_tests.rs"]
mod tests;
