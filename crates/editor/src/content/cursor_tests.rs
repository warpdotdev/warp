use pathfinder_color::ColorU;
use string_offset::CharOffset;
use sum_tree::SumTree;

use super::{BufferCursor, BufferSumTree, BufferTextBatch};
use crate::content::text::{BufferBlockStyle, BufferText, BufferTextStyle, ColorMarker, MarkerDir};

/// Helper function to count the number of Text fragments in a SumTree
fn count_text_fragments(tree: &SumTree<BufferText>) -> usize {
    let mut cursor = tree.cursor::<(), ()>();
    cursor.descend_to_first_item(tree, |_| true);
    let mut count = 0;
    while let Some(item) = cursor.item() {
        if matches!(item, BufferText::Text { .. }) {
            count += 1;
        }
        cursor.next();
    }
    count
}

#[test]
fn test_plain_text_before_markers() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("This is some text");
    tree.push(BufferText::Newline);
    tree.append_str("New line veryyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy long text");
    assert_eq!(
        tree.debug(),
        "<text>This is some text\\nNew line veryyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy long text"
    );

    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut text_cursor = BufferCursor::new(cursor);
    text_cursor.seek_to_offset_before_markers(CharOffset::from(3));
    let new_content = text_cursor.slice_to_offset_before_markers(CharOffset::from(6));
    assert_eq!(new_content.debug(), "is ");

    let new_content = text_cursor.slice_to_offset_before_markers(CharOffset::from(20));
    assert_eq!(new_content.debug(), "is some text\\nN");

    let new_content = text_cursor.slice_to_offset_before_markers(CharOffset::from(40));
    assert_eq!(new_content.debug(), "ew line veryyyyyyyyy");
}

#[test]
fn test_plain_text_after_markers() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("This is some text");
    tree.push(BufferText::Newline);
    tree.append_str("New line veryyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy long text");
    assert_eq!(
        tree.debug(),
        "<text>This is some text\\nNew line veryyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy long text"
    );

    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut text_cursor = BufferCursor::new(cursor);
    text_cursor.seek_to_offset_after_markers(CharOffset::from(3));
    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(6));
    assert_eq!(new_content.debug(), "is ");

    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(20));
    assert_eq!(new_content.debug(), "is some text\\nN");

    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(40));
    assert_eq!(new_content.debug(), "ew line veryyyyyyyyy");
}

#[test]
fn test_append_str() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.append_str("Som");
    tree.append_str("ething");
    tree.append_str(" long stringggggggggggggg");
    assert_eq!(tree.debug(), "Something long stringggggggggggggg");
}

#[test]
fn test_append_str_merges_with_existing_fragment() {
    // Test the bug fix: is_first should be true when appending to allow merging
    // with the last text fragment if it has space remaining
    let mut tree: SumTree<BufferText> = SumTree::new();

    // Add initial content that creates a text fragment with remaining capacity
    tree.append_str("Initial");

    // Count fragments before second append
    let text_fragments_before = count_text_fragments(&tree);

    // Append more text - this should merge with the existing fragment if possible
    tree.append_str(" text");

    // Count fragments after second append
    let text_fragments_after = count_text_fragments(&tree);

    // The result should be a single merged fragment, not separate ones
    assert_eq!(tree.debug(), "Initial text");
    assert_eq!(
        text_fragments_before, 1,
        "Should have 1 fragment before second append"
    );
    assert_eq!(
        text_fragments_after, 1,
        "Should still have 1 fragment after merging"
    );

    // Verify the internal structure by checking we can iterate correctly
    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut buffer_cursor = BufferCursor::new(cursor);
    assert_eq!(buffer_cursor.char_at(CharOffset::from(0)), Some('I'));
    assert_eq!(buffer_cursor.char_at(CharOffset::from(7)), Some(' '));
    assert_eq!(buffer_cursor.char_at(CharOffset::from(8)), Some('t'));
    assert_eq!(buffer_cursor.char_at(CharOffset::from(11)), Some('t'));
}

