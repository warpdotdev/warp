use crate::parser::{ParsedArgument, ParsedArgumentResult, ParsedArgumentsIterator};

#[test]
fn test_parsed_arguments_iterator() {
    let mut args_iterator = ParsedArgumentsIterator::new(
        "one two{{three}} {{four}}{{ab東早}} \ne{{восибing}}}".chars(),
    );

    let arg_three = args_iterator.next();
    assert_eq!(
        arg_three,
        Some(ParsedArgument {
            chars_range: 9..14,
            result: ParsedArgumentResult::Valid {
                current_word_index: 2
            },
        })
    );
    let arg_four = args_iterator.next();
    assert_eq!(
        arg_four,
        Some(ParsedArgument {
            chars_range: 19..23,
            result: ParsedArgumentResult::Valid {
                current_word_index: 3
            },
        })
    );
    let arg_ab = args_iterator.next();
    assert_eq!(
        arg_ab,
        Some(ParsedArgument {
            chars_range: 27..31,
            result: ParsedArgumentResult::Valid {
                current_word_index: 5
            },
        })
    );
    let arg_vosibing = args_iterator.next();
    assert_eq!(
        arg_vosibing,
        Some(ParsedArgument {
            chars_range: 38..46,
            result: ParsedArgumentResult::Valid {
                current_word_index: 7
            },
        })
    );

    let mut invalid_ranges_iter = ParsedArgumentsIterator::new(
        "one {{two}} {{inv\nal東旪!}} \n{{1malformed}} {{overlap{{bad }}".chars(),
    );

    let arg_two = invalid_ranges_iter.next();
    assert_eq!(
        arg_two,
        Some(ParsedArgument {
            chars_range: 6..9,
            result: ParsedArgumentResult::Valid {
                current_word_index: 1
            },
        })
    );

    let arg_invalid = invalid_ranges_iter.next();
    assert_eq!(
        arg_invalid,
        Some(ParsedArgument {
            chars_range: 14..23,
            result: ParsedArgumentResult::Invalid,
        })
    );
    let arg_malformed = invalid_ranges_iter.next();
    assert_eq!(
        arg_malformed,
        Some(ParsedArgument {
            chars_range: 29..39,
            result: ParsedArgumentResult::Invalid,
        })
    );
    let arg_bad = invalid_ranges_iter.next();
    assert_eq!(
        arg_bad,
        Some(ParsedArgument {
            chars_range: 53..57,
            result: ParsedArgumentResult::Invalid,
        })
    );
}

#[test]
fn test_parsed_arguments_iterator_with_escaped_args() {
    let mut escaped_ranges_iter = ParsedArgumentsIterator::new(
        "one {{TWO}} {{{.ID!}}} {{{{{not_3 an arg?}}} {{real_arg}} {{.invalid}}".chars(),
    );

    let arg_two = escaped_ranges_iter.next();
    assert_eq!(
        arg_two,
        Some(ParsedArgument {
            chars_range: 6..9,
            result: ParsedArgumentResult::Valid {
                current_word_index: 1
            }
        })
    );

    let arg_real = escaped_ranges_iter.next();
    assert_eq!(
        arg_real,
        Some(ParsedArgument {
            chars_range: 47..55,
            result: ParsedArgumentResult::Valid {
                current_word_index: 4
            }
        })
    );

    let arg_invalid = escaped_ranges_iter.next();
    assert_eq!(
        arg_invalid,
        Some(ParsedArgument {
            chars_range: 60..68,
            result: ParsedArgumentResult::Invalid
        })
    )
}
