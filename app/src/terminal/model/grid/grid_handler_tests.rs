// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

use std::ops::BitOrAssign;

use warp_terminal::model::char_or_str::CharOrStr;
use warp_util::path::LineAndColumnArg;
use warpui::text::words::is_default_word_boundary;

use crate::terminal::model::secrets::{ObfuscateSecrets, SecretLevel};
use crate::terminal::model::{blockgrid::BlockGrid, secrets::IsObfuscated};
use crate::test_util::mock_blockgrid;

use super::*;

const MAX_SCROLL_LIMIT: usize = 1000;

fn has_wide_char_character(blockgrid: &BlockGrid) -> bool {
    let mut point = Point { row: 0, col: 0 };

    while let Some(cell_type) = blockgrid.grid_handler().cell_type(point) {
        if matches!(cell_type, CellType::WideChar) {
            return true;
        }
        point = point.wrapping_add(blockgrid.grid_handler().columns(), 1);
    }

    false
}

#[test]
fn test_line_selection() {
    let blockgrid = mock_blockgrid(
        "\
        hello\r\n:)\nearth!",
    );

    // This will create a blockgrid with the following cells:
    // // [h][e][l][l][o][ ]
    // // [:][)][ ][ ][ ][ ] <- WRAPLINE flag set
    // // [e][a][r][t][h][!]
    assert_eq!(
        blockgrid.grid_handler.line_search_left(Point::new(0, 2)),
        Point::new(0, 0)
    );
    assert_eq!(
        blockgrid.grid_handler.line_search_left(Point::new(1, 2)),
        Point::new(1, 0)
    );
    assert_eq!(
        blockgrid.grid_handler.line_search_left(Point::new(2, 2)),
        Point::new(1, 0)
    );
    assert_eq!(
        blockgrid.grid_handler.line_search_right(Point::new(0, 5)),
        Point::new(0, 5)
    );
    assert_eq!(
        blockgrid.grid_handler.line_search_right(Point::new(1, 2)),
        Point::new(2, 5)
    );
}

#[test]
fn regex_left_on_line_wrap() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("aat\nest\r\n111");

    // Check regex across wrapped and unwrapped lines.
    let dfas = RegexDFAs::new("t.*st").unwrap();
    let start = Point::new(1, 2);
    let end = Point::new(0, 0);
    let match_start = Point::new(0, 2);
    let match_end = Point::new(1, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn regex_right() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        testing66\r\n\
        Warp\n\
        123\r\n\
        Warp\r\n\
        123\
    ");

    // Check regex across wrapped and unwrapped lines.
    let dfas = RegexDFAs::new("Wa.*123").unwrap();
    let start = Point::new(1, 0);
    let end = Point::new(4, 2);
    let match_start = Point::new(1, 0);
    let match_end = Point::new(2, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn regex_left() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        testing66\r\n\
        Warp\n\
        123\r\n\
        Warp\r\n\
        123\
    ");

    // Check regex across wrapped and unwrapped lines.
    let dfas = RegexDFAs::new("Wa.*123").unwrap();
    let start = Point::new(4, 2);
    let end = Point::new(1, 0);
    let match_start = Point::new(1, 0);
    let match_end = Point::new(2, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn nested_regex() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        Wa -> Warp -> rp\r\n\
        rp\
    ");

    // Greedy stopped at linebreak.
    let dfas = RegexDFAs::new("Wa.*rp").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 15);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );

    // Greedy stopped at dead state.
    let dfas = RegexDFAs::new("Wa[^y]*rp").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 9);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );
}

#[test]
fn regex_word_boundary() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        echo foo-bar foobar foo
    ");

    // Greedy stopped at linebreak.
    let dfas = RegexDFAs::new("\\bfoo\\b").unwrap();
    let end = Point::new(0, 22);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, Point::new(0, 0), end),
        Some(Point::new(0, 5)..=Point::new(0, 7))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, Point::new(0, 8), end),
        Some(Point::new(0, 20)..=Point::new(0, 22))
    );
}

#[test]
fn no_match_right() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        first line\n\
        broken second\r\n\
        third\
    ");

    let dfas = RegexDFAs::new("nothing").unwrap();
    let start = Point::new(2, 0);
    let end = Point::new(0, 4);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        None
    );
}

#[test]
fn no_match_left() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        first line\n\
        broken second\r\n\
        third\
    ");

    let dfas = RegexDFAs::new("nothing").unwrap();
    let start = Point::new(0, 4);
    let end = Point::new(2, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        None
    );
}

#[test]
fn include_linebreak_left() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        testing123\r\n\
        xxx\
    ");

    // Make sure the cell containing the linebreak is not skipped.
    let dfas = RegexDFAs::new("te.*123").unwrap();
    let start = Point::new(1, 0);
    let end = Point::new(0, 0);
    let match_start = Point::new(0, 0);
    let match_end = Point::new(0, 9);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn include_linebreak_right() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        xxx\r\n\
        testing123\
    ");

    // Make sure the cell containing the linebreak is not skipped.
    let dfas = RegexDFAs::new("te.*123").unwrap();
    let start = Point::new(0, 2);
    let end = Point::new(1, 9);
    let match_start = Point::new(1, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(match_start..=end)
    );
}

#[test]
fn skip_dead_cell() {
    let blockgrid = mock_blockgrid("hellooo");

    // Make sure dead state cell is skipped when reversing.
    let dfas = RegexDFAs::new("hello").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 4);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );
}

#[test]
fn reverse_search_dead_recovery() {
    let blockgrid = mock_blockgrid("zooo lense");

    // Make sure the reverse DFA operates the same as a forward DFA.
    let dfas = RegexDFAs::new("zoo").unwrap();
    let start = Point::new(0, 9);
    let end = Point::new(0, 0);
    let match_start = Point::new(0, 0);
    let match_end = Point::new(0, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn multibyte_unicode() {
    let blockgrid = mock_blockgrid("testвосибing");

    let dfas = RegexDFAs::new("te.*ing").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 11);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );

    let dfas = RegexDFAs::new("te.*ing").unwrap();
    let start = Point::new(0, 11);
    let end = Point::new(0, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(end..=start)
    );
}

#[test]
fn double_width_chars() {
    let blockgrid = mock_blockgrid("大家好😀");

    let mut dfas = RegexDFAs::new("大家好").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 7);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(Point::new(0, 0)..=Point::new(0, 5))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, end, start),
        Some(Point::new(0, 0)..=Point::new(0, 5))
    );

    dfas = RegexDFAs::new("😀").unwrap();
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(Point::new(0, 6)..=Point::new(0, 7))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, end, start),
        Some(Point::new(0, 6)..=Point::new(0, 7))
    );

    dfas = RegexDFAs::new("大").unwrap();
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(Point::new(0, 0)..=Point::new(0, 1))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, end, start),
        Some(Point::new(0, 0)..=Point::new(0, 1))
    );
}

#[test]
fn wrapping() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        xxx\r\n\
        xxx\
    ");

    let dfas = RegexDFAs::new("xxx").unwrap();
    let start = Point::new(0, 2);
    let end = Point::new(1, 2);
    let match_start = Point::new(1, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(match_start..=end)
    );

    let dfas = RegexDFAs::new("xxx").unwrap();
    let start = Point::new(1, 0);
    let end = Point::new(0, 0);
    let match_end = Point::new(0, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(end..=match_end)
    );
}

