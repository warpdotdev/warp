use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{App, SingletonEntity};

use super::{HAS_COMPLETED_ONBOARDING_KEY, RootView, has_completed_local_onboarding};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::server::server_api::ServerApiProvider;

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

fn set_local_onboarding_completed(app: &mut App, completed: bool) {
    app.update(|ctx| {
        ctx.private_user_preferences()
            .write_value(
                HAS_COMPLETED_ONBOARDING_KEY,
                serde_json::to_string(&completed).unwrap(),
            )
            .unwrap();
    });
}

/// Regression test for the bug fixed by introducing
/// `RootView::sync_local_onboarding_to_server`: when a user completed onboarding
/// pre-login and later authenticated via a non-login-slide entrypoint (i.e. while
/// already in `Terminal` state), the server-side `is_onboarded` flag was never
/// flipped. The helper runs unconditionally on `AuthComplete` and must flip the
/// flag when all preconditions hold.
#[test]
fn test_sync_flips_server_is_onboarded_when_local_onboarding_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Seed the "has_completed_local_onboarding" preference and make the user
        // appear not yet onboarded on the server. The default test user is
        // non-anonymous, so the guards in the helper won't short-circuit.
        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert!(has_completed_local_onboarding(ctx));
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true),
                "sync should have invoked AuthManager::set_user_onboarded"
            );
        });
    });
}

/// If the user hasn't completed local onboarding, the helper must leave the
/// server-side flag untouched — onboarding hasn't actually happened yet.
#[test]
fn test_sync_noop_when_local_onboarding_not_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Do not set HAS_COMPLETED_ONBOARDING_KEY; it defaults to false.
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false),
                "sync should not have changed is_onboarded when local onboarding is incomplete"
            );
        });
    });
}

/// Regression test for #13514: a regular window opened from the dedicated
/// hotkey (quake) window must land on that window's current screen, centered at
/// the default size — not on the primary display via a stale last-closed
/// position, and not inheriting the quake panel's pinned-strip geometry.
///
/// Given a secondary display at `x = 1920`, the new window's bounds must be
/// centered inside that display and stay fully within it horizontally.
#[test]
fn test_centered_default_window_bounds_centers_on_the_given_screen() {
    use pathfinder_geometry::rect::RectF;
    use pathfinder_geometry::vector::vec2f;
    use warpui::platform::WindowBounds;

    let secondary = RectF::new(vec2f(1920.0, 0.0), vec2f(1920.0, 1080.0));

    let WindowBounds::ExactPosition(rect) = super::centered_default_window_bounds(secondary) else {
        panic!("expected ExactPosition bounds on the active screen");
    };

    // Default size (fits within the 1920x1080 display).
    assert_eq!(rect.size(), vec2f(1280.0, 800.0));
    // Centered within the secondary display.
    assert_eq!(
        rect.origin(),
        vec2f(1920.0 + (1920.0 - 1280.0) / 2.0, (1080.0 - 800.0) / 2.0)
    );
    // The window stays on the secondary display, not the primary one at x < 1920.
    assert!(rect.origin().x() >= 1920.0);
    assert!(rect.origin().x() + rect.size().x() <= 1920.0 + 1920.0);
}

/// The default window size is clamped to fit displays smaller than the default,
/// so the new window never overflows the active screen.
#[test]
fn test_centered_default_window_bounds_clamps_to_small_screen() {
    use pathfinder_geometry::rect::RectF;
    use pathfinder_geometry::vector::vec2f;
    use warpui::platform::WindowBounds;

    let small = RectF::new(vec2f(0.0, 0.0), vec2f(1000.0, 600.0));

    let WindowBounds::ExactPosition(rect) = super::centered_default_window_bounds(small) else {
        panic!("expected ExactPosition bounds on the active screen");
    };

    assert_eq!(rect.size(), vec2f(1000.0, 600.0));
    assert_eq!(rect.origin(), vec2f(0.0, 0.0));
}

/// A configured custom window size larger than the hotkey window's display must
/// still be clamped to that display and re-centered, so a large rows/columns
/// setting can't push the new window off the screen (#13514 follow-up).
#[test]
fn test_window_bounds_centered_on_clamps_and_recenters_oversized_size() {
    use pathfinder_geometry::rect::RectF;
    use pathfinder_geometry::vector::vec2f;
    use warpui::platform::WindowBounds;

    let secondary = RectF::new(vec2f(1920.0, 0.0), vec2f(1920.0, 1080.0));
    // A custom size wider/taller than the display.
    let oversized = vec2f(4000.0, 3000.0);

    let WindowBounds::ExactPosition(rect) = super::window_bounds_centered_on(oversized, secondary)
    else {
        panic!("expected ExactPosition bounds on the active screen");
    };

    // Clamped to the display size...
    assert_eq!(rect.size(), vec2f(1920.0, 1080.0));
    // ...and pinned to the display's origin (centered for the clamped size).
    assert_eq!(rect.origin(), vec2f(1920.0, 0.0));
    // Never overflows the secondary display.
    assert!(rect.origin().x() >= 1920.0);
    assert!(rect.origin().x() + rect.size().x() <= 1920.0 + 1920.0);
}

/// A custom size smaller than the display stays centered on it (not pinned to a
/// stale default-size origin).
#[test]
fn test_window_bounds_centered_on_centers_custom_size() {
    use pathfinder_geometry::rect::RectF;
    use pathfinder_geometry::vector::vec2f;
    use warpui::platform::WindowBounds;

    let secondary = RectF::new(vec2f(1920.0, 0.0), vec2f(1920.0, 1080.0));
    let custom = vec2f(900.0, 500.0);

    let WindowBounds::ExactPosition(rect) = super::window_bounds_centered_on(custom, secondary)
    else {
        panic!("expected ExactPosition bounds on the active screen");
    };

    assert_eq!(rect.size(), vec2f(900.0, 500.0));
    assert_eq!(
        rect.origin(),
        vec2f(1920.0 + (1920.0 - 900.0) / 2.0, (1080.0 - 500.0) / 2.0)
    );
}

/// The server-side flag should also be left untouched when it is already set,
/// even if local onboarding is complete — avoids redundant server calls.
#[test]
fn test_sync_noop_when_already_onboarded_on_server() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            // User::test() defaults to is_onboarded = true; assert that and
            // leave it in place.
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });
    });
}
