use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use tempfile::TempDir;
use warp::settings::{
    TuiZeroStateExtrusionDepthSetting, TuiZeroStateObject, TuiZeroStateObjectSetting,
    TuiZeroStateRotationPeriodSeconds, TuiZeroStateRotationPeriodSecondsSetting,
    TuiZeroStateSettings,
};
use warp_core::settings::Setting as _;
use warpui::{EntityIdMap, SingletonEntity as _};
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiLocalPoint, TuiPaintContext, TuiPaintSurface, TuiPoint, TuiRect,
    TuiScreenPosition, TuiSize, TuiStyle,
};
use warpui_core::event::ModifiersState;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView};

use super::config::{
    AsciiArtError, AsciiArtMask, ReloadObjectOutcome, ZeroStateAnimationConfig,
    ZeroStateAnimationLoadFailure, ZeroStateShape, resolve_ascii_art_path,
};
use super::{
    BUILT_IN_LOGO_CELL_ASPECT_RATIO, LogoCell, LogoSurface, MAX_INTERACTIVE_RADIANS_PER_SECOND,
    MIN_ANIMATION_COLS, MIN_ANIMATION_ROWS, MOMENTUM_SETTLE_DURATION, WarpLogoStyles,
    ZeroStateAnimationElement, ZeroStateInteractionHandle, configured_idle_velocity,
    fitted_logo_size, idle_angle, logo_frame_at, object_frame_at, object_frame_at_angle,
    star_count_for_size, warp_logo_contains,
};

const PANEL_SIZE: TuiSize = TuiSize::new(52, 20);
const DIAMOND_ART: &str = "   #\n  ###\n #####\n  ###\n   #\n";
const ROCKET_ART: &str = "    #\n   ###\n  ####\n #####\n   ###\n  #  #\n";
const WARP_W_ART: &str = "#       #\n#       #\n#   #   #\n#  # #  #\n ##   ##\n";

fn custom_config(
    art: &str,
    rotation_period_secs: f64,
    extrusion_depth: f64,
) -> ZeroStateAnimationConfig {
    ZeroStateAnimationConfig {
        active_object: TuiZeroStateObject::BuiltIn,
        shape: Arc::new(ZeroStateShape::Ascii(AsciiArtMask::parse(art).unwrap())),
        rotation_period: Duration::from_secs_f64(rotation_period_secs),
        extrusion_depth,
        load_failure: None,
    }
}
#[test]
fn starfield_density_scales_with_the_full_panel_area() {
    assert_eq!(star_count_for_size(TuiSize::new(18, 7)), 18);
    assert_eq!(star_count_for_size(PANEL_SIZE), 36);
    assert_eq!(star_count_for_size(TuiSize::new(104, 20)), 72);
    assert_eq!(star_count_for_size(TuiSize::new(1_000, 200)), 6_923);
    assert_eq!(star_count_for_size(TuiSize::new(2_000, 200)), 8_192);
    assert_eq!(star_count_for_size(TuiSize::new(u16::MAX, u16::MAX)), 8_192);
}

fn logo_cells(frame: &super::LogoFrame) -> Vec<(usize, usize, LogoCell)> {
    frame
        .iter_cells()
        .filter(|(_, _, cell)| cell.surface != LogoSurface::Background)
        .collect()
}

fn write_art(config_dir: &Path, relative_path: &str, art: &str) -> PathBuf {
    let path = config_dir.join(relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, art).unwrap();
    path
}

struct AnimationTestView {
    config: Arc<ZeroStateAnimationConfig>,
}

impl Entity for AnimationTestView {
    type Event = ();
}

impl TypedActionView for AnimationTestView {
    type Action = ();
}

