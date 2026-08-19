use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use warpui::{App, SingletonEntity};

use super::{AuthManager, AuthManagerEvent, request_device_code_with_timeout};
use crate::ServerApiProvider;
use crate::auth::auth_view_modal::AuthRedirectPayload;
use crate::auth::credentials::{Credentials, LoginToken, RefreshToken};
use crate::auth::user::{FirebaseAuthTokens, TEST_USER_UID, User};
use crate::auth::{AuthStateProvider, UserUid};
use crate::server::server_api::auth::{MockAuthClient, UserAuthenticationError};

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

/// Subscribes to `AuthManager` events and returns a flag that becomes `true`
/// if an `AuthFailed(InvalidStateParameter)` event is observed.
fn track_invalid_state_failures(app: &mut App) -> Arc<AtomicBool> {
    let saw_invalid_state = Arc::new(AtomicBool::new(false));
    let saw_invalid_state_for_closure = saw_invalid_state.clone();
    app.update(|ctx| {
        ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, _| {
            if matches!(
                event,
                AuthManagerEvent::AuthFailed(UserAuthenticationError::InvalidStateParameter)
            ) {
                saw_invalid_state_for_closure.store(true, Ordering::Relaxed);
            }
        });
    });
    saw_invalid_state
}

/// After a logged-in user successfully completes auth, pressing the browser's
/// "Take me to Warp" button a second time should silently drop the stale
/// redirect rather than surface an `InvalidStateParameter` error.
#[test]
fn test_duplicate_redirect_for_logged_in_user_is_silently_ignored() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Fail immediately if we see InvalidStateParameter at any point.
        app.update(|ctx| {
            ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, _| {
                if matches!(
                    event,
                    AuthManagerEvent::AuthFailed(UserAuthenticationError::InvalidStateParameter)
                ) {
                    panic!("Test failed: Received InvalidStateParameter error");
                }
            });
        });

        // Generate a state token and create the auth payload that we'll use for both calls.
        // The incoming user_uid matches the default test user so the stale second redirect
        // qualifies for the silent-ignore branch.
        let auth_payload = AuthManager::handle(&app).update(&mut app, |auth_manager, _ctx| {
            let state = auth_manager.generate_auth_state();

            AuthRedirectPayload {
                refresh_token: RefreshToken::new("test_refresh_token"),
                user_uid: Some(UserUid::new(TEST_USER_UID)),
                deleted_anonymous_user: Some(false),
                state: Some(state),
            }
        });

        // First call: state validates and is consumed.
        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.initialize_user_from_auth_payload(auth_payload.clone(), true, ctx);
        });

        // The CSRF token must be single-use: successful validation clears it.
        AuthManager::handle(&app).update(&mut app, |auth_manager, _ctx| {
            assert!(
                auth_manager.pending_auth_state.is_none(),
                "pending_auth_state should be cleared after successful validation"
            );
        });

        // Second call with the same (now-consumed) state: the user is already
        // logged in as the test user and the incoming user_uid matches, so we
        // must silently drop the redirect without emitting any AuthFailed event.
        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.initialize_user_from_auth_payload(auth_payload, true, ctx);
        });
    });
}

#[test]
fn pending_api_key_failure_leaves_auth_state_logged_out() {
    App::test((), |mut app| async move {
        let mut auth_client = MockAuthClient::new();
        auth_client
            .expect_fetch_user()
            .withf(|token, for_refresh| {
                matches!(token, LoginToken::ApiKey(key) if key == "wk-inherited-key")
                    && !for_refresh
            })
            .times(1)
            .return_once(|_, _| Err(UserAuthenticationError::Unexpected(anyhow!("Unauthorized"))));

        initialize_app(&mut app);
        let auth_state = app.read(|ctx| AuthStateProvider::as_ref(ctx).get().clone());
        auth_state.set_user(None);
        auth_state.set_credentials(None);
        AuthManager::handle(&app).update(&mut app, |auth_manager, _| {
            auth_manager.auth_client = Arc::new(auth_client);
        });

        let saw_auth_failure = Arc::new(AtomicBool::new(false));
        let saw_auth_failure_for_subscription = saw_auth_failure.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, _| {
                if matches!(event, AuthManagerEvent::AuthFailed(_)) {
                    saw_auth_failure_for_subscription.store(true, Ordering::Relaxed);
                }
            });
        });

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.authenticate_api_key("inherited-key".to_owned(), ctx);
        });

        assert!(auth_state.credentials().is_none());
        assert!(auth_state.user_id().is_none());
        assert!(!auth_state.is_logged_in());

        warpui::r#async::Timer::after(Duration::from_millis(100)).await;

        assert!(saw_auth_failure.load(Ordering::Relaxed));
        assert!(auth_state.credentials().is_none());
        assert!(auth_state.user_id().is_none());
        assert!(!auth_state.is_logged_in());
    });
}

#[test]
fn validated_api_key_is_promoted_with_its_user() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let auth_state = app.read(|ctx| AuthStateProvider::as_ref(ctx).get().clone());
        auth_state.set_user(None);
        auth_state.set_credentials(None);

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.complete_authentication(
                User::test(),
                Credentials::ApiKey {
                    key: "wk-validated-key".to_owned(),
                    owner_type: None,
                },
                ctx,
            );
        });

        assert_eq!(auth_state.api_key().as_deref(), Some("wk-validated-key"));
        assert_eq!(auth_state.user_id(), Some(UserUid::new(TEST_USER_UID)));
        assert!(auth_state.is_logged_in());
    });
}

