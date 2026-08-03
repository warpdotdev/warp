use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use tempfile::TempDir;
use warp::settings::{
    TuiZeroStateExtrusionDepthSetting, TuiZeroStateFreezeAnimationWhenUnfocusedSetting,
    TuiZeroStateObject, TuiZeroStateObjectSetting, TuiZeroStateRotationPeriodSeconds,
    TuiZeroStateRotationPeriodSecondsSetting, TuiZeroStateSettings,
    TuiZeroStateShowAnimationSetting, TuiZeroStateShowChangelogSetting, TuiZeroStateShowMcpSetting,
    TuiZeroStateShowProjectInfoSetting, TuiZeroStateShowSignedInUserSetting,
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
    ActivePress, BUILT_IN_LOGO_CELL_ASPECT_RATIO, FLICK_RELEASE_VELOCITY_GAIN, LogoCell, LogoGlyph,
    LogoProjector, LogoSurface, MAX_INTERACTIVE_RADIANS_PER_SECOND, MIN_ANIMATION_COLS,
    MIN_ANIMATION_ROWS, MOMENTUM_SETTLE_DURATION, REPAINT_INTERVAL, WarpLogoStyles,
    ZeroStateAnimationElement, ZeroStateInteractionHandle, configured_idle_velocity,
    face_linger_angle, fitted_logo_size, glyph_for_tangent, idle_angle, is_ghost_stipple_cell,
    logo_frame_at, object_frame_at, object_frame_at_angle, object_frame_at_angle_with_background,
    object_frame_at_with_background, rotation_angle, star_count_for_size, starfield_emitter_x,
    warp_logo_contains,
};

const PANEL_SIZE: TuiSize = TuiSize::new(52, 20);
const DIAMOND_ART: &str = "   #\n  ###\n #####\n  ###\n   #\n";
const ROCKET_ART: &str = "    #\n   ###\n  ####\n #####\n   ###\n  #  #\n";
const WARP_W_ART: &str = "#       #\n#       #\n#   #   #\n#  # #  #\n ##   ##\n";

#[test]
fn idle_animation_uses_a_fifteen_frame_per_second_cadence() {
    assert_eq!(REPAINT_INTERVAL, Duration::from_millis(66));
}