impl TuiView for AnimationTestView {
    fn ui_name() -> &'static str {
        "AnimationTestView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        let style = TuiStyle::default();
        ZeroStateAnimationElement::new(
            AnimationClock::starting_at(Duration::ZERO),
            self.config.clone(),
            super::ZeroStateInteractionHandle::default(),
            WarpLogoStyles {
                front: style,
                back: style,
                side: style,
                background: style,
            },
        )
        .finish()
    }
}

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
    assert_eq!(
        fitted_logo_size(TuiSize::new(100, 40), BUILT_IN_LOGO_CELL_ASPECT_RATIO),
        Some((43, 17))
    );
    assert_eq!(
        fitted_logo_size(TuiSize::new(30, 12), BUILT_IN_LOGO_CELL_ASPECT_RATIO),
        Some((25, 10))
    );
    assert_eq!(fitted_logo_size(TuiSize::new(100, 40), 4.0), Some((68, 17)));
}

#[test]
fn animation_is_hidden_when_the_panel_is_too_small() {
    assert!(logo_frame_at(Duration::ZERO, TuiSize::new(17, 20)).is_none());
    assert!(logo_frame_at(Duration::ZERO, TuiSize::new(30, 6)).is_none());
}

#[test]
fn ascii_parser_normalizes_crlf_trims_borders_and_pads_ragged_rows() {
    let mask =
        AsciiArtMask::parse("\r\n     \r\n   #\r\n  ###\r\n #####\r\n  ##\r\n     \r\n").unwrap();

    assert_eq!(mask.size(), (5, 4));
    let shape = ZeroStateShape::Ascii(mask);
    assert!(shape.contains(0.0, -1.0));
    assert!(shape.contains(-0.5, 0.5));
    assert!(!shape.contains(1.0, 1.0));
}

#[test]
fn representative_ascii_fixtures_have_distinct_dimensions() {
    assert_eq!(AsciiArtMask::parse(DIAMOND_ART).unwrap().size(), (5, 5));
    assert_eq!(AsciiArtMask::parse(ROCKET_ART).unwrap().size(), (5, 6));
    assert_eq!(AsciiArtMask::parse(WARP_W_ART).unwrap().size(), (9, 5));
}

#[test]
fn ascii_parser_rejects_invalid_empty_and_oversized_input() {
    assert!(matches!(
        AsciiArtMask::parse("\t#"),
        Err(AsciiArtError::InvalidCharacter)
    ));
    assert!(matches!(
        AsciiArtMask::parse("é"),
        Err(AsciiArtError::InvalidCharacter)
    ));
    assert!(matches!(
        AsciiArtMask::parse("  \n  \n"),
        Err(AsciiArtError::Empty)
    ));
    assert!(matches!(
        AsciiArtMask::parse(&"#".repeat(129)),
        Err(AsciiArtError::TooManyColumns { .. })
    ));
    assert!(matches!(
        AsciiArtMask::parse(&"#\n".repeat(65)),
        Err(AsciiArtError::TooManyRows { .. })
    ));
    assert!(matches!(
        AsciiArtMask::parse(&" ".repeat(65 * 1024)),
        Err(AsciiArtError::TooLarge { .. })
    ));
}

#[test]
fn relative_ascii_paths_resolve_from_the_tui_config_directory() {
    let config_dir = Path::new("/tmp/warp-tui-config");
    assert_eq!(
        resolve_ascii_art_path(Path::new("logos/diamond.txt"), config_dir),
        config_dir.join("logos/diamond.txt")
    );
    assert_eq!(
        resolve_ascii_art_path(Path::new("/tmp/rocket.txt"), config_dir),
        PathBuf::from("/tmp/rocket.txt")
    );
}

#[test]
fn startup_loader_reads_relative_ascii_art_and_retains_motion_settings() {
    let temp_dir = TempDir::new().unwrap();
    write_art(temp_dir.path(), "logos/rocket.txt", ROCKET_ART);
    let config = ZeroStateAnimationConfig::load(
        &TuiZeroStateObject::AsciiFile {
            path: PathBuf::from("logos/rocket.txt"),
        },
        3.5,
        0.3,
        temp_dir.path(),
    );

    assert_eq!(config.rotation_period, Duration::from_secs_f64(3.5));
    assert_eq!(config.extrusion_depth, 0.3);
    assert_eq!(config.load_failure(), None);
    let ZeroStateShape::Ascii(mask) = config.shape.as_ref() else {
        panic!("valid custom art should produce an ASCII shape");
    };
    assert_eq!(mask.size(), (5, 6));
}

