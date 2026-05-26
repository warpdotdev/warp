use warpui::text::point::Point;

use super::super::Buffer;

#[test]
fn test_subword_boundaries_forward_starts() {
    let mut buffer: Buffer;
    let mut starts: Vec<Point>;
    let mut starts_expected: Vec<Point>;

    buffer = Buffer::new("snake_case");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 6), Point::new(0, 10)];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("camelCase");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 5), Point::new(0, 9)];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("ALetter");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 1), Point::new(0, 7)];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("endWithA");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 3), Point::new(0, 7), Point::new(0, 8)];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("ABcD");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 1), Point::new(0, 3), Point::new(0, 4)];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("oneTwo_threeFour");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![
        Point::new(0, 3),
        Point::new(0, 7),
        Point::new(0, 12),
        Point::new(0, 16),
    ];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("s_hOrt_Word");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![
        Point::new(0, 2),
        Point::new(0, 3),
        Point::new(0, 7),
        Point::new(0, 11),
    ];
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("test/c/ab/word_with_underscoresAndUHHCaps {восибing}");
    starts = buffer
        .subword_starts_from_offset(Point::zero())
        .unwrap()
        .collect();
    starts_expected = vec![
        Point::new(0, 5),
        Point::new(0, 7),
        Point::new(0, 10),
        Point::new(0, 15),
        Point::new(0, 20),
        Point::new(0, 31),
        Point::new(0, 34),
        Point::new(0, 37),
        Point::new(0, 43),
        Point::new(0, 52),
    ];
    assert_eq!(starts, starts_expected);
}

#[test]
fn test_subword_boundaries_forward_ends() {
    let mut buffer: Buffer;
    let mut ends: Vec<Point>;
    let mut ends_expected: Vec<Point>;

    buffer = Buffer::new("snake_case");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![Point::new(0, 5), Point::new(0, 10)];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("camelCase");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![Point::new(0, 5), Point::new(0, 9)];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("ALetter");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![Point::new(0, 1), Point::new(0, 7)];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("endWithA");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![Point::new(0, 3), Point::new(0, 7), Point::new(0, 8)];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("ABcD");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![Point::new(0, 1), Point::new(0, 3), Point::new(0, 4)];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("oneTwo_threeFour");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![
        Point::new(0, 3),
        Point::new(0, 6),
        Point::new(0, 12),
        Point::new(0, 16),
    ];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("s_hOrt_Word");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![
        Point::new(0, 1),
        Point::new(0, 3),
        Point::new(0, 6),
        Point::new(0, 11),
    ];
    assert_eq!(ends, ends_expected);

    buffer = Buffer::new("test/c/ab/word_with_underscoresAndUHHCaps {восибing}");
    ends = buffer
        .subword_ends_from_offset_exclusive(Point::zero())
        .unwrap()
        .collect();
    ends_expected = vec![
        Point::new(0, 4),
        Point::new(0, 6),
        Point::new(0, 9),
        Point::new(0, 14),
        Point::new(0, 19),
        Point::new(0, 31),
        Point::new(0, 34),
        Point::new(0, 37),
        Point::new(0, 41),
        Point::new(0, 51),
        Point::new(0, 52),
    ];
    assert_eq!(ends, ends_expected);
}

#[test]
fn test_subword_boundaries_backward_starts() {
    let mut buffer: Buffer;
    let mut starts: Vec<Point>;
    let mut starts_expected: Vec<Point>;

    buffer = Buffer::new("snake_case");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 10))
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 0), Point::new(0, 6)];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("camelCase");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 9))
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 0), Point::new(0, 5)];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("ALetter");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 7))
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 0), Point::new(0, 1)];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("endWithA");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 8))
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 0), Point::new(0, 3), Point::new(0, 7)];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("ABcD");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 4))
        .unwrap()
        .collect();
    starts_expected = vec![Point::new(0, 0), Point::new(0, 1), Point::new(0, 3)];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("oneTwo_threeFour");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 16))
        .unwrap()
        .collect();
    starts_expected = vec![
        Point::new(0, 0),
        Point::new(0, 3),
        Point::new(0, 7),
        Point::new(0, 12),
    ];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("s_hOrt_Word");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 11))
        .unwrap()
        .collect();
    starts_expected = vec![
        Point::new(0, 0),
        Point::new(0, 2),
        Point::new(0, 3),
        Point::new(0, 7),
    ];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);

    buffer = Buffer::new("test/c/ab/word_with_underscoresAndUHHCaps {восибing}");
    starts = buffer
        .subword_backward_starts_from_offset_exclusive(Point::new(0, 52))
        .unwrap()
        .collect();
    starts_expected = vec![
        Point::new(0, 0),
        Point::new(0, 5),
        Point::new(0, 7),
        Point::new(0, 10),
        Point::new(0, 15),
        Point::new(0, 20),
        Point::new(0, 31),
        Point::new(0, 34),
        Point::new(0, 37),
        Point::new(0, 43),
    ];
    starts_expected.reverse();
    assert_eq!(starts, starts_expected);
}
