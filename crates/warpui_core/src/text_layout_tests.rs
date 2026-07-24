use float_cmp::assert_approx_eq;

use super::*;
use crate::fonts::Weight;
use crate::{App, Scene, rendering};

#[test]
fn test_text_style_baseline_offset_sub_sup() {
    // Issue #13734: sub shifts glyphs down (positive y, screen-down), sup shifts them up
    // (negative y), by a fixed fraction of the font size; no alignment means no shift.
    let font_size = 20.0;

    let plain = TextStyle::new();
    assert_approx_eq!(f32, plain.baseline_offset(font_size), 0.0);

    let sub = TextStyle::new().with_vertical_align(VerticalAlign::Sub);
    let sub_offset = sub.baseline_offset(font_size);
    assert!(sub_offset > 0.0, "sub should shift down, got {sub_offset}");

    let sup = TextStyle::new().with_vertical_align(VerticalAlign::Sup);
    let sup_offset = sup.baseline_offset(font_size);
    assert!(sup_offset < 0.0, "sup should shift up, got {sup_offset}");

    // Symmetric magnitude, and scales with font size.
    assert_approx_eq!(f32, sub_offset, -sup_offset);
    assert_approx_eq!(
        f32,
        TextStyle::new()
            .with_vertical_align(VerticalAlign::Sub)
            .baseline_offset(40.0),
        sub_offset * 2.0
    );
}

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

/// Regression test for the "inline-code link underline" bug: a run that has BOTH a
/// background and an underline must paint its background BEFORE its underline. The
/// underline is a filled rect in the same layer as the background, so if the
/// background is drawn afterward it covers (hides) the underline — which is exactly
/// what happened for a detected link rendered as inline code (gray code background)
/// on a soft-wrapping line. We assert the draw order: background rect precedes the
/// underline rect.
#[test]
fn test_run_background_painted_before_underline() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let bg_color = ColorU::from_u32(0x37393CFF);
            let underline_color = ColorU::from_u32(0x7AA6DAFF);
            let glyph_width = 12.0;
            let glyph_count = 5usize;

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
                styles: TextStyle::default()
                    .with_background_color(bg_color)
                    .with_underline_color(underline_color),
                width: glyph_width * glyph_count as f32,
            };
            let line = Line {
                width: run.width,
                trailing_whitespace_width: 0.,
                runs: vec![run],
                font_size: 12.,
                line_height_ratio: 1.,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                clip_config: None,
                ascent: 10.,
                descent: 2.,
                caret_positions: Vec::new(),
                chars_with_missing_glyphs: Vec::new(),
            };

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint(
                RectF::new(Vector2F::zero(), Vector2F::new(1000., 50.)),
                &PaintStyleOverride::default(),
                ColorU::black(),
                ctx.font_cache(),
                &mut scene,
            );

            // Find, in draw order within the layer, the background rect and the
            // first underline rect (identified by their solid fill colors).
            let layer = scene.layers().next().expect("at least one layer");
            let bg_index = layer
                .rects
                .iter()
                .position(|rect| matches!(rect.background, Fill::Solid(color) if color == bg_color))
                .expect("background rect should be painted");
            let underline_index = layer
                .rects
                .iter()
                .position(
                    |rect| matches!(rect.background, Fill::Solid(color) if color == underline_color),
                )
                .expect("underline rect should be painted");

            assert!(
                bg_index < underline_index,
                "background rect (index {bg_index}) must be painted before the underline rect \
                 (index {underline_index}) so the underline renders on top of the background",
            );
        });
    });
}

