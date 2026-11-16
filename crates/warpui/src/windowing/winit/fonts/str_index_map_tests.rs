use super::*;

#[test]
fn test_str_index_map_get_byte_index() {
    let text = "ab😈■d";
    let str_index_map = StrIndexMap::new(text);

    assert_eq!(str_index_map.byte_index(0), Some(0));
    assert_eq!(str_index_map.byte_index(1), Some(1));
    assert_eq!(str_index_map.byte_index(2), Some(2));

    // The character at index 2 (😈) is 4 bytes, which means the character at index 3 (■) starts at
    // byte index 6.
    assert_eq!(str_index_map.byte_index(3), Some(6));
    // The character at index 3 (■) is 3 bytes, which means the character at index 4 (d) starts at
    // byte index 9.
    assert_eq!(str_index_map.byte_index(4), Some(9));

    // The backing string only has 5 characters. Ensure we return None in the case a character index
    // that isn't included in the string is passed.
    assert_eq!(str_index_map.byte_index(5), None);
}

#[test]
fn test_str_index_map_get_char_index() {
    let text = "ab😈■d";
    let str_index_map = StrIndexMap::new(text);

    assert_eq!(str_index_map.char_index(0), Some(0));
    assert_eq!(str_index_map.char_index(1), Some(1));
    assert_eq!(str_index_map.char_index(2), Some(2));

    // The character at index 2 (😈) is 4 bytes, which means the character at index 3 (■) starts at
    // byte index 6.
    assert_eq!(str_index_map.char_index(6), Some(3));
    // The character at index 3 (■) is 3 bytes, which means the character at index 4 (d) starts at
    // byte index 9.
    assert_eq!(str_index_map.char_index(9), Some(4));

    // The backing string only has 10 bytes. Ensure we return None in the case a byte index
    // that isn't included in the string is passed.
    assert_eq!(str_index_map.char_index(10), None);

    // Byte index 3 is not a char boundary, so we should return None.
    assert_eq!(str_index_map.char_index(3), None);
}
