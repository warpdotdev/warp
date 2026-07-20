use super::point::Point;
use super::{BufferIndex, TextBuffer, char_slice, count_chars_up_to_byte, str_to_byte_vec};

#[test]
fn test_str_to_byte_vec() {
    assert_eq!(
        str_to_byte_vec("foo bar"),
        vec![0x66, 0x6f, 0x6f, 0x20, 0x62, 0x61, 0x72]
    );
}

/// Test the [`str`] implementation of [`TextBuffer`], which we rely on in other unit tests.
#[test]
fn test_str_buffer() -> anyhow::Result<()> {
    let buf = "Hello\nWorld!";

    assert_eq!(buf.chars_at(2.into())?.collect::<String>(), "llo\nWorld!");
    assert_eq!(buf.chars_rev_at(3.into())?.collect::<String>(), "leH");

    // For simplicity, we do not wrap newlines into new rows.
    assert_eq!(buf.to_point(7.into())?, Point::new(0, 7));
    assert!(Point::new(1, 1).to_char_offset(buf).is_err());
    assert_eq!(Point::new(0, 7).to_char_offset(buf)?, 7.into());

    Ok(())
}

#[test]
fn test_char_slice() {
    let has_nonbreaking_space = "A\u{a0}non-breaking space occupies 2 bytes in UTF-8";
    assert_eq!(char_slice(has_nonbreaking_space, 0, 3), Some("A\u{a0}n"));

    // This string has characters ['A', '❤', '\u{fe0f}', '\u{200d}', '🔥', 'b']
    assert_eq!(char_slice("A❤️‍🔥b", 4, 5), Some("🔥"));

    assert_eq!(char_slice("abc", 5, 10), None);
    assert_eq!(char_slice("abc", 2, 0), None);
    assert_eq!(char_slice("abc", 1, 4), None);

    assert_eq!(char_slice("A string", 2, 4), Some("st"));

    assert_eq!(char_slice("The end: 🫥??", 10, 12), Some("??"));

    assert_eq!(char_slice("🫥", 0, 0), Some(""));
}

#[test]
fn test_char_counts_up_to_byte() {
    let text = "abc🔥abc☄️abc😬";
    assert_eq!(count_chars_up_to_byte(text, 0.into()), Some(0.into()));
    assert_eq!(
        count_chars_up_to_byte(text, "abc🔥".len().into()),
        Some(4.into())
    );
    assert_eq!(
        count_chars_up_to_byte(text, text.len().into()),
        Some(text.chars().count().into())
    );
}