#[test]
fn test_basic_input_and_newline() {
    let size = SizeInfo::new_without_font_metrics(5, 5);

    let mut grid = GridHandler::new(
        size,
        MAX_SCROLL_LIMIT,
        ChannelEventListener::new_for_test(),
        false,
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );
    grid.input('a');
    grid.linefeed();
    grid.carriage_return();
    grid.input('b');

    assert_eq!(
        grid.bounds_to_string(
            Point::new(0, 0),
            Point::new(4, 4),
            true,
            RespectObfuscatedSecrets::No,
            false, /*  */
            RespectDisplayedOutput::No
        ),
        "a\r\nb\r\n\r\n\r\n\r\n"
    );
}

#[test]
fn test_empty_grid_bounds_to_string() {
    let size = SizeInfo::new_without_font_metrics(0, 0);

    let grid_handler = GridHandler::new(
        size,
        MAX_SCROLL_LIMIT,
        ChannelEventListener::new_for_test(),
        false,
        ObfuscateSecrets::No,
        PerformResetGridChecks::No,
    );
    assert_eq!(
        grid_handler.bounds_to_string(
            Point::new(0, 0),
            Point::new(0, 0),
            true,
            RespectObfuscatedSecrets::No,
            false,
            RespectDisplayedOutput::No
        ),
        ""
    );
}

#[test]
fn test_semantic_search() {
    let blockgrid =
        mock_blockgrid("/usr/local/bin\r\nword_with_underscores\r\nвосибing\r\nsome«quotes»");

    // Left search
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(0, 7), is_default_word_boundary),
        Point::new(0, 5)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(1, 7), is_default_word_boundary),
        Point::new(1, 0)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(2, 4), is_default_word_boundary),
        Point::new(2, 0)
    );

    // Right search
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_right(Point::new(0, 7), is_default_word_boundary),
        Point::new(0, 9)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(1, 20), is_default_word_boundary),
        Point::new(1, 0)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(2, 7), is_default_word_boundary),
        Point::new(2, 0)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(3, 9), is_default_word_boundary),
        Point::new(3, 5)
    );
}

#[test]
fn test_line_to_fragments() {
    let blockgrid = mock_blockgrid("ab,c\\восиб file link\r\nnext line");
    assert_eq!(
        blockgrid
            .grid_handler
            .line_to_fragments(0, 0..8, IncludeFirstWideChar::Yes),
        vec![
            Fragment {
                content: "ab".to_string(),
                total_cell_width: 2,
            },
            Fragment {
                content: ",".to_string(),
                total_cell_width: 1,
            },
            Fragment {
                content: "c".to_string(),
                total_cell_width: 1,
            },
            Fragment {
                content: "\\".to_string(),
                total_cell_width: 1,
            },
            Fragment {
                content: "воси".to_string(),
                total_cell_width: 4,
            }
        ]
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .line_to_fragments(0, 9..10, IncludeFirstWideChar::No),
        vec![
            Fragment {
                content: "б".to_string(),
                total_cell_width: 1,
            },
            Fragment {
                content: " ".to_string(),
                total_cell_width: 1,
            },
        ]
    );
}

#[test]
fn test_secrets_serialization() {
    let mut blockgrid = mock_blockgrid("foo zach@warp.dev bar");
    blockgrid.maybe_enable_secret_obfuscation(ObfuscateSecrets::Yes);
    blockgrid.grid_handler_mut().mark_secret_range(
        Point::new(0, 4)..=Point::new(0, 16),
        IsObfuscated::Yes,
        "zach@warp.dev".to_string(),
        SecretLevel::User,
    );

    assert_eq!(
        "foo ************* bar",
        blockgrid.grid_handler.bounds_to_string(
            Point::new(0, 0),
            Point::new(0, 21),
            false,
            RespectObfuscatedSecrets::Yes,
            false, /* force_secrets_obfuscated */
            RespectDisplayedOutput::No
        )
    );

    let (handle, _) = blockgrid
        .grid_handler()
        .secret_at_displayed_point(Point::new(0, 4))
        .expect("handle should be defined");

    blockgrid
        .grid_handler_mut()
        .unobfuscate_secret(handle)
        .expect("should unobfuscate secret");

    assert_eq!(
        "foo zach@warp.dev bar",
        blockgrid.grid_handler.bounds_to_string(
            Point::new(0, 0),
            Point::new(0, 21),
            false,
            RespectObfuscatedSecrets::Yes,
            false, /* force_secrets_obfuscated */
            RespectDisplayedOutput::No
        )
    );
}

#[test]
fn test_finds_url_in_grid() {
    // Test url in one line.
    let blockgrid = mock_blockgrid("https://google.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 17 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 17 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 17 },
            is_empty: false
        })
    );

    // Test url with some other texts.
    let blockgrid = mock_blockgrid("abc https://google.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        None
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 21 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_line_wrapping() {
    let blockgrid = mock_blockgrid("abc https://goog\nle.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 1, col: 5 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 15 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 1, col: 5 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_with_delimiter() {
    let blockgrid = mock_blockgrid("https://google.com'");
    // Should not include the trailing delimiter.
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 17 },
            is_empty: false
        })
    );

    let blockgrid = mock_blockgrid("https://google.com/search?q=warp");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 31 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_line_breaks() {
    let blockgrid = mock_blockgrid("abc https://goog\r\nle.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 0 }),
        None
    );
}

