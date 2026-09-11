use string_offset::CharOffset;
use sum_tree::SumTree;

use super::{BufferCursor, BufferSumTree};
use crate::content::text::{
    BufferBlockStyle, BufferText, BufferTextStyle, MarkerDir, TEXT_FRAGMENT_SIZE,
};

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
fn append_str_preserves_fragment_invariants_across_multibyte_lines() {
    let prefix = "x".repeat(TEXT_FRAGMENT_SIZE - 1);
    let first_line = "é".repeat(TEXT_FRAGMENT_SIZE);
    let second_line = "🦀".repeat(TEXT_FRAGMENT_SIZE);
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.append_str(&prefix);

    tree.append_str(&format!("{first_line}\r\n{second_line}"));

    let items = tree.items();
    let fragment_metadata = items
        .iter()
        .filter_map(|item| match item {
            BufferText::Text {
                fragment,
                char_count,
            } => Some((fragment.len(), *char_count)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        fragment_metadata,
        vec![
            (TEXT_FRAGMENT_SIZE - 1, (TEXT_FRAGMENT_SIZE - 1) as u8),
            (TEXT_FRAGMENT_SIZE, (TEXT_FRAGMENT_SIZE / 2) as u8),
            (TEXT_FRAGMENT_SIZE, (TEXT_FRAGMENT_SIZE / 2) as u8),
            (TEXT_FRAGMENT_SIZE, (TEXT_FRAGMENT_SIZE / 4) as u8),
            (TEXT_FRAGMENT_SIZE, (TEXT_FRAGMENT_SIZE / 4) as u8),
            (TEXT_FRAGMENT_SIZE, (TEXT_FRAGMENT_SIZE / 4) as u8),
            (TEXT_FRAGMENT_SIZE, (TEXT_FRAGMENT_SIZE / 4) as u8),
        ]
    );
    assert_eq!(items[3], BufferText::Newline);
    assert_eq!(
        tree.debug(),
        format!("{prefix}{first_line}\\n{second_line}")
    );
    assert_eq!(
        tree.extent::<CharOffset>(),
        CharOffset::from(TEXT_FRAGMENT_SIZE * 3)
    );
}

#[test]
fn test_append_str_merges_with_existing_fragment() {
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
