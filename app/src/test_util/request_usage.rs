//! Helpers for driving the request-usage model's billing unit in tests.

use warpui::{App, SingletonEntity};

use crate::ai::AIRequestUsageModel;
use crate::ai::request_usage_model::RequestLimitInfo;
use crate::server::server_api::ServerApiProvider;

/// Registers a request-usage model backed by the already-registered test server API provider.
pub fn add_request_usage_model(app: &mut App) {
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

/// Marks the subject as billed in dollars (an `included_usage_cents` figure is present) or in
/// credits, the way a request-limit refresh from the server would.
pub fn set_billed_in_dollars(app: &mut App, billed_in_dollars: bool) {
    AIRequestUsageModel::handle(app).update(app, |model, ctx| {
        model.update_request_limit_info(
            RequestLimitInfo {
                included_usage_cents: billed_in_dollars.then_some(1_000.0),
                ..RequestLimitInfo::default()
            },
            ctx,
        );
    });
}