#[test]
fn startup_loader_falls_back_for_missing_or_invalid_art_only() {
    let temp_dir = TempDir::new().unwrap();
    write_art(temp_dir.path(), "invalid.txt", "\tinvalid");
    for path in ["missing.txt", "invalid.txt"] {
        let config = ZeroStateAnimationConfig::load(
            &TuiZeroStateObject::AsciiFile {
                path: PathBuf::from(path),
            },
            7.0,
            0.4,
            temp_dir.path(),
        );

        assert!(matches!(config.shape.as_ref(), ZeroStateShape::BuiltInWarp));
        assert_eq!(config.rotation_period, Duration::from_secs(7));
        assert_eq!(config.extrusion_depth, 0.4);
        assert_eq!(
            config.load_failure(),
            Some(ZeroStateAnimationLoadFailure::InitialLoad)
        );
    }
}

#[test]
fn object_path_change_reloads_shape_without_changing_motion_settings() {
    let temp_dir = TempDir::new().unwrap();
    write_art(temp_dir.path(), "diamond.txt", DIAMOND_ART);
    write_art(temp_dir.path(), "rocket.txt", ROCKET_ART);
    let diamond = TuiZeroStateObject::AsciiFile {
        path: PathBuf::from("diamond.txt"),
    };
    let rocket = TuiZeroStateObject::AsciiFile {
        path: PathBuf::from("rocket.txt"),
    };
    let mut config = ZeroStateAnimationConfig::load(&diamond, 3.5, 0.3, temp_dir.path());
    let initial = object_frame_at(Duration::ZERO, PANEL_SIZE, &config).unwrap();

    assert_eq!(
        config.reload_object(&rocket, temp_dir.path()),
        ReloadObjectOutcome::Reloaded
    );
    assert_eq!(config.load_failure(), None);
    assert_eq!(config.rotation_period, Duration::from_secs_f64(3.5));
    assert_eq!(config.extrusion_depth, 0.3);
    let ZeroStateShape::Ascii(mask) = config.shape.as_ref() else {
        panic!("valid replacement art should produce an ASCII shape");
    };
    assert_eq!(mask.size(), (5, 6));
    let reloaded = object_frame_at(Duration::ZERO, PANEL_SIZE, &config).unwrap();
    assert_ne!(logo_cells(&initial), logo_cells(&reloaded));
}

#[test]
fn linked_file_content_change_is_ignored_when_object_path_is_unchanged() {
    let temp_dir = TempDir::new().unwrap();
    write_art(temp_dir.path(), "active.txt", DIAMOND_ART);
    let object = TuiZeroStateObject::AsciiFile {
        path: PathBuf::from("active.txt"),
    };
    let mut config = ZeroStateAnimationConfig::load(&object, 4.0, 0.18, temp_dir.path());

    write_art(temp_dir.path(), "active.txt", ROCKET_ART);

    assert_eq!(
        config.reload_object(&object, temp_dir.path()),
        ReloadObjectOutcome::Unchanged
    );
    let ZeroStateShape::Ascii(mask) = config.shape.as_ref() else {
        panic!("unchanged path should retain the loaded ASCII shape");
    };
    assert_eq!(mask.size(), (5, 5));
}

#[test]
fn invalid_object_path_change_keeps_last_valid_shape() {
    let temp_dir = TempDir::new().unwrap();
    write_art(temp_dir.path(), "diamond.txt", DIAMOND_ART);
    let diamond = TuiZeroStateObject::AsciiFile {
        path: PathBuf::from("diamond.txt"),
    };
    let missing = TuiZeroStateObject::AsciiFile {
        path: PathBuf::from("missing.txt"),
    };
    let mut config = ZeroStateAnimationConfig::load(&diamond, 4.0, 0.18, temp_dir.path());

    assert_eq!(
        config.reload_object(&missing, temp_dir.path()),
        ReloadObjectOutcome::Failed
    );
    assert_eq!(
        config.load_failure(),
        Some(ZeroStateAnimationLoadFailure::Reload)
    );
    let ZeroStateShape::Ascii(mask) = config.shape.as_ref() else {
        panic!("invalid replacement path should retain the previous ASCII shape");
    };
    assert_eq!(mask.size(), (5, 5));
}