#[test]
fn test_find_url_wide_characters() {
    let blockgrid = mock_blockgrid("https://google.com/啊啊啊啊");
    assert!(has_wide_char_character(&blockgrid));
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 5 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 25 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_omits_trailing_periods() {
    // Test that it omits a single trailing period.
    let blockgrid = mock_blockgrid("Visit https://github.com/warpdotdev/Warp/issues.");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 46 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 47 }),
        None
    );

    // Test that it omits multiple trailing periods.
    let blockgrid = mock_blockgrid("Visit https://github.com/warpdotdev/Warp/issues...");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 46 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 48 }),
        None
    );

    // Test that it handles a period in the middle of the URL path somewhere.
    let blockgrid = mock_blockgrid("Visit https://github.com/warp.dev/Warp/issues.");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 44 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 33 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 44 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_with_percent() {
    let blockgrid = mock_blockgrid("url with percent https://example.com/search?q=hello%20world");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 55 }),
        Some(Link {
            range: Point { row: 0, col: 17 }..=Point { row: 0, col: 58 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_long_url() {
    let blockgrid = mock_blockgrid(
        "\
this is long https://very-long-subdomain-name-123456789.another-long-subdomain-p\n\
art-123456789.example.com/this/is/a/very/long/path/with/many/segments/and/we/nee\n\
d/to/make/it/longer/still/so/adding/more/segments/here/and/there/plus/some/query\n\
/parameters?user=john&id=123456789&token=abcdef",
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 3, col: 14 }),
        Some(Link {
            range: Point { row: 0, col: 13 }..=Point { row: 3, col: 46 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_super_long_url() {
    let blockgrid = mock_blockgrid(
        "\
this is long https://very-long-subdomain-name-123456789.another-long-subdomain-p\n\
art-123456789.example.com/this/is/a/very/long/path/with/many/segments/and/we/nee\n\
d/to/make/it/longer/still/so/adding/more/segments/here/and/there/plus/some/query\n\
/parameters?user=john&id=123456789&token=abcdefsdfdsfpiojdsfpojsdfpojsdfpojsdfpo\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsqwertyuioplkj and then it ends and there\n\
s some more content over here\n\
",
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 12, col: 52 }),
        Some(Link {
            range: Point { row: 0, col: 13 }..=Point { row: 12, col: 52 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_super_duper_long_url() {
    let blockgrid = mock_blockgrid(
        "\
this is long https://very-long-subdomain-name-123456789.another-long-subdomain-p\n\
art-123456789.example.com/this/is/a/very/long/path/with/many/segments/and/we/nee\n\
d/to/make/it/longer/still/so/adding/more/segments/here/and/there/plus/some/query\n\
/parameters?user=john&id=123456789&token=abcdefsdfdsfpiojdsfpojsdfpojsdfpojsdfpo\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsqwertyuioplkj and then it ends and there\n\
s some more content over here\n\
",
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 40 }),
        None,
    );
}

#[test]
fn test_possible_file_paths() {
    let blockgrid = mock_blockgrid("file link: src/восиб,abcвосиб");
    let valid_path = vec![
        PossiblePath {
            path: CleanPathResult {
                path: "file link: src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "file link: src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "file link: src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " link: src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " link: src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " link: src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "link: src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 5 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "link: src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 5 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "link: src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 5 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: ": src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 9 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: ": src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 9 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: ": src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 9 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 10 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 10 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 10 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 11 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 11 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 11 }..=Point { row: 0, col: 19 },
        },
    ];

    // Hover on "s" in src/восиб.
    assert_eq!(
        blockgrid
            .grid_handler
            .possible_file_paths_at_point(Point { row: 0, col: 11 }),
        valid_path
    );

    // Hover on a single "." in one line.
    let blockgrid = mock_blockgrid(".");
    assert_eq!(
        blockgrid
            .grid_handler
            .possible_file_paths_at_point(Point { row: 0, col: 0 }),
        vec![PossiblePath {
            path: CleanPathResult {
                path: ".".into(),
                line_and_column_num: None
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 0 }
        }]
    );

    // Hover on a file path with line and column numbers.
    let blockgrid = mock_blockgrid("восиб:100");
    assert_eq!(
        blockgrid
            .grid_handler
            .possible_file_paths_at_point(Point { row: 0, col: 6 }),
        vec![
            PossiblePath {
                path: CleanPathResult {
                    path: "восиб".into(),
                    line_and_column_num: Some(LineAndColumnArg {
                        line_num: 100,
                        column_num: None
                    })
                },
                range: Point { row: 0, col: 0 }..=Point { row: 0, col: 8 }
            },
            PossiblePath {
                path: CleanPathResult {
                    path: "".into(),
                    line_and_column_num: Some(LineAndColumnArg {
                        line_num: 100,
                        column_num: None
                    })
                },
                range: Point { row: 0, col: 5 }..=Point { row: 0, col: 8 }
            },
            PossiblePath {
                path: CleanPathResult {
                    path: "100".into(),
                    line_and_column_num: None
                },
                range: Point { row: 0, col: 6 }..=Point { row: 0, col: 8 }
            },
        ]
    );
}

#[test]
fn test_fragment_boundary_at_point() {
    let assert_fragment_boundary =
        |blockgrid: &BlockGrid,
         (cursor_row, cursor_col),
         expected_fragment: Range<(usize, usize)>| {
            let cursor_point = Point {
                row: cursor_row,
                col: cursor_col,
            };
            let expected_fragment_start = Point {
                row: expected_fragment.start.0,
                col: expected_fragment.start.1,
            };
            let expected_fragment_end = Point {
                row: expected_fragment.end.0,
                col: expected_fragment.end.1,
            };
            let actual_fragment = blockgrid
                .grid_handler
                .fragment_boundary_at_point(&cursor_point)
                .0;
            assert_eq!(
                actual_fragment,
                expected_fragment_start..expected_fragment_end
            );
        };

    let blockgrid = mock_blockgrid("/abc\\text gef");
    assert_fragment_boundary(&blockgrid, (0, 2), (0, 0)..(0, 4)); // Should give boundary of fragment "/abc"
    assert_fragment_boundary(&blockgrid, (0, 4), (0, 4)..(0, 5)); // Should give boundary of fragment "\"
    assert_fragment_boundary(&blockgrid, (0, 7), (0, 5)..(0, 9)); // Should give boundary of fragment "text"
    assert_fragment_boundary(&blockgrid, (0, 10), (0, 10)..(0, 13)); // Should give boundary of fragment "gef"

    // If hovering over the line length boundary, we should return the boundary
    // of the empty fragment from end of line to the max column length.
    assert_fragment_boundary(&blockgrid, (0, 13), (0, 13)..(0, 13));

    let blockgrid = mock_blockgrid("line one\r\nline two");
    assert_fragment_boundary(&blockgrid, (0, 6), (0, 5)..(0, 8)); // Should give boundary of fragment "one"
    assert_fragment_boundary(&blockgrid, (1, 3), (1, 0)..(1, 4)); // Should give boundary of fragment "line"

    // The `ls` command performs indenting via a tab '\t' followed by a series of null characters '\0'
    let blockgrid = mock_blockgrid("migrations\t\0\0\0resources");
    assert_fragment_boundary(&blockgrid, (0, 5), (0, 0)..(0, 10)); // Should give boundary of fragment "migrations"
    assert_fragment_boundary(&blockgrid, (0, 10), (0, 10)..(0, 11)); // Should give boundary of fragment "\t"
    assert_fragment_boundary(&blockgrid, (0, 13), (0, 13)..(0, 14)); // Should give boundary of fragment "\0"
    assert_fragment_boundary(&blockgrid, (0, 16), (0, 14)..(0, 23)); // Should give boundary of fragment "resources"
}

#[test]
fn test_serializing_string() {
    // a
    let a = Cell::from('a');
    assert_eq!(GridHandler::cell_to_string(&a), "a");

    // b, red foreground
    let mut b = Cell::from('b');
    b.fg = Color::Named(NamedColor::Red);
    assert_eq!(GridHandler::cell_to_string(&b), "\x1b[31mb\x1b[0m");

    // c, blue foreground, white background
    let mut c = Cell::from('c');
    c.fg = Color::Named(NamedColor::Blue);
    c.bg = Color::Named(NamedColor::White);
    assert_eq!(GridHandler::cell_to_string(&c), "\x1b[34;47mc\x1b[0m");

    // d, bold
    let mut d = Cell::from('d');
    d.flags = Flags::BOLD;
    assert_eq!(GridHandler::cell_to_string(&d), "\x1b[1md\x1b[0m");

    // e, italic, underline, strike
    let mut e = Cell::from('e');
    e.flags = Flags::ITALIC | Flags::UNDERLINE | Flags::STRIKEOUT;
    assert_eq!(GridHandler::cell_to_string(&e), "\x1b[3;4;9me\x1b[0m");

    // f, italic, green background
    let mut f = Cell::from('f');
    f.bg = Color::Named(NamedColor::Green);
    f.flags = Flags::ITALIC;
    assert_eq!(GridHandler::cell_to_string(&f), "\x1b[42;3mf\x1b[0m");

    // g, yellow background, white foreground, underline, strike
    let mut g = Cell::from('g');
    g.bg = Color::Named(NamedColor::Yellow);
    g.fg = Color::Named(NamedColor::White);
    g.flags = Flags::UNDERLINE | Flags::STRIKEOUT;
    assert_eq!(GridHandler::cell_to_string(&g), "\x1b[37;43;4;9mg\x1b[0m");

    // h, named fg, 256-color bg
    let mut h = Cell::from('h');
    h.bg = Color::Indexed(196);
    h.fg = Color::Named(NamedColor::Cyan);
    assert_eq!(GridHandler::cell_to_string(&h), "\x1b[36;48;5;196mh\x1b[0m");

    // i, true-color fg, named bg
    let mut i = Cell::from('i');
    i.bg = Color::Named(NamedColor::Magenta);
    i.fg = Color::Spec(ColorU::new(24, 143, 3, 255));
    assert_eq!(
        GridHandler::cell_to_string(&i),
        "\x1b[38;2;24;143;3;45mi\x1b[0m"
    );

    // j, 256-color fg, true-color bg
    let mut j = Cell::from('j');
    j.bg = Color::Spec(ColorU::new(130, 32, 10, 255));
    j.fg = Color::Indexed(67);
    assert_eq!(
        GridHandler::cell_to_string(&j),
        "\x1b[38;5;67;48;2;130;32;10mj\x1b[0m"
    );

    // k, default fg and bg
    let mut k = Cell::from('k');
    k.bg = Color::Named(NamedColor::Background);
    k.fg = Color::Named(NamedColor::Foreground);
    assert_eq!(GridHandler::cell_to_string(&k), "k");

    // l, bright colors
    let mut l = Cell::from('l');
    l.bg = Color::Named(NamedColor::BrightBlue);
    l.fg = Color::Named(NamedColor::BrightRed);
    assert_eq!(GridHandler::cell_to_string(&l), "\x1b[91;104ml\x1b[0m");

    // m, handles an exceptional case with impossible colors
    let mut m = Cell::from('m');
    // background color should never be NamedColor::Foreground, and vice versa
    m.bg = Color::Named(NamedColor::Foreground);
    m.fg = Color::Named(NamedColor::Background);
    // should not crash. should fall back to default codes for fg/bg
    assert_eq!(GridHandler::cell_to_string(&m), "\x1b[39;49mm\x1b[0m");

    // n, dim colors preserved
    let mut n = Cell::from('n');
    n.fg = Color::Named(NamedColor::Red);
    n.flags = Flags::DIM;
    assert_eq!(GridHandler::cell_to_string(&n), "\x1b[31;2mn\x1b[0m");
}

#[test]
fn test_split_block_grid() {
    // Sanity check splitting a block grid works correctly.
    // Note that we have more comprehensive tests at the Grid level for splitting as well.
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        aaaa\r\n\
        bbbb\n\
        cccccc\r\n\
        dd\r\n\
        eeeeeeee\
    ");

    let (top_grid, bottom_grid) =
        blockgrid.split(NonZeroUsize::new(2).expect("should not be zero"));
    let bottom_grid = bottom_grid.expect("Expected Some() response from splitting blockgrid!");

    let g1 = top_grid.grid_storage();
    let g2 = bottom_grid.grid_storage();

    // Check row counts
    assert_eq!(g1.total_rows(), 2);
    assert_eq!(g1.visible_rows(), 2);
    assert_eq!(g1.raw.len(), 2);

    assert_eq!(g2.total_rows(), 3);
    assert_eq!(g2.visible_rows(), 3);
    assert_eq!(g2.raw.len(), 3);

    // Verify character content for grid1 and grid2
    assert_eq!(g1[0][0].c, 'a');
    assert_eq!(g1[0][1].c, 'a');
    assert_eq!(g1[1][0].c, 'b');

    assert_eq!(g2[0][0].c, 'c');
    assert_eq!(g2[0][1].c, 'c');
    assert_eq!(g2[1][0].c, 'd');
    assert_eq!(g2[2][0].c, 'e');

    // Check if scroll_region got adjusted correctly
    // Note that these ranges are EXCLUSIVE of the last index (hence # of rows)
    assert_eq!(
        *top_grid.grid_handler.scroll_region(),
        VisibleRow(0)..VisibleRow(2)
    );
    assert_eq!(
        *bottom_grid.grid_handler.scroll_region(),
        VisibleRow(0)..VisibleRow(3)
    );
}

/// Tests whether we correctly handle emoji variation selectors (to turn 1-width character
/// into a 2-width character).
#[test]
fn test_emoji_variation_selector() {
    // Manually create BlockGrid so we can use input() to insert characters.
    let size = SizeInfo::new_without_font_metrics(2, 10);

    let mut blockgrid = BlockGrid::new(
        size,
        MAX_SCROLL_LIMIT,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    blockgrid.start();
    blockgrid.input('a');
    // ☁️ should be a wide character - it is \0x2601\0xFE0F (uses emoji variation selector from Unicode).
    // See https://www.unicode.org/reports/tr51/#def_emoji_presentation_selector.
    blockgrid.input('\u{2601}');
    blockgrid.input('\u{FE0F}');
    blockgrid.input('b');
    blockgrid.input('c');

    assert!(has_wide_char_character(&blockgrid));
    let grid = blockgrid.grid_storage();
    assert_eq!(grid[0][0].c, 'a');
    // Not this is only \0x2601, without the variation selector.
    assert_eq!(grid[0][1].c, '☁');
    // Assert that it is a String (has zerowidth characters).
    assert!(matches!(
        grid[0][1].content_for_display(),
        CharOrStr::Str(_)
    ));
    assert!(grid[0][1].flags.intersects(Flags::WIDE_CHAR));
    assert_eq!(grid[0][2].c, '\0');
    assert!(grid[0][2].flags.intersects(Flags::WIDE_CHAR_SPACER));
    assert_eq!(grid[0][3].c, 'b');
    assert_eq!(grid[0][4].c, 'c');
}

#[test]
pub fn test_grid_agnostic_point() {
    let mut grid = mock_blockgrid(
        "\
        This is a wrap.\r\n\
        Short line\r\n\
        Another long line.\r\n",
    );
    grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    let grid_handler = grid.grid_handler();

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 0, col: 0 });
    assert_eq!(grid_agnostic_point, Point { row: 0, col: 0 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 0, col: 9 });
    assert_eq!(grid_agnostic_point, Point { row: 0, col: 9 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 1, col: 0 });
    assert_eq!(grid_agnostic_point, Point { row: 0, col: 10 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 4, col: 0 });
    assert_eq!(grid_agnostic_point, Point { row: 2, col: 10 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 4, col: 7 });
    assert_eq!(grid_agnostic_point, Point { row: 2, col: 17 });
}

#[test]
pub fn test_compatible_point() {
    let mut grid = mock_blockgrid(
        "\
        This is a wrap.\r\n\
        Short line\r\n\
        Another long line.\r\n",
    );
    grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    let grid_handler = grid.grid_handler();

    let compatible_point = grid_handler.compatible_point(Point { row: 0, col: 0 });
    assert_eq!(compatible_point, Point { row: 0, col: 0 });

    let compatible_point = grid_handler.compatible_point(Point { row: 0, col: 11 });
    assert_eq!(compatible_point, Point { row: 1, col: 1 });

    let compatible_point = grid_handler.compatible_point(Point { row: 2, col: 10 });
    assert_eq!(compatible_point, Point { row: 4, col: 0 });
}

#[test]
fn multibyte_char_offset() {
    let blockgrid = mock_blockgrid(
        "\
        echo 汉字 yo\r\n\
        hi",
    );

    assert_eq!(
        blockgrid
            .grid_handler()
            .byte_offset_between_points(Point::new(0, 0), Point::new(1, 0)),
        14.into()
    );
}

#[test]
fn advance_point_by_bytes() {
    let blockgrid = mock_blockgrid(
        "\
        echo 汉字 yo\r\n\
        hi",
    );

    assert_eq!(
        blockgrid
            .grid_handler()
            .advance_point_by_bytes(Point::new(0, 0), ByteOffset::from(12)),
        Point::new(0, 10)
    );
}

#[test]
fn test_grid_bottommost_nonempty_row_partial_grid() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);

    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['\0', '\0', '\0', '\0', '\0', '\0'],
    ]);

    assert_eq!(grid.bottommost_nonempty_row(), Some(2));
}

#[test]
fn test_grid_bottommost_nonempty_row_full_grid() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);

    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['d', '\0', '\0', '\0', '\0', '\0'],
    ]);

    assert_eq!(grid.bottommost_nonempty_row(), Some(3));
}

