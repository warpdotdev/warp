use pathfinder_color::ColorU;
use warpui::elements::{Highlight, HighlightedRange};
use warpui::text_layout::TextStyle;

use super::normalize_highlighted_ranges;

fn highlighted_range(range: std::ops::Range<usize>, highlight: Highlight) -> HighlightedRange {
    HighlightedRange {
        highlight,
        highlight_indices: range.collect(),
    }
}

#[test]
fn normalize_highlighted_ranges_preserves_find_style_inside_url_link() {
    let link_foreground = ColorU::new(20, 80, 240, 255);
    let find_background = ColorU::new(255, 190, 80, 255);
    let find_foreground = ColorU::new(0, 0, 0, 255);
    let link_highlight = Highlight::new().with_text_style(
        TextStyle::new()
            .with_foreground_color(link_foreground)
            .with_underline_color(link_foreground),
    );
    let find_highlight = Highlight::new().with_text_style(
        TextStyle::new()
            .with_background_color(find_background)
            .with_foreground_color(find_foreground),
    );

    let normalized = normalize_highlighted_ranges(vec![
        highlighted_range(0..10, link_highlight),
        highlighted_range(4..9, find_highlight),
    ]);

    assert_eq!(normalized.len(), 3);
    assert_eq!(normalized[0], highlighted_range(0..4, link_highlight));
    assert_eq!(normalized[2], highlighted_range(9..10, link_highlight));
    assert_eq!(normalized[1].highlight_indices, (4..9).collect::<Vec<_>>());
    let combined_style = normalized[1].highlight.text_style();
    assert_eq!(combined_style.foreground_color, Some(find_foreground));
    assert_eq!(combined_style.background_color, Some(find_background));
    assert_eq!(combined_style.underline_color, Some(link_foreground));
}
