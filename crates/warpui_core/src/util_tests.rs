use super::*;

#[test]
fn parse_invalid_u32() {
    assert_eq!(parse_u32(b"1abc"), None);
    assert_eq!(parse_u32(b"-100"), None);
    // u32::MAX + 1
    assert_eq!(parse_u32(b"4294967296"), None);
}

#[test]
fn parse_valid_u32() {
    assert_eq!(parse_u32(b"123"), Some(123));
    assert_eq!(parse_u32(b"4294967294"), Some(u32::MAX - 1));
}

#[test]
fn parse_invalid_i32() {
    assert_eq!(parse_i32(b"1abc"), None);
    // i32::MAX + 1
    assert_eq!(parse_i32(b"2147483648"), None);
    assert_eq!(parse_i32(b"1.0"), None);
}

#[test]
fn parse_valid_i32() {
    assert_eq!(parse_i32(b"123"), Some(123));
    assert_eq!(parse_i32(b"-200"), Some(-200));
    assert_eq!(parse_i32(b"2147483646"), Some(i32::MAX - 1));
}
