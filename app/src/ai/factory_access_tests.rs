use warp_core::channel::ChannelState;
use warpui::{App, ModelHandle};

use super::*;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::auth::user::User;
use crate::server::server_api::ServerApiProvider;

/// A logged-in `AuthStateProvider` makes the model's constructor take the "already
/// authenticated at startup" path immediately, without needing a real `AuthComplete` event.
fn initialize_logged_in_app(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

/// Waits for the model's in-flight probe, if any, to finish running its completion callback,
/// driven directly by the spawned future rather than an arbitrary sleep. No-ops if no probe is
/// in flight (e.g. it already resolved, or `reset` cleared it).
async fn await_probe(app: &mut App, model: &ModelHandle<FactoryAccessModel>) {
    let future_id = model.read(&*app, |model, _| {
        model.probe.as_ref().map(|p| p.future_id())
    });
    if let Some(future_id) = future_id {
        app.update(|ctx| ctx.await_spawned_future(future_id)).await;
    }
}

#[test]
fn new_for_test_and_set_access_for_test_set_access_directly() {
    let mut model = FactoryAccessModel::new_for_test(FactoryAccess::Unknown);
    assert_eq!(model.access(), FactoryAccess::Unknown);
    model.set_access_for_test(FactoryAccess::Allowed);
    assert_eq!(model.access(), FactoryAccess::Allowed);
}

#[test]
#[serial_test::serial]
fn fetches_eagerly_and_resolves_allowed() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":true}"#)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        await_probe(&mut app, &model).await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn resolves_denied_for_an_allowed_false_response() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":false}"#)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        await_probe(&mut app, &model).await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Denied);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn http_failure_resolves_unknown_and_is_not_retried() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(500)
            .with_body(r#"{"error":"internal"}"#)
            .expect(1)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        await_probe(&mut app, &model).await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });

        // A later call (mirroring a subsequent AuthComplete, e.g. a token refresh) must not
        // issue a second request: the session holds the first outcome, success or failure. There
        // is no event to wait for here since the guarded call returns without spawning anything;
        // `mock.assert()` below is what actually verifies no second request was made.
        model.update(&mut app, |model, ctx| model.request_if_needed(ctx));
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn malformed_response_body_resolves_unknown() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body("not json")
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        await_probe(&mut app, &model).await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn reset_allows_a_fresh_request_on_the_next_authenticated_session() {
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":true}"#)
            .expect(2)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        await_probe(&mut app, &model).await;
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });

        // Simulates `auth::log_out`'s explicit reset call.
        model.update(&mut app, |model, _| model.reset());
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });

        // The next authenticated session (a real AuthComplete after re-login) starts a new
        // request rather than reusing the stale result.
        model.update(&mut app, |model, ctx| model.request_if_needed(ctx));
        await_probe(&mut app, &model).await;
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn reset_aborts_a_stale_in_flight_probe() {
    // Whether the request the abort races against ever reaches the server is inherently
    // nondeterministic (the abort may pre-empt the background task's first poll), so this mock
    // is not asserted on hit count. It is explicitly removed at the end (rather than left to
    // `Drop`, which does not deregister it): the mock server is a process-wide singleton, and a
    // mock that never got its one implicitly-expected hit would otherwise outrank every later
    // test's own mock for this route, since mockito prioritizes the oldest not-yet-satisfied
    // mock over more recently created ones.
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":true}"#)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);

        // Simulates logging out while the ending session's probe is still in flight: the abort
        // must suppress its response so it cannot land afterward and apply to whatever access
        // the next session resolves.
        let probe_completed = model.update(&mut app, |model, ctx| {
            let future_id = model
                .probe
                .as_ref()
                .expect("constructor should start a probe for an already logged-in session")
                .future_id();
            model.reset();
            ctx.await_spawned_future(future_id)
        });
        probe_completed.await;

        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });
    });
    mock.remove();
}

