use super::*;

const CELL_WIDTH: f32 = 8.0;
const CELL_HEIGHT: f32 = 18.0;

/// Returns procedural rectangles for the standard test cell size.
fn standard_rects(c: char) -> SmallVec<[RectF; 3]> {
    rects(c, CELL_WIDTH, CELL_HEIGHT)
}

/// Returns whether the rectangles cover the given point.
fn covers_point(rects: &[RectF], x: f32, y: f32) -> bool {
    rects.iter().any(|rect| {
        let origin = rect.origin();
        x >= origin.x()
            && x < origin.x() + rect.width()
            && y >= origin.y()
            && y < origin.y() + rect.height()
    })
}

/// Verifies that vertical lines reach both cell edges.
#[test]
fn vertical_line_fills_cell_height() {
    for (width, height) in [(8.0, 18.0), (10.0, 22.0), (13.0, 30.0), (7.0, 15.0)] {
        let rects = rects('│', width, height);
        let top = rects
            .iter()
            .map(|rect| rect.origin().y())
            .fold(f32::INFINITY, f32::min);
        let bottom = rects
            .iter()
            .map(|rect| rect.origin().y() + rect.height())
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(top, 0.0);
        assert_eq!(bottom, height);
    }
}

/// Verifies that stacked vertical lines leave no uncovered seam.
#[test]
fn vertical_line_has_no_stacked_seam() {
    for (width, height) in [(8.0, 18.0), (10.0, 22.0), (7.0, 15.0)] {
        let rects = rects('│', width, height);
        let stroke_x = width / 2.0;
        assert!(covers_point(&rects, stroke_x, height - 0.01));
        assert!(covers_point(&rects, stroke_x, 0.0));
    }
}

/// Verifies that horizontal lines reach both cell edges.
#[test]
fn horizontal_line_fills_cell_width() {
    let rects = standard_rects('─');
    let left = rects
        .iter()
        .map(|rect| rect.origin().x())
        .fold(f32::INFINITY, f32::min);
    let right = rects
        .iter()
        .map(|rect| rect.origin().x() + rect.width())
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(left, 0.0);
    assert_eq!(right, CELL_WIDTH);
}

/// Verifies that a cross reaches every cell edge.
#[test]
fn cross_covers_all_edges() {
    let rects = standard_rects('┼');
    let center_x = CELL_WIDTH / 2.0;
    let center_y = CELL_HEIGHT / 2.0;
    assert!(covers_point(&rects, center_x, 0.0));
    assert!(covers_point(&rects, center_x, CELL_HEIGHT - 0.01));
    assert!(covers_point(&rects, 0.0, center_y));
    assert!(covers_point(&rects, CELL_WIDTH - 0.01, center_y));
}

/// Verifies that junction rectangles remain disjoint.
#[test]
fn junction_rects_do_not_overlap() {
    for c in ['┼', '╋', '├', '┤', '┬', '┴', '┌', '┐', '└', '┘'] {
        for (width, height) in [(8.0, 18.0), (11.0, 24.0), (7.0, 15.0)] {
            let rects = rects(c, width, height);
            assert!(
                !rects_overlap(&rects),
                "{c:?} produced overlapping rects for cell {width}x{height}"
            );
        }
    }
}

/// Verifies that heavy strokes are visibly thicker.
#[test]
fn heavy_line_is_thicker_than_light() {
    let light_width: f32 = standard_rects('│').iter().map(|rect| rect.width()).sum();
    let heavy_width: f32 = standard_rects('┃').iter().map(|rect| rect.width()).sum();
    assert!(heavy_width > light_width);
}

/// Verifies that unsupported glyphs return no procedural geometry.
#[test]
fn unsupported_glyphs_return_no_rects() {
    for c in ['═', '║', '╬', '╭', '╱', 'a', ' '] {
        assert!(!is_supported(c));
        assert!(standard_rects(c).is_empty());
    }
}

/// Verifies that support detection matches generated geometry.
#[test]
fn supported_glyphs_have_rects() {
    for codepoint in 0x2500u32..=0x257F {
        let c = char::from_u32(codepoint).unwrap();
        assert_eq!(
            is_supported(c),
            !standard_rects(c).is_empty(),
            "mismatch for U+{codepoint:04X} {c:?}"
        );
    }
}
