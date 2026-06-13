use super::*;

#[test]
fn writes_ascii_text_and_pads_with_blanks() {
    let mut buffer = TuiBuffer::new(TuiSize::new(5, 1));

    let advanced = buffer.set_str(0, 0, 5, "abc", TuiStyle::default());

    assert_eq!(advanced, 3);
    assert_eq!(buffer.to_lines(), vec!["abc  "]);
}

#[test]
fn writes_text_at_an_offset() {
    let mut buffer = TuiBuffer::new(TuiSize::new(5, 2));

    buffer.set_str(2, 1, 5, "hi", TuiStyle::default());

    assert_eq!(
        buffer.to_lines(),
        vec!["     ".to_owned(), "  hi ".to_owned()]
    );
}

#[test]
fn out_of_bounds_writes_are_clipped_not_panicking() {
    let mut buffer = TuiBuffer::new(TuiSize::new(3, 2));

    // Entirely out of bounds in x, y: no-ops.
    assert_eq!(buffer.set_str(9, 0, 5, "xx", TuiStyle::default()), 0);
    assert_eq!(buffer.set_str(0, 9, 5, "xx", TuiStyle::default()), 0);
    buffer.set_cell(9, 9, Cell::new("z", TuiStyle::default()));
    assert!(buffer.get(9, 9).is_none());

    // Partially off the right edge: only what fits is written.
    let advanced = buffer.set_str(1, 0, 5, "world", TuiStyle::default());
    assert_eq!(advanced, 2);
    assert_eq!(buffer.to_lines(), vec![" wo".to_owned(), "   ".to_owned()]);
}

#[test]
fn max_width_clips_before_the_buffer_edge() {
    let mut buffer = TuiBuffer::new(TuiSize::new(10, 1));

    let advanced = buffer.set_str(0, 0, 3, "abcdef", TuiStyle::default());

    assert_eq!(advanced, 3);
    assert_eq!(buffer.to_lines(), vec!["abc       "]);
}

#[test]
fn set_cell_writes_one_glyph_and_get_reads_it_back() {
    let mut buffer = TuiBuffer::new(TuiSize::new(2, 1));
    let style = TuiStyle::default().with_foreground(Color::Red);

    buffer.set_cell(0, 0, Cell::new("x", style));

    assert_eq!(buffer.get(0, 0).unwrap().symbol(), "x");
    assert_eq!(buffer.get(0, 0).unwrap().style(), style);
    assert_eq!(buffer.get(1, 0).unwrap(), &Cell::blank());
}

#[test]
fn fill_paints_a_styled_background_over_a_rect() {
    let mut buffer = TuiBuffer::new(TuiSize::new(4, 3));
    let style = TuiStyle::default().with_background(Color::Blue);

    buffer.fill(TuiRect::new(1, 1, 2, 1), Cell::new(" ", style));

    assert_eq!(buffer.get(0, 1).unwrap().style(), TuiStyle::default());
    assert_eq!(buffer.get(1, 1).unwrap().style(), style);
    assert_eq!(buffer.get(2, 1).unwrap().style(), style);
    assert_eq!(buffer.get(3, 1).unwrap().style(), TuiStyle::default());
}

#[test]
fn buffers_compare_equal_iff_contents_and_styles_match() {
    let mut a = TuiBuffer::new(TuiSize::new(3, 1));
    let mut b = TuiBuffer::new(TuiSize::new(3, 1));
    a.set_str(0, 0, 3, "ab", TuiStyle::default());
    b.set_str(0, 0, 3, "ab", TuiStyle::default());
    assert_eq!(a, b);

    // Same glyphs, different style => not equal.
    let mut c = TuiBuffer::new(TuiSize::new(3, 1));
    c.set_str(0, 0, 3, "ab", TuiStyle::default().with_bold(true));
    assert_ne!(a, c);

    // Different size => not equal.
    let d = TuiBuffer::new(TuiSize::new(3, 2));
    assert_ne!(a, d);
}

#[test]
fn combining_grapheme_occupies_a_single_cell() {
    let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));

    buffer.set_str(0, 0, 4, "e\u{301}x", TuiStyle::default());

    assert_eq!(buffer.to_lines(), vec!["e\u{301}x  "]);
    assert_eq!(buffer.get(0, 0).unwrap().symbol(), "e\u{301}");
    assert!(!buffer.get(0, 0).unwrap().is_continuation());
    assert_eq!(buffer.get(1, 0).unwrap().symbol(), "x");
}

#[test]
fn wide_grapheme_spans_two_columns_with_a_continuation_cell() {
    let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));

    buffer.set_str(0, 0, 4, "界a", TuiStyle::default());

    assert_eq!(buffer.to_lines(), vec!["界a "]);
    assert_eq!(buffer.get(0, 0).unwrap().symbol(), "界");
    assert!(buffer.get(1, 0).unwrap().is_continuation());
    assert_eq!(buffer.get(2, 0).unwrap().symbol(), "a");
}

#[test]
fn a_wide_grapheme_that_would_cross_the_edge_is_dropped_whole() {
    let mut buffer = TuiBuffer::new(TuiSize::new(3, 1));

    buffer.set_str(0, 0, 3, "ab界", TuiStyle::default());

    assert_eq!(buffer.to_lines(), vec!["ab "]);
}

#[test]
fn overwriting_a_wide_grapheme_clears_its_continuation_cell() {
    let mut buffer = TuiBuffer::new(TuiSize::new(4, 1));
    buffer.set_str(0, 0, 4, "界a", TuiStyle::default());

    buffer.set_str(0, 0, 1, "b", TuiStyle::default());

    assert_eq!(buffer.to_lines(), vec!["b a "]);
    assert_eq!(buffer.get(0, 0).unwrap().symbol(), "b");
    assert!(!buffer.get(1, 0).unwrap().is_continuation());
    assert_eq!(buffer.get(1, 0).unwrap().symbol(), " ");
}
