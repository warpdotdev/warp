use super::*;

#[test]
fn test_word_boundaries() {
    let buffer = "test/c/ab/word_with_underscores {восибing}";

    let starts: Vec<_> = buffer
        .word_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    assert_eq!(
        starts,
        [
            Point::new(0, 5),
            Point::new(0, 7),
            Point::new(0, 10),
            Point::new(0, 33),
            Point::new(0, 42),
        ]
    );

    let ends: Vec<_> = buffer
        .word_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    assert_eq!(
        ends,
        [
            Point::new(0, 4),
            Point::new(0, 6),
            Point::new(0, 9),
            Point::new(0, 31),
            Point::new(0, 41),
            Point::new(0, 42),
        ]
    );

    let starts_only_space: Vec<_> = buffer
        .word_starts_from_offset(Point::zero())
        .unwrap()
        .with_policy(WordBoundariesPolicy::OnlyWhitespace)
        .collect();
    assert_eq!(starts_only_space, [Point::new(0, 32), Point::new(0, 42)]);

    let ends_only_space: Vec<_> = buffer
        .word_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .with_policy(WordBoundariesPolicy::OnlyWhitespace)
        .collect();
    assert_eq!(ends_only_space, [Point::new(0, 31), Point::new(0, 42)]);

    let starts_custom: Vec<_> = buffer
        .word_starts_from_offset(Point::zero())
        .unwrap()
        .with_policy(WordBoundariesPolicy::Custom(HashSet::from(['{', '}'])))
        .collect();
    assert_eq!(starts_custom, [Point::new(0, 33), Point::new(0, 42)]);

    let ends_custom: Vec<_> = buffer
        .word_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .with_policy(WordBoundariesPolicy::Custom(HashSet::from(['{', '}'])))
        .collect();
    assert_eq!(
        ends_custom,
        [Point::new(0, 31), Point::new(0, 41), Point::new(0, 42)]
    );

    let starts_reversed: Vec<_> = buffer
        .word_starts_backward_from_offset_exclusive(Point::new(0, 42))
        .unwrap()
        .collect();
    assert_eq!(
        starts_reversed,
        [
            Point::new(0, 33),
            Point::new(0, 10),
            Point::new(0, 7),
            Point::new(0, 5),
            Point::new(0, 0),
        ]
    );

    let starts_mid: Vec<_> = buffer
        .word_starts_from_offset(Point::new(0, 7))
        .unwrap()
        .collect();
    assert_eq!(
        starts_mid,
        [Point::new(0, 10), Point::new(0, 33), Point::new(0, 42),]
    );

    let ends_mid: Vec<_> = buffer
        .word_ends_from_offset_exclusive(Point::new(0, 6))
        .unwrap()
        .collect();
    assert_eq!(
        ends_mid,
        [
            Point::new(0, 9),
            Point::new(0, 31),
            Point::new(0, 41),
            Point::new(0, 42),
        ]
    );

    let starts_reversed_mid: Vec<_> = buffer
        .word_starts_backward_from_offset_exclusive(Point::new(0, 8))
        .unwrap()
        .collect();
    assert_eq!(
        starts_reversed_mid,
        [Point::new(0, 7), Point::new(0, 5), Point::new(0, 0),]
    );

    let ends_inclusive: Vec<_> = buffer
        .word_ends_from_offset_inclusive(Point::new(0, 6))
        .unwrap()
        .collect();
    assert_eq!(
        ends_inclusive,
        [
            Point::new(0, 6),
            Point::new(0, 9),
            Point::new(0, 31),
            Point::new(0, 41),
            Point::new(0, 42),
        ]
    );

    let starts_reversed_inclusive: Vec<_> = buffer
        .word_starts_backward_from_offset_inclusive(Point::new(0, 10))
        .unwrap()
        .collect();
    assert_eq!(
        starts_reversed_inclusive,
        [
            Point::new(0, 10),
            Point::new(0, 7),
            Point::new(0, 5),
            Point::new(0, 0),
        ]
    );
}

#[test]
fn test_unicode_whitespace() {
    // See https://en.wikipedia.org/wiki/Whitespace_character
    let text = "first\tsecond\u{A0}third\u{2003}fourth";
    let starts: Vec<_> = text
        .word_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    assert_eq!(
        starts,
        [
            Point::new(0, 6),
            Point::new(0, 13),
            Point::new(0, 19),
            Point::new(0, 25)
        ]
    );
}
