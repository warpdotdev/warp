use crate::CharCounter;

#[test]
fn test_char_counter_sequential() {
    let mut counter = CharCounter::new("It’s a 🌯!");
    assert_eq!(counter.char_offset(0), Some(0.into())); // I
    assert_eq!(counter.char_offset(1), Some(1.into())); // t
    assert_eq!(counter.char_offset(2), Some(2.into())); // ’ (3 bytes)
    assert_eq!(counter.char_offset(5), Some(3.into())); // s
    assert_eq!(counter.char_offset(6), Some(4.into())); // <space>
    assert_eq!(counter.char_offset(7), Some(5.into())); // a
    assert_eq!(counter.char_offset(8), Some(6.into())); // <space>
    assert_eq!(counter.char_offset(9), Some(7.into())); // 🌯 (4 bytes)
    assert_eq!(counter.char_offset(13), Some(8.into())); // !
}

#[test]
fn test_char_counter_out_of_order_fails() {
    let mut counter = CharCounter::new("words");
    assert_eq!(counter.char_offset(2), Some(2.into()));
    // Because the iterator only advances forwards, using an earlier offset is unsupported.
    assert_eq!(counter.char_offset(1), None);
}

#[test]
fn test_char_counter_non_boundary() {
    let mut counter = CharCounter::new("It’s a 🌯!");
    // Byte 11 is in the middle of the emoji.
    assert_eq!(counter.char_offset(11), None);
}

#[test]
fn test_char_counter_out_of_bounds() {
    let mut counter = CharCounter::new("short string");
    assert_eq!(counter.char_offset(7), Some(7.into()));
    assert_eq!(counter.char_offset(100), None);
}

#[test]
fn test_char_counter_nonsequential() {
    let mut counter = CharCounter::new("It’s a 🌯!");

    // Skip ahead to the t.
    assert_eq!(counter.char_offset(1), Some(1.into()));

    // Now, skip past the multi-byte apostrophe to the first space.
    assert_eq!(counter.char_offset(6), Some(4.into()));

    // Skip ahead to the emoji.
    assert_eq!(counter.char_offset(9), Some(7.into()));
}