#[test]
fn retained_projector_matches_reference_across_resizes_and_shape_changes() {
    let mut projector = LogoProjector::default();
    let configs = [
        ZeroStateAnimationConfig::default(),
        custom_config(ROCKET_ART, 4.0, 0.18),
    ];
    for config in &configs {
        for size in [PANEL_SIZE, TuiSize::new(32, 12), TuiSize::new(52, 30)] {
            for elapsed in [
                Duration::ZERO,
                Duration::from_millis(850),
                Duration::from_secs(2),
            ] {
                let angle = idle_angle(elapsed, configured_idle_velocity(config));
                let retained = projector
                    .project(elapsed, size, config, angle, true)
                    .cloned();
                let reference =
                    object_frame_at_angle_with_background(elapsed, size, config, angle, true);
                assert_eq!(retained, reference);
            }
        }
    }
}

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
fn release_velocity_from_samples(samples: &[(u16, u64)], release_at_ms: u64) -> Option<f64> {
    let start = Instant::now();
    let (first_x, first_at_ms) = samples[0];
    let mut press = ActivePress::new(
        TuiPoint::new(first_x, 1),
        start + Duration::from_millis(first_at_ms),
    );
    for &(x, at_ms) in &samples[1..] {
        press.record_horizontal_sample(TuiPoint::new(x, 1), start + Duration::from_millis(at_ms));
    }
    press.release_velocity(start + Duration::from_millis(release_at_ms))
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

#[test]
fn starfield_emitter_tracks_the_centered_logo_panel() {
    assert_eq!(starfield_emitter_x(TuiSize::new(80, 20), 48, 32), 63.5);
    assert_eq!(starfield_emitter_x(TuiSize::new(120, 20), 48, 32), 83.5);
    assert_eq!(starfield_emitter_x(TuiSize::new(160, 20), 48, 32), 103.5);
    assert_eq!(
        starfield_emitter_x(TuiSize::new(60, 20), 48, 32),
        29.5,
        "when the logo is hidden, stars should fall back to the screen center"
    );
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
fn rotation_lingers_on_faces_and_preserves_cardinal_angles() {
    let period = Duration::from_secs(5);
    for (elapsed, expected) in [
        (Duration::ZERO, 0.0),
        (Duration::from_millis(1_250), std::f64::consts::FRAC_PI_2),
        (Duration::from_millis(2_500), std::f64::consts::PI),
        (
            Duration::from_millis(3_750),
            3.0 * std::f64::consts::FRAC_PI_2,
        ),
    ] {
        let actual = rotation_angle(elapsed, period);
        assert!(
            (actual - expected).abs() < f64::EPSILON * 4.0,
            "expected {expected}, got {actual}"
        );
    }

    let face_step =
        rotation_angle(Duration::from_millis(100), period) - rotation_angle(Duration::ZERO, period);
    let edge_step = rotation_angle(Duration::from_millis(1_350), period)
        - rotation_angle(Duration::from_millis(1_250), period);
    assert!(
        face_step < edge_step,
        "the logo should move more slowly near a face ({face_step}) than near an edge ({edge_step})"
    );
}

#[test]
fn face_lingering_reparameterizes_interactive_and_reverse_phases_too() {
    let period = Duration::from_secs(5);
    let idle_velocity = TAU / period.as_secs_f64();
    // The paint path eases whatever phase the interaction resolves, so an
    // uninterrupted idle frame still matches the composed idle helper.
    for elapsed in [
        Duration::from_millis(400),
        Duration::from_millis(1_900),
        Duration::from_millis(4_100),
    ] {
        assert_approx_eq(
            face_linger_angle(idle_angle(elapsed, idle_velocity)),
            rotation_angle(elapsed, period),
        );
    }

    // Cardinal angles survive the negative phases a reverse flick produces.
    for quarter_turns in -8..=8 {
        let phase = f64::from(quarter_turns) * std::f64::consts::FRAC_PI_2;
        assert_approx_eq(face_linger_angle(phase), phase);
    }

    // The mapping is strictly increasing, so scrubbing never stalls or
    // backtracks against the pointer.
    let mut previous = f64::NEG_INFINITY;
    for step in -400..=400 {
        let eased = face_linger_angle(f64::from(step) * TAU / 100.0);
        assert!(eased > previous, "face lingering must stay monotonic");
        previous = eased;
    }
}

#[test]
fn tangent_glyphs_use_equal_angular_sectors() {
    assert_eq!(glyph_for_tangent(0.0, 0.0), None);
    assert_eq!(glyph_for_tangent(2.5, 1.0), Some(LogoGlyph::Horizontal));
    assert_eq!(glyph_for_tangent(2.0, 1.0), Some(LogoGlyph::Backslash));
    assert_eq!(glyph_for_tangent(1.0, 2.0), Some(LogoGlyph::Backslash));
    assert_eq!(glyph_for_tangent(1.0, 2.5), Some(LogoGlyph::Vertical));
    assert_eq!(glyph_for_tangent(1.0, -1.0), Some(LogoGlyph::ForwardSlash));
}

#[test]
fn screen_space_ghost_stipple_is_sparse_and_deterministic() {
    let stippled_cells = (0..32)
        .flat_map(|x| (0..17).map(move |y| (x, y)))
        .filter(|(x, y)| is_ghost_stipple_cell(*x, *y))
        .count();

    assert_eq!(stippled_cells, 29);
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
fn adjacent_builtin_frames_have_bounded_occupancy_and_content_churn() {
    let config = ZeroStateAnimationConfig::default();
    let size = TuiSize::new(32, 28);
    let frames = (0..=76)
        .map(|frame| {
            object_frame_at_with_background(REPAINT_INTERVAL * frame, size, &config, false).unwrap()
        })
        .collect::<Vec<_>>();
    let (max_occupancy_changes, max_content_changes) = frames
        .windows(2)
        .map(|frames| {
            let occupancy_changes = frames[0]
                .cells
                .iter()
                .zip(&frames[1].cells)
                .filter(|(before, after)| before.is_some() != after.is_some())
                .count();
            let content_changes = frames[0]
                .cells
                .iter()
                .zip(&frames[1].cells)
                .filter(|(before, after)| before != after)
                .count();
            (occupancy_changes, content_changes)
        })
        .fold((0, 0), |maximums, changes| {
            (maximums.0.max(changes.0), maximums.1.max(changes.1))
        });

    assert!(
        max_occupancy_changes <= 80,
        "adjacent built-in frames changed occupancy in {max_occupancy_changes} cells"
    );
    assert!(
        max_content_changes <= 120,
        "adjacent built-in frames changed glyph or surface content in {max_content_changes} cells"
    );
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
    assert_eq!(
        fitted_logo_size(TuiSize::new(32, 28), BUILT_IN_LOGO_CELL_ASPECT_RATIO),
        Some((30, 12)),
        "the layout panel should keep the restored dev animation compact"
    );
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
                show_signed_in_user: TuiZeroStateShowSignedInUserSetting::new(None),
                show_changelog: TuiZeroStateShowChangelogSetting::new(None),
                show_project_info: TuiZeroStateShowProjectInfoSetting::new(None),
                show_mcp: TuiZeroStateShowMcpSetting::new(None),
                show_animation: TuiZeroStateShowAnimationSetting::new(None),
                freeze_animation_when_unfocused:
                    TuiZeroStateFreezeAnimationWhenUnfocusedSetting::new(None),
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
    assert!(interaction.release_at(
        TuiPoint::new(20, 10),
        Duration::from_millis(1040),
        idle_velocity,
        released_at,
    ));
    let released = interaction.resolve_at(Duration::from_millis(1040), idle_velocity, released_at);
    assert_approx_eq(
        released.angle,
        idle_angle(Duration::from_millis(1040), idle_velocity),
    );
    assert_approx_eq(released.velocity, idle_velocity);
}

#[test]
fn horizontal_drag_scrubs_both_directions_with_retuned_sensitivity() {
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let drag_at = start + Duration::from_secs(1);
    let idle_at_drag = idle_angle(Duration::from_secs(1), idle_velocity);

    let forward = ZeroStateInteractionHandle::default();
    assert!(forward.press_at(TuiPoint::new(40, 1), start));
    assert!(forward.drag_at(
        TuiPoint::new(48, 1),
        Duration::from_secs(1),
        idle_velocity,
        drag_at,
    ));
    assert_approx_eq(
        forward
            .resolve_at(Duration::from_secs(1), idle_velocity, drag_at)
            .angle,
        idle_at_drag + TAU / 12.0,
    );

    let reverse = ZeroStateInteractionHandle::default();
    assert!(reverse.press_at(TuiPoint::new(40, 1), start));
    assert!(reverse.drag_at(
        TuiPoint::new(32, 1),
        Duration::from_secs(1),
        idle_velocity,
        drag_at,
    ));
    assert_approx_eq(
        reverse
            .resolve_at(Duration::from_secs(1), idle_velocity, drag_at)
            .angle,
        idle_at_drag - TAU / 12.0,
    );
}

#[test]
fn pause_before_first_drag_does_not_accumulate_rate_limit_allowance() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let drag_at = start + Duration::from_secs(5);
    assert!(interaction.press_at(TuiPoint::new(1, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(33, 1),
        Duration::from_secs(5),
        idle_velocity,
        drag_at,
    ));
    assert_approx_eq(
        interaction
            .resolve_at(Duration::from_secs(5), idle_velocity, drag_at)
            .angle,
        idle_angle(Duration::from_secs(5), idle_velocity)
            + MAX_INTERACTIVE_RADIANS_PER_SECOND * REPAINT_INTERVAL.as_secs_f64(),
    );
}

#[test]
fn fast_direction_reversal_is_rate_limited_from_the_last_applied_angle() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(100, 1), start));

    let forward_at = start + Duration::from_millis(100);
    assert!(interaction.drag_at(
        TuiPoint::new(200, 1),
        Duration::from_millis(100),
        idle_velocity,
        forward_at,
    ));
    let forward = interaction.resolve_at(Duration::from_millis(100), idle_velocity, forward_at);

    let reverse_at = start + Duration::from_millis(110);
    assert!(interaction.drag_at(
        TuiPoint::new(0, 1),
        Duration::from_millis(110),
        idle_velocity,
        reverse_at,
    ));
    let reversed = interaction.resolve_at(Duration::from_millis(110), idle_velocity, reverse_at);
    assert_approx_eq(
        forward.angle - reversed.angle,
        MAX_INTERACTIVE_RADIANS_PER_SECOND * 0.01,
    );
}

