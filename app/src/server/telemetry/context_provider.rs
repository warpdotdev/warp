use warp_core::telemetry::{TelemetryContextModel, TelemetryContextProvider};
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;

pub struct AppTelemetryContextProvider {}

impl AppTelemetryContextProvider {
    pub fn new_context_provider(
        _ctx: &mut ModelContext<TelemetryContextModel>,
    ) -> TelemetryContextModel {
        Box::new(Self {})
    }
}

impl TelemetryContextProvider for AppTelemetryContextProvider {
    fn user_id(&self, ctx: &AppContext) -> Option<String> {
        let auth_state = AuthStateProvider::as_ref(ctx).get();
        auth_state.user_id().map(|uid| uid.as_string())
    }

    fn anonymous_id(&self, ctx: &AppContext) -> String {
        let auth_state = AuthStateProvider::as_ref(ctx).get();
        auth_state.anonymous_id()
    }
}
