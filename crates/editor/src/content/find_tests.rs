use std::iter;
use std::pin::pin;
use std::sync::Once;

use futures_lite::future;
use itertools::Itertools;
use rangemap::RangeSet;
use string_offset::CharOffset;
use sum_tree::SumTree;
use warpui_core::App;

use super::{Engine, Match, SearchConfig};
use crate::content::buffer::Buffer;
use crate::content::cursor::BufferSumTree;
use crate::content::text::{BufferBlockStyle, BufferText, IndentBehavior};

#[test]
fn test_search_inline_styles() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "The **first** word, last `word`",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.read(&app, |buffer, _| {
            assert_matches(
                buffer,
                &SearchConfig {
                    query: "t w",
                    case_sensitive: true,
                    regex: false,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(9, 12, "t w"), (20, 23, "t w")],
            );
        });
    });
}

#[test]
fn test_search_across_link() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "visit [our website](https://warp.dev) for more",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"visit[\w\s]+site",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(1, 18, "visit our website")],
            );
        });
    });
}

#[test]
fn test_search_longest_match() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "git pull && git log",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"git\s+[\w\-]+",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(1, 9, "git pull"), (13, 20, "git log")],
            );
        });
    });
}

#[test]
fn test_end_of_buffer() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "abc",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"[a-z]c",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(2, 4, "bc")],
            );

            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"c$",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(3, 4, "c")],
            );

            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"c\b",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(3, 4, "c")],
            );
        });
    });
}

#[test]
fn test_search_non_ascii() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "你好aaaaa\n再見你好",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            // ASCII literal search works regardless of the bug.
            assert_matches(
                buffer,
                &SearchConfig::new("a"),
                [
                    (3, 4, "a"),
                    (4, 5, "a"),
                    (5, 6, "a"),
                    (6, 7, "a"),
                    (7, 8, "a"),
                ],
            );

            // Multi-byte literal search must find every occurrence. Each CJK
            // character is one `CharOffset` but several UTF-8 bytes; the reverse
            // DFA pass that locates a match's start has to consume those bytes in
            // reverse order, which is what this regression guards.
            assert_matches(
                buffer,
                &SearchConfig::new("你好"),
                [(1, 3, "你好"), (11, 13, "你好")],
            );

            // A multi-byte query mixing scripts also works.
            assert_matches(buffer, &SearchConfig::new("好a"), [(2, 4, "好a")]);

            // Regex over multi-byte content keeps working.
            assert_matches(
                buffer,
                &SearchConfig::regex("你."),
                [(1, 3, "你好"), (11, 13, "你好")],
            );
        });
    });
}

#[test]
fn test_search_non_ascii_case_insensitive() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "CAFÉ café",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            // Case-insensitive matching over multi-byte characters (`É`/`é` are
            // two UTF-8 bytes) must find both the upper- and lower-case forms,
            // exercising the reverse-DFA byte order on case-folded input.
            assert_matches(
                buffer,
                &SearchConfig::new("café").with_case_sensitive(false),
                [(1, 5, "CAFÉ"), (6, 10, "café")],
            );
        });
    });
}

#[test]
fn test_word_boundaries() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "a cat\nlala\npizza\n***\n* A\n* B",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.debug(),
                "<text>a cat\\nlala\\npizza<hr><ul0>A<ul0>B<text>"
            );

            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"a\b",
                    case_sensitive: false,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(1, 2, "a"), (10, 11, "a"), (16, 17, "a"), (19, 20, "A")],
            );
        });
    });
}

#[test]
fn test_no_match_across_block_items() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "word\n***\nword",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.debug(), "<text>word<hr><text>word");
            assert_matches(
                buffer,
                &SearchConfig {
                    query: "word",
                    regex: false,
                    case_sensitive: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                [(1, 5, "word"), (7, 11, "word")],
            );

            assert_no_matches(
                buffer,
                &SearchConfig {
                    query: r"word.*word",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
            );
        });
    });
}

#[test]
fn test_block_boundaries_as_whitespace() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "nee\ndle nee\n1. dle\n```rust\nnee\n```\n```sh\ndle\n```",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.debug(),
                r"<text>nee\ndle nee<ol0@1>dle<code:Rust>nee<code:Shell>dle<text>"
            );

            let expected_matches = [
                (1, 8, "nee\ndle"),
                (9, 16, "nee\ndle"),
                (17, 24, "nee\ndle"),
            ];

            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"nee\ndle",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                expected_matches,
            );

            // The `s` flag is needed for `.` to match newlines.
            assert_no_matches(
                buffer,
                &SearchConfig {
                    query: r"nee.dle",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
            );
            assert_matches(
                buffer,
                &SearchConfig {
                    query: r"(?s)nee.dle",
                    case_sensitive: true,
                    regex: true,
                    skip_hidden: false,
                    hidden_ranges: None,
                },
                expected_matches,
            );
        });
    });
}