#[test]
fn vertical_only_drag_leaves_idle_motion_untouched() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(20, 1), start));
    let drag_at = start + Duration::from_millis(500);
    assert!(interaction.drag_at(
        TuiPoint::new(20, 8),
        Duration::from_millis(500),
        idle_velocity,
        drag_at,
    ));
    let during = interaction.resolve_at(Duration::from_millis(500), idle_velocity, drag_at);
    assert_approx_eq(
        during.angle,
        idle_angle(Duration::from_millis(500), idle_velocity),
    );
    assert_approx_eq(during.velocity, idle_velocity);

    let release_at = start + Duration::from_millis(800);
    assert!(interaction.release_at(
        TuiPoint::new(20, 8),
        Duration::from_millis(800),
        idle_velocity,
        release_at,
    ));
    let released = interaction.resolve_at(Duration::from_millis(800), idle_velocity, release_at);
    assert_approx_eq(
        released.angle,
        idle_angle(Duration::from_millis(800), idle_velocity),
    );
    assert_approx_eq(released.velocity, idle_velocity);
}

#[test]
fn recent_release_velocity_tracks_speed_for_the_same_distance() {
    let slow = release_velocity_from_samples(&[(100, 0), (104, 100), (108, 200)], 200).unwrap();
    let fast = release_velocity_from_samples(&[(100, 100), (104, 150), (108, 200)], 200).unwrap();

    assert_approx_eq(slow, 40.0 * FLICK_RELEASE_VELOCITY_GAIN * TAU / 96.0);
    assert_approx_eq(fast, 80.0 * FLICK_RELEASE_VELOCITY_GAIN * TAU / 96.0);
    assert!(fast > slow);
}