#[test]
fn test_grid_bottommost_nonempty_row_empty_grid() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);

    grid.grid_storage_mut().populate_from_array(&[]);

    assert_eq!(grid.bottommost_nonempty_row(), None);
}

#[test]
fn test_grid_rightmost_visible_nonempty_cell_in_row() {
    let mut grid1 = GridHandler::new_for_test_with_scroll_limit(1, 6, 1);
    assert_eq!(grid1.rightmost_visible_nonempty_cell_in_row(0), None);

    grid1.grid_storage_mut()[0][0].c = 'a';
    assert_eq!(grid1.rightmost_visible_nonempty_cell_in_row(0), Some(0));

    // An explicit space (as opposed to '\0') should not be considered a visible, non-empty cell.
    grid1.grid_storage_mut()[0][1].c = ' ';
    assert_eq!(grid1.rightmost_visible_nonempty_cell_in_row(0), Some(0));

    grid1.grid_storage_mut()[0][1].c = 'b';
    assert_eq!(grid1.rightmost_visible_nonempty_cell_in_row(0), Some(1));

    grid1.grid_storage_mut()[0][2]
        .flags
        .bitor_assign(Flags::UNDERLINE);
    assert_eq!(grid1.rightmost_visible_nonempty_cell_in_row(0), Some(2));

    grid1.grid_storage_mut()[0][3].bg = Color::Named(NamedColor::Red);
    assert_eq!(grid1.rightmost_visible_nonempty_cell_in_row(0), Some(3));

    let mut grid2 = GridHandler::new_for_test_with_scroll_limit(6, 6, 6);
    grid2.grid_storage_mut()[0][3].c = 'a';
    grid2.grid_storage_mut()[1][1].c = 'b';
    grid2.grid_storage_mut()[3][4].c = 'c';
    grid2.grid_storage_mut()[3][2].c = 'd';
    grid2.grid_storage_mut()[4][0].c = 'e';
    grid2.grid_storage_mut()[5][5].c = '\t';
    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(3), Some(4));

    grid2.grid_storage_mut()[5][5].c = 'f';
    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(5), Some(5));

    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(4), Some(0));
    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(3), Some(4));
    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(2), None);
    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(1), Some(1));
    assert_eq!(grid2.rightmost_visible_nonempty_cell_in_row(0), Some(3));
}

