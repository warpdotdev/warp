use super::*;
use std::collections::HashSet;
use std::iter::FromIterator;

#[test]
fn test_basic() {
    let text = Text::from(String::from("ab\ncd€\nfghij\nkl¢m"));
    assert_eq!(text.len(), 17.into());
    assert_eq!(text.as_str(), "ab\ncd€\nfghij\nkl¢m");
    assert_eq!(text.lines(), Point::new(3, 4));
    assert_eq!(text.line_len(0), 2);
    assert_eq!(text.line_len(1), 3);
    assert_eq!(text.line_len(2), 5);
    assert_eq!(text.line_len(3), 4);
    assert_eq!(text.rightmost_point(), Point::new(2, 5));

    assert_eq!(text.byte_offset_for_point(Point::new(1, 0)), 3.into());
    assert_eq!(text.byte_offset_for_point(Point::new(3, 3)), 19.into());

    assert_eq!(text.char_offset_for_byte_offset(3.into()), 3.into());
    // The string is 20 bytes but only 17 characters.
    assert_eq!(text.char_offset_for_byte_offset(20.into()), 17.into());

    let b_to_g = text.slice(CharOffset::from(1)..CharOffset::from(9));
    assert_eq!(b_to_g.as_str(), "b\ncd€\nfg");
    assert_eq!(b_to_g.len(), 8.into());
    assert_eq!(b_to_g.lines(), Point::new(2, 2));
    assert_eq!(b_to_g.line_len(0), 1);
    assert_eq!(b_to_g.line_len(1), 3);
    assert_eq!(b_to_g.line_len(2), 2);
    assert_eq!(b_to_g.line_len(3), 0);
    assert_eq!(b_to_g.rightmost_point(), Point::new(1, 3));

    assert_eq!(b_to_g.byte_offset_for_point(Point::new(1, 0)), 2.into());
    assert_eq!(b_to_g.byte_offset_for_point(Point::new(2, 1)), 9.into());

    assert_eq!(b_to_g.char_offset_for_byte_offset(6.into()), 4.into());
    // The string is 10 bytes but only 8 characters.
    assert_eq!(b_to_g.char_offset_for_byte_offset(9.into()), 7.into());

    let d_to_i = text.slice(CharOffset::from(4)..CharOffset::from(11));
    assert_eq!(d_to_i.as_str(), "d€\nfghi");
    assert_eq!(&d_to_i[CharOffset::from(1)..CharOffset::from(5)], "€\nfg");
    assert_eq!(d_to_i.len(), 7.into());
    assert_eq!(d_to_i.lines(), Point::new(1, 4));
    assert_eq!(d_to_i.line_len(0), 2);
    assert_eq!(d_to_i.line_len(1), 4);
    assert_eq!(d_to_i.line_len(2), 0);
    assert_eq!(d_to_i.rightmost_point(), Point::new(1, 4));

    assert_eq!(d_to_i.byte_offset_for_point(Point::new(1, 0)), 5.into());
    assert_eq!(d_to_i.byte_offset_for_point(Point::new(1, 3)), 8.into());

    // A byte index in the middle of a character should return the character before.
    assert_eq!(d_to_i.char_offset_for_byte_offset(1.into()), 1.into());
    assert_eq!(d_to_i.char_offset_for_byte_offset(2.into()), 1.into());

    // The string is 8 bytes but only 7 characters.
    assert_eq!(d_to_i.char_offset_for_byte_offset(9.into()), 7.into());

    let d_to_j = text.slice(CharOffset::from(4)..=CharOffset::from(11));
    assert_eq!(d_to_j.as_str(), "d€\nfghij");
    assert_eq!(&d_to_j[CharOffset::from(1)..], "€\nfghij");
    assert_eq!(d_to_j.len(), 8.into());
}

#[test]
fn test_random() {
    use rand::prelude::*;

    for seed in 0..100 {
        println!("buffer::text seed: {seed}");
        let rng = &mut StdRng::seed_from_u64(seed);

        let len: i32 = rng.gen_range(0..50);
        let mut string = String::new();
        for _ in 0..len {
            if rng.gen_ratio(1, 5) {
                string.push('\n');
            } else {
                string.push(rng.gen());
            }
        }
        let text = Text::from(string.clone());

        for _ in 0..10 {
            let start = CharOffset::from(rng.gen_range(0..text.len().as_usize() + 1));
            let end = CharOffset::from(rng.gen_range(start.as_usize()..text.len().as_usize() + 2));

            let string_slice = string
                .chars()
                .skip(start.as_usize())
                .take(end.as_usize() - start.as_usize())
                .collect::<String>();
            let expected_line_endpoints = string_slice
                .split('\n')
                .enumerate()
                .map(|(row, line)| Point::new(row as u32, line.chars().count() as u32))
                .collect::<Vec<_>>();
            let text_slice = text.slice(start..end);

            assert_eq!(text_slice.lines(), lines(&string_slice));

            let mut rightmost_points: HashSet<Point> = HashSet::new();
            for endpoint in &expected_line_endpoints {
                if let Some(rightmost_point) = rightmost_points.iter().next().cloned() {
                    if endpoint.column > rightmost_point.column {
                        rightmost_points.clear();
                    }
                    if endpoint.column >= rightmost_point.column {
                        rightmost_points.insert(*endpoint);
                    }
                } else {
                    rightmost_points.insert(*endpoint);
                }

                assert_eq!(text_slice.line_len(endpoint.row), endpoint.column);
            }

            assert!(rightmost_points.contains(&text_slice.rightmost_point()));

            for _ in 0..10 {
                let offset = CharOffset::from(rng.gen_range(0..string_slice.chars().count() + 1));
                let point = lines(
                    &string_slice
                        .chars()
                        .take(offset.as_usize())
                        .collect::<String>(),
                );
                assert_eq!(text_slice.point_for_offset(offset), point);
                assert_eq!(text_slice.offset_for_point(point), offset);
                if offset < string_slice.chars().count().into() {
                    assert_eq!(
                        &text_slice[offset..offset + 1],
                        String::from_iter(string_slice.chars().nth(offset.as_usize())).as_str()
                    );
                }
            }
        }
    }
}

pub fn lines(s: &str) -> Point {
    let mut row = 0;
    let mut column = 0;
    for ch in s.chars() {
        if ch == '\n' {
            row += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    Point::new(row, column)
}
