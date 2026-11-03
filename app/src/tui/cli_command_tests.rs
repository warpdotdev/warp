use std::cell::Cell;
use std::rc::Rc;

use warpui::App;

use super::*;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::auth::UserAuthenticationError;

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|ctx| {
        IapManager::new(None, Box::new(|_| Box::pin(async { None })), None, ctx)
    });
}

fn emit_auth_event(event: AuthManagerEvent, app: &mut App) {
    AuthManager::handle(app).update(app, |_, ctx| ctx.emit(event));
}

#[test]
fn signed_out_command_exits_without_execution() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            let state = AuthStateProvider::as_ref(ctx).get();
            state.set_user(None);
            state.set_credentials(None);
            authenticate_then_execute(Box::new(|_| panic!("signed-out command ran")), ctx);
        });

        let error = app.termination_result().unwrap().unwrap_err();
        assert_eq!(
            error.to_string(),
            "You are not logged in. Open Warp Agent CLI to log in first."
        );
    });
}

#[test]
fn cached_login_waits_for_confirmation_and_executes_only_once() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let runs = Rc::new(Cell::new(0));
        let runs_for_command = runs.clone();
        app.update(|ctx| {
            authenticate_then_execute(
                Box::new(move |_| runs_for_command.set(runs_for_command.get() + 1)),
                ctx,
            );
        });

        assert_eq!(runs.get(), 0);

        emit_auth_event(AuthManagerEvent::AuthComplete, &mut app);
        emit_auth_event(AuthManagerEvent::AuthComplete, &mut app);
        emit_auth_event(AuthManagerEvent::NeedsReauth, &mut app);

        assert_eq!(runs.get(), 1);
        assert!(app.termination_result().is_none());
    });
}

#[test]
fn failed_authentication_prevents_later_execution() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            authenticate_then_execute(Box::new(|_| panic!("failed command ran")), ctx);
        });

        emit_auth_event(
            AuthManagerEvent::AuthFailed(UserAuthenticationError::Unexpected(anyhow!(
                "test authentication failure"
            ))),
            &mut app,
        );
        emit_auth_event(AuthManagerEvent::AuthComplete, &mut app);

        let error = app.termination_result().unwrap().unwrap_err();
        assert!(error.to_string().contains("test authentication failure"));
    });
}

#[test]
fn invalid_credentials_prevent_later_execution() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(|ctx| {
            authenticate_then_execute(Box::new(|_| panic!("invalid credentials accepted")), ctx);
        });

        emit_auth_event(AuthManagerEvent::NeedsReauth, &mut app);
        emit_auth_event(AuthManagerEvent::AuthComplete, &mut app);

        let error = app.termination_result().unwrap().unwrap_err();
        assert_eq!(
            error.to_string(),
            "Your credentials are invalid. Open Warp Agent CLI to log in again."
        );
    });
}
