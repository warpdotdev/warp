use float_cmp::assert_approx_eq;
use pathfinder_geometry::vector::vec2i;

use super::*;
use crate::fonts::Weight;
use crate::{App, Scene, rendering};

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
    synthetic_line_with_runs(&[(FontId(0), glyph_count)], glyph_width, clip_config)
}

/// Build a synthetic `Line` whose glyphs are split across one run per `(font, glyph count)` pair,
/// laid out left to right at `glyph_width` each.
fn synthetic_line_with_runs(
    runs: &[(FontId, usize)],
    glyph_width: f32,
    clip_config: ClipConfig,
) -> Line {
    let mut index = 0;
    let mut line_runs = Vec::with_capacity(runs.len());
    for (font_id, glyph_count) in runs {
        let mut glyphs = Vec::with_capacity(*glyph_count);
        for _ in 0..*glyph_count {
            glyphs.push(Glyph {
                id: 0,
                position_along_baseline: vec2f(glyph_width * index as f32, 0.),
                index,
                width: glyph_width,
            });
            index += 1;
        }
        line_runs.push(Run {
            font_id: *font_id,
            glyphs,
            styles: TextStyle::default(),
            width: glyph_width * *glyph_count as f32,
        });
    }
    Line {
        width: line_runs.iter().map(|run| run.width).sum(),
        trailing_whitespace_width: 0.,
        runs: line_runs,
        font_size: SYNTHETIC_FONT_SIZE,
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

/// A line asking for `ClipStyle::Ellipsis` must still be clipped when no run on it can draw an
/// ellipsis, which nothing guarantees: a title starting with `✳` puts that glyph in its own
/// fallback run, and Zapf Dingbats — the font Core Text picks for U+2733 — has no '…'. Without
/// either an ellipsis or a fade the glyph loop paints the glyph straddling the boundary in full,
/// so the line overhangs its own bounds and collides with whatever the layout placed after it.
/// The platform test `FontDB` reports a zero advance for '…', so it exercises exactly that case.
#[test]
fn test_paint_end_ellipsis_fades_when_no_run_can_draw_an_ellipsis() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            const GLYPH_WIDTH: f32 = 12.;
            let bounds = RectF::new(Vector2F::zero(), Vector2F::new(50., 20.));
            let line = synthetic_line(10, GLYPH_WIDTH, ClipConfig::ellipsis());

            let mut scene = Scene::new(1., rendering::Config::default());
            line.paint(
                bounds,
                &PaintStyleOverride::default(),
                ColorU::black(),
                ctx.font_cache(),
                &mut scene,
            );

            let glyphs: Vec<_> = scene
                .layers()
                .flat_map(|layer| layer.glyphs.iter())
                .collect();
            assert!(
                glyphs
                    .iter()
                    .any(|glyph| glyph.position.x() + GLYPH_WIDTH > bounds.upper_right().x()),
                "no glyph straddles the right edge, so the fade would go untested"
            );
            for glyph in glyphs {
                let Some(GlyphFade::Horizontal { start, end }) = glyph.fade else {
                    panic!("glyph at x={} was painted with no fade", glyph.position.x());
                };
                assert_approx_eq!(f32, end, bounds.upper_right().x());
                assert_approx_eq!(f32, start, bounds.upper_right().x() - LINE_FADE_MAX_PIXELS);
            }
        });
    });
}

/// Font size and per-glyph advance the ellipsis-resolution tests below build their lines from.
/// `ELLIPSIS_ADVANCE` is in font units, which the test `FontDB`'s 2048 units-per-em turns into
/// `ELLIPSIS_WIDTH` at `SYNTHETIC_FONT_SIZE`.
const SYNTHETIC_FONT_SIZE: f32 = 12.;
const SYNTHETIC_GLYPH_WIDTH: f32 = 10.;
const ELLIPSIS_ADVANCE: i32 = 2048;
const ELLIPSIS_WIDTH: f32 = SYNTHETIC_FONT_SIZE;

/// A `FontDB` where `symbol_font` cannot draw '…' and `text_font` can, mirroring a line whose
/// leading symbol fell back to a font with no ellipsis.
fn ellipsis_font_cache(symbol_font: FontId, text_font: FontId) -> FontCache {
    FontCache::new(Box::new(
        crate::platform::test::FontDB::new()
            .without_glyph(symbol_font, '…')
            .with_advance(text_font, vec2i(ELLIPSIS_ADVANCE, 0)),
    ))
}