#[test]
fn recent_release_velocity_follows_the_final_direction_after_reversal() {
    let velocity =
        release_velocity_from_samples(&[(100, 0), (120, 50), (118, 100), (116, 150)], 150).unwrap();

    assert_approx_eq(velocity, -40.0 * FLICK_RELEASE_VELOCITY_GAIN * TAU / 96.0);
}

#[test]
fn recent_release_velocity_rejects_stale_or_one_cell_motion() {
    assert_eq!(
        release_velocity_from_samples(&[(100, 0), (104, 50), (108, 100)], 251),
        None,
    );
    assert_eq!(
        release_velocity_from_samples(&[(100, 0), (101, 50), (100, 100)], 100),
        None,
    );
}

#[test]
fn recent_release_velocity_is_hard_clamped() {
    assert_approx_eq(
        release_velocity_from_samples(&[(100, 0), (200, 30), (300, 60)], 60).unwrap(),
        MAX_INTERACTIVE_RADIANS_PER_SECOND,
    );
    assert_approx_eq(
        release_velocity_from_samples(&[(300, 0), (200, 30), (100, 60)], 60).unwrap(),
        -MAX_INTERACTIVE_RADIANS_PER_SECOND,
    );
    // Six cells in 60 ms is 100 cells per second: below the cap unadjusted, but
    // above it once amplified, so the gain must be applied before clamping.
    assert_approx_eq(
        release_velocity_from_samples(&[(100, 0), (106, 60)], 60).unwrap(),
        MAX_INTERACTIVE_RADIANS_PER_SECOND,
    );
    let unamplified_cells_per_second = std::hint::black_box(100.0);
    assert!(unamplified_cells_per_second * TAU / 96.0 < MAX_INTERACTIVE_RADIANS_PER_SECOND);
}