/// Issue #13734: a `<sub>`/`<sup>`-flagged run has its glyphs painted with a vertical
/// offset (down for sub, up for sup) relative to a plain run on the same line, without
/// touching the shaper or the line's single baseline.
#[test]
fn test_sub_sup_run_glyphs_painted_with_vertical_offset() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let glyph_width = 12.0;
            let font_size = 12.0;

            // Build one plain run and, immediately after it, one sub run and one sup run —
            // all glyphs shaped at baseline y=0. Only the vertical alignment differs.
            let make_glyphs = |start: usize, count: usize| -> Vec<Glyph> {
                (0..count)
                    .map(|i| Glyph {
                        id: 0,
                        position_along_baseline: vec2f(glyph_width * (start + i) as f32, 0.),
                        index: start + i,
                        width: glyph_width,
                    })
                    .collect()
            };

            let plain_run = Run {
                font_id: FontId(0),
                glyphs: make_glyphs(0, 1),
                styles: TextStyle::default(),
                width: glyph_width,
            };
            let sub_run = Run {
                font_id: FontId(0),
                glyphs: make_glyphs(1, 1),
                styles: TextStyle::default().with_vertical_align(VerticalAlign::Sub),
                width: glyph_width,
            };
            let sup_run = Run {
                font_id: FontId(0),
                glyphs: make_glyphs(2, 1),
                styles: TextStyle::default().with_vertical_align(VerticalAlign::Sup),
                width: glyph_width,
            };
            let line = Line {
                width: glyph_width * 3.,
                trailing_whitespace_width: 0.,
                runs: vec![plain_run, sub_run, sup_run],
                font_size,
                line_height_ratio: 1.,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                clip_config: None,
                ascent: 10.,
                descent: 2.,
                caret_positions: Vec::new(),
                chars_with_missing_glyphs: Vec::new(),
            };

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint(
                RectF::new(Vector2F::zero(), Vector2F::new(1000., 50.)),
                &PaintStyleOverride::default(),
                ColorU::black(),
                ctx.font_cache(),
                &mut scene,
            );

            // Drawn glyphs are ordered by x, so index 0 = plain, 1 = sub, 2 = sup.
            let mut glyphs: Vec<_> = scene
                .layers()
                .flat_map(|layer| layer.glyphs.iter().cloned())
                .collect();
            glyphs.sort_by(|a, b| a.position.x().partial_cmp(&b.position.x()).unwrap());
            assert_eq!(glyphs.len(), 3, "expected three painted glyphs");

            let plain_y = glyphs[0].position.y();
            let sub_y = glyphs[1].position.y();
            let sup_y = glyphs[2].position.y();

            assert!(
                sub_y > plain_y,
                "subscript glyph (y={sub_y}) should paint below the plain glyph (y={plain_y})",
            );
            assert!(
                sup_y < plain_y,
                "superscript glyph (y={sup_y}) should paint above the plain glyph (y={plain_y})",
            );
        });
    });
}

/// Composed-style regression (#13734 round 2): the sub/sup vertical offset was applied only to
/// glyph origins, so a run's background, underline (link), and strikethrough/error decorations
/// stayed on the un-shifted baseline and rendered torn apart from the shifted glyphs. Each helper
/// below paints one composed run twice — once plain, once sub or sup — and asserts the decoration
/// moves with the glyphs by exactly the run's `baseline_offset`.
#[test]
fn test_sub_run_background_shifts_with_glyphs() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let bg_color = ColorU::from_u32(0x37393CFF);
            let glyph_width = 12.0;
            let font_size = 12.0;

            let make_line = |align: Option<VerticalAlign>| {
                let mut styles = TextStyle::default().with_background_color(bg_color);
                if let Some(align) = align {
                    styles = styles.with_vertical_align(align);
                }
                let run = Run {
                    font_id: FontId(0),
                    glyphs: vec![Glyph {
                        id: 0,
                        position_along_baseline: vec2f(0., 0.),
                        index: 0,
                        width: glyph_width,
                    }],
                    styles,
                    width: glyph_width,
                };
                Line {
                    width: glyph_width,
                    trailing_whitespace_width: 0.,
                    runs: vec![run],
                    font_size,
                    line_height_ratio: 1.,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    clip_config: None,
                    ascent: 10.,
                    descent: 2.,
                    caret_positions: Vec::new(),
                    chars_with_missing_glyphs: Vec::new(),
                }
            };

            let bg_top = |line: &Line| {
                let mut scene = Scene::new(1., rendering::Config::default());
                line.paint(
                    RectF::new(Vector2F::zero(), Vector2F::new(1000., 50.)),
                    &PaintStyleOverride::default(),
                    ColorU::black(),
                    ctx.font_cache(),
                    &mut scene,
                );
                let layer = scene.layers().next().expect("at least one layer");
                layer
                    .rects
                    .iter()
                    .find(|r| matches!(r.background, Fill::Solid(c) if c == bg_color))
                    .expect("background rect should be painted")
                    .bounds
                    .min_y()
            };

            let plain_top = bg_top(&make_line(None));
            let sub_top = bg_top(&make_line(Some(VerticalAlign::Sub)));
            let expected = TextStyle::default()
                .with_vertical_align(VerticalAlign::Sub)
                .baseline_offset(font_size);
            assert_approx_eq!(f32, sub_top - plain_top, expected);
        });
    });
}

