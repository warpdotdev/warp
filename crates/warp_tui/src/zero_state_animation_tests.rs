use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use warp::settings::{
    TuiZeroStateExtrusionDepthSetting, TuiZeroStateObject, TuiZeroStateObjectSetting,
    TuiZeroStateRotationPeriodSeconds, TuiZeroStateRotationPeriodSecondsSetting,
    TuiZeroStateSettings,
};
use warp_core::settings::Setting as _;
use warpui::SingletonEntity as _;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{TuiBufferExt, TuiElement, TuiRect, TuiSize, TuiStyle};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{AddWindowOptions, App, AppContext, Entity, TuiView, TypedActionView};

use super::config::{
    AsciiArtError, AsciiArtMask, ReloadObjectOutcome, ZeroStateAnimationConfig,
    ZeroStateAnimationLoadFailure, ZeroStateShape, resolve_ascii_art_path,
};
use super::{
    BUILT_IN_LOGO_CELL_ASPECT_RATIO, LogoCell, LogoSurface, WarpLogoStyles,
    ZeroStateAnimationElement, fitted_logo_size, logo_frame_at, object_frame_at,
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