#[test]
fn recent_release_velocity_handles_sparse_edge_crossing_flick() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(70, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(46, 1),
        Duration::from_millis(20),
        idle_velocity,
        start + Duration::from_millis(20),
    ));
    let release_at = start + Duration::from_millis(25);
    assert!(interaction.release_at(
        TuiPoint::new(46, 1),
        Duration::from_millis(25),
        idle_velocity,
        release_at,
    ));

    assert_approx_eq(
        interaction
            .resolve_at(Duration::from_millis(25), idle_velocity, release_at)
            .velocity,
        -MAX_INTERACTIVE_RADIANS_PER_SECOND,
    );
}
#[test]
fn recent_release_velocity_pause_before_release_resumes_idle_phase_continuously() {
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let interaction = ZeroStateInteractionHandle::default();
    assert!(interaction.press_at(TuiPoint::new(100, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(104, 1),
        Duration::from_millis(50),
        idle_velocity,
        start + Duration::from_millis(50),
    ));
    assert!(interaction.drag_at(
        TuiPoint::new(108, 1),
        Duration::from_millis(100),
        idle_velocity,
        start + Duration::from_millis(100),
    ));
    let release_at = start + Duration::from_millis(251);
    let before_release =
        interaction.resolve_at(Duration::from_millis(251), idle_velocity, release_at);
    assert!(interaction.release_at(
        TuiPoint::new(108, 1),
        Duration::from_millis(251),
        idle_velocity,
        release_at,
    ));
    let released = interaction.resolve_at(Duration::from_millis(251), idle_velocity, release_at);
    assert_approx_eq(released.angle, before_release.angle);
    assert_approx_eq(released.velocity, idle_velocity);
}

#[test]
fn released_velocity_smoothly_settles_to_idle_in_three_seconds() {
    for period in [Duration::from_secs(1), Duration::from_secs(60)] {
        let interaction = ZeroStateInteractionHandle::default();
        let start = Instant::now();
        let idle_velocity = TAU / period.as_secs_f64();
        assert!(interaction.press_at(TuiPoint::new(1, 1), start));
        assert!(interaction.drag_at(
            TuiPoint::new(20, 1),
            Duration::from_millis(20),
            idle_velocity,
            start + Duration::from_millis(20),
        ));
        assert!(interaction.drag_at(
            TuiPoint::new(40, 1),
            Duration::from_millis(60),
            idle_velocity,
            start + Duration::from_millis(60),
        ));
        let release_at = start + Duration::from_millis(80);
        let angle_before_release =
            interaction.resolve_at(Duration::from_millis(80), idle_velocity, release_at);
        assert!(interaction.release_at(
            TuiPoint::new(40, 1),
            Duration::from_millis(80),
            idle_velocity,
            release_at,
        ));
        let released = interaction.resolve_at(Duration::from_millis(80), idle_velocity, release_at);
        assert_approx_eq(released.angle, angle_before_release.angle);

        let halfway = interaction.resolve_at(
            Duration::from_millis(1580),
            idle_velocity,
            release_at + Duration::from_millis(1500),
        );
        assert_approx_eq(halfway.velocity, (released.velocity + idle_velocity) * 0.5);
        let settled = interaction.resolve_at(
            Duration::from_millis(3080),
            idle_velocity,
            release_at + MOMENTUM_SETTLE_DURATION,
        );
        assert_approx_eq(settled.velocity, idle_velocity);
        let after = interaction.resolve_at(
            Duration::from_millis(4080),
            idle_velocity,
            release_at + MOMENTUM_SETTLE_DURATION + Duration::from_secs(1),
        );
        assert_approx_eq(after.velocity, idle_velocity);
        assert_approx_eq(after.angle - settled.angle, idle_velocity);
    }
}

/// Drives a three-sample horizontal flick and returns its release instant. The
/// gesture always spans 100 ms so callers vary only direction and distance.
fn flick(
    interaction: &ZeroStateInteractionHandle,
    press_at: Instant,
    press_elapsed: Duration,
    columns: [u16; 3],
    idle_velocity: f64,
) -> Instant {
    assert!(interaction.press_at(TuiPoint::new(columns[0], 1), press_at));
    for (column, offset) in columns[1..].iter().zip([40, 80]) {
        assert!(interaction.drag_at(
            TuiPoint::new(*column, 1),
            press_elapsed + Duration::from_millis(offset),
            idle_velocity,
            press_at + Duration::from_millis(offset),
        ));
    }
    let release_at = press_at + Duration::from_millis(100);
    assert!(interaction.release_at(
        TuiPoint::new(columns[2], 1),
        press_elapsed + Duration::from_millis(100),
        idle_velocity,
        release_at,
    ));
    release_at
}

#[test]
fn reverse_flick_settles_to_reverse_idle_and_keeps_spinning_backward() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let release_at = flick(
        &interaction,
        start,
        Duration::ZERO,
        [100, 76, 52],
        idle_velocity,
    );
    assert!(
        interaction
            .resolve_at(Duration::from_millis(100), idle_velocity, release_at)
            .velocity
            < 0.0
    );

    let settled_at = release_at + MOMENTUM_SETTLE_DURATION;
    let settled = interaction.resolve_at(Duration::from_millis(3100), idle_velocity, settled_at);
    assert_approx_eq(settled.velocity, -idle_velocity);

    let later = interaction.resolve_at(
        Duration::from_millis(4100),
        idle_velocity,
        settled_at + Duration::from_secs(1),
    );
    assert_approx_eq(later.velocity, -idle_velocity);
    assert_approx_eq(later.angle - settled.angle, -idle_velocity);
}

