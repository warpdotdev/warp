//! Unit tests for the [`super`] box-drawing geometry module.

use super::*;

// Cell dimensions are in device pixels (whole numbers, as the caller snaps
// them to the pixel grid before calling in).
const W: f32 = 8.0;
const H: f32 = 18.0;

fn rects(c: char) -> SmallVec<[CellRect; 8]> {
    cell_rects(c, W, H)
}

fn covers_row(rects: &[CellRect], y: f32) -> bool {
    rects.iter().any(|r| {
        let y0 = r.bounds.origin().y();
        let y1 = y0 + r.bounds.height();
        y >= y0 && y < y1
    })
}

fn covers_col(rects: &[CellRect], x: f32) -> bool {
    rects.iter().any(|r| {
        let x0 = r.bounds.origin().x();
        let x1 = x0 + r.bounds.width();
        x >= x0 && x < x1
    })
}

#[test]
fn sprite_vline_fills_cell_height() {
    // The vertical bar of `│` must span the FULL cell height (0..=cell_h) at
    // several cell sizes, so that stacked cells — which share an integer
    // pixel boundary — abut with no gap.
    for (w, h) in [(8.0, 18.0), (10.0, 22.0), (13.0, 30.0), (7.0, 15.0)] {
        let r = cell_rects('│', w, h);
        assert!(!r.is_empty(), "no rects for cell {w}x{h}");
        let top = r
            .iter()
            .map(|r| r.bounds.origin().y())
            .fold(f32::INFINITY, f32::min);
        let bot = r
            .iter()
            .map(|r| r.bounds.origin().y() + r.bounds.height())
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(top, 0.0, "top should be exactly the cell top for {w}x{h}");
        assert_eq!(
            bot, h,
            "bottom should be exactly the cell bottom for {w}x{h}"
        );
    }
}

#[test]
fn sprite_vline_no_seam_when_stacked() {
    // Two `│` cells stacked vertically share the boundary at y == H. Because
    // each bar spans its full cell height exactly, the top cell covers its
    // bottom edge and the bottom cell covers its top edge at the same x, so
    // there is no uncovered row at the seam.
    for (w, h) in [(8.0, 18.0), (10.0, 22.0), (7.0, 15.0)] {
        let cell = cell_rects('│', w, h);
        let stroke_x = w / 2.0;
        assert!(
            covers_row_at(&cell, h - 0.01, stroke_x),
            "cell must cover its bottom edge for {w}x{h}"
        );
        assert!(
            covers_row_at(&cell, 0.0, stroke_x),
            "cell must cover its top edge for {w}x{h}"
        );
    }
}

fn covers_row_at(rects: &[CellRect], y: f32, x: f32) -> bool {
    rects.iter().any(|r| {
        let x0 = r.bounds.origin().x();
        let x1 = x0 + r.bounds.width();
        let y0 = r.bounds.origin().y();
        let y1 = y0 + r.bounds.height();
        x >= x0 && x < x1 && y >= y0 && y < y1
    })
}

#[test]
fn sprite_hline_fills_cell_width() {
    // The horizontal bar of `─` must span the full cell width so horizontally
    // adjacent cells connect.
    let r = rects('─');
    let left = r
        .iter()
        .map(|r| r.bounds.origin().x())
        .fold(f32::INFINITY, f32::min);
    let right = r
        .iter()
        .map(|r| r.bounds.origin().x() + r.bounds.width())
        .fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(left, 0.0);
    assert_eq!(right, W);
}

#[test]
fn sprite_cross_covers_all_edges() {
    // `┼` must have ink touching all four cell edges through the centered
    // strokes.
    let r = rects('┼');
    assert!(covers_row(&r, 0.0), "top edge");
    assert!(covers_row(&r, H - 0.01), "bottom edge");
    assert!(covers_col(&r, 0.0), "left edge");
    assert!(covers_col(&r, W - 0.01), "right edge");
}

#[test]
fn sprite_cross_is_non_overlapping() {
    // The crossing of `┼`/`╋` is the canonical overlap risk; verify the
    // partition is disjoint (required for non-opaque foreground colors)
    // across a range of cell sizes.
    for c in ['┼', '╋', '├', '┤', '┬', '┴', '┌', '┐', '└', '┘'] {
        for (w, h) in [(8.0, 18.0), (11.0, 24.0), (7.0, 15.0)] {
            let r = cell_rects(c, w, h);
            assert!(
                !rects_overlap(&r),
                "{c:?} produced overlapping rects for cell {w}x{h}"
            );
        }
    }
}

#[test]
fn sprite_heavy_is_thicker_than_light() {
    let light = rects('│');
    let heavy = rects('┃');
    let light_w: f32 = light.iter().map(|r| r.bounds.width()).sum();
    let heavy_w: f32 = heavy.iter().map(|r| r.bounds.width()).sum();
    assert!(
        heavy_w > light_w,
        "heavy bar ({heavy_w}) should be wider than light ({light_w})"
    );
}

#[test]
fn sprite_full_block_fills_cell() {
    let r = rects('█');
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].bounds.origin(), vec2f(0.0, 0.0));
    assert_eq!(r[0].bounds.width(), W);
    assert_eq!(r[0].bounds.height(), H);
    assert_eq!(r[0].alpha_scale, 1.0);
}

#[test]
fn sprite_half_blocks() {
    let upper = rects('▀');
    assert_eq!(upper[0].bounds.height(), H / 2.0);
    assert_eq!(upper[0].bounds.origin().y(), 0.0);

    let lower = rects('▄');
    assert_eq!(lower[0].bounds.origin().y(), H / 2.0);

    let left = rects('▌');
    assert_eq!(left[0].bounds.width(), W / 2.0);

    let right = rects('▐');
    assert_eq!(right[0].bounds.origin().x(), W / 2.0);
}

#[test]
fn sprite_shades_use_alpha() {
    assert_eq!(rects('░')[0].alpha_scale, 0.25);
    assert_eq!(rects('▒')[0].alpha_scale, 0.5);
    assert_eq!(rects('▓')[0].alpha_scale, 0.75);
}

#[test]
fn sprite_quadrants_are_disjoint_and_cover() {
    // `▟` = top-right + bottom-left + bottom-right; three disjoint quarters.
    let r = rects('▟');
    assert_eq!(r.len(), 3);
    assert!(!rects_overlap(&r));
}

#[test]
fn unsupported_glyphs_return_empty() {
    // Double lines, rounded corners, diagonals and plain text are not
    // handled here (they fall back to the font).
    for c in ['═', '║', '╬', '╭', '╱', 'a', ' '] {
        assert!(!super::is_supported(c), "{c:?} should be unsupported");
        assert!(cell_rects(c, W, H).is_empty(), "{c:?} should be empty");
    }
}

#[test]
fn is_supported_matches_cell_rects() {
    for cp in 0x2500u32..=0x259F {
        let c = char::from_u32(cp).unwrap();
        assert_eq!(
            is_supported(c),
            !cell_rects(c, W, H).is_empty(),
            "mismatch for U+{cp:04X} {c:?}"
        );
    }
}
