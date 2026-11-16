use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use warpui::App;

use super::{
    MarkerLoadResult, MarkerPersistenceAction, TuiOnboardingMarker, TuiOnboardingMarkers,
    TuiOnboardingMarkersState, load_markers_with_timeout, marker_persistence_action,
};
use crate::server::server_api::tui_onboarding::{
    MockTuiOnboardingClient, TuiOnboardingClient, TuiOnboardingMarkersSnapshot,
};

struct PendingTuiOnboardingClient;

#[async_trait]
impl TuiOnboardingClient for PendingTuiOnboardingClient {
    async fn get_tui_onboarding_markers(&self) -> anyhow::Result<TuiOnboardingMarkersSnapshot> {
        futures::future::pending().await
    }
}

#[test]
fn markers_are_unavailable_until_initial_load_resolves() {
    let mut markers = TuiOnboardingMarkers {
        state: TuiOnboardingMarkersState::Loading,
        load_generation: 0,
        persist_markers: false,
        onboarding_client: None,
        load_timeout: Duration::from_secs(3),
    };

    assert!(!markers.is_ready());
    assert!(!markers.take_available(TuiOnboardingMarker::FirstZeroState));
    assert!(!markers.take_available(TuiOnboardingMarker::FirstCreditGate));
}

#[test]
fn absent_or_false_cloud_markers_are_available_and_true_markers_are_consumed() {
    let marker_is_available = |value: Option<bool>| value != Some(true);

    assert!(marker_is_available(None));
    assert!(marker_is_available(Some(false)));
    assert!(!marker_is_available(Some(true)));
}

#[test]
fn marker_consumption_is_monotonic_and_independent() {
    let mut markers = TuiOnboardingMarkers::new_ready_for_test(true, true);

    assert!(markers.take_available(TuiOnboardingMarker::FirstZeroState));
    assert!(!markers.take_available(TuiOnboardingMarker::FirstZeroState));
    assert!(markers.take_available(TuiOnboardingMarker::FirstCreditGate));
    assert!(!markers.take_available(TuiOnboardingMarker::FirstCreditGate));
}

#[test]
fn marker_persistence_creates_updates_or_skips_without_writing_false() {
    assert_eq!(
        marker_persistence_action(None),
        MarkerPersistenceAction::Create
    );
    assert_eq!(
        marker_persistence_action(Some(false)),
        MarkerPersistenceAction::Update
    );
    assert_eq!(
        marker_persistence_action(Some(true)),
        MarkerPersistenceAction::None
    );
}

#[test]
fn targeted_marker_load_returns_the_server_snapshot() {
    App::test((), |_app| async move {
        let snapshot = TuiOnboardingMarkersSnapshot {
            first_zero_state_shown: true,
            first_credit_gate_shown: false,
        };
        let mut client = MockTuiOnboardingClient::new();
        client
            .expect_get_tui_onboarding_markers()
            .once()
            .return_once(move || Ok(snapshot));

        let result = load_markers_with_timeout(Arc::new(client), Duration::from_millis(100)).await;

        let MarkerLoadResult::Loaded(Ok(loaded)) = result else {
            panic!("expected successful targeted marker load");
        };
        assert_eq!(loaded, snapshot);
    });
}

#[test]
fn targeted_marker_load_preserves_request_failures_for_fail_open_handling() {
    App::test((), |_app| async move {
        let mut client = MockTuiOnboardingClient::new();
        client
            .expect_get_tui_onboarding_markers()
            .once()
            .return_once(|| Err(anyhow!("request failed")));

        let result = load_markers_with_timeout(Arc::new(client), Duration::from_millis(100)).await;

        let MarkerLoadResult::Loaded(Err(error)) = result else {
            panic!("expected targeted marker request failure");
        };
        assert_eq!(error.to_string(), "request failed");
    });
}

#[test]
fn targeted_marker_load_times_out_when_the_request_never_resolves() {
    App::test((), |_app| async move {
        let result = load_markers_with_timeout(
            Arc::new(PendingTuiOnboardingClient),
            Duration::from_millis(1),
        )
        .await;

        assert!(matches!(result, MarkerLoadResult::TimedOut));
    });
}

#[test]
fn successful_snapshot_controls_marker_availability() {
    App::test((), |mut app| async move {
        let markers =
            app.add_singleton_model(|_| TuiOnboardingMarkers::new_ready_for_test(false, false));

        markers.update(&mut app, |markers, ctx| {
            markers.load_generation = 1;
            markers.handle_load_result(
                1,
                MarkerLoadResult::Loaded(Ok(TuiOnboardingMarkersSnapshot {
                    first_zero_state_shown: false,
                    first_credit_gate_shown: true,
                })),
                ctx,
            );
        });

        markers.update(&mut app, |markers, _| {
            assert!(markers.take_available(TuiOnboardingMarker::FirstZeroState));
            assert!(!markers.take_available(TuiOnboardingMarker::FirstCreditGate));
        });
    });
}

#[test]
fn failed_marker_load_allows_startup_without_one_time_surfaces() {
    App::test((), |mut app| async move {
        let markers = app.add_singleton_model(|_| {
            TuiOnboardingMarkers::new_loading_for_test(
                Arc::new(PendingTuiOnboardingClient),
                Duration::from_millis(1),
            )
        });

        markers.update(&mut app, |markers, ctx| {
            markers.handle_load_result(
                0,
                MarkerLoadResult::Loaded(Err(anyhow!("request failed"))),
                ctx,
            );
        });

        markers.read(&app, |markers, _| {
            assert!(markers.is_ready());
        });
        markers.update(&mut app, |markers, _| {
            assert!(!markers.take_available(TuiOnboardingMarker::FirstZeroState));
            assert!(!markers.take_available(TuiOnboardingMarker::FirstCreditGate));
        });
    });
}

#[test]
fn stale_account_load_result_is_ignored() {
    App::test((), |mut app| async move {
        let markers = app.add_singleton_model(|_| {
            TuiOnboardingMarkers::new_loading_for_test(
                Arc::new(PendingTuiOnboardingClient),
                Duration::from_secs(1),
            )
        });

        markers.update(&mut app, |markers, ctx| {
            markers.load_generation = 2;
            markers.handle_load_result(
                1,
                MarkerLoadResult::Loaded(Ok(TuiOnboardingMarkersSnapshot {
                    first_zero_state_shown: false,
                    first_credit_gate_shown: false,
                })),
                ctx,
            );
        });

        markers.read(&app, |markers, _| {
            assert!(!markers.is_ready());
        });
    });
}
