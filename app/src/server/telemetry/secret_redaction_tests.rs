use serde_json::json;

use super::*;

// AWS-style access keys used in tests; these match `AWS_ACCESS_ID` from
// `DEFAULT_REGEXES_WITH_NAMES`. The example value is the standard one used in
// AWS documentation and is not a real key.
const AWS_KEY_1: &str = "AKIAIOSFODNN7EXAMPLE";
const AWS_KEY_2: &str = "AKIA1234567890123456";

#[test]
fn redact_secrets_in_string_with_no_match_is_noop() {
    let mut s = String::from("hello world, no secrets here");
    let original = s.clone();
    redact_secrets_in_string(&mut s);
    assert_eq!(s, original);
}

#[test]
fn redact_secrets_in_string_redacts_single_secret_in_middle() {
    let mut s = format!("prefix {AWS_KEY_1} suffix");
    redact_secrets_in_string(&mut s);
    let expected = format!("prefix {} suffix", "*".repeat(AWS_KEY_1.len()));
    assert_eq!(s, expected);
}

#[test]
fn redact_secrets_in_string_redacts_multiple_independent_secrets() {
    // This exercises the "replace from the end so earlier byte indices stay
    // valid" requirement: replacing the second secret first must not invalidate
    // the byte indices of the first secret.
    let mut s = format!("a {AWS_KEY_1} b {AWS_KEY_2} c");
    redact_secrets_in_string(&mut s);
    let expected = format!(
        "a {} b {} c",
        "*".repeat(AWS_KEY_1.len()),
        "*".repeat(AWS_KEY_2.len()),
    );
    assert_eq!(s, expected);
}

#[test]
fn redact_secrets_in_string_redacts_string_that_is_entirely_a_secret() {
    let mut s = AWS_KEY_1.to_string();
    redact_secrets_in_string(&mut s);
    assert_eq!(s, "*".repeat(AWS_KEY_1.len()));
}

#[test]
fn replace_byte_ranges_with_asterisks_with_empty_ranges_is_noop() {
    let mut s = String::from("no changes here");
    let original = s.clone();
    replace_byte_ranges_with_asterisks(&mut s, vec![]);
    assert_eq!(s, original);
}

#[test]
fn replace_byte_ranges_with_asterisks_replaces_independent_ranges() {
    // Separate, non-overlapping ranges are replaced independently. This
    // exercises the reverse-iteration so earlier byte indices stay valid as
    // the string mutates.
    let mut s = String::from("0123456789ABCDEF");
    let ranges = vec![0..3, 6..9, 12..15];
    replace_byte_ranges_with_asterisks(&mut s, ranges);
    assert_eq!(s, "***345***9AB***F");
}

#[test]
fn replace_byte_ranges_with_asterisks_merges_overlapping_ranges() {
    // Ranges 0..10 and 5..15 overlap; they should merge into 0..15 so we don't
    // double-replace the bytes in 5..10.
    let mut s = String::from("0123456789ABCDEF");
    let ranges = vec![0..10, 5..15];
    replace_byte_ranges_with_asterisks(&mut s, ranges);
    assert_eq!(s, "***************F");
}

#[test]
fn replace_byte_ranges_with_asterisks_merges_adjacent_ranges() {
    // Adjacent (touching) ranges should also merge.
    let mut s = String::from("0123456789ABCDEF");
    let ranges = vec![0..5, 5..10];
    replace_byte_ranges_with_asterisks(&mut s, ranges);
    assert_eq!(s, "**********ABCDEF");
}

#[test]
fn replace_byte_ranges_with_asterisks_handles_unsorted_ranges() {
    // Input ranges may be in arbitrary order; the function must sort before
    // merging or replacing.
    let mut s = String::from("0123456789ABCDEF");
    let ranges = vec![10..12, 0..2, 4..6];
    replace_byte_ranges_with_asterisks(&mut s, ranges);
    assert_eq!(s, "**23**6789**CDEF");
}

#[test]
fn replace_byte_ranges_with_asterisks_handles_fully_contained_range() {
    // A range fully contained in another should merge to the larger range.
    let mut s = String::from("0123456789ABCDEF");
    let ranges = vec![2..14, 5..8];
    replace_byte_ranges_with_asterisks(&mut s, ranges);
    assert_eq!(s, "01************EF");
}

