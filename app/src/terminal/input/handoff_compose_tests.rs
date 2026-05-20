use super::HandoffComposeState;
use crate::ai::ambient_agents::telemetry::HandoffEntryPoint;
use crate::server::ids::{ClientId, SyncId};
use warpui::App;

#[test]
fn preserves_explicit_environment_selection() {
    App::test((), |mut app| async move {
        let state = app.add_model(|_| HandoffComposeState::default());
        let default_environment_id = SyncId::ClientId(ClientId::new());
        let explicit_environment_id = SyncId::ClientId(ClientId::new());

        state.update(&mut app, |state, ctx| {
            state.activate(HandoffEntryPoint::Ampersand, ctx);
            state.ensure_default_environment_id(default_environment_id, ctx);
        });
        state.read(&app, |state, _| {
            assert_eq!(
                state.selected_environment_id(),
                Some(&default_environment_id)
            );
        });

        // Explicit selection should stick even when ensure_default tries to overwrite.
        state.update(&mut app, |state, ctx| {
            state.set_environment_id(Some(explicit_environment_id), true, ctx);
            state.ensure_default_environment_id(default_environment_id, ctx);
        });
        state.read(&app, |state, _| {
            assert_eq!(
                state.selected_environment_id(),
                Some(&explicit_environment_id)
            );
        });
    });
}