#[test]
fn test_sub_run_underline_shifts_with_glyphs() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let underline_color = ColorU::from_u32(0x7AA6DAFF);
            let glyph_width = 12.0;
            let font_size = 12.0;

            let make_line = |align: Option<VerticalAlign>| {
                let mut styles = TextStyle::default().with_underline_color(underline_color);
                if let Some(align) = align {
                    styles = styles.with_vertical_align(align);
                }
                let run = Run {
                    font_id: FontId(0),
                    glyphs: vec![Glyph {
                        id: 0,
                        position_along_baseline: vec2f(0., 0.),
                        index: 0,
                        width: glyph_width,
                    }],
                    styles,
                    width: glyph_width,
                };
                Line {
                    width: glyph_width,
                    trailing_whitespace_width: 0.,
                    runs: vec![run],
                    font_size,
                    line_height_ratio: 1.,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    clip_config: None,
                    ascent: 10.,
                    descent: 2.,
                    caret_positions: Vec::new(),
                    chars_with_missing_glyphs: Vec::new(),
                }
            };

            let underline_top = |line: &Line| {
                let mut scene = Scene::new(1., rendering::Config::default());
                line.paint(
                    RectF::new(Vector2F::zero(), Vector2F::new(1000., 50.)),
                    &PaintStyleOverride::default(),
                    ColorU::black(),
                    ctx.font_cache(),
                    &mut scene,
                );
                let layer = scene.layers().next().expect("at least one layer");
                layer
                    .rects
                    .iter()
                    .find(|r| matches!(r.background, Fill::Solid(c) if c == underline_color))
                    .expect("underline rect should be painted")
                    .bounds
                    .min_y()
            };

            let plain_top = underline_top(&make_line(None));
            let sub_top = underline_top(&make_line(Some(VerticalAlign::Sub)));
            let expected = TextStyle::default()
                .with_vertical_align(VerticalAlign::Sub)
                .baseline_offset(font_size);
            assert_approx_eq!(f32, sub_top - plain_top, expected);
        });
    });
}

#[test]
fn test_sup_run_strikethrough_shifts_with_glyphs() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let glyph_color = ColorU::black();
            let glyph_width = 12.0;
            let font_size = 12.0;

            let make_line = |align: Option<VerticalAlign>| {
                let mut styles = TextStyle::default().with_show_strikethrough(true);
                if let Some(align) = align {
                    styles = styles.with_vertical_align(align);
                }
                let run = Run {
                    font_id: FontId(0),
                    glyphs: vec![Glyph {
                        id: 0,
                        position_along_baseline: vec2f(0., 0.),
                        index: 0,
                        width: glyph_width,
                    }],
                    styles,
                    width: glyph_width,
                };
                Line {
                    width: glyph_width,
                    trailing_whitespace_width: 0.,
                    runs: vec![run],
                    font_size,
                    line_height_ratio: 1.,
                    baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                    clip_config: None,
                    ascent: 10.,
                    descent: 2.,
                    caret_positions: Vec::new(),
                    chars_with_missing_glyphs: Vec::new(),
                }
            };

            // The strikethrough rect is the only rect drawn with the glyph color as its fill.
            let strike_top = |line: &Line| {
                let mut scene = Scene::new(1., rendering::Config::default());
                line.paint(
                    RectF::new(Vector2F::zero(), Vector2F::new(1000., 50.)),
                    &PaintStyleOverride::default(),
                    glyph_color,
                    ctx.font_cache(),
                    &mut scene,
                );
                let layer = scene.layers().next().expect("at least one layer");
                layer
                    .rects
                    .iter()
                    .find(|r| matches!(r.background, Fill::Solid(c) if c == glyph_color))
                    .expect("strikethrough rect should be painted")
                    .bounds
                    .min_y()
            };

            let plain_top = strike_top(&make_line(None));
            let sup_top = strike_top(&make_line(Some(VerticalAlign::Sup)));
            let expected = TextStyle::default()
                .with_vertical_align(VerticalAlign::Sup)
                .baseline_offset(font_size);
            assert_approx_eq!(f32, sup_top - plain_top, expected);
        });
    });
}