#[test]
fn redact_secrets_in_value_redacts_strings_in_objects() {
    let mut value = json!({
        "with_secret": format!("contains {AWS_KEY_1} secret"),
        "without_secret": "no secret here",
    });
    redact_secrets_in_value(&mut value);
    assert_eq!(
        value["with_secret"],
        format!("contains {} secret", "*".repeat(AWS_KEY_1.len())),
    );
    assert_eq!(value["without_secret"], "no secret here");
}

#[test]
fn redact_secrets_in_value_redacts_strings_in_arrays() {
    let mut value = json!([
        format!("first {AWS_KEY_1}"),
        "second clean",
        format!("third {AWS_KEY_2}"),
    ]);
    redact_secrets_in_value(&mut value);
    assert_eq!(value[0], format!("first {}", "*".repeat(AWS_KEY_1.len())),);
    assert_eq!(value[1], "second clean");
    assert_eq!(value[2], format!("third {}", "*".repeat(AWS_KEY_2.len())),);
}

#[test]
fn redact_secrets_in_value_recurses_into_nested_structures() {
    let mut value = json!({
        "outer": {
            "inner_array": [
                format!("nested {AWS_KEY_1}"),
                {"inner_object": format!("deep {AWS_KEY_2}")},
            ],
            "scalar_int": 42,
            "scalar_bool": true,
            "scalar_null": null,
        }
    });
    redact_secrets_in_value(&mut value);
    assert_eq!(
        value["outer"]["inner_array"][0],
        format!("nested {}", "*".repeat(AWS_KEY_1.len())),
    );
    assert_eq!(
        value["outer"]["inner_array"][1]["inner_object"],
        format!("deep {}", "*".repeat(AWS_KEY_2.len())),
    );
    // Non-string scalars are left untouched.
    assert_eq!(value["outer"]["scalar_int"], 42);
    assert_eq!(value["outer"]["scalar_bool"], true);
    assert!(value["outer"]["scalar_null"].is_null());
}

#[test]
fn redact_secrets_in_value_leaves_non_string_scalars_untouched() {
    let mut value = json!({"n": 42, "b": true, "z": null});
    let expected = value.clone();
    redact_secrets_in_value(&mut value);
    assert_eq!(value, expected);
}

#[test]
fn compose_patterns_includes_defaults_when_user_and_enterprise_are_empty() {
    let patterns = compose_patterns(std::iter::empty(), std::iter::empty());
    assert_eq!(patterns.len(), DEFAULT_REGEXES_WITH_NAMES.len());
    for default in DEFAULT_REGEXES_WITH_NAMES {
        assert!(
            patterns.contains(&default.pattern),
            "expected default pattern {} to be present",
            default.pattern,
        );
    }
}

#[test]
fn compose_patterns_layers_user_and_enterprise_on_top_of_defaults() {
    let user = [r"\bUSER-\d+\b"];
    let enterprise = [r"\bENT-\d+\b"];
    let patterns = compose_patterns(user.iter().copied(), enterprise.iter().copied());
    // Enterprise comes first, then user, then defaults.
    assert_eq!(patterns[0], r"\bENT-\d+\b");
    assert_eq!(patterns[1], r"\bUSER-\d+\b");
    // Defaults are still all present.
    for default in DEFAULT_REGEXES_WITH_NAMES {
        assert!(
            patterns.contains(&default.pattern),
            "expected default pattern {} to be present alongside user/enterprise",
            default.pattern,
        );
    }
}

#[test]
fn compose_patterns_dedups_user_pattern_that_matches_a_default() {
    // Pick the first default pattern; passing the same string as a "user"
    // pattern should not cause it to appear twice in the composed list.
    let duplicated = DEFAULT_REGEXES_WITH_NAMES[0].pattern;
    let patterns = compose_patterns(std::iter::once(duplicated), std::iter::empty());
    let occurrences = patterns.iter().filter(|p| **p == duplicated).count();
    assert_eq!(
        occurrences, 1,
        "duplicate pattern should appear at most once in composed list",
    );
    // Total length is the defaults (the user pattern was deduped away).
    assert_eq!(patterns.len(), DEFAULT_REGEXES_WITH_NAMES.len());
}

#[test]
fn compose_patterns_dedups_enterprise_pattern_that_matches_a_user_pattern() {
    let user = [r"\bSHARED-\d+\b"];
    let enterprise = [r"\bSHARED-\d+\b"];
    let patterns = compose_patterns(user.iter().copied(), enterprise.iter().copied());
    let occurrences = patterns.iter().filter(|p| **p == r"\bSHARED-\d+\b").count();
    assert_eq!(occurrences, 1);
}
