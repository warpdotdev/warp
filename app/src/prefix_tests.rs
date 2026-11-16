use crate::prefix::longest_common_prefix;

#[test]
fn test_basic_prefix() {
    let strings = ["foo", "foobar", "foofoobar"];

    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "foo");
}

#[test]
fn test_no_prefix() {
    let strings = ["foo", "foobar", "bar"];
    let result = longest_common_prefix(strings);
    assert_eq!(result, None);
}

#[test]
fn test_single_string() {
    let strings = ["foo"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "foo");
}

#[test]
fn test_no_string() {
    let strings = [];
    let result = longest_common_prefix(strings);
    assert_eq!(result, None);
}

#[test]
fn test_multibyte_strings() {
    let strings = ["ay東cz", "ay東ab"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "ay東");
}

#[test]
fn test_multibyte_strings_common_bytes() {
    let strings = ["ab東早", "ab東旪"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "ab東");
}