#[test]
fn test_grid_rightmost_nonempty_cell() {
    let mut grid1 = GridHandler::new_for_test_with_scroll_limit(1, 6, 1);
    assert_eq!(grid1.rightmost_nonempty_cell(None), None);

    grid1.grid_storage_mut()[0][0].c = 'a';
    assert_eq!(grid1.rightmost_nonempty_cell(None), Some(0));

    // An explicit space (as opposed to '\0') should be not considered a visible, non-empty cell.
    grid1.grid_storage_mut()[0][1].c = ' ';
    assert_eq!(grid1.rightmost_nonempty_cell(None), Some(0));

    grid1.grid_storage_mut()[0][1].c = 'b';
    assert_eq!(grid1.rightmost_nonempty_cell(None), Some(1));

    grid1.grid_storage_mut()[0][2]
        .flags
        .bitor_assign(Flags::UNDERLINE);
    assert_eq!(grid1.rightmost_nonempty_cell(None), Some(2));

    grid1.grid_storage_mut()[0][3].bg = Color::Named(NamedColor::Red);
    assert_eq!(grid1.rightmost_nonempty_cell(None), Some(3));

    let mut grid2 = GridHandler::new_for_test_with_scroll_limit(6, 6, 6);
    grid2.grid_storage_mut()[0][3].c = 'a';
    grid2.grid_storage_mut()[1][1].c = 'b';
    grid2.grid_storage_mut()[3][4].c = 'c';
    grid2.grid_storage_mut()[3][2].c = 'd';
    grid2.grid_storage_mut()[4][0].c = 'e';
    grid2.grid_storage_mut()[5][5].c = '\t';
    assert_eq!(grid2.rightmost_nonempty_cell(None), Some(4));

    grid2.grid_storage_mut()[5][5].c = 'f';
    assert_eq!(grid2.rightmost_nonempty_cell(None), Some(5));

    // Testing `max_row` (last row it looks at).
    assert_eq!(grid2.rightmost_nonempty_cell(Some(5)), Some(5));
    assert_eq!(grid2.rightmost_nonempty_cell(Some(4)), Some(4));
    assert_eq!(grid2.rightmost_nonempty_cell(Some(3)), Some(4));
    assert_eq!(grid2.rightmost_nonempty_cell(Some(2)), Some(3));
    assert_eq!(grid2.rightmost_nonempty_cell(Some(1)), Some(3));
    assert_eq!(grid2.rightmost_nonempty_cell(Some(0)), Some(3));
}

/// Asserts that no orphaned WIDE_CHAR or WIDE_CHAR_SPACER flags exist in
/// the visible row of a GridHandler.
fn assert_no_orphaned_wide_chars(grid: &GridHandler, visible_row: VisibleRow) {
    let num_cols = grid.columns();
    let row = &grid.grid_storage()[visible_row];
    for col in 0..num_cols {
        if row[col].flags.contains(Flags::WIDE_CHAR) {
            assert!(
                col + 1 < num_cols && row[col + 1].flags.contains(Flags::WIDE_CHAR_SPACER),
                "Orphaned WIDE_CHAR at column {col}: next cell is not a WIDE_CHAR_SPACER."
            );
        }
        if row[col].flags.contains(Flags::WIDE_CHAR_SPACER) {
            assert!(
                col > 0 && row[col - 1].flags.contains(Flags::WIDE_CHAR),
                "Orphaned WIDE_CHAR_SPACER at column {col}: previous cell is not a WIDE_CHAR."
            );
        }
    }
}

