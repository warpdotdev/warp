use warp::tui_export::{dark_theme, light_theme};
use warpui_core::runtime::ProbedRgb;

use super::*;

fn rgb(r: u8, g: u8, b: u8) -> ProbedRgb {
    ProbedRgb { r, g, b }
}

#[test]
fn terminal_background_state_tracks_exact_background_changes() {
    let initial = Some(rgb(20, 20, 20));
    let changed = Some(rgb(220, 220, 220));
    let dark = dark_theme();
    let mut state = TerminalBackgroundState::new(initial);

    assert_eq!(
        state.record_probe_result(initial, &dark, CONSECUTIVE_MISS_CUTOFF),
        ProbeResultAction::default()
    );
    assert_eq!(state.background, initial);

    assert_eq!(
        state.record_probe_result(changed, &dark, CONSECUTIVE_MISS_CUTOFF),
        ProbeResultAction::SetTheme(light_theme()),
    );
    assert_eq!(state.background, changed);
}

#[test]
fn probe_result_actions_distinguish_theme_and_background_changes() {
    let dark = dark_theme();
    let light = light_theme();
    let dark_background = Some(rgb(20, 20, 20));
    let dark_same_class = Some(rgb(80, 80, 80));
    let light_background = Some(rgb(240, 240, 240));

    let mut state = TerminalBackgroundState::new(dark_background);
    assert_eq!(
        state.record_probe_result(light_background, &dark, CONSECUTIVE_MISS_CUTOFF),
        ProbeResultAction::SetTheme(light.clone()),
    );

    let mut state = TerminalBackgroundState::new(dark_background);
    assert_eq!(
        state.record_probe_result(dark_same_class, &dark, CONSECUTIVE_MISS_CUTOFF),
        ProbeResultAction::Repaint,
    );

    let mut state = TerminalBackgroundState::new(dark_background);
    state.probe_budget = TerminalBackgroundProbeBudget::Exhausted;
    assert_eq!(
        state.record_probe_result(light_background, &light, CONSECUTIVE_MISS_CUTOFF),
        ProbeResultAction::default()
    );
    let mut state = TerminalBackgroundState::new(None);
    assert_eq!(
        state.record_probe_result(dark_background, &dark, CONSECUTIVE_MISS_CUTOFF),
        ProbeResultAction::Repaint,
    );
}

#[test]
fn probe_stops_after_cutoff_and_success_resets_misses() {
    let dark = Some(rgb(20, 20, 20));
    let dark_theme = dark_theme();
    let cutoff = CONSECUTIVE_MISS_CUTOFF;

    let mut state = TerminalBackgroundState::new(dark);
    for miss in 1..cutoff {
        assert_eq!(
            state.record_probe_result(None, &dark_theme, cutoff),
            ProbeResultAction::default()
        );
        assert_eq!(
            state.probe_budget,
            TerminalBackgroundProbeBudget::Available {
                consecutive_misses: miss,
            }
        );
    }
    assert_eq!(
        state.record_probe_result(None, &dark_theme, cutoff),
        ProbeResultAction::default()
    );
    assert_eq!(state.probe_budget, TerminalBackgroundProbeBudget::Exhausted);
    assert!(!state.probe_enabled(TuiTheme::Auto));
    assert!(!state.probe_enabled(TuiTheme::Dark));

    let mut state = TerminalBackgroundState::new(dark);
    for _ in 0..cutoff - 1 {
        state.record_probe_result(None, &dark_theme, cutoff);
    }
    state.record_probe_result(dark, &dark_theme, cutoff);
    assert_eq!(
        state.probe_budget,
        TerminalBackgroundProbeBudget::Available {
            consecutive_misses: 0,
        }
    );
}

#[test]
fn explicit_theme_preserves_the_session_miss_count() {
    let dark = Some(rgb(20, 20, 20));
    let dark_theme = dark_theme();
    let mut state = TerminalBackgroundState::new(dark);

    state.record_probe_result(None, &dark_theme, CONSECUTIVE_MISS_CUTOFF);
    assert!(!state.probe_enabled(TuiTheme::Dark));
    assert_eq!(
        state.probe_budget,
        TerminalBackgroundProbeBudget::Available {
            consecutive_misses: 1,
        }
    );
    assert!(state.probe_enabled(TuiTheme::Auto));
}

#[test]
fn host_updates_reader_probe_gate_from_theme_and_budget() {
    let probe_enabled = Arc::new(AtomicBool::new(false));
    let mut host = TuiHostTerminalBackground {
        state: TerminalBackgroundState::new(Some(rgb(20, 20, 20))),
        probe_enabled: probe_enabled.clone(),
    };

    host.update_probe_enabled(TuiTheme::Auto);
    assert!(probe_enabled.load(Ordering::Relaxed));

    host.update_probe_enabled(TuiTheme::Dark);
    assert!(!probe_enabled.load(Ordering::Relaxed));

    host.state.probe_budget = TerminalBackgroundProbeBudget::Exhausted;
    host.update_probe_enabled(TuiTheme::Auto);
    assert!(!probe_enabled.load(Ordering::Relaxed));
}
