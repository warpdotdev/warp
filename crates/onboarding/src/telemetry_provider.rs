use warp_core::telemetry::{TelemetryContextModel, TelemetryContextProvider};
use warpui_core::{AppContext, ModelContext};

/// A mock telemetry context provider for the onboarding binary that logs events
/// instead of sending them to a server.
///
/// Since the onboarding binary runs standalone without authentication,
/// we use a fixed anonymous ID and log all telemetry events.
pub struct MockTelemetryContextProvider {
    anonymous_id: String,
}

impl MockTelemetryContextProvider {
    pub fn new_context_provider(
        _ctx: &mut ModelContext<TelemetryContextModel>,
    ) -> TelemetryContextModel {
        let anonymous_id = uuid::Uuid::new_v4().to_string();
        Box::new(Self { anonymous_id })
    }
}

impl TelemetryContextProvider for MockTelemetryContextProvider {
    fn user_id(&self, _ctx: &AppContext) -> Option<String> {
        // No user ID for the standalone onboarding binary
        None
    }

    fn anonymous_id(&self, _ctx: &AppContext) -> String {
        self.anonymous_id.clone()
    }
}