#[test]
fn test_append_str_creates_new_fragment_when_full() {
    use crate::content::text::TEXT_FRAGMENT_SIZE;

    let mut tree: SumTree<BufferText> = SumTree::new();

    // Create a text fragment that's at the TEXT_FRAGMENT_SIZE limit
    let large_text = "a".repeat(TEXT_FRAGMENT_SIZE);
    tree.append_str(&large_text);

    let fragments_before = count_text_fragments(&tree);

    // Append additional text - this should create a new fragment since the first is full
    tree.append_str("extra");

    let fragments_after = count_text_fragments(&tree);

    // Should create a new fragment since the first one is at capacity
    let expected = format!("{large_text}extra");
    assert_eq!(tree.debug(), expected);
    assert_eq!(fragments_before, 1, "Should have 1 fragment before append");
    assert_eq!(
        fragments_after, 2,
        "Should have 2 fragments after append when first is full"
    );
}

/// Builds a tree whose trailing item takes the named shape, covering each case
/// [`BufferSumTree::append_str`] can meet when it tries to top up the trailing fragment.
fn seed_tree(shape: &str) -> SumTree<BufferText> {
    use arrayvec::ArrayString;

    use crate::content::text::TEXT_FRAGMENT_SIZE;

    let mut tree: SumTree<BufferText> = SumTree::new();
    match shape {
        "empty" => (),
        "empty_fragment" => tree.push(BufferText::Text {
            fragment: ArrayString::new(),
            char_count: 0,
        }),
        "room" => tree.append_str("Init"),
        "at_cap" => tree.append_str(&"a".repeat(TEXT_FRAGMENT_SIZE)),
        "one_short" => tree.append_str(&"a".repeat(TEXT_FRAGMENT_SIZE - 1)),
        "newline" => {
            tree.append_str("ab");
            tree.push(BufferText::Newline);
        }
        "marker" => {
            tree.append_str("ab");
            tree.push(BufferText::Marker {
                marker_type: BufferTextStyle::bold(),
                dir: MarkerDir::Start,
            });
        }
        other => panic!("unknown seed shape: {other}"),
    }
    tree
}

/// Asserts that no fragment's leading character would have fitted in the fragment before it,
/// which is what it means for the text to be packed as tightly as [`TEXT_FRAGMENT_SIZE`]
/// allows. Together with the content and fragment-count assertions this pins the fragmentation
/// exactly, since maximal left-to-right packing of a given sequence is unique. The count is
/// load-bearing: this walker skips an empty fragment, whose leading character does not exist.
fn assert_fragments_packed(tree: &SumTree<BufferText>, case: &str) {
    use crate::content::text::TEXT_FRAGMENT_SIZE;

    let mut cursor = tree.cursor::<(), ()>();
    cursor.descend_to_first_item(tree, |_| true);

    let mut previous_len = None;
    while let Some(item) = cursor.item() {
        if let BufferText::Text {
            fragment,
            char_count,
        } = item
        {
            assert!(
                fragment.len() <= TEXT_FRAGMENT_SIZE,
                "{case}: fragment holds {} bytes, over the cap",
                fragment.len()
            );
            assert_eq!(
                *char_count as usize,
                fragment.chars().count(),
                "{case}: char_count disagrees with the fragment"
            );
            if let Some(previous_len) = previous_len
                && let Some(first) = fragment.chars().next()
            {
                assert!(
                    previous_len + first.len_utf8() > TEXT_FRAGMENT_SIZE,
                    "{case}: fragment {fragment:?} should have been packed into the previous one"
                );
            }
            previous_len = Some(fragment.len());
        } else {
            previous_len = None;
        }
        cursor.next();
    }
}