#[test]
fn settings_model_reloads_only_object_changes() {
    let temp_dir = TempDir::new().unwrap();
    let diamond_path = write_art(temp_dir.path(), "diamond.txt", DIAMOND_ART);
    let rocket_path = write_art(temp_dir.path(), "rocket.txt", ROCKET_ART);
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| TuiZeroStateSettings {
                object: TuiZeroStateObjectSetting::new(Some(TuiZeroStateObject::AsciiFile {
                    path: diamond_path,
                })),
                rotation_period_seconds: TuiZeroStateRotationPeriodSecondsSetting::new(None),
                extrusion_depth: TuiZeroStateExtrusionDepthSetting::new(None),
            });
            ZeroStateAnimationConfig::register(ctx);
        });

        let initial_period = app.read(|ctx| ZeroStateAnimationConfig::as_ref(ctx).rotation_period);
        app.update(|ctx| {
            TuiZeroStateSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .object
                    .load_value(
                        TuiZeroStateObject::AsciiFile { path: rocket_path },
                        true,
                        ctx,
                    )
                    .unwrap();
                settings
                    .rotation_period_seconds
                    .load_value(
                        serde_json::from_value::<TuiZeroStateRotationPeriodSeconds>(
                            serde_json::json!(12.0),
                        )
                        .unwrap(),
                        true,
                        ctx,
                    )
                    .unwrap();
            });
        });

        app.read(|ctx| {
            let config = ZeroStateAnimationConfig::as_ref(ctx);
            assert_eq!(config.rotation_period, initial_period);
            let ZeroStateShape::Ascii(mask) = config.shape.as_ref() else {
                panic!("object setting event should reload the replacement ASCII shape");
            };
            assert_eq!(mask.size(), (5, 6));
        });
    });
}

fn assert_approx_eq(left: f64, right: f64) {
    assert!(
        (left - right).abs() < 1e-9,
        "expected {left} to approximately equal {right}"
    );
}

fn left_mouse_down(position: TuiPoint) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position,
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

fn left_mouse_dragged(position: TuiPoint) -> TuiEvent {
    TuiEvent::LeftMouseDragged {
        position,
        modifiers: ModifiersState::default(),
    }
}

fn left_mouse_up(position: TuiPoint) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position,
        modifiers: ModifiersState::default(),
    }
}

#[test]
fn idle_frames_are_unchanged_without_interaction() {
    for config in [
        ZeroStateAnimationConfig::default(),
        custom_config(ROCKET_ART, 4.0, 0.18),
    ] {
        for elapsed in [
            Duration::ZERO,
            Duration::from_millis(775),
            Duration::from_secs(3),
        ] {
            let idle = object_frame_at(elapsed, PANEL_SIZE, &config).unwrap();
            let explicit = object_frame_at_angle(
                elapsed,
                PANEL_SIZE,
                &config,
                idle_angle(elapsed, configured_idle_velocity(&config)),
            )
            .unwrap();
            assert_eq!(idle, explicit);
        }
    }
}

#[test]
fn background_stars_ignore_interactive_object_phase() {
    let elapsed = Duration::from_millis(850);
    let mut before_interaction = super::LogoFrame::new(PANEL_SIZE);
    let mut during_interaction = super::LogoFrame::new(PANEL_SIZE);
    super::draw_background_stars(&mut before_interaction, elapsed);
    super::draw_background_stars(&mut during_interaction, elapsed);
    assert_eq!(before_interaction, during_interaction);
}