#[test]
fn reverse_idle_follows_configured_period_changes_without_turning_around() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let updated_idle_velocity = TAU / 10.0;
    let release_at = flick(
        &interaction,
        start,
        Duration::ZERO,
        [100, 76, 52],
        idle_velocity,
    );
    let settled_at = release_at + MOMENTUM_SETTLE_DURATION;
    let settled = interaction.resolve_at(Duration::from_millis(3100), idle_velocity, settled_at);

    let reconfigured = interaction.resolve_at(
        Duration::from_millis(3100),
        updated_idle_velocity,
        settled_at,
    );
    assert_approx_eq(reconfigured.angle, settled.angle);
    assert_approx_eq(reconfigured.velocity, -updated_idle_velocity);

    let after_reconfiguration = interaction.resolve_at(
        Duration::from_millis(4100),
        updated_idle_velocity,
        settled_at + Duration::from_secs(1),
    );
    assert_approx_eq(
        after_reconfiguration.angle - reconfigured.angle,
        -updated_idle_velocity,
    );
}

#[test]
fn a_later_forward_flick_restores_forward_idle_direction() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let reverse_release_at = flick(
        &interaction,
        start,
        Duration::ZERO,
        [100, 76, 52],
        idle_velocity,
    );
    let reverse_settled_at = reverse_release_at + MOMENTUM_SETTLE_DURATION;
    assert!(
        interaction
            .resolve_at(
                Duration::from_millis(3100),
                idle_velocity,
                reverse_settled_at
            )
            .velocity
            < 0.0
    );

    let forward_release_at = flick(
        &interaction,
        reverse_settled_at,
        Duration::from_millis(3100),
        [52, 76, 100],
        idle_velocity,
    );
    assert!(
        interaction
            .resolve_at(
                Duration::from_millis(3200),
                idle_velocity,
                forward_release_at
            )
            .velocity
            > 0.0
    );
    let settled = interaction.resolve_at(
        Duration::from_millis(6200),
        idle_velocity,
        forward_release_at + MOMENTUM_SETTLE_DURATION,
    );
    assert_approx_eq(settled.velocity, idle_velocity);
}

#[test]
fn filtered_releases_keep_the_established_reverse_direction() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let release_at = flick(
        &interaction,
        start,
        Duration::ZERO,
        [100, 76, 52],
        idle_velocity,
    );
    let settled_at = release_at + MOMENTUM_SETTLE_DURATION;

    assert!(interaction.press_at(TuiPoint::new(52, 1), settled_at));
    assert!(interaction.drag_at(
        TuiPoint::new(56, 1),
        Duration::from_millis(3150),
        idle_velocity,
        settled_at + Duration::from_millis(50),
    ));
    let stale_release_at = settled_at + Duration::from_millis(300);
    let before_release =
        interaction.resolve_at(Duration::from_millis(3400), idle_velocity, stale_release_at);
    assert!(interaction.release_at(
        TuiPoint::new(56, 1),
        Duration::from_millis(3400),
        idle_velocity,
        stale_release_at,
    ));

    let released =
        interaction.resolve_at(Duration::from_millis(3400), idle_velocity, stale_release_at);
    assert_approx_eq(released.angle, before_release.angle);
    assert_approx_eq(released.velocity, -idle_velocity);
}

#[test]
fn leaving_the_zero_state_restores_forward_idle_direction() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let idle_velocity = TAU / 5.0;
    let release_at = flick(
        &interaction,
        start,
        Duration::ZERO,
        [100, 76, 52],
        idle_velocity,
    );
    assert!(
        interaction
            .resolve_at(
                Duration::from_millis(3100),
                idle_velocity,
                release_at + MOMENTUM_SETTLE_DURATION
            )
            .velocity
            < 0.0
    );

    interaction.set_visible(false);
    interaction.set_visible(true);
    let returned = interaction.resolve_at(
        Duration::from_secs(4),
        idle_velocity,
        release_at + Duration::from_secs(4),
    );
    assert_approx_eq(returned.velocity, idle_velocity);
    assert_approx_eq(
        returned.angle,
        idle_angle(Duration::from_secs(4), idle_velocity),
    );
}