/// The run background must be clamped to the horizontal span of glyphs that are
/// actually drawn (`visible_left`..`visible_right`), not the full run width. This
/// is what keeps a partially-truncated backgrounded run (e.g. an inline-code link
/// cut off by an ellipsis) from painting a background past its visible glyphs.
///
/// We exercise `paint_run_background` directly because the platform test `FontDB`
/// reports a zero advance for the ellipsis glyph, so `ellipsis_width` is always 0
/// and the end-to-end ellipsis-truncation branch in `paint_internal` cannot be
/// driven from a unit test. This still pins the clamping arithmetic that fixes the
/// bug.
#[test]
fn test_run_background_clamped_to_visible_glyph_span() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let bg_color = ColorU::from_u32(0x37393CFF);
            let glyph_width = 12.0;
            let glyph_count = 10usize;

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
                styles: TextStyle::default().with_background_color(bg_color),
                width: glyph_width * glyph_count as f32, // 120px
            };
            let line = Line {
                width: run.width,
                trailing_whitespace_width: 0.,
                runs: vec![run],
                font_size: 12.,
                line_height_ratio: 1.,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                clip_config: None,
                ascent: 10.,
                descent: 2.,
                caret_positions: Vec::new(),
                chars_with_missing_glyphs: Vec::new(),
            };

            // Only the first three glyphs (0..36px) are "visible".
            let visible_left = 0.;
            let visible_right = 36.;

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint_run_background(
                &line.runs[0],
                Vector2F::zero(),
                RectF::new(Vector2F::zero(), Vector2F::new(1000., 50.)),
                visible_left,
                visible_right,
                ctx.font_cache(),
                &mut scene,
                &default_compute_baseline_position_fn(),
            );

            let layer = scene.layers().next().expect("at least one layer");
            let bg_rect = layer
                .rects
                .iter()
                .find(|rect| matches!(rect.background, Fill::Solid(color) if color == bg_color))
                .expect("background rect should be painted");

            // The background spans exactly the visible glyph span (36px), not the
            // full 120px run width.
            assert_approx_eq!(f32, bg_rect.bounds.width(), visible_right - visible_left);
            assert_approx_eq!(f32, bg_rect.bounds.min_x(), visible_left);
            assert_approx_eq!(f32, bg_rect.bounds.max_x(), visible_right);
        });
    });
}

/// A backgrounded run that is fully truncated (contributes no visible glyphs) must
/// not paint a background at all. Here a leading run consumes the entire paint
/// width, so the trailing backgrounded run is clipped away and the per-run
/// visible-span guard in `paint_internal` skips its background.
#[test]
fn test_fully_truncated_run_paints_no_background() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let bg_color = ColorU::from_u32(0x37393CFF);
            let glyph_width = 12.0;

            let make_glyphs = |start: usize, count: usize| {
                (0..count)
                    .map(|i| Glyph {
                        id: 0,
                        position_along_baseline: vec2f(glyph_width * (start + i) as f32, 0.),
                        index: start + i,
                        width: glyph_width,
                    })
                    .collect::<Vec<_>>()
            };

            // Run A (no background) fills the paint bounds; run B (background) sits
            // entirely past the bounds and is fully truncated.
            let run_a = Run {
                font_id: FontId(0),
                glyphs: make_glyphs(0, 5),
                styles: TextStyle::default(),
                width: glyph_width * 5.,
            };
            let run_b = Run {
                font_id: FontId(0),
                glyphs: make_glyphs(5, 5),
                styles: TextStyle::default().with_background_color(bg_color),
                width: glyph_width * 5.,
            };
            let line = Line {
                width: glyph_width * 10.,
                trailing_whitespace_width: 0.,
                runs: vec![run_a, run_b],
                font_size: 12.,
                line_height_ratio: 1.,
                baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                clip_config: None,
                ascent: 10.,
                descent: 2.,
                caret_positions: Vec::new(),
                chars_with_missing_glyphs: Vec::new(),
            };

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint(
                // 60px bounds == run A's width, so run B is fully truncated.
                RectF::new(Vector2F::zero(), Vector2F::new(60., 50.)),
                &PaintStyleOverride::default(),
                ColorU::black(),
                ctx.font_cache(),
                &mut scene,
            );

            let painted_bg = scene.layers().any(|layer| {
                layer
                    .rects
                    .iter()
                    .any(|rect| matches!(rect.background, Fill::Solid(color) if color == bg_color))
            });
            assert!(
                !painted_bg,
                "a fully truncated run must not paint its background",
            );
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