#[test]
fn test_input_overwriting_wide_char_spacer_resets_wide_char() {
    // Input a wide char at col 0-1, move cursor to col 1 (the spacer),
    // then input a narrow char.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}'); // Cols 0 (WIDE_CHAR) and 1 (SPACER).
    grid.goto(VisibleRow(0), 1);
    grid.input('x');

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
    assert_eq!(grid.grid_storage()[VisibleRow(0)][1].c, 'x');
}

#[test]
fn test_input_overwriting_wide_char_resets_spacer() {
    // Input a wide char at col 0-1, move cursor back to col 0 (the
    // WIDE_CHAR), then input a narrow char.  The spacer at col 1 must be
    // cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.goto(VisibleRow(0), 0);
    grid.input('y');

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert_eq!(grid.grid_storage()[VisibleRow(0)][0].c, 'y');
    assert!(!grid.grid_storage()[VisibleRow(0)][1]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER));
}

#[test]
fn test_erase_chars_at_wide_char_spacer_boundary() {
    // Input a wide char at col 0-1, then move cursor to col 1 (the spacer)
    // and erase one char.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.erase_chars(1);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
}

#[test]
fn test_erase_chars_at_wide_char_end_boundary() {
    // Input narrow chars then a wide char at col 2-3, then erase from col 1
    // to col 3.  The end of the erase range falls on the spacer at col 3,
    // so the WIDE_CHAR at col 2 must also be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 1);
    grid.erase_chars(2); // Erases cols 1..3.

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
}

#[test]
fn test_delete_chars_at_wide_char_spacer_boundary() {
    // Input a wide char at col 0-1, then move cursor to col 1 (spacer)
    // and delete one char.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.delete_chars(1);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
}

#[test]
fn test_insert_blank_at_wide_char_spacer_boundary() {
    // Input a wide char at col 0-1, move cursor to col 1 (spacer), and
    // insert a blank.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.insert_blank(1);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
}

#[test]
fn test_clear_line_right_at_wide_char_spacer() {
    // Input a wide char at col 0-1, move cursor to col 1 (spacer), and
    // clear line right.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.clear_line(ansi::LineClearMode::Right);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
}

#[test]
fn test_clear_line_left_at_wide_char() {
    // Input narrow chars then a wide char at col 2-3, move cursor to col 2
    // (the WIDE_CHAR), and clear line left.  The spacer at col 3 must be
    // cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 2);
    grid.clear_line(ansi::LineClearMode::Left);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][3]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER));
}

#[test]
fn test_clear_screen_below_at_wide_char_spacer() {
    // Input a wide char at col 0-1, move cursor to col 1 (spacer), and
    // clear screen below.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.clear_screen(ansi::ClearMode::Below);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
}

#[test]
fn test_clear_screen_above_at_wide_char() {
    // Input narrow chars then a wide char at col 2-3, move cursor to col 2
    // (the WIDE_CHAR), and clear screen above.  The spacer at col 3 must
    // be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 2);
    grid.clear_screen(ansi::ClearMode::Above);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][3]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER));
}

fn assert_visible_grid_blank(grid: &GridHandler) {
    for row in 0..grid.visible_rows() {
        for col in 0..grid.columns() {
            assert_eq!(grid.grid_storage()[VisibleRow(row)][col].c, '\0');
        }
    }
}

fn write_two_visible_rows(grid: &mut GridHandler) {
    grid.input_at_cursor("abc");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("def");
}

#[test]
fn test_clear_screen_all_primary_preserves_visible_rows_in_history_by_default() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 5, MAX_SCROLL_LIMIT);
    write_two_visible_rows(&mut grid);

    grid.clear_screen(ansi::ClearMode::All);

    assert!(grid.history_size() > 0);
    assert_visible_grid_blank(&grid);
}

#[test]
fn test_clear_screen_all_primary_with_full_grid_clear_behavior_clears_in_place() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 5, MAX_SCROLL_LIMIT);
    write_two_visible_rows(&mut grid);
    grid.enable_full_grid_clear_behavior();

    grid.clear_screen(ansi::ClearMode::All);

    assert_eq!(grid.history_size(), 0);
    assert_visible_grid_blank(&grid);
}

#[test]
fn test_clear_screen_all_alt_screen_clears_in_place() {
    let mut grid = GridHandler::new_for_alt_screen_test(3, 5);
    write_two_visible_rows(&mut grid);

    grid.clear_screen(ansi::ClearMode::All);

    assert_eq!(grid.history_size(), 0);
    assert_visible_grid_blank(&grid);
}

#[test]
fn test_resize_primary_preserves_visible_rows_in_history_by_default() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, MAX_SCROLL_LIMIT);
    grid.input_at_cursor("12345");

    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert!(grid.history_size() > 0);
}

#[test]
fn test_resize_primary_with_full_grid_clear_behavior_keeps_visible_rows_in_place() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, MAX_SCROLL_LIMIT);
    grid.input_at_cursor("12345");
    grid.enable_full_grid_clear_behavior();
    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert_eq!(grid.history_size(), 0);
}

#[test]
fn test_resize_finished_primary_with_full_grid_clear_behavior_uses_scrollback() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, MAX_SCROLL_LIMIT);
    grid.input_at_cursor("12345");
    grid.finish();
    grid.enable_full_grid_clear_behavior();

    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert!(grid.history_size() > 0);
}

#[test]
fn test_clear_line_left_preserves_adjacent_wide_char() {
    // Wide char at cols 2-3, cursor at col 1.  Clearing left should only
    // affect cols 0-1 and must not touch the wide char at cols 2-3.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 1);
    grid.clear_line(ansi::LineClearMode::Left);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    // The wide char at cols 2-3 must be preserved.
    assert!(
        grid.grid_storage()[VisibleRow(0)][2]
            .flags
            .contains(Flags::WIDE_CHAR),
        "WIDE_CHAR at col 2 was incorrectly cleared."
    );
    assert!(
        grid.grid_storage()[VisibleRow(0)][3]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER),
        "WIDE_CHAR_SPACER at col 3 was incorrectly cleared."
    );
}

#[test]
fn test_erase_chars_preserves_adjacent_wide_char_at_end() {
    // Wide char at cols 2-3, cursor at col 0.  Erasing 2 chars clears
    // cols 0-1 and must not touch the wide char at cols 2-3.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 0);
    grid.erase_chars(2);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    // The wide char at cols 2-3 must be preserved.
    assert!(
        grid.grid_storage()[VisibleRow(0)][2]
            .flags
            .contains(Flags::WIDE_CHAR),
        "WIDE_CHAR at col 2 was incorrectly cleared."
    );
    assert!(
        grid.grid_storage()[VisibleRow(0)][3]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER),
        "WIDE_CHAR_SPACER at col 3 was incorrectly cleared."
    );
}