#[test]
fn press_and_click_without_horizontal_motion_preserve_phase_and_velocity() {
    let interaction = ZeroStateInteractionHandle::default();
    let now = Instant::now();
    let idle_velocity = TAU / 5.0;
    let before = interaction.resolve_at(Duration::from_secs(1), idle_velocity, now);

    assert!(interaction.press_at(TuiPoint::new(20, 10), now));
    let pressed = interaction.resolve_at(Duration::from_secs(1), idle_velocity, now);
    assert_approx_eq(pressed.angle, before.angle);
    assert_approx_eq(pressed.velocity, before.velocity);

    let released_at = now + Duration::from_millis(40);
    assert!(interaction.release_at(Duration::from_millis(1040), idle_velocity, released_at));
    let released = interaction.resolve_at(Duration::from_millis(1040), idle_velocity, released_at);
    assert_approx_eq(
        released.angle,
        idle_angle(Duration::from_millis(1040), idle_velocity),
    );
    assert_approx_eq(released.velocity, idle_velocity);
}

#[test]
fn horizontal_drag_scrubs_forward_without_reversal() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(1, 1), start));
    let drag_at = start + Duration::from_millis(100);
    assert!(interaction.drag_at(
        TuiPoint::new(33, 1),
        Duration::from_millis(100),
        idle_velocity,
        drag_at,
    ));
    let after_forward = interaction.resolve_at(Duration::from_millis(100), idle_velocity, drag_at);
    assert_approx_eq(
        after_forward.angle,
        idle_angle(Duration::from_millis(100), idle_velocity) + TAU,
    );

    assert!(interaction.drag_at(
        TuiPoint::new(20, 1),
        Duration::from_millis(110),
        idle_velocity,
        start + Duration::from_millis(110),
    ));
    assert!(interaction.drag_at(
        TuiPoint::new(20, 8),
        Duration::from_millis(120),
        idle_velocity,
        start + Duration::from_millis(120),
    ));
    let after_opposite = interaction.resolve_at(
        Duration::from_millis(120),
        idle_velocity,
        start + Duration::from_millis(120),
    );
    assert_approx_eq(after_opposite.angle, after_forward.angle);
}

#[test]
fn release_velocity_clamps_to_zero_and_playful_maximum() {
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;

    let zero = ZeroStateInteractionHandle::default();
    assert!(zero.press_at(TuiPoint::new(10, 1), start));
    assert!(zero.drag_at(
        TuiPoint::new(9, 1),
        Duration::from_millis(20),
        idle_velocity,
        start + Duration::from_millis(20),
    ));
    assert!(zero.release_at(
        Duration::from_millis(40),
        idle_velocity,
        start + Duration::from_millis(40),
    ));
    assert_approx_eq(
        zero.resolve_at(
            Duration::from_millis(40),
            idle_velocity,
            start + Duration::from_millis(40),
        )
        .velocity,
        0.0,
    );

    let maximum = ZeroStateInteractionHandle::default();
    assert!(maximum.press_at(TuiPoint::new(1, 1), start));
    assert!(maximum.drag_at(
        TuiPoint::new(200, 1),
        Duration::from_millis(1),
        idle_velocity,
        start + Duration::from_millis(1),
    ));
    assert!(maximum.release_at(
        Duration::from_millis(2),
        idle_velocity,
        start + Duration::from_millis(2),
    ));
    assert_approx_eq(
        maximum
            .resolve_at(
                Duration::from_millis(2),
                idle_velocity,
                start + Duration::from_millis(2),
            )
            .velocity,
        MAX_INTERACTIVE_RADIANS_PER_SECOND,
    );
}