#[test]
fn test_append_str_fragmentation() {
    use crate::content::text::TEXT_FRAGMENT_SIZE;

    let full = "a".repeat(TEXT_FRAGMENT_SIZE);
    let almost_full = "a".repeat(TEXT_FRAGMENT_SIZE - 1);
    let over_cap = "a".repeat(TEXT_FRAGMENT_SIZE + 5);
    // Three bytes per character, so splitting at the cap lands mid-character.
    let snowmen = "☃".repeat(30);

    // (seed shape, appended text, expected content, expected text fragment count)
    let cases: Vec<(&str, &str, String, usize)> = vec![
        ("empty", "", String::new(), 0),
        ("empty", "\n", "\\n".to_string(), 0),
        ("empty", "\nabc", "\\nabc".to_string(), 1),
        ("empty", "ab\ncd", "ab\\ncd".to_string(), 2),
        ("empty", "ab\n", "ab\\n".to_string(), 1),
        // `lines` strips the carriage return, so CRLF collapses to a single newline item.
        ("empty", "ab\r\ncd", "ab\\ncd".to_string(), 2),
        ("empty", "é", "é".to_string(), 1),
        // A carriage return that is not followed by a newline is ordinary text.
        ("empty", "a\rb", "a\rb".to_string(), 1),
        ("empty", &over_cap, over_cap.clone(), 2),
        ("empty", &snowmen, snowmen.clone(), 2),
        ("empty_fragment", "", String::new(), 1),
        ("empty_fragment", "ab", "ab".to_string(), 1),
        ("room", "", "Init".to_string(), 1),
        ("room", " text", "Init text".to_string(), 1),
        ("room", "\n", "Init\\n".to_string(), 1),
        // A non-empty top-up followed by a trailing newline: the one case where a mis-sliced
        // remainder would lose the `Newline` item.
        ("room", "ab\n", "Initab\\n".to_string(), 1),
        ("room", "ab\ncd", "Initab\\ncd".to_string(), 2),
        ("room", "ab\r\ncd", "Initab\\ncd".to_string(), 2),
        ("room", "é", "Inité".to_string(), 1),
        ("room", &over_cap, format!("Init{over_cap}"), 2),
        ("room", &snowmen, format!("Init{snowmen}"), 2),
        ("at_cap", "extra", format!("{full}extra"), 2),
        ("at_cap", "é", format!("{full}é"), 2),
        ("one_short", "ab", format!("{almost_full}ab"), 2),
        // Only one byte is free and the character needs two, so none of it can be topped up.
        ("one_short", "é", format!("{almost_full}é"), 2),
        ("newline", "cd", "ab\\ncd".to_string(), 2),
        ("marker", "cd", "ab<b_s>cd".to_string(), 2),
    ];

    for (shape, text, expected_content, expected_fragments) in cases {
        let case = format!("seed {shape:?} + {text:?}");
        let mut tree = seed_tree(shape);
        tree.append_str(text);

        assert_eq!(tree.debug(), expected_content, "{case}");
        assert_eq!(count_text_fragments(&tree), expected_fragments, "{case}");
        assert_fragments_packed(&tree, &case);
    }
}

/// One step of a code-block rebuild: the only two things the rebuild loop emits.
#[derive(Clone, Debug)]
enum Step {
    Marker(ColorMarker),
    Char(char),
}

fn text_steps(s: &str) -> Vec<Step> {
    s.chars().map(Step::Char).collect()
}

/// Appends each step on its own, the way the rebuild did before it batched.
fn append_unbatched(steps: &[Step]) -> SumTree<BufferText> {
    let mut tree: SumTree<BufferText> = SumTree::new();
    for step in steps {
        match step {
            Step::Marker(marker) => tree.push(BufferText::Color(marker.clone())),
            Step::Char(c) => tree.append_str(&c.to_string()),
        }
    }
    tree
}

fn append_batched(steps: &[Step]) -> SumTree<BufferText> {
    let mut tree: SumTree<BufferText> = SumTree::new();
    let mut batch = BufferTextBatch::new(&mut tree);
    for step in steps {
        match step {
            Step::Marker(marker) => batch.push_marker(marker.clone()),
            Step::Char(c) => batch.push_char(*c),
        }
    }
    batch.finish();
    tree
}

/// Batching may only change how densely the items are stored, never the items themselves.
///
/// Carriage returns are the case worth pinning: [`str::lines`] drops one that precedes a
/// newline, so buffering a run into a single string would delete a character that the
/// character-at-a-time path stored as ordinary text.
#[test]
fn test_batched_appends_match_unbatched() {
    use crate::content::text::TEXT_FRAGMENT_SIZE;

    let start = Step::Marker(ColorMarker::Start(ColorU::white()));
    let end = Step::Marker(ColorMarker::End);

    let programs: Vec<Vec<Step>> = vec![
        vec![],
        text_steps("a\r\nb"),
        text_steps("\r"),
        text_steps("a\r"),
        text_steps("\r\n"),
        text_steps("a\r\r\nb"),
        text_steps("a\r\n\r\nb"),
        text_steps("ab\ncd"),
        text_steps("ab\n"),
        text_steps("é☃é"),
        text_steps(&"a".repeat(TEXT_FRAGMENT_SIZE + 5)),
        text_steps(&"☃".repeat(30)),
        [
            text_steps("ab"),
            vec![start.clone()],
            text_steps("cd"),
            vec![end.clone()],
            text_steps("ef"),
        ]
        .concat(),
        [vec![start.clone()], text_steps("ab"), vec![end.clone()]].concat(),
        [text_steps("a"), vec![end.clone()], text_steps("\nb")].concat(),
        [vec![start.clone()], text_steps("a\r\nb"), vec![end.clone()]].concat(),
        [
            text_steps("a"),
            vec![start],
            text_steps("\r"),
            vec![end],
            text_steps("\nb"),
        ]
        .concat(),
    ];

    for program in &programs {
        assert_batched_matches_unbatched(program);
    }
}