#[test]
fn test_delete_chars_preserves_adjacent_wide_char_at_end() {
    // Wide char at cols 2-3, cursor at col 0.  Deleting 2 chars shifts
    // the wide char left to cols 0-1, which must remain intact.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 0);
    grid.delete_chars(2);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    // The wide char should have been shifted left to cols 0-1.
    assert!(
        grid.grid_storage()[VisibleRow(0)][0]
            .flags
            .contains(Flags::WIDE_CHAR),
        "WIDE_CHAR was not preserved after delete_chars shift."
    );
    assert!(
        grid.grid_storage()[VisibleRow(0)][1]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER),
        "WIDE_CHAR_SPACER was not preserved after delete_chars shift."
    );
}

#[test]
fn test_clear_screen_above_preserves_adjacent_wide_char() {
    // Wide char at cols 2-3, cursor at col 1.  Clearing above should only
    // affect cols 0-1 on the cursor row and must not touch cols 2-3.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 1);
    grid.clear_screen(ansi::ClearMode::Above);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    // The wide char at cols 2-3 must be preserved.
    assert!(
        grid.grid_storage()[VisibleRow(0)][2]
            .flags
            .contains(Flags::WIDE_CHAR),
        "WIDE_CHAR at col 2 was incorrectly cleared."
    );
    assert!(
        grid.grid_storage()[VisibleRow(0)][3]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER),
        "WIDE_CHAR_SPACER at col 3 was incorrectly cleared."
    );
}

#[test]
fn test_insert_mode_at_spacer_resets_wide_char() {
    // Wide char at cols 0-1, cursor at col 1 (spacer), INSERT mode.
    // The INSERT mode shift moves the spacer away before write_at_cursor
    // runs, so the start boundary must be checked before the shift.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}'); // Cols 0 (WIDE_CHAR), 1 (SPACER).
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.set_mode(ansi::Mode::Insert);
    grid.input('x');

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
}

#[test]
fn test_insert_mode_pushes_wide_char_off_end() {
    // Wide char at cols 8-9 in a 10-column row.  Inserting a character
    // at col 0 in INSERT mode shifts cells right, pushing the spacer
    // off the end and orphaning the WIDE_CHAR.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('c');
    grid.input('d');
    grid.input('e');
    grid.input('f');
    grid.input('g');
    grid.input('h');
    grid.input('\u{1F600}'); // Cols 8 (WIDE_CHAR), 9 (SPACER).
    grid.goto(VisibleRow(0), 0);
    grid.set_mode(ansi::Mode::Insert);
    grid.input('x');

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
}

#[test]
fn test_write_at_cursor_clears_leading_wide_char_spacer() {
    // Fill a 6-column row up to col 4, then write a wide char that does
    // not fit.  This places a LEADING_WIDE_CHAR_SPACER at the last column
    // of row 0 and wraps the wide char to row 1 cols 0-1.  Overwriting
    // col 0 on row 1 must clear the stale LEADING_WIDE_CHAR_SPACER on
    // the previous row.
    let mut grid = GridHandler::new_for_test(5, 6);
    grid.input('a');
    grid.input('b');
    grid.input('c');
    grid.input('d');
    grid.input('e');
    // Cursor at col 5 (last column).  Wide char does not fit.
    grid.input('\u{1F600}');

    // Verify the setup is correct.
    assert!(
        grid.grid_storage()[VisibleRow(0)][5]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER),
        "Expected LEADING_WIDE_CHAR_SPACER at row 0 col 5."
    );
    assert!(
        grid.grid_storage()[VisibleRow(1)][0]
            .flags
            .contains(Flags::WIDE_CHAR),
        "Expected WIDE_CHAR at row 1 col 0."
    );

    // Move cursor back to row 1 col 0 and overwrite the wide char.
    grid.goto(VisibleRow(1), 0);
    grid.input('x');

    // The LEADING_WIDE_CHAR_SPACER on row 0 col 5 must be cleared.
    assert!(
        !grid.grid_storage()[VisibleRow(0)][5]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER),
        "LEADING_WIDE_CHAR_SPACER was not cleared on previous row."
    );
    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert_no_orphaned_wide_chars(&grid, VisibleRow(1));
}

#[test]
fn test_write_at_cursor_clears_leading_wide_char_spacer_at_col_1() {
    // Same setup as test_write_at_cursor_clears_leading_wide_char_spacer,
    // but overwrite at col 1 (the spacer) instead of col 0.  The
    // LEADING_WIDE_CHAR_SPACER on the previous row must still be cleared.
    let mut grid = GridHandler::new_for_test(5, 6);
    grid.input('a');
    grid.input('b');
    grid.input('c');
    grid.input('d');
    grid.input('e');
    grid.input('\u{1F600}');

    grid.goto(VisibleRow(1), 1);
    grid.input('x');

    assert!(
        !grid.grid_storage()[VisibleRow(0)][5]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER),
        "LEADING_WIDE_CHAR_SPACER was not cleared when overwriting col 1."
    );
    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert_no_orphaned_wide_chars(&grid, VisibleRow(1));
}

#[test]
fn test_wide_char_wrap_preserves_own_leading_spacer() {
    // Wrapping a wide char must preserve the LEADING_WIDE_CHAR_SPACER it
    // just placed on the previous row.  The cleanup logic in
    // write_at_cursor must not fire during the normal wrapping flow.
    let mut grid = GridHandler::new_for_test(5, 6);
    grid.input('a');
    grid.input('b');
    grid.input('c');
    grid.input('d');
    grid.input('e');
    grid.input('\u{1F600}');

    assert!(
        grid.grid_storage()[VisibleRow(0)][5]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER),
        "LEADING_WIDE_CHAR_SPACER was incorrectly removed during wrap."
    );
    assert!(
        grid.grid_storage()[VisibleRow(1)][0]
            .flags
            .contains(Flags::WIDE_CHAR),
        "Wide char was not placed at row 1 col 0."
    );
    assert!(
        grid.grid_storage()[VisibleRow(1)][1]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER),
        "Wide char spacer was not placed at row 1 col 1."
    );
}

#[test]
fn test_insert_blank_resets_wide_char_pushed_off_end() {
    // Wide char at cols 5-6 in an 8-column row.  Inserting 2 blanks at
    // col 0 shifts cells right; the spacer at col 6 is pushed past the
    // row end, which would leave an orphaned WIDE_CHAR at col 7.
    let mut grid = GridHandler::new_for_test(5, 8);
    grid.input('a');
    grid.input('b');
    grid.input('c');
    grid.input('d');
    grid.input('e');
    grid.input('\u{1F600}'); // Cols 5 (WIDE_CHAR), 6 (SPACER).
    grid.input('g');
    grid.goto(VisibleRow(0), 0);
    grid.insert_blank(2);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
}

// ─── content_len / trailing blank row trimming ───────────────────────

#[test]
fn content_len_interspersed_blanks_preserved() {
    // Blank rows between content are interior, not trailing.
    // "line1\n\n\nline4" → content_len() == 4
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("line1");
    grid.carriage_return();
    grid.linefeed();
    // row 1 blank
    grid.linefeed();
    // row 2 blank
    grid.linefeed();
    grid.input_at_cursor("line4");
    assert_eq!(grid.content_len(), 4);
}

#[test]
fn content_len_trailing_blanks_trimmed() {
    // "line1\nline2" then cursor moved to row 4 via goto → content_len() == 2
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("line1");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("line2");
    // Move cursor below content via CUP (as real TUI apps do)
    grid.goto(VisibleRow(4), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid.content_len(), 2);
}

