use super::{surface_unavailable_reason, SurfaceDestination};
use crate::auth::AuthStateProvider;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn agent_management_surface_reports_unavailable_when_ai_disabled() {
    warpui::App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        assert_eq!(
            app.update(|ctx| {
                surface_unavailable_reason(SurfaceDestination::AgentManagement, ctx)
            }),
            Some("agent management is unavailable or disabled")
        );
    });
}
