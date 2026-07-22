use std::time::Duration;

use warpui_core::elements::tui::TuiSize;

use super::{LogoSurface, fitted_logo_size, logo_frame_at, warp_logo_contains};

const PANEL_SIZE: TuiSize = TuiSize::new(52, 20);

#[test]
fn logo_mask_preserves_the_offset_warp_faces() {
    assert!(warp_logo_contains(0.25, -0.65));
    assert!(warp_logo_contains(-0.55, 0.45));
    assert!(!warp_logo_contains(-0.85, -0.85));
    assert!(!warp_logo_contains(0.0, 0.9));
}

#[test]
fn full_face_frame_is_recognizable_and_centered() {
    let frame = logo_frame_at(Duration::ZERO, PANEL_SIZE).unwrap();
    let lines = frame.to_lines();
    let occupied = frame.iter_cells().count();

    assert!(
        (90..220).contains(&occupied),
        "expected a sparse logo outline, got {occupied} cells"
    );
    assert!(
        frame
            .iter_cells()
            .filter(|(_, _, cell)| cell.surface != LogoSurface::Background)
            .all(|(_, y, _)| y > 0 && y < usize::from(PANEL_SIZE.height) - 1)
    );
    assert!(lines.iter().any(|line| line.contains("------")));
    assert!(lines.iter().any(|line| line.contains('.')));
    assert!(lines.iter().all(|line| !line.contains(['█', '▓', '▒'])));
}
#[test]
fn background_starfield_stays_low_density() {
    let frame = logo_frame_at(Duration::ZERO, PANEL_SIZE).unwrap();
    let stars = frame
        .iter_cells()
        .filter(|(_, _, cell)| cell.surface == LogoSurface::Background)
        .count();

    assert!(
        (12..=36).contains(&stars),
        "expected a subtle background starfield, got {stars} visible stars"
    );
}

#[test]
fn background_stars_move_between_frames() {
    let initial = logo_frame_at(Duration::ZERO, PANEL_SIZE).unwrap();
    let advanced = logo_frame_at(Duration::from_millis(700), PANEL_SIZE).unwrap();
    let star_positions = |frame: &super::LogoFrame| {
        frame
            .iter_cells()
            .filter_map(|(x, y, cell)| (cell.surface == LogoSurface::Background).then_some((x, y)))
            .collect::<Vec<_>>()
    };

    assert_ne!(star_positions(&initial), star_positions(&advanced));
}

#[test]
fn quarter_turn_is_narrower_and_exposes_the_side() {
    let face = logo_frame_at(Duration::ZERO, PANEL_SIZE).unwrap();
    let edge = logo_frame_at(Duration::from_millis(1250), PANEL_SIZE).unwrap();

    assert!(edge.iter_cells().count() < face.iter_cells().count());
    assert!(
        edge.iter_cells()
            .any(|(_, _, cell)| cell.surface == LogoSurface::Side)
    );
    assert_ne!(face.to_lines(), edge.to_lines());
}

#[test]
fn half_turn_exposes_the_back_face() {
    let frame = logo_frame_at(Duration::from_millis(2500), PANEL_SIZE).unwrap();

    assert!(
        frame
            .iter_cells()
            .all(|(_, _, cell)| cell.surface != LogoSurface::Front)
    );
    assert!(
        frame
            .iter_cells()
            .any(|(_, _, cell)| cell.surface == LogoSurface::Back)
    );
}

#[test]
fn one_revolution_returns_to_the_initial_frame() {
    let initial = logo_frame_at(Duration::ZERO, PANEL_SIZE).unwrap();
    let revolved = logo_frame_at(Duration::from_secs(5), PANEL_SIZE).unwrap();
    let logo_cells = |frame: &super::LogoFrame| {
        frame
            .iter_cells()
            .filter(|(_, _, cell)| cell.surface != LogoSurface::Background)
            .collect::<Vec<_>>()
    };

    assert_eq!(logo_cells(&initial), logo_cells(&revolved));
}

#[test]
fn logo_scales_down_while_preserving_cell_aspect() {
    assert_eq!(fitted_logo_size(TuiSize::new(100, 40)), Some((42, 17)));
    assert_eq!(fitted_logo_size(TuiSize::new(30, 12)), Some((25, 10)));
}

#[test]
fn animation_is_hidden_when_the_panel_is_too_small() {
    assert!(logo_frame_at(Duration::ZERO, TuiSize::new(17, 20)).is_none());
    assert!(logo_frame_at(Duration::ZERO, TuiSize::new(30, 6)).is_none());
}