#[test]
fn visible_content_len_for_trimming_ignores_whitespace_rows() {
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("line1");
    grid.goto(VisibleRow(4), 0);
    grid.input_at_cursor("   ");
    grid.goto(VisibleRow(6), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(grid.visible_content_len_for_trimming(), Some(1));
    assert_eq!(grid.content_len(), 1);
}

#[test]
fn visible_content_len_for_trimming_ignores_style_only_rows() {
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("line1");
    grid.grid_storage_mut()[VisibleRow(4)][0].bg = Color::Named(NamedColor::Red);
    grid.grid_storage_mut()[VisibleRow(4)][1]
        .flags
        .bitor_assign(Flags::UNDERLINE);
    grid.goto(VisibleRow(6), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(grid.visible_content_len_for_trimming(), Some(1));
    assert_eq!(grid.content_len(), 1);
}

#[test]
fn visible_content_len_for_trimming_visible_glyph_after_blanks_restores_height() {
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("line1");
    grid.goto(VisibleRow(4), 0);
    grid.input_at_cursor("   ");
    grid.grid_storage_mut()[VisibleRow(5)][0].bg = Color::Named(NamedColor::Red);
    grid.grid_storage_mut()[VisibleRow(5)][1]
        .flags
        .bitor_assign(Flags::UNDERLINE);
    grid.goto(VisibleRow(6), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(grid.visible_content_len_for_trimming(), Some(1));

    grid.input_at_cursor("visible");

    assert_eq!(grid.visible_content_len_for_trimming(), Some(7));
    assert_eq!(grid.content_len(), 7);
}

#[test]
fn content_len_cursor_below_content() {
    // Content on rows 0-2, cursor moved to row 5 via goto → content_len() == 3
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("row0");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("row1");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("row2");
    // Move cursor below content via CUP (as real TUI apps do)
    grid.goto(VisibleRow(5), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid.content_len(), 3);
}

#[test]
fn content_len_new_output_restores_height() {
    // After trimming, new content grows content_len()
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("row0");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("row1");
    // Move cursor below content via CUP
    grid.goto(VisibleRow(4), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid.content_len(), 2);
    // Write on row 2
    grid.goto(VisibleRow(2), 0);
    grid.input_at_cursor("row2");
    assert_eq!(grid.content_len(), 3);
}

#[test]
fn content_len_single_trailing_newline() {
    // "line1" then cursor moved to row 1 via goto → content_len() == 1
    let mut grid = GridHandler::new_for_test(10, 10);
    grid.set_track_content_length(true);
    grid.input_at_cursor("line1");
    grid.goto(VisibleRow(1), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid.content_len(), 1);
}

#[test]
fn content_len_all_blank_grid() {
    // Grid started but no visible chars → content_len() falls back to
    // max_cursor_point-based length (no trimming to 0).
    let mut grid = GridHandler::new_for_test(10, 10);
    grid.set_track_content_length(true);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    let expected = grid.grid_storage().max_cursor_point.row.0 + grid.history_size() + 1;
    assert_eq!(grid.visible_content_len_for_trimming(), None);
    assert_eq!(grid.content_len(), expected);
}

#[test]
fn content_len_equals_len_when_no_trailing_blanks() {
    // No trailing blanks → content_len() == max_cursor-based length
    let mut grid = GridHandler::new_for_test(10, 10);
    grid.set_track_content_length(true);
    grid.input_at_cursor("abc");
    let expected = grid.grid_storage().max_cursor_point.row.0 + grid.history_size() + 1;
    assert_eq!(grid.content_len(), expected);
}

// ─── FullGridClearBehavior::Clear resize + scroll desync ─────────────

#[test]
fn test_full_grid_clear_resize_then_scroll_does_not_panic_on_row_iteration() {
    // Regression test for issue with `FullGridClearBehavior`: make sure that
    // when FullGridClearBehavior::Clear is active, resize_storage
    // resizes the active GridStorage and the flat storage correctly.
    // Before, we didn't set 'flat_storage.set_columns(), and subsequent scrolls pushed
    // wider rows into the narrower flat storage, corrupting the index. Iterating
    // those rows then panicked.
    let old_cols = 10;
    let new_cols = 20;
    let num_rows = 3;

    let mut grid =
        GridHandler::new_for_test_with_scroll_limit(num_rows, old_cols, MAX_SCROLL_LIMIT);
    grid.enable_full_grid_clear_behavior();

    // Resize grid wider.
    grid.resize(SizeInfo::new_without_font_metrics(num_rows, new_cols));

    // Fill visible rows with new-width content and trigger a scroll so
    // a wide row gets pushed into narrow flat storage.
    for _ in 0..num_rows {
        for c in "abcdefghijklmnopqrst".chars() {
            grid.input(c);
        }
        grid.carriage_return();
        grid.linefeed();
    }

    // Iterating flat storage rows should not panic.
    for row_idx in 0..grid.flat_storage.total_rows() {
        let _ = grid.flat_storage.rows_from(row_idx).next();
    }
}

#[test]
fn test_full_grid_clear_resize_narrower_then_scroll_does_not_panic() {
    // Same scenario but resizing to a narrower width.
    let old_cols = 20;
    let new_cols = 10;
    let num_rows = 3;

    let mut grid =
        GridHandler::new_for_test_with_scroll_limit(num_rows, old_cols, MAX_SCROLL_LIMIT);

    // Fill grid with wide content before enabling Clear behavior.
    for _ in 0..num_rows {
        for c in "abcdefghijklmnopqrst".chars() {
            grid.input(c);
        }
        grid.carriage_return();
        grid.linefeed();
    }

    grid.enable_full_grid_clear_behavior();
    grid.resize(SizeInfo::new_without_font_metrics(num_rows, new_cols));

    // Trigger more scrolling with narrow content.
    for _ in 0..num_rows {
        for c in "abcdefghij".chars() {
            grid.input(c);
        }
        grid.carriage_return();
        grid.linefeed();
    }

    for row_idx in 0..grid.flat_storage.total_rows() {
        let _ = grid.flat_storage.rows_from(row_idx).next();
    }
}

#[test]
fn test_full_grid_clear_resize_then_bounds_to_string_does_not_panic() {
    // End-to-end repro via the same code path as block_snapshot:
    // bounds_to_string → line_to_string → row() → RowIterator::next.
    let old_cols = 10;
    let new_cols = 20;
    let num_rows = 3;

    let mut grid =
        GridHandler::new_for_test_with_scroll_limit(num_rows, old_cols, MAX_SCROLL_LIMIT);
    grid.enable_full_grid_clear_behavior();
    grid.resize(SizeInfo::new_without_font_metrics(num_rows, new_cols));

    for _ in 0..num_rows {
        for c in "abcdefghijklmnopqrst".chars() {
            grid.input(c);
        }
        grid.carriage_return();
        grid.linefeed();
    }

    let total = grid.total_rows();
    if total > 0 {
        let _ = grid.bounds_to_string(
            Point::new(0, 0),
            Point::new(total - 1, grid.columns().saturating_sub(1)),
            false,
            RespectObfuscatedSecrets::No,
            false,
            RespectDisplayedOutput::No,
        );
    }
}