#[test]
#[serial_test::serial]
fn a_completion_already_resolved_before_reset_is_discarded() {
    // Covers the half of the race `reset_aborts_a_stale_in_flight_probe` cannot: `abort()` has
    // no effect once a probe's result has already resolved and is sitting in the completion
    // channel on its way to `complete_probe`. Exercises `complete_probe` directly with the
    // pre-reset generation/user to simulate that channel delivery landing after `reset()`, since
    // there is no way to interpose deterministically on the real channel from a test.
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":true}"#)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        // Let the real startup probe finish cleanly (no dangling background task) before
        // capturing its session and simulating a late/duplicate delivery of its completion.
        await_probe(&mut app, &model).await;
        let (generation, session_user) =
            model.read(&app, |model, _| (model.generation, model.session_user));

        model.update(&mut app, |model, _| model.reset());
        model.update(&mut app, |model, ctx| {
            model.complete_probe(
                generation,
                session_user,
                Ok(FactoryAccessResponse { allowed: true }),
                ctx,
            );
        });

        model.read(&app, |model, _| {
            assert_eq!(
                model.access(),
                FactoryAccess::Unknown,
                "a completion captured before reset must not overwrite the reset session's access"
            );
        });
    });
    mock.assert();
}

#[test]
#[serial_test::serial]
fn auth_complete_for_a_different_user_without_logout_starts_a_fresh_session() {
    // A remote-server daemon handoff (or similar) can replace the authenticated user with a
    // fresh `AuthComplete` that never goes through `auth::log_out`. `request_if_needed` must
    // still treat this as a new session rather than keeping the outgoing user's entitlement
    // just because `requested` was already true. Two real probes fire (A's startup, B's
    // switch), both served by this mock.
    let mock = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/access")
            .with_status(200)
            .with_body(r#"{"allowed":false}"#)
            .expect(2)
            .create()
    };
    App::test((), |mut app| async move {
        initialize_logged_in_app(&mut app);
        let model = app.add_singleton_model(FactoryAccessModel::new);
        // Let user A's real startup probe finish cleanly before switching accounts.
        await_probe(&mut app, &model).await;
        let (user_a_generation, user_a) =
            model.read(&app, |model, _| (model.generation, model.session_user));

        // User B replaces user A directly in auth state, then a real `AuthComplete` handler
        // would call exactly this.
        let user_b = User {
            local_id: UserUid::new("user-b"),
            ..User::test()
        };
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_user(Some(user_b));
        });
        model.update(&mut app, |model, ctx| model.request_if_needed(ctx));

        // `begin_session`'s effects are synchronous: a different user immediately starts a new
        // session with access reset, even before its own probe resolves.
        let (user_b_generation, user_b_session_user) =
            model.read(&app, |model, _| (model.generation, model.session_user));
        assert_ne!(
            user_b_generation, user_a_generation,
            "a different authenticated user must start a new session"
        );
        assert_eq!(user_b_session_user, Some(UserUid::new("user-b")));
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Unknown);
        });

        // Let B's real probe finish cleanly too (no dangling background task) before exercising
        // the completion logic directly; its specific resolved value doesn't matter here.
        await_probe(&mut app, &model).await;
        let access_after_b_probe = model.read(&app, |model, _| model.access());

        // A's probe, had it still resolved after the switch, must not apply to B's session.
        model.update(&mut app, |model, ctx| {
            model.complete_probe(
                user_a_generation,
                user_a,
                Ok(FactoryAccessResponse { allowed: true }),
                ctx,
            );
        });
        model.read(&app, |model, _| {
            assert_eq!(
                model.access(),
                access_after_b_probe,
                "a stale completion for the outgoing user must not change B's access"
            );
        });

        // B's own completion applies normally.
        model.update(&mut app, |model, ctx| {
            model.complete_probe(
                user_b_generation,
                user_b_session_user,
                Ok(FactoryAccessResponse { allowed: true }),
                ctx,
            );
        });
        model.read(&app, |model, _| {
            assert_eq!(model.access(), FactoryAccess::Allowed);
        });
    });
    mock.assert();
}
