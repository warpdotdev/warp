use itertools::Itertools;
use markdown_parser::parse_markdown;
use string_offset::CharOffset;
use warp_core::features::FeatureFlag;
use warpui::App;
use warpui::text::TextBuffer;
use warpui::text::point::Point;
use warpui::text::word_boundaries::WordBoundariesPolicy;

use crate::content::buffer::{Buffer, EditOrigin};
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::IndentBehavior;

#[test]
fn test_forward_iteration() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.replace_with_formatted_text(
                CharOffset::from(0)..CharOffset::from(1),
                parse_markdown("```\nText\n```\n**bold**\nAnd *italic* too.")
                    .expect("Markdown should parse"),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<code:Shell>Text<text><b_s>bold<b_e>\\nAnd <i_s>italic<i_e> too."
            );

            let mut chars = buffer.chars_at(CharOffset::from(1)).expect("Offset valid");
            // Block markers are converted to whitespace.
            assert_eq!(chars.next(), Some('T'));
            assert_eq!(chars.next(), Some('e'));
            assert_eq!(chars.next(), Some('x'));
            assert_eq!(chars.next(), Some('t'));
            assert_eq!(chars.next(), Some('\n'));
            // This transparently skips over the style markers.
            assert_eq!(chars.next(), Some('b'));

            // We should also be able to start from partway through.
            let chars = buffer
                .chars_at(CharOffset::from(12))
                .expect("Offset valid")
                .collect_vec();
            assert_eq!(
                chars,
                vec![
                    'n', 'd', ' ', 'i', 't', 'a', 'l', 'i', 'c', ' ', 't', 'o', 'o', '.'
                ]
            );
        });
    });
}

#[test]
fn test_table_word_boundaries_include_full_cell_text() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownTables.override_enabled(true);
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "| Hello | Value |\n| --- | --- |\n| World | Cell |\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.read(&app, |buffer, _| {
            let chars = buffer
                .chars_at(CharOffset::from(1))
                .expect("Offset valid")
                .take(12)
                .collect_vec();
            assert_eq!(
                chars,
                vec!['H', 'e', 'l', 'l', 'o', '\t', 'V', 'a', 'l', 'u', 'e', '\n']
            );

            let policy = WordBoundariesPolicy::Default;
            let start = buffer.word_start(CharOffset::from(1), &policy);
            let end = buffer.word_end(CharOffset::from(1), &policy);

            assert_eq!(start, CharOffset::from(1));
            assert_eq!(buffer.text_in_range(start..end).into_string(), "Hello");
        });
    });
}

#[test]
fn test_start_styled() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.replace_with_formatted_text(
                CharOffset::from(0)..CharOffset::from(1),
                parse_markdown("*styled* text").expect("Markdown should parse"),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            let mut chars = buffer.chars_at(CharOffset::from(1)).expect("Offset valid");
            assert_eq!(chars.next(), Some('s'));
            assert_eq!(chars.next(), Some('t'));
        });
    });
}

#[test]
fn test_reverse_iteration() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.replace_with_formatted_text(
                CharOffset::from(0)..CharOffset::from(1),
                parse_markdown("some *text*").expect("Markdown should parse"),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            let chars = buffer
                .chars_rev_at(CharOffset::from(4))
                .expect("Offset valid")
                .collect_vec();
            assert_eq!(chars, vec!['m', 'o', 's', '\n']);

            let chars = buffer
                .chars_rev_at(CharOffset::from(0))
                .expect("Offset valid")
                .collect_vec();
            assert!(chars.is_empty());

            let mut chars = buffer
                .chars_rev_at(CharOffset::from(8))
                .expect("Offset valid");
            assert_eq!(chars.next(), Some('e'));
            assert_eq!(chars.next(), Some('t'));
            assert_eq!(chars.next(), Some(' '));
            assert_eq!(chars.next(), Some('e'));
        });
    });
}

#[test]
fn test_plain_text_boundaries() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.replace_with_formatted_text(
                CharOffset::from(0)..CharOffset::from(1),
                parse_markdown("this *is* plain\ntext").expect("Markdown should parse"),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            let offsets = buffer
                .word_starts_from_offset(CharOffset::from(3))
                .unwrap()
                .collect_vec();
            assert_eq!(
                offsets,
                vec![
                    Point::new(1, 5),
                    Point::new(1, 8),
                    Point::new(2, 0),
                    Point::new(2, 4)
                ]
            );

            let ends_exclusive = buffer
                .word_ends_from_offset_exclusive(CharOffset::from(5))
                .unwrap()
                .collect_vec();
            // This should exclude the end of "this".
            assert_eq!(
                ends_exclusive,
                vec![Point::new(1, 7), Point::new(1, 13), Point::new(2, 4)]
            );
        });
    });
}

#[test]
fn test_empty_buffer() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let _selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.read(&app, |buffer, _| {
            // Clearly out-of-bounds indices should fail.
            assert!(buffer.chars_at(4.into()).is_err());
            assert!(buffer.chars_rev_at(4.into()).is_err());

            // Starting at 0, on the other hand, produces an empty iterator.
            // All buffers implicitly contain a leading block marker. However, when segmenting words, the
            // marker should not be included.
            let mut chars = buffer.chars_at(CharOffset::zero()).expect("Can start at 0");
            assert_eq!(chars.next(), Some('\n'));

            let mut chars = buffer
                .chars_rev_at(CharOffset::zero())
                .expect("Can start at 0");
            assert_eq!(chars.next(), None);

            let mut words = buffer
                .word_starts_from_offset(CharOffset::zero())
                .expect("Can start at 0");
            // Since the buffer is not truly empty (due to the leading block marker), WordBoundaries::next
            // considers the end of the buffer a word boundary.
            assert_eq!(words.next(), Some(Point::new(1, 0)));

            // Likewise, when moving backwards, the start of the buffer is a word boundary.
            let mut words_rev = buffer
                .word_starts_backward_from_offset_exclusive(CharOffset::zero())
                .expect("Can start at 0");
            assert_eq!(words_rev.next(), Some(Point::new(0, 0)));
        });
    });
}
