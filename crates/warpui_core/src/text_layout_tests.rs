use float_cmp::assert_approx_eq;

use super::*;
use crate::{fonts::Weight, rendering, App, Scene};

#[test]
fn test_empty_line() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let line_style = LineStyle {
                font_size: 12.,
                line_height_ratio: 1.,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                fixed_width_tab_size: None,
            };
            let styles = [];

            let layout_cache = LayoutCache::new();
            let line = layout_cache.layout_line(
                "",
                line_style,
                &styles,
                f32::MAX,
                ClipConfig::end(),
                &ctx.font_cache().text_layout_system(),
            );

            // There should be no contents.
            assert_eq!(line.runs.len(), 0);

            // It should have the described line style.
            assert_eq!(line.font_size, line_style.font_size);
            assert_eq!(line.line_height_ratio, line_style.line_height_ratio);

            // It should have zero width, but have a height the same as the line height.
            assert_eq!(
                line.height(),
                line_style.font_size * line_style.line_height_ratio
            );
            assert_eq!(line.width, 0.);
        });
    });
}

#[test]
fn test_empty_text_frame() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let line_style = LineStyle {
                font_size: 12.,
                line_height_ratio: 1.,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                fixed_width_tab_size: None,
            };
            let styles = [];

            let layout_cache = LayoutCache::new();
            let frame = layout_cache.layout_text(
                "",
                line_style,
                &styles,
                f32::MAX,
                f32::MAX,
                Default::default(),
                None,
                &ctx.font_cache().text_layout_system(),
            );

            // There should be one line with no contents.
            assert_eq!(frame.lines.len(), 1);
            let line = &frame.lines()[0];
            assert_eq!(line.runs.len(), 0);

            // It should have the described line style.
            assert_eq!(line.font_size, line_style.font_size);
            assert_eq!(line.line_height_ratio, line_style.line_height_ratio);

            // It should have zero width, but have a height the same as the line height.
            assert_eq!(
                line.height(),
                line_style.font_size * line_style.line_height_ratio
            );
            assert_eq!(line.width, 0.);
        })
    });
}

#[test]
fn test_cache_key_includes_fixed_width_tab_size() {
    let text = "abc";
    let style_runs: &[(Range<usize>, StyleAndFont)] = &[];

    let key_4 = CacheKeyRef {
        text,
        font_size: OrderedFloat(12.),
        line_height_ratio: OrderedFloat(1.),
        fixed_width_tab_size: Some(4),
        style_runs,
        max_width: OrderedFloat(100.),
        max_height: None,
        alignment: TextAlignment::Left,
        first_line_head_indent: None,
        clip_config: None,
    };
    let key_8 = CacheKeyRef {
        fixed_width_tab_size: Some(8),
        ..key_4
    };

    assert!(key_4 != key_8);
}

#[test]
fn test_calculate_line_baseline_position() {
    let baseline_position = default_compute_baseline_position(
        16.,  /* font_size */
        1.2,  /* line_height_ratio */
        12.8, /* ascent */
        3.2,  /* descent */
    );
    // In the default case, we center the text within the line (top padding = font_size * line_height_ratio / 2).
    // Then, we move the baseline down by the ascent.
    assert_approx_eq!(f32, baseline_position, 14.4);
}

#[test]
fn test_strip_leading_unicode_bom() {
    let text = "\u{FEFF}Hello world";
    // Here is how the text is originally styled:
    // "\u{FEFF}": Black
    // "Hello ": Bold, White
    // "world": Black
    let mut style_runs = vec![
        // We include empty ranges because when laying out style runs we often have
        // multiple empty ranges.
        (
            0..0,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            0..1,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default(),
                TextStyle::default().with_foreground_color(ColorU::black()),
            ),
        ),
        (
            1..1,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            1..7,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default().weight(Weight::Bold),
                TextStyle::default().with_foreground_color(ColorU::white()),
            ),
        ),
        (
            7..7,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            7..13,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default(),
                TextStyle::default().with_foreground_color(ColorU::black()),
            ),
        ),
    ];
    let (stripped_text, adjusted_style_runs) =
        strip_leading_unicode_bom(text, style_runs.as_mut_slice());
    assert_eq!(stripped_text, "Hello world");

    // Here is how the text should be styled after stripping the leading BOM character:
    // "Hello ": Bold, White
    // "world": Black
    let expected_style_runs = vec![
        (
            0..0,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            0..0,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default(),
                TextStyle::default().with_foreground_color(ColorU::black()),
            ),
        ),
        (
            0..0,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            0..6,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default().weight(Weight::Bold),
                TextStyle::default().with_foreground_color(ColorU::white()),
            ),
        ),
        (
            6..6,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            6..12,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default(),
                TextStyle::default().with_foreground_color(ColorU::black()),
            ),
        ),
    ];
    assert_eq!(adjusted_style_runs, Some(expected_style_runs));
}