#[test]
fn released_velocity_smoothly_settles_to_idle_in_three_seconds() {
    for period in [Duration::from_secs(1), Duration::from_secs(60)] {
        let interaction = ZeroStateInteractionHandle::default();
        let start = Instant::now();
        let idle_velocity = TAU / period.as_secs_f64();
        assert!(interaction.press_at(TuiPoint::new(1, 1), start));
        let release_at = start + Duration::from_millis(40);
        assert!(interaction.drag_at(
            TuiPoint::new(40, 1),
            Duration::from_millis(20),
            idle_velocity,
            start + Duration::from_millis(20),
        ));
        let angle_before_release =
            interaction.resolve_at(Duration::from_millis(40), idle_velocity, release_at);
        assert!(interaction.release_at(Duration::from_millis(40), idle_velocity, release_at,));
        let released = interaction.resolve_at(Duration::from_millis(40), idle_velocity, release_at);
        assert_approx_eq(released.angle, angle_before_release.angle);

        let halfway = interaction.resolve_at(
            Duration::from_millis(1540),
            idle_velocity,
            release_at + Duration::from_millis(1500),
        );
        assert_approx_eq(halfway.velocity, (released.velocity + idle_velocity) * 0.5);
        let settled = interaction.resolve_at(
            Duration::from_millis(3040),
            idle_velocity,
            release_at + MOMENTUM_SETTLE_DURATION,
        );
        assert_approx_eq(settled.velocity, idle_velocity);
        let after = interaction.resolve_at(
            Duration::from_millis(4040),
            idle_velocity,
            release_at + MOMENTUM_SETTLE_DURATION + Duration::from_secs(1),
        );
        assert_approx_eq(after.velocity, idle_velocity);
        assert_approx_eq(after.angle - settled.angle, idle_velocity);
    }
}

fn interaction_after_flick(start: Instant) -> (ZeroStateInteractionHandle, Instant, f64) {
    let interaction = ZeroStateInteractionHandle::default();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(1, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(9, 1),
        Duration::from_millis(80),
        idle_velocity,
        start + Duration::from_millis(80),
    ));
    let release_at = start + Duration::from_millis(100);
    assert!(interaction.release_at(Duration::from_millis(100), idle_velocity, release_at,));
    (interaction, release_at, idle_velocity)
}

#[test]
fn interaction_phase_is_independent_of_repaint_schedule() {
    let start = Instant::now();
    let schedules = [
        Duration::from_millis(66),
        Duration::from_millis(33),
        Duration::from_millis(417),
    ];
    let mut results = Vec::new();
    for cadence in schedules {
        let (interaction, release_at, idle_velocity) = interaction_after_flick(start);
        let mut elapsed = cadence;
        while elapsed < Duration::from_secs(2) {
            let _ = interaction.resolve_at(
                Duration::from_millis(100) + elapsed,
                idle_velocity,
                release_at + elapsed,
            );
            elapsed += cadence;
        }
        results.push(interaction.resolve_at(
            Duration::from_millis(2100),
            idle_velocity,
            release_at + Duration::from_secs(2),
        ));
    }
    for result in &results[1..] {
        assert_approx_eq(result.angle, results[0].angle);
        assert_approx_eq(result.velocity, results[0].velocity);
    }
}

#[test]
fn interaction_survives_resize_and_rebuild_then_resets_on_zero_state_exit() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(1, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(8, 1),
        Duration::from_millis(80),
        idle_velocity,
        start + Duration::from_millis(80),
    ));
    let before_resize = interaction.resolve_at(
        Duration::from_millis(80),
        idle_velocity,
        start + Duration::from_millis(80),
    );
    let config = ZeroStateAnimationConfig::default();
    let small = object_frame_at_angle(
        Duration::from_millis(80),
        TuiSize::new(52, 20),
        &config,
        before_resize.angle,
    )
    .unwrap();
    let large = object_frame_at_angle(
        Duration::from_millis(80),
        TuiSize::new(100, 35),
        &config,
        before_resize.angle,
    )
    .unwrap();
    assert!(small.object_bounds().is_some());
    assert!(large.object_bounds().is_some());
    let after_resize = interaction.resolve_at(
        Duration::from_millis(80),
        idle_velocity,
        start + Duration::from_millis(80),
    );
    assert_approx_eq(after_resize.angle, before_resize.angle);
    assert_approx_eq(after_resize.velocity, before_resize.velocity);

    interaction.set_visible(false);
    interaction.set_visible(true);
    let returned = interaction.resolve_at(
        Duration::from_secs(2),
        idle_velocity,
        start + Duration::from_secs(2),
    );
    assert_approx_eq(
        returned.angle,
        idle_angle(Duration::from_secs(2), idle_velocity),
    );
    assert_approx_eq(returned.velocity, idle_velocity);
}