fn painted_glyphs(scene: &Scene) -> Vec<crate::scene::Glyph> {
    scene
        .layers()
        .flat_map(|layer| layer.glyphs.iter())
        .cloned()
        .collect()
}

/// The ellipsis is resolved from whichever run can draw one, not only from the run at the line's
/// leading edge. A tab title starting with a symbol its font lacks puts that symbol in its own
/// fallback run, and looking there alone leaves the line unclipped.
#[test]
fn test_paint_end_ellipsis_resolves_from_a_later_run() {
    let symbol_font = FontId(1);
    let text_font = FontId(2);
    let font_cache = ellipsis_font_cache(symbol_font, text_font);
    // 10 glyphs at 10px each = 100px of line painted into 50px of bounds.
    let line = synthetic_line_with_runs(
        &[(symbol_font, 1), (text_font, 9)],
        SYNTHETIC_GLYPH_WIDTH,
        ClipConfig::ellipsis(),
    );
    let bounds = RectF::new(Vector2F::zero(), Vector2F::new(50., 20.));

    let mut scene = Scene::new(1., rendering::Config::default());
    line.paint(
        bounds,
        &PaintStyleOverride::default(),
        ColorU::black(),
        &font_cache,
        &mut scene,
    );

    // Three 10px glyphs fit in the 38px left over once the ellipsis reserves its 12px, so the
    // ellipsis lands right after them.
    let glyphs = painted_glyphs(&scene);
    assert_eq!(glyphs.len(), 4, "expected 3 glyphs plus an ellipsis");
    let ellipsis = glyphs
        .iter()
        .max_by(|a, b| a.position.x().total_cmp(&b.position.x()))
        .expect("just asserted the line painted glyphs");
    assert_approx_eq!(f32, ellipsis.position.x(), 30.);
    assert_eq!(
        ellipsis.glyph_key.font_id, text_font,
        "the ellipsis must come from the run that can draw one"
    );
    assert!(
        ellipsis.position.x() + ELLIPSIS_WIDTH <= bounds.upper_right().x(),
        "the ellipsis was painted past the line's bounds"
    );
    assert!(
        glyphs.iter().all(|glyph| glyph.fade.is_none()),
        "a line that resolved an ellipsis must not also fade"
    );
}

/// Start-clipping searches the runs from the other end, so the same fallback-run shape has to
/// resolve when the run that cannot draw '…' is the trailing one.
#[test]
fn test_paint_start_ellipsis_resolves_from_an_earlier_run() {
    let symbol_font = FontId(1);
    let text_font = FontId(2);
    let font_cache = ellipsis_font_cache(symbol_font, text_font);
    let line = synthetic_line_with_runs(
        &[(text_font, 9), (symbol_font, 1)],
        SYNTHETIC_GLYPH_WIDTH,
        ClipConfig {
            direction: ClipDirection::Start,
            style: ClipStyle::Ellipsis,
        },
    );
    let bounds = RectF::new(Vector2F::zero(), Vector2F::new(50., 20.));

    let mut scene = Scene::new(1., rendering::Config::default());
    line.paint(
        bounds,
        &PaintStyleOverride::default(),
        ColorU::black(),
        &font_cache,
        &mut scene,
    );

    let glyphs = painted_glyphs(&scene);
    assert_eq!(glyphs.len(), 4, "expected 3 glyphs plus an ellipsis");
    let ellipsis = glyphs
        .iter()
        .min_by(|a, b| a.position.x().total_cmp(&b.position.x()))
        .expect("just asserted the line painted glyphs");
    assert_eq!(
        ellipsis.glyph_key.font_id, text_font,
        "the ellipsis must come from the run that can draw one"
    );
    let leftmost_text_glyph = glyphs
        .iter()
        .filter(|glyph| glyph.position.x() > ellipsis.position.x())
        .map(|glyph| glyph.position.x())
        .fold(f32::MAX, f32::min);
    assert!(
        ellipsis.position.x() + ELLIPSIS_WIDTH <= leftmost_text_glyph,
        "the ellipsis at x={} overlaps the leftmost visible glyph at x={leftmost_text_glyph}",
        ellipsis.position.x()
    );
    assert!(
        glyphs.iter().all(|glyph| glyph.fade.is_none()),
        "a line that resolved an ellipsis must not also fade"
    );
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
