//! Module that builds a static context to attach to each of our events that are sent to Rudderstack.
//! This is needed so we know the backing operating system and version of each telemetry event.

use std::sync::OnceLock;

use serde::Serialize;
use serde_json::{Value, json};
use warp_errors::report_error;
#[cfg(target_family = "wasm")]
use warpui::platform::wasm;

use super::rudder_message::Message as RudderMessage;
use crate::server::OperatingSystemInfo;

static TELEMETRY_CONTEXT: OnceLock<TelemetryContext> = OnceLock::new();

#[derive(Serialize)]
struct TelemetryContextInfo {
    /// Info about the operating system of the client.
    #[serde(skip_serializing_if = "Option::is_none")]
    os: Option<&'static OperatingSystemInfo>,
    /// The user agent provided by the browser, if running on Web. If not on
    /// Web, this is always `None`.
    #[serde(rename = "userAgent", skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
}

/// Newtype representing a [`Value`] with a serialized version of the context that we send to
/// Rudderstack.
/// See https://www.rudderstack.com/docs/event-spec/standard-events/common-fields/#contextual-fields.
pub struct TelemetryContext(Value);

impl TelemetryContext {
    pub fn as_value(&self) -> Value {
        self.0.clone()
    }
}

impl TelemetryContext {
    fn new() -> Self {
        let context = TelemetryContextInfo {
            os: OperatingSystemInfo::get().ok(),
            user_agent: user_agent(),
        };

        match serde_json::to_value(context) {
            Ok(value) => Self(value),
            Err(e) => {
                report_error!(
                    anyhow::Error::new(e)
                        .context("Failed to serialize telemetry context info to JSON value")
                );
                Self(json!({}))
            }
        }
    }
}

/// Extension trait used to attach a telemetry context.
pub(super) trait AttachContext {
    /// Attaches a context to the given object.
    fn attach_context(&mut self);
}

impl AttachContext for RudderMessage {
    /// Attaches the context to the [`RudderMessage`]. Note this is currently last write wins; if a
    /// message already has a `context` set it will be overridden.
    // TODO(alokedesai): Merge the incoming context with the static `TelemetryContext`, if set.
    fn attach_context(&mut self) {
        let context = telemetry_context().as_value();
        match self {
            RudderMessage::Identify(identify) => {
                identify.context = Some(context);
            }
            RudderMessage::Track(track) => track.context = Some(context),
            RudderMessage::Page(page) => page.context = Some(context),
            RudderMessage::Screen(screen) => screen.context = Some(context),
            RudderMessage::Group(group) => group.context = Some(context),
            RudderMessage::Alias(alias) => alias.context = Some(context),
            RudderMessage::Batch(batch) => batch.context = Some(context),
        }
    }
}

/// Returns the user agent provided by the browser, if on Web. If not on Web,
/// or if the user agent was not able to be read, returns None.
fn user_agent() -> Option<String> {
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            wasm::user_agent()
        } else {
            None
        }
    }
}

/// Returns the telemetry context
/// that should be attached to all telemetry events associated to this client.
///
/// [Rudderstack](https://www.rudderstack.com/docs/event-spec/standard-events/common-fields/#contextual-fields)
pub fn telemetry_context() -> &'static TelemetryContext {
    TELEMETRY_CONTEXT.get_or_init(TelemetryContext::new)
}