#[test]
fn drag_starts_only_inside_current_object_bounds_and_captures_through_release() {
    App::test((), |mut app| async move {
        let config = Arc::new(ZeroStateAnimationConfig::default());
        let interaction = ZeroStateInteractionHandle::default();
        let (_, view) = app.update(|ctx| {
            ctx.add_tui_window(AddWindowOptions::default(), {
                let config = config.clone();
                move |_| AnimationTestView { config }
            })
        });

        app.read(|ctx| {
            let style = TuiStyle::default();
            let mut element = ZeroStateAnimationElement::new(
                AnimationClock::starting_at(Duration::ZERO),
                config,
                interaction,
                WarpLogoStyles {
                    front: style,
                    back: style,
                    side: style,
                    background: style,
                },
            );
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = element.layout(TuiConstraint::loose(PANEL_SIZE), &mut layout_ctx, ctx);
            let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            let bounds = element.object_bounds.expect("rendered object target");
            let inside = TuiPoint::new(
                ((bounds.min_x + bounds.max_x) / 2) as u16,
                ((bounds.min_y + bounds.max_y) / 2) as u16,
            );
            let outside = TuiPoint::new(PANEL_SIZE.width + 10, PANEL_SIZE.height + 10);
            let mut event_views = EntityIdMap::default();
            let mut event_ctx =
                TuiEventContext::new(Rc::new(paint_ctx.scene.clone()), &mut event_views);
            event_ctx.set_origin_view(Some(view.id()));

            assert!(element.dispatch_event(&left_mouse_down(inside), &mut event_ctx, ctx));
            assert!(element.dispatch_event(&left_mouse_dragged(outside), &mut event_ctx, ctx));
            assert!(element.dispatch_event(&left_mouse_up(outside), &mut event_ctx, ctx));
            assert!(!element.dispatch_event(&left_mouse_down(outside), &mut event_ctx, ctx));
            assert!(!element.dispatch_event(&left_mouse_dragged(outside), &mut event_ctx, ctx));
            assert!(!element.dispatch_event(&left_mouse_up(outside), &mut event_ctx, ctx));
        });
    });
}

#[test]
fn object_bounds_cover_custom_and_edge_on_objects_but_not_background_stars() {
    for (config, angle) in [
        (ZeroStateAnimationConfig::default(), TAU * 0.25),
        (custom_config(ROCKET_ART, 4.0, 0.18), 0.0),
    ] {
        let frame =
            object_frame_at_angle(Duration::from_millis(400), PANEL_SIZE, &config, angle).unwrap();
        let bounds = frame.object_bounds().expect("object bounds");
        assert!(bounds.contains(TuiLocalPoint::new(
            ((bounds.min_x + bounds.max_x) / 2) as i32,
            ((bounds.min_y + bounds.max_y) / 2) as i32,
        )));
        assert!(frame.iter_cells().any(|(x, y, cell)| {
            cell.surface == LogoSurface::Background
                && !bounds.contains(TuiLocalPoint::new(x as i32, y as i32))
        }));
    }
}

#[test]
fn hidden_or_too_small_animation_has_no_drag_target() {
    let interaction = ZeroStateInteractionHandle::default();
    interaction.set_visible(false);
    assert!(!interaction.press_at(TuiPoint::new(1, 1), Instant::now()));
    assert!(
        object_frame_at(
            Duration::ZERO,
            TuiSize::new(MIN_ANIMATION_COLS - 1, MIN_ANIMATION_ROWS),
            &ZeroStateAnimationConfig::default(),
        )
        .is_none()
    );
}

#[test]
fn representative_ascii_shapes_rotate_through_front_side_and_back() {
    for art in [DIAMOND_ART, ROCKET_ART, WARP_W_ART] {
        let config = custom_config(art, 4.0, 0.18);
        let face = object_frame_at(Duration::ZERO, PANEL_SIZE, &config).unwrap();
        let edge = object_frame_at(Duration::from_secs(1), PANEL_SIZE, &config).unwrap();
        let back = object_frame_at(Duration::from_secs(2), PANEL_SIZE, &config).unwrap();

        assert!(logo_cells(&face).len() > 20);
        assert!(
            edge.iter_cells()
                .any(|(_, _, cell)| cell.surface == LogoSurface::Side)
        );
        assert!(
            back.iter_cells()
                .all(|(_, _, cell)| cell.surface != LogoSurface::Front)
        );
        assert!(
            back.iter_cells()
                .any(|(_, _, cell)| cell.surface == LogoSurface::Back)
        );
        assert_ne!(logo_cells(&face), logo_cells(&edge));
    }
}