#[test]
fn test_device_code_request_retries_then_times_out() {
    App::test((), |_app| async move {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_request = attempts.clone();

        let result = request_device_code_with_timeout(
            move || {
                attempts_for_request.fetch_add(1, Ordering::Relaxed);
                futures::future::pending()
            },
            Duration::from_millis(1),
            2,
        )
        .await;

        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        let error = result.unwrap_err();
        assert!(matches!(
            &error,
            UserAuthenticationError::DeviceCodeRequestTimedOut { attempts: 2 }
        ));
        assert_eq!(
            error.to_string(),
            "Timed out requesting a sign-in link after 2 attempts"
        );
    });
}

/// When the user is fully logged out, a redirect carrying a state that does
/// not match the pending token must surface an `InvalidStateParameter` error.
#[test]
fn test_stale_state_when_logged_out_emits_invalid_state_parameter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Clear the default test user so we're fully logged out.
        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            auth_state.set_user(None);
            auth_state.set_credentials(None);
        });

        let saw_invalid_state = track_invalid_state_failures(&mut app);

        // Generate a real pending state, then deliver a redirect whose state
        // doesn't match it.
        AuthManager::handle(&app).update(&mut app, |auth_manager, _ctx| {
            let _known_state = auth_manager.generate_auth_state();
        });

        let bogus_payload = AuthRedirectPayload {
            refresh_token: RefreshToken::new("test_refresh_token"),
            user_uid: Some(UserUid::new("some_user_uid")),
            deleted_anonymous_user: Some(false),
            state: Some("not_the_real_state".to_owned()),
        };

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.initialize_user_from_auth_payload(bogus_payload, true, ctx);
        });

        assert!(
            saw_invalid_state.load(Ordering::Relaxed),
            "expected AuthFailed(InvalidStateParameter) when logged out and state does not match"
        );
    });
}

/// Even when a user is logged in, a redirect with a bad state and a `user_uid`
/// that does NOT match the current user must surface an `InvalidStateParameter`
/// error: the silent-ignore branch is reserved for redirects that target the
/// same user who is already authenticated.
#[test]
fn test_mismatched_state_with_different_user_uid_emits_invalid_state_parameter() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let saw_invalid_state = track_invalid_state_failures(&mut app);

        // Test user is logged in by default; generate a pending state so the
        // validation below fails because the incoming state is different.
        AuthManager::handle(&app).update(&mut app, |auth_manager, _ctx| {
            let _known_state = auth_manager.generate_auth_state();
        });

        // Attacker-style payload: bogus state, plus a user_uid that differs
        // from the currently logged-in user's uid.
        let attacker_payload = AuthRedirectPayload {
            refresh_token: RefreshToken::new("attacker_refresh_token"),
            user_uid: Some(UserUid::new("not_the_current_user")),
            deleted_anonymous_user: Some(false),
            state: Some("not_the_real_state".to_owned()),
        };

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.initialize_user_from_auth_payload(attacker_payload, true, ctx);
        });

        assert!(
            saw_invalid_state.load(Ordering::Relaxed),
            "expected AuthFailed(InvalidStateParameter) when incoming user_uid differs from current user"
        );
    });
}

/// `log_out` must clear the active-team header seam so a subsequent session
/// (even before its own `UserWorkspaces` resync happens) never sends
/// GraphQL v2 requests carrying the outgoing session's team UID.
#[test]
fn test_log_out_clears_active_team_uid() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("warp_test", ctx);
        });

        let active_team_uid =
            app.read(|ctx| ServerApiProvider::as_ref(ctx).active_team_uid_handle());
        active_team_uid.set(Some("stale-team".to_string()));

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.log_out(ctx);
        });

        assert_eq!(
            active_team_uid.get(),
            None,
            "log_out should clear the active-team header seam"
        );
    });
}

/// `log_out` must clear any pending CSRF state from an auth flow that was
/// started but never completed, so the token cannot be replayed against the
/// next session in the same process.
#[test]
fn test_log_out_clears_pending_auth_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // `log_out` clears user+credentials and then calls `persist`, which
        // routes to `PersistedUser::remove_from_secure_storage`. That requires
        // a `SecureStorage` singleton, so register a no-op one for this test.
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("warp_test", ctx);
        });

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            let _pending = auth_manager.generate_auth_state();
            assert!(
                auth_manager.pending_auth_state.is_some(),
                "precondition: generate_auth_state should populate pending_auth_state"
            );

            auth_manager.log_out(ctx);

            assert!(
                auth_manager.pending_auth_state.is_none(),
                "log_out should clear pending_auth_state"
            );
        });
    });
}

// These two tests verify that `persist` skips writing to secure storage under certain conditions.
// They rely on the fact that no secure storage singleton is registered in the test app: if
// `write_to_secure_storage` were ever called, it would panic trying to look up the unregistered
// singleton, causing the test to fail.

#[test]
fn test_persist_skips_when_refresh_token_is_empty() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Override default test credentials with Firebase tokens that have an empty refresh token.
        app.update(|ctx| {
            let tokens = FirebaseAuthTokens {
                id_token: String::new(),
                refresh_token: String::new(),
                expiration_time: chrono::Utc::now().fixed_offset() + chrono::Duration::days(365),
            };
            AuthStateProvider::as_ref(ctx)
                .get()
                .set_credentials(Some(Credentials::Firebase(tokens)));
        });

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.persist(ctx);
        });
    });
}

#[test]
fn test_persist_skips_when_api_key_authenticated() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx)
                .get()
                .set_credentials(Some(Credentials::ApiKey {
                    key: "wk-test-key".to_owned(),
                    owner_type: None,
                }));
        });

        AuthManager::handle(&app).update(&mut app, |auth_manager, ctx| {
            auth_manager.persist(ctx);
        });
    });
}