/// The same equivalence, over every short program rather than hand-picked ones. The carriage
/// return bug was an interaction between two characters that only shows when they land
/// adjacent inside one buffered run, which is the kind of case nobody thinks to write down.
#[test]
fn test_batched_appends_match_unbatched_exhaustively() {
    let alphabet = [
        Step::Char('a'),
        Step::Char('\r'),
        Step::Char('\n'),
        Step::Marker(ColorMarker::Start(ColorU::white())),
        Step::Marker(ColorMarker::End),
    ];

    for length in 1..=5 {
        for mut encoded in 0..alphabet.len().pow(length) {
            let program: Vec<Step> = (0..length)
                .map(|_| {
                    let step = alphabet[encoded % alphabet.len()].clone();
                    encoded /= alphabet.len();
                    step
                })
                .collect();

            assert_batched_matches_unbatched(&program);
        }
    }
}

fn assert_batched_matches_unbatched(program: &[Step]) {
    let case = format!("program {program:?}");
    let unbatched = append_unbatched(program);
    let batched = append_batched(program);

    assert_eq!(batched.debug(), unbatched.debug(), "{case}");
    assert_eq!(
        count_text_fragments(&batched),
        count_text_fragments(&unbatched),
        "{case}: fragmentation differs"
    );
    assert_fragments_packed(&batched, &case);
    // The whole point of batching: the same items, in no more leaves than before.
    assert!(
        batched.node_stats().leaves <= unbatched.node_stats().leaves,
        "{case}: batching added leaves"
    );
}

#[test]
#[should_panic(expected = "dropped without finish")]
fn test_unfinished_batch_is_reported() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    let mut batch = BufferTextBatch::new(&mut tree);
    batch.push_char('a');
}

#[test]
fn test_styled_text_before_markers() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("Plain text");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::Start,
    });
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::Start,
    });
    tree.append_str("BI");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::End,
    });
    tree.append_str("Just Italic");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::End,
    });
    tree.append_str("Plain text");
    assert_eq!(
        tree.debug(),
        "<text>Plain text<b_s><i_s>BI<b_e>Just Italic<i_e>Plain text"
    );

    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut text_cursor = BufferCursor::new(cursor);
    text_cursor.seek_to_offset_before_markers(CharOffset::from(11));
    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(13));
    assert_eq!(new_content.debug(), "<b_s><i_s>BI<b_e>");

    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(17));
    assert_eq!(new_content.debug(), "Just");

    let new_content = text_cursor.slice_to_offset_before_markers(CharOffset::from(24));
    assert_eq!(new_content.debug(), " Italic");

    let new_content = text_cursor.suffix();
    assert_eq!(new_content.debug(), "<i_e>Plain text");
}

#[test]
fn test_char_at() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.append_str("Line");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::Start,
    });
    tree.append_str("String");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::End,
    });
    tree.push(BufferText::Newline);
    tree.append_str("Next");
    assert_eq!(tree.debug(), "Line<b_s>String<b_e>\\nNext");

    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut text_cursor = BufferCursor::new(cursor);
    assert_eq!(text_cursor.char_at(CharOffset::from(1)), Some('i'));
    assert_eq!(text_cursor.char_at(CharOffset::from(3)), Some('e'));
    assert_eq!(text_cursor.char_at(CharOffset::from(4)), Some('S'));
    assert_eq!(text_cursor.char_at(CharOffset::from(10)), Some('\n'));
}
