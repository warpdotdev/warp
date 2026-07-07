use super::replace_unicode_word_boundaries;

#[test]
fn replaces_word_boundary_assertions() {
    assert_eq!(
        replace_unicode_word_boundaries(r"\bTOKEN\b"),
        r"(?-u:\b)TOKEN(?-u:\b)"
    );

    assert_eq!(
        replace_unicode_word_boundaries(r"\B_TOKEN\B"),
        r"(?-u:\B)_TOKEN(?-u:\B)"
    );
}

#[test]
fn preserves_escaped_literal_backslash_b() {
    assert_eq!(replace_unicode_word_boundaries(r"\\bTOKEN"), r"\\bTOKEN");
    assert_eq!(replace_unicode_word_boundaries(r"\\BTOKEN"), r"\\BTOKEN");
}

#[test]
fn preserves_backslash_b_in_character_classes() {
    assert_eq!(replace_unicode_word_boundaries(r"[\b]TOKEN"), r"[\b]TOKEN");
    assert_eq!(replace_unicode_word_boundaries(r"[\B]TOKEN"), r"[\B]TOKEN");
}

#[test]
fn replaces_boundary_after_escaped_literal_backslash() {
    assert_eq!(
        replace_unicode_word_boundaries(r"\\\bTOKEN"),
        r"\\(?-u:\b)TOKEN"
    );
}