#[test]
fn test_anchors() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "word\nsword\nword\nwords\nword",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.debug(), r"<text>word\nsword\nword\nwords\nword");

            // This fails because of the initial block marker.
            assert_no_matches(buffer, &SearchConfig::regex(r"\Aword\z"));

            // This succeeds because there's no ending block marker.
            assert_matches(buffer, &SearchConfig::regex(r"^word\z"), [(23, 27, "word")]);

            assert_matches(
                buffer,
                &SearchConfig::regex("^word$"),
                [(1, 5, "word"), (12, 16, "word"), (23, 27, "word")],
            );

            assert_matches(
                buffer,
                &SearchConfig::regex("word$"),
                [
                    (1, 5, "word"),
                    (7, 11, "word"),
                    (12, 16, "word"),
                    (23, 27, "word"),
                ],
            );

            assert_matches(
                buffer,
                &SearchConfig::regex("^word"),
                [
                    (1, 5, "word"),
                    (12, 16, "word"),
                    (17, 21, "word"),
                    (23, 27, "word"),
                ],
            );
        });
    });
}

#[test]
fn test_skip_hidden_content() {
    let mut buffer = SumTree::new();
    buffer.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    buffer.append_str("before\nword\nafter");

    let mut range_set = RangeSet::new();
    range_set.insert(CharOffset::from(8)..CharOffset::from(12));

    // With skip_hidden: false, finds text in hidden regions
    let mut engine = Engine::new(&SearchConfig {
        query: "word",
        case_sensitive: true,
        regex: false,
        skip_hidden: false,
        hidden_ranges: Some(&range_set),
    })
    .unwrap();
    let matches = engine.find_blocking(&buffer, CharOffset::zero()).unwrap();
    assert_eq!(matches.len(), 1);

    // With skip_hidden: true, does not find text in hidden regions
    let mut engine = Engine::new(&SearchConfig {
        query: "word",
        case_sensitive: true,
        regex: false,
        skip_hidden: true,
        hidden_ranges: Some(&range_set),
    })
    .unwrap();
    let matches = engine.find_blocking(&buffer, CharOffset::zero()).unwrap();
    assert_eq!(matches.len(), 0);
}

#[test]
fn test_cooperative_search() {
    // Manually construct a giant buffer for testing.
    let mut buffer = SumTree::new();
    buffer.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    for _ in 0..2000 {
        buffer.append_str("ab");
    }

    let mut engine = Engine::new(&SearchConfig {
        query: "a",
        case_sensitive: true,
        regex: false,
        skip_hidden: false,
        hidden_ranges: None,
    })
    .unwrap();
    let mut search_future = pin!(engine.find(&buffer, CharOffset::zero()));

    // With 2000 matches, the search should need 3 polls.
    future::block_on(async move {
        assert!(future::poll_once(&mut search_future).await.is_none());
        assert!(future::poll_once(&mut search_future).await.is_none());
        let result = future::poll_once(&mut search_future).await;
        match result {
            Some(Ok(matches)) => assert_eq!(matches.len(), 2000),
            Some(Err(err)) => panic!("Search failed: {err}"),
            None => panic!("Expected search to complete"),
        }
    });
}

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = env_logger::builder().is_test(true).try_init();
    })
}

/// Asserts that running `search` against `buffer` produces the expected matches.
fn assert_matches<'a>(
    buffer: &Buffer,
    search: &SearchConfig,
    expected_matches: impl IntoIterator<Item = impl Into<ExpectedMatch<'a>>>,
) {
    init_logging();

    let mut engine = Engine::new(search).expect("Could not compile search");
    let matches = engine
        .find_blocking(&buffer.content, CharOffset::zero())
        .expect("Could not run search")
        .into_iter()
        .map(|m| {
            let text = buffer.text_in_range(m.start..m.end).into_string();
            (m, text)
        })
        .collect_vec();

    let expected_matches = expected_matches.into_iter().map(Into::into).collect_vec();

    assert_eq!(
        expected_matches, matches,
        "Incorrect search results for {search:?}"
    );
}

/// Asserts that `search` has no matches in `buffer`.
fn assert_no_matches(buffer: &Buffer, search: &SearchConfig) {
    assert_matches(
        buffer,
        search,
        iter::empty::<(usize, usize, &'static str)>(),
    );
}

/// Helper for comparing to expected match results - this is needed because `String` and `&str`
/// aren't directly comparable.
#[derive(Debug)]
struct ExpectedMatch<'a> {
    start: CharOffset,
    end: CharOffset,
    match_text: &'a str,
}

impl<'a> From<(usize, usize, &'a str)> for ExpectedMatch<'a> {
    fn from((start, end, match_text): (usize, usize, &'a str)) -> Self {
        Self {
            start: start.into(),
            end: end.into(),
            match_text,
        }
    }
}

impl PartialEq<(Match, String)> for ExpectedMatch<'_> {
    fn eq(&self, (other_offsets, other_text): &(Match, String)) -> bool {
        self.start == other_offsets.start
            && self.end == other_offsets.end
            && self.match_text == other_text
    }
}