#[test]
fn test_strip_leading_unicode_bom_with_initial_range() {
    let text = "\u{FEFF}A";
    let mut style_runs = vec![
        // We include these empty ranges because when laying out style runs we often have
        // multiple empty ranges.
        (
            0..0,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            0..2,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default(),
                TextStyle::default().with_foreground_color(ColorU::black()),
            ),
        ),
    ];
    let (stripped_text, adjusted_style_runs) =
        strip_leading_unicode_bom(text, style_runs.as_mut_slice());
    assert_eq!(stripped_text, "A");

    let expected_style_runs = vec![
        (
            0..0,
            StyleAndFont::new(FamilyId(0), Properties::default(), TextStyle::default()),
        ),
        (
            0..1,
            StyleAndFont::new(
                FamilyId(0),
                Properties::default(),
                TextStyle::default().with_foreground_color(ColorU::black()),
            ),
        ),
    ];
    assert_eq!(adjusted_style_runs, Some(expected_style_runs));
}

#[test]
fn test_strip_leading_unicode_bom_with_single_style_run() {
    let text = "\u{FEFF}Hello world";
    let mut style_runs = vec![(
        0..13,
        StyleAndFont::new(
            FamilyId(0),
            Properties::default(),
            TextStyle::default().with_foreground_color(ColorU::black()),
        ),
    )];
    let (stripped_text, adjusted_style_runs) =
        strip_leading_unicode_bom(text, style_runs.as_mut_slice());
    assert_eq!(stripped_text, "Hello world");

    let expected_style_runs = vec![(
        0..12,
        StyleAndFont::new(
            FamilyId(0),
            Properties::default(),
            TextStyle::default().with_foreground_color(ColorU::black()),
        ),
    )];
    assert_eq!(adjusted_style_runs, Some(expected_style_runs));
}

/// Build a synthetic `Line` for paint tests. The platform test `FontDB` stubs
/// out real text layout so we cannot exercise the paint path through
/// `layout_line`; instead we hand-roll a single run of fixed-width glyphs.
fn synthetic_line(glyph_count: usize, glyph_width: f32, clip_config: ClipConfig) -> Line {
    let glyphs = (0..glyph_count)
        .map(|i| Glyph {
            id: 0,
            position_along_baseline: vec2f(glyph_width * i as f32, 0.),
            index: i,
            width: glyph_width,
        })
        .collect();
    let run = Run {
        font_id: FontId(0),
        glyphs,
        styles: TextStyle::default(),
        width: glyph_width * glyph_count as f32,
    };
    Line {
        width: run.width,
        trailing_whitespace_width: 0.,
        runs: vec![run],
        font_size: 12.,
        line_height_ratio: 1.,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        clip_config: Some(clip_config),
        ascent: 10.,
        descent: 2.,
        caret_positions: Vec::new(),
        chars_with_missing_glyphs: Vec::new(),
    }
}

/// When start-clipping with an ellipsis, the leftmost painted glyph must not
/// overlap the ellipsis glyph. Before the offset fix in `paint_internal`, the
/// ellipsis-reservation shifted visible glyphs leftward so the leftmost glyph
/// shared an x position with the ellipsis.
#[test]
fn test_paint_start_ellipsis_does_not_overlap_leftmost_glyph() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // 10 glyphs at 12px each = 120px line, painted into a 50px bounds —
            // this forces the loop into the ellipsis branch.
            let line = synthetic_line(
                10,
                12.,
                ClipConfig {
                    direction: ClipDirection::Start,
                    style: ClipStyle::Ellipsis,
                },
            );

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint(
                RectF::new(Vector2F::zero(), Vector2F::new(50., 20.)),
                &PaintStyleOverride::default(),
                ColorU::black(),
                ctx.font_cache(),
                &mut scene,
            );

            // The platform test FontDB returns `glyph_advance == 0` for the
            // ellipsis lookup, so `ellipsis_width` ends up zero and the
            // ellipsis-glyph drawing is skipped. We can still verify that the
            // visible glyphs are painted at distinct x positions (regression
            // protection for the offset arithmetic). The deeper guarantee
            // — ellipsis vs leftmost-glyph non-overlap — is covered by
            // platform-level integration tests where real fonts are loaded.
            let mut x_positions: Vec<f32> = scene
                .layers()
                .flat_map(|layer| layer.glyphs.iter())
                .map(|glyph| glyph.position.x())
                .collect();
            x_positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for window in x_positions.windows(2) {
                assert_ne!(
                    window[0], window[1],
                    "two glyphs painted at the same x={}",
                    window[0],
                );
            }
        });
    });
}

/// When start-clipping without an ellipsis (fade style), the offset fix must
/// not change the existing layout — visible glyphs should remain right-aligned
/// in the paint bounds with no extra horizontal shift.
#[test]
fn test_paint_start_fade_unchanged_by_ellipsis_offset() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let line = synthetic_line(10, 12., ClipConfig::start());

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint(
                RectF::new(Vector2F::zero(), Vector2F::new(50., 20.)),
                &PaintStyleOverride::default(),
                ColorU::black(),
                ctx.font_cache(),
                &mut scene,
            );

            let max_x = scene
                .layers()
                .flat_map(|layer| layer.glyphs.iter())
                .map(|glyph| glyph.position.x())
                .fold(f32::NEG_INFINITY, f32::max);

            // The rightmost glyph occupies [available_width - glyph_width,
            // available_width]; its origin must be at exactly that boundary.
            assert_approx_eq!(f32, max_x, 50. - 12.);
        });
    });
}
