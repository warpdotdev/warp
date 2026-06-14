use super::format_token_count;

#[test]
fn test_format_token_count_zero() {
    assert_eq!(format_token_count(0), "0 tok");
}

#[test]
fn test_format_token_count_small() {
    assert_eq!(format_token_count(42), "42 tok");
}

#[test]
fn test_format_token_count_thousands() {
    assert_eq!(format_token_count(1247), "1247 tok");
}

#[test]
fn test_format_token_count_large() {
    assert_eq!(format_token_count(1_000_000), "1000000 tok");
}
