use super::*;
use crate::text::SelectionDirection;

/// Convenience wrapper for the forward exclusive-end target under the default policy.
fn fwd(buffer: &str, offset: usize) -> Point {
    buffer
        .semantic_expansion_target(
            CharOffset::from(offset),
            SelectionDirection::Forward,
            &WordBoundariesPolicy::Default,
        )
        .unwrap()
}

/// Convenience wrapper for the backward inclusive-start target under the default policy.
fn bwd(buffer: &str, offset: usize) -> Point {
    buffer
        .semantic_expansion_target(
            CharOffset::from(offset),
            SelectionDirection::Backward,
            &WordBoundariesPolicy::Default,
        )
        .unwrap()
}

#[test]
fn test_semantic_expansion_matches_block_list() {
    // semantic_expansion_target must behave exactly like the terminal grid's
    // semantic_search_left/right, which crucially never includes trailing whitespace when the
    // tail sits on a non-word character.

    // Char offsets: f0 o1 o2 .3 .4 .5 <space>6 b7 a8 r9 <space>10 b11 a12 z13
    let dots = "foo... bar baz";

    // Forward, tail on the LAST char of a punctuation run: the run is included up to that char,
    // but the trailing space is NOT (exclusive end 6 = "foo...", not 7 which would add the space).
    assert_eq!(
        fwd(dots, 5),
        Point::new(0, 6),
        "trailing space must be excluded"
    );
    // Forward, tail on an earlier punctuation char: stops right after that char (next is a
    // boundary), per the grid's per-cell behavior.
    assert_eq!(fwd(dots, 3), Point::new(0, 4));
    assert_eq!(fwd(dots, 4), Point::new(0, 5));
    // Forward, tail on a word char: ordinary word selection (unchanged).
    assert_eq!(fwd(dots, 1), Point::new(0, 3));
    assert_eq!(fwd(dots, 0), Point::new(0, 3));
    // Forward, tail directly on whitespace whose next char is a word: extends through that word
    // (matches the grid, where a space's right neighbor pulls in the word).
    assert_eq!(fwd(dots, 6), Point::new(0, 10));

    // Backward, tail on a punctuation char whose left neighbor is also a boundary: stays put.
    assert_eq!(bwd(dots, 5), Point::new(0, 5));
    assert_eq!(bwd(dots, 4), Point::new(0, 4));
    // Backward, tail on a word char: ordinary word selection (start of "bar").
    assert_eq!(bwd(dots, 8), Point::new(0, 7));

    // A comma followed by a space: forward from the comma includes the comma but excludes the
    // following space (the reporter's exact case).
    // Char offsets: f0 o1 o2 ,3 <space>4 b5 a6 r7
    let comma = "foo, bar";
    assert_eq!(
        fwd(comma, 3),
        Point::new(0, 4),
        "comma included, trailing space excluded",
    );
}

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