#[test]
fn setting_changes_take_effect_phase_continuously_after_the_fixed_settle() {
    let interaction = ZeroStateInteractionHandle::default();
    let start = Instant::now();
    let release_idle_velocity = TAU / 5.0;
    let updated_idle_velocity = TAU / 10.0;
    let later_idle_velocity = TAU / 20.0;
    assert!(interaction.press_at(TuiPoint::new(40, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(64, 1),
        Duration::from_millis(40),
        release_idle_velocity,
        start + Duration::from_millis(40),
    ));
    assert!(interaction.drag_at(
        TuiPoint::new(88, 1),
        Duration::from_millis(80),
        release_idle_velocity,
        start + Duration::from_millis(80),
    ));
    let release_at = start + Duration::from_millis(100);
    let release_angle = interaction
        .resolve_at(
            Duration::from_millis(100),
            release_idle_velocity,
            release_at,
        )
        .angle;
    assert!(interaction.release_at(
        TuiPoint::new(88, 1),
        Duration::from_millis(100),
        release_idle_velocity,
        release_at,
    ));
    let released = interaction.resolve_at(
        Duration::from_millis(100),
        release_idle_velocity,
        release_at,
    );

    let halfway = interaction.resolve_at(
        Duration::from_millis(1600),
        updated_idle_velocity,
        release_at + Duration::from_millis(1500),
    );
    assert_approx_eq(
        halfway.velocity,
        (released.velocity + release_idle_velocity) * 0.5,
    );

    let settled_at = release_at + MOMENTUM_SETTLE_DURATION;
    let settled = interaction.resolve_at(
        Duration::from_millis(3100),
        updated_idle_velocity,
        settled_at,
    );
    assert_approx_eq(settled.velocity, updated_idle_velocity);
    assert_approx_eq(
        settled.angle,
        release_angle
            + (released.velocity + release_idle_velocity)
                * MOMENTUM_SETTLE_DURATION.as_secs_f64()
                * 0.5,
    );

    let one_second_later = interaction.resolve_at(
        Duration::from_millis(4100),
        updated_idle_velocity,
        settled_at + Duration::from_secs(1),
    );
    assert_approx_eq(
        one_second_later.angle - settled.angle,
        updated_idle_velocity,
    );
    let reconfigured = interaction.resolve_at(
        Duration::from_millis(4100),
        later_idle_velocity,
        settled_at + Duration::from_secs(1),
    );
    assert_approx_eq(reconfigured.angle, one_second_later.angle);
    assert_approx_eq(reconfigured.velocity, later_idle_velocity);
    let after_reconfiguration = interaction.resolve_at(
        Duration::from_millis(5100),
        later_idle_velocity,
        settled_at + Duration::from_secs(2),
    );
    assert_approx_eq(
        after_reconfiguration.angle - reconfigured.angle,
        later_idle_velocity,
    );
}

fn interaction_after_flick(start: Instant) -> (ZeroStateInteractionHandle, Instant, f64) {
    let interaction = ZeroStateInteractionHandle::default();
    let idle_velocity = TAU / 5.0;
    assert!(interaction.press_at(TuiPoint::new(1, 1), start));
    assert!(interaction.drag_at(
        TuiPoint::new(5, 1),
        Duration::from_millis(40),
        idle_velocity,
        start + Duration::from_millis(40),
    ));
    assert!(interaction.drag_at(
        TuiPoint::new(9, 1),
        Duration::from_millis(80),
        idle_velocity,
        start + Duration::from_millis(80),
    ));
    let release_at = start + Duration::from_millis(100);
    assert!(interaction.release_at(
        TuiPoint::new(9, 1),
        Duration::from_millis(100),
        idle_velocity,
        release_at,
    ));
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
    for (name, art) in [
        ("diamond", DIAMOND_ART),
        ("rocket", ROCKET_ART),
        ("Warp W", WARP_W_ART),
    ] {
        let config = custom_config(art, 4.0, 0.18);
        let face = object_frame_at(Duration::ZERO, PANEL_SIZE, &config).unwrap();
        let edge = object_frame_at(Duration::from_secs(1), PANEL_SIZE, &config).unwrap();
        let back = object_frame_at(Duration::from_secs(2), PANEL_SIZE, &config).unwrap();

        assert!(logo_cells(&face).len() > 20);
        assert!(
            edge.iter_cells()
                .any(|(_, _, cell)| cell.surface == LogoSurface::Side),
            "{name} should retain visible side stitches at a quarter turn"
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
