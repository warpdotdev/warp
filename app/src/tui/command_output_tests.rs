use warpui_core::elements::tui::{Color, TuiBufferExt};

use super::{render_output_to_buffer, MAX_OUTPUT_LINES};

#[test]
fn plain_text_is_wrapped_to_width() {
    let buffer = render_output_to_buffer(b"ab\ncd", 4);
    assert_eq!(buffer.area.width, 4);
    assert_eq!(buffer.area.height, 2);
    assert_eq!(
        buffer.to_lines(),
        vec!["ab  ".to_string(), "cd  ".to_string()]
    );
}

#[test]
fn sgr_color_sets_cell_foreground() {
    // Red "hi" via SGR 31, then reset.
    let buffer = render_output_to_buffer(b"\x1b[31mhi\x1b[0m", 4);
    let cell = buffer.cell((0, 0)).expect("a painted cell at the origin");
    assert_eq!(cell.symbol(), "h");
    assert_eq!(cell.fg, Color::Red);
}

#[test]
fn invalid_utf8_does_not_panic() {
    // Leading invalid bytes are replaced; rendering must still succeed.
    let buffer = render_output_to_buffer(&[0xff, 0xfe, b'h', b'i'], 8);
    assert!(buffer.area.height >= 1);
}

#[test]
fn output_is_capped_to_the_most_recent_lines() {
    let many = (1..=500)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let buffer = render_output_to_buffer(many.as_bytes(), 8);

    assert!(buffer.area.height as usize <= MAX_OUTPUT_LINES);
    let lines = buffer.to_lines();
    // The newest line is kept; the oldest is dropped.
    assert!(lines.iter().any(|line| line.trim_end() == "500"));
    assert!(!lines.iter().any(|line| line.trim_end() == "1"));
}
