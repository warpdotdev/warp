use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use ratatui::text::Text;

use crate::elements::tui::{
    rasterize_text, TuiBuffer, TuiBufferExt, TuiCanvas, TuiCanvasCache, TuiConstraint, TuiElement,
    TuiLayoutContext, TuiRect, TuiSize, TuiStyle,
};

/// Builds a content buffer sized to `lines` (width = longest line) with each
/// line written at the left edge.
fn buffer_with(lines: &[&str]) -> TuiBuffer {
    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let height = lines.len() as u16;
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, width, height));
    for (row, line) in lines.iter().enumerate() {
        buffer.set_string(0, row as u16, line, TuiStyle::default());
    }
    buffer
}

/// Measures a leaf TUI element with no embedded child views.
fn layout_size(element: &mut dyn TuiElement, width: u16) -> TuiSize {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.layout(
        TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
        &mut ctx,
    )
}

/// Renders a leaf TUI element with no embedded child views.
fn render_to_buffer(element: &dyn TuiElement, area: TuiRect, buffer: &mut TuiBuffer) {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    element.render(area, buffer, &mut ctx);
}

#[test]
fn rasterize_text_produces_one_row_per_hard_line() {
    let buffer = rasterize_text(Text::from("ab\ncd"), 4);
    assert_eq!(buffer.area, TuiRect::new(0, 0, 4, 2));
    // Each row is padded with spaces out to the requested width.
    assert_eq!(
        buffer.to_lines(),
        vec!["ab  ".to_string(), "cd  ".to_string()]
    );
}

#[test]
fn rasterize_text_zero_width_is_empty() {
    let buffer = rasterize_text(Text::from("anything"), 0);
    assert_eq!(buffer.area, TuiRect::new(0, 0, 0, 0));
}

#[test]
fn canvas_reports_grid_height_and_blits_cells() {
    let mut canvas = TuiCanvas::new(TuiCanvasCache::new(), 0, |_width| {
        buffer_with(&["ab", "cd"])
    });
    assert_eq!(layout_size(&mut canvas, 10).height, 2);

    let mut dest = TuiBuffer::empty(TuiRect::new(0, 0, 4, 3));
    render_to_buffer(&canvas, TuiRect::new(0, 0, 4, 3), &mut dest);
    assert_eq!(
        dest.to_lines(),
        vec!["ab  ".to_string(), "cd  ".to_string(), "    ".to_string()]
    );
}

#[test]
fn canvas_regenerates_on_width_or_generation_change() {
    let calls = Rc::new(Cell::new(0usize));
    let cache = TuiCanvasCache::new();
    // All canvases share one cache + producer-call counter.
    let canvas = |generation: u64| {
        let counter = calls.clone();
        TuiCanvas::new(cache.clone(), generation, move |_width| {
            counter.set(counter.get() + 1);
            buffer_with(&["row"])
        })
    };

    // Same width + generation: produced once, then reused.
    let mut gen0 = canvas(0);
    assert_eq!(layout_size(&mut gen0, 8).height, 1);
    assert_eq!(layout_size(&mut gen0, 8).height, 1);
    assert_eq!(calls.get(), 1);

    // A new width (same generation) regenerates.
    layout_size(&mut gen0, 4);
    assert_eq!(calls.get(), 2);

    // A new generation (same width) regenerates: this is the streaming case.
    let mut gen1 = canvas(1);
    layout_size(&mut gen1, 4);
    assert_eq!(calls.get(), 3);
}

#[test]
fn canvas_clips_to_a_smaller_area() {
    let canvas = TuiCanvas::new(TuiCanvasCache::new(), 0, |_width| {
        buffer_with(&["abcd", "efgh"])
    });
    let mut dest = TuiBuffer::empty(TuiRect::new(0, 0, 2, 1));
    render_to_buffer(&canvas, TuiRect::new(0, 0, 2, 1), &mut dest);
    assert_eq!(dest.to_lines(), vec!["ab".to_string()]);
}