#[test]
fn configured_period_controls_phase_and_repeats_exactly() {
    let four_seconds = custom_config(ROCKET_ART, 4.0, 0.18);
    let eight_seconds = custom_config(ROCKET_ART, 8.0, 0.18);
    let four_second_quarter =
        object_frame_at(Duration::from_secs(1), PANEL_SIZE, &four_seconds).unwrap();
    let eight_second_quarter =
        object_frame_at(Duration::from_secs(2), PANEL_SIZE, &eight_seconds).unwrap();
    let revolved = object_frame_at(Duration::from_secs(4), PANEL_SIZE, &four_seconds).unwrap();
    let initial = object_frame_at(Duration::ZERO, PANEL_SIZE, &four_seconds).unwrap();

    assert_eq!(
        logo_cells(&four_second_quarter),
        logo_cells(&eight_second_quarter)
    );
    assert_eq!(logo_cells(&initial), logo_cells(&revolved));
}

#[test]
fn configured_depth_changes_edge_on_width() {
    let shallow = custom_config(DIAMOND_ART, 4.0, 0.02);
    let deep = custom_config(DIAMOND_ART, 4.0, 0.5);
    let shallow = object_frame_at(Duration::from_secs(1), PANEL_SIZE, &shallow).unwrap();
    let deep = object_frame_at(Duration::from_secs(1), PANEL_SIZE, &deep).unwrap();
    let horizontal_span = |frame: &super::LogoFrame| {
        let cells = logo_cells(frame);
        let min = cells.iter().map(|(x, _, _)| *x).min().unwrap();
        let max = cells.iter().map(|(x, _, _)| *x).max().unwrap();
        max - min + 1
    };

    assert!(horizontal_span(&deep) > horizontal_span(&shallow));
}

#[test]
fn custom_shapes_preserve_their_authored_cell_aspect() {
    assert_eq!(fitted_logo_size(PANEL_SIZE, 1.0), Some((17, 17)));
    assert_eq!(fitted_logo_size(PANEL_SIZE, 9.0 / 5.0), Some((31, 17)));
}

#[test]
fn extreme_ascii_aspect_ratios_clamp_to_a_visible_minimum() {
    let wide = custom_config(&"#".repeat(128), 4.0, 0.18);
    let tall = custom_config(&"#\n".repeat(64), 4.0, 0.18);

    assert_eq!(fitted_logo_size(PANEL_SIZE, 128.0), Some((50, 5)));
    assert_eq!(fitted_logo_size(PANEL_SIZE, 1.0 / 64.0), Some((5, 17)));
    for config in [wide, tall] {
        let frame = object_frame_at(Duration::ZERO, PANEL_SIZE, &config).unwrap();
        assert!(!logo_cells(&frame).is_empty());
    }
}

#[test]
fn custom_animation_element_paints_and_requests_another_frame() {
    App::test((), |mut app| async move {
        let config = Arc::new(custom_config(WARP_W_ART, 4.0, 0.18));
        let (_, view) = app.update(|ctx| {
            ctx.add_tui_window(AddWindowOptions::default(), move |_| AnimationTestView {
                config,
            })
        });
        let mut presenter = TuiPresenter::new();
        let frame = app.update(|ctx| {
            presenter.present(
                ctx,
                &view,
                TuiRect::new(0, 0, PANEL_SIZE.width, PANEL_SIZE.height),
            )
        });

        assert!(
            frame
                .buffer
                .to_lines()
                .iter()
                .any(|line| line.chars().any(|character| character != ' '))
        );
        assert!(frame.repaint_at.is_some());
    });
}
