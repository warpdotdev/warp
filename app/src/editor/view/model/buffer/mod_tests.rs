// Allow the `single_range_in_vec_init` clippy rule. The buffer#edit API takes in a `Vec` of ranges,
// which is at odds with this clippy rule.
#![allow(clippy::single_range_in_vec_init)]

use crate::editor::{soft_wrap::ClampDirection, tests::RandomCharIter};
use async_channel::Receiver;
use test::Network;

use super::*;
use enclose::enclose;
use futures::StreamExt;
use rand::prelude::StdRng;
use std::{
    cmp::Ordering,
    collections::HashSet,
    pin::{pin, Pin},
};
use warpui::{color::ColorU, App, ModelHandle};

fn visible_text_styles(buffer: &Buffer) -> Vec<Option<TextStyle>> {
    buffer
        .fragments
        .items()
        .into_iter()
        .filter_map(|fragment| {
            fragment
                .is_visible(&buffer.undo_history)
                .then_some(fragment.text.text_style)
        })
        .collect()
}

type OpsReceiver = Pin<Box<Receiver<Rc<Vec<Operation>>>>>;

/// Creates a new [`Buffer`], registers it as a model in the App
/// and creates a proxy receiver for any [`Event::UpdatePeers`] events
/// emitted by the buffer.
fn new_buffer_with_ops_receiver(
    app: &mut App,
    replica_id: ReplicaId,
    text: &str,
) -> (ModelHandle<Buffer>, OpsReceiver) {
    let model = app.add_model(|_| Buffer::new_with_replica_id(replica_id, text));
    let (tx, rx) = async_channel::unbounded();

    app.update(|ctx| {
        ctx.subscribe_to_model(&model, move |_, event, _| {
            if let Event::UpdatePeers { operations } = event {
                tx.try_send(operations.clone()).expect("can send message");
            }
        });
    });

    (model, Box::pin(rx))
}

impl ToCharOffset for usize {
    fn to_char_offset(&self, _: &Buffer) -> Result<CharOffset> {
        Ok(CharOffset::from(*self))
    }
}

fn to_char_index_range(range: Range<usize>) -> Range<CharOffset> {
    CharOffset::from(range.start)..CharOffset::from(range.end)
}

#[test]
#[should_panic]
fn test_nested_batches_are_not_allowed() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(""));
        buffer.update(&mut app, |buffer, _ctx| {
            // The buffer should not start in a batch.
            assert!(!buffer.batch_state.is_batching());

            // Start a batch.
            buffer.start_selection_changes_only_batch();

            // The buffer is in a batching state.
            assert!(buffer.batch_state.is_batching());

            // Start another batch. This should panic -- nested batches are not allowed.
            buffer.start_selection_changes_only_batch();
        })
    })
}

#[test]
#[should_panic]
fn test_edit_without_batching() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(""));
        buffer.update(&mut app, |buffer, ctx| {
            // The buffer should not start in a batch.
            assert!(!buffer.batch_state.is_batching());

            // Try to edit. This should panic.
            let _ = buffer.edit(vec![to_char_index_range(0..0)], "a", ctx);
        })
    })
}

#[test]
#[should_panic]
fn test_change_selections_without_batching() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(""));
        buffer.update(&mut app, |buffer, _| {
            // The buffer should not start in a batch.
            assert!(!buffer.batch_state.is_batching());

            // Try to change selections. This should panic.
            buffer
                .change_selections(
                    vec1![LocalSelection::new_for_test(Anchor::Start, Anchor::End)].into(),
                )
                .unwrap();
        })
    })
}

#[test]
fn test_change_selections() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("abc"));
        let new_selections: LocalSelections = vec1![LocalSelection {
            selection: Selection {
                start: Anchor::End,
                end: Anchor::Start,
                reversed: true,
            },
            clamp_direction: ClampDirection::Up,
            goal_end_column: None,
            goal_start_column: None,
        }]
        .into();

        buffer.read(&app, |buffer, _| {
            assert_ne!(buffer.local_selections, new_selections);
        });

        buffer.update(&mut app, |buffer, ctx| {
            buffer.start_selection_changes_only_batch();
            buffer.change_selections(new_selections.clone()).unwrap();
            buffer.end_batch(ctx);
        });

        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.local_selections, new_selections);
        });
    })
}

#[test]
fn test_merge_local_selections() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("abcde"));
        let (b_anchor, d_anchor, e_anchor) = buffer.read(&app, |buffer, _ctx| {
            (
                buffer.anchor_before(1).unwrap(),
                buffer.anchor_before(3).unwrap(),
                buffer.anchor_after(4).unwrap(),
            )
        });

        // Create two contiguous selections of "bc" and "de".
        buffer.update(&mut app, |buffer, _| {
            buffer.local_selections = LocalSelections {
                pending: None,
                selections: vec1![
                    LocalSelection::new_for_test(b_anchor.clone(), d_anchor.clone()),
                    LocalSelection::new_for_test(d_anchor.clone(), e_anchor.clone()),
                ],
                marked_text_state: Default::default(),
            }
        });

        buffer.update(&mut app, |buffer, ctx| {
            buffer.start_selection_changes_only_batch();
            buffer.merge_local_selections().unwrap();
            buffer.end_batch(ctx);
        });

        // After merging, we should just have one selection of "bcde".
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.local_selections,
                LocalSelections {
                    pending: None,
                    selections: vec1![LocalSelection::new_for_test(
                        b_anchor.clone(),
                        e_anchor.clone()
                    ),],
                    marked_text_state: Default::default()
                }
            )
        });
    })
}

#[test]
fn test_edit_for_test() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("abc"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                "def",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "abcdef");
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "ghi",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "ghiabcdef");
            buffer.edit_for_test(
                vec![to_char_index_range(5..5)],
                "jkl",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "ghiabjklcdef");
            buffer.edit_for_test(
                vec![to_char_index_range(6..7)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "ghiabjlcdef");
            buffer.edit_for_test(
                vec![to_char_index_range(4..9)],
                "mno",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "ghiamnoef");
            Ok(())
        })
    })
}

#[test]
fn test_edit_with_text_style() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("abc"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("def", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Ensure the last fragment has the correct text style.
            let text_styles = visible_text_styles(buffer);
            assert_eq!(
                text_styles,
                vec![None, None, Some(black_highlight_text_style)]
            );

            Ok(())
        })
    })
}

#[test]
fn test_edit_multiple_ranges() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("abcdefghi"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abcdefghi");
            buffer.edit_for_test(
                vec![to_char_index_range(1..3), to_char_index_range(5..8)],
                "_foo_",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "a_foo_de_foo_i");
            Ok(())
        })
    })
}

#[test]
fn test_edit_multiple_ranges_empty_string() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("abcdefghi"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abcdefghi");
            buffer.edit_for_test(
                vec![to_char_index_range(1..3), to_char_index_range(5..8)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "adei");
            Ok(())
        })
    })
}

#[test]
fn test_update_text_style_entire_run() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize buffer with "abc" (no styles).
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");
            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());
            // We add "def" to the buffer with a black highlight style.
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("def", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Ensure the last fragment has the correct text style.
            let text_styles = visible_text_styles(buffer);
            assert_eq!(
                text_styles,
                vec![None, None, Some(black_highlight_text_style)]
            );

            // We update "abc" in the buffer to have a black underline style
            // (previously had no styles).
            let black_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            buffer.update_styles(
                vec![to_char_index_range(0..3)],
                black_underline_style_operation,
                ctx,
            )?;

            // Check against all expected text style runs.
            let text = buffer.text();
            assert_eq!(text, "abcdef");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "abc".to_owned(),
                        black_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "def".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(3)..ByteOffset::from(6)
                    )
                ]
            );
            Ok(())
        })
    })
}

/// Test to make sure backspacing logic i.e. deletions, works correctly
/// with updating styles.
#[test]
fn test_update_text_style_clear_style() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize a buffer with "abc" (no styles).
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            let white_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::white());
            // We add "def" to the end of the buffer with a white underline
            // (new state is "abcdef").
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("def", Some(white_underline_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Ensure the last fragment has the correct text style.
            let text_styles: Vec<_> = visible_text_styles(buffer);
            assert_eq!(text_styles, vec![None, None, Some(white_underline_style)]);

            let black_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            // We style "abc" with a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..3)],
                black_underline_style_operation,
                ctx,
            )?;

            // Check the state of the buffer against what we expect.
            let text = buffer.text();
            assert_eq!(text, "abcdef");
            assert_eq!(text.len(), buffer.len().as_usize());
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "abc".to_owned(),
                        black_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "def".to_owned(),
                        white_underline_style,
                        ByteOffset::from(3)..ByteOffset::from(6)
                    )
                ]
            );

            // We clear the white underline from "def".
            // After this, we expect "abcdef" where only "abc" has a black underline.
            buffer.update_styles(
                vec![to_char_index_range(3..6)],
                TextStyleOperation::default().clear_error_underline_color(),
                ctx,
            )?;

            // Check the state of the buffer against what we expect.
            let text = buffer.text();
            assert_eq!(text, "abcdef");
            assert_eq!(text.len(), buffer.len().as_usize());
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "abc".to_owned(),
                        black_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "def".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(3)..ByteOffset::from(6)
                    )
                ]
            );
            Ok(())
        })
    })
}

/// Test style inheritance with edit operations and inheritable/non-inheritable
/// styles.
#[test]
fn test_edit_style_inheritable() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize a buffer with "abc" (no styles).
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            // Inheritable background style
            let black_background_style =
                TextStyle::default().with_background_color(ColorU::black());
            // Non-inheritable error underline style
            let black_error_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());

            // Add "def" to end of buffer with black background.
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("def", Some(black_background_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Add "ghi" to end of buffer - we expect this to inherit the black
            // background style.
            buffer.edit_for_test(
                vec![to_char_index_range(6..6)],
                Text::new("ghi", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Add "jkl" to end of buffer with black underline style.
            buffer.edit_for_test(
                vec![to_char_index_range(9..9)],
                Text::new("jkl", Some(black_error_underline_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Add "mno" to end of buffer with no style - we don't
            // expect it to inherit the black underline style!
            buffer.edit_for_test(
                vec![to_char_index_range(12..12)],
                Text::new("mno", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Check buffer against what we expect.
            let text = buffer.text();
            assert_eq!(text, "abcdefghijklmno");
            assert_eq!(text.len(), buffer.len().as_usize());
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "abc".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    // "ghi" inherited black background from "def"
                    TextRun::new(
                        "defghi".to_owned(),
                        black_background_style,
                        ByteOffset::from(3)..ByteOffset::from(9)
                    ),
                    TextRun::new(
                        "jkl".to_owned(),
                        black_error_underline_style,
                        ByteOffset::from(9)..ByteOffset::from(12)
                    ),
                    // "mno" did not inherit black underline from "jkl"
                    TextRun::new(
                        "mno".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(12)..ByteOffset::from(15)
                    ),
                ]
            );
            Ok(())
        })
    })
}

#[test]
fn test_update_text_style_backspace() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize an empty buffer.
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "");

            let black_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            // Add "g" with no text style to the buffer.
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                Text::new("g", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Update "g" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..1)],
                black_underline_style_operation,
                ctx,
            )?;

            // Add "i" with no text style to the buffer.
            buffer.edit_for_test(
                vec![to_char_index_range(1..1)],
                Text::new("i", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Update "gi" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;

            // Check whether the state of our buffer matches what we expect.
            let text = buffer.text();
            assert_eq!(text, "gi");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![TextRun::new(
                    "gi".to_owned(),
                    black_underline_style,
                    ByteOffset::from(0)..ByteOffset::from(2)
                ),]
            );
            // Add a space " " to the end of the buffer.
            buffer.edit_for_test(
                vec![to_char_index_range(2..2)],
                " ",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Update "gi" to have a black underline (no visible change here).
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;
            // Add "s" to the end of the buffer.
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                "s",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Update "gi" to have a black underline (no visible change here).
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;
            assert_eq!(buffer.text(), "gi s");
            // Delete "i s" from the buffer (last 3 characters).
            buffer.edit_for_test(
                vec![to_char_index_range(1..4)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "g");
            // Add "p" to the end of the buffer
            buffer.edit_for_test(
                vec![to_char_index_range(1..1)],
                "p",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "gp");
            // Update "gp" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;
            // Confirm that we deleted fragments correctly - check what is
            // in our buffer.
            assert_eq!(buffer.text(), "gp");
            Ok(())
        })
    })
}

/// Longer test to test deletions logic in conjunction with splicing
/// and style inheritance.
#[test]
fn test_update_text_style_backspace_splice_inheritance() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize an empty buffer.
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "");

            // Non-inheritable error underline style.
            let black_error_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_error_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            let green_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::new(0, 255, 0, 0));
            let green_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::new(0, 255, 0, 0));
            // Add "g" to end of empty buffer.
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                Text::new("g", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Update "g" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..1)],
                black_error_underline_style_operation,
                ctx,
            )?;
            assert_eq!(buffer.text(), "g");
            // Add "iabcdef" to the end of the buffer. Note that error
            // underlining is not inheritable hence they should not
            // have a black underline.
            buffer.edit_for_test(
                vec![to_char_index_range(1..1)],
                Text::new("iabcdef", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "giabcdef");
            // Update "gia" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..3)],
                black_error_underline_style_operation,
                ctx,
            )?;
            assert_eq!(buffer.text(), "giabcdef");
            // Update "bcd" to have a green underline.
            buffer.update_styles(
                vec![to_char_index_range(3..6)],
                green_underline_style_operation,
                ctx,
            )?;
            // Check the state of the buffer against what we expect (both text
            // and styles).
            let text = buffer.text();
            assert_eq!(text, "giabcdef");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "gia".to_owned(),
                        black_error_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "bcd".to_owned(),
                        green_underline_style,
                        ByteOffset::from(3)..ByteOffset::from(6)
                    ),
                    // "ef" did not inherit the black error underline.
                    TextRun::new(
                        "ef".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(6)..ByteOffset::from(8)
                    ),
                ]
            );
            // Delete "cde" from the buffer.
            buffer.edit_for_test(
                vec![to_char_index_range(4..7)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            // Check new state of buffer against what we expect.
            assert_eq!(buffer.text(), "giabf");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "gia".to_owned(),
                        black_error_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "b".to_owned(),
                        green_underline_style,
                        ByteOffset::from(3)..ByteOffset::from(4)
                    ),
                    TextRun::new(
                        "f".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(4)..ByteOffset::from(5)
                    ),
                ]
            );
            let blue_foreground_style_operation =
                TextStyleOperation::default().set_foreground_color(ColorU::new(0, 0, 255, 0));
            // Update "ab" to have a blue foreground color.
            buffer.update_styles(
                vec![to_char_index_range(2..4)],
                blue_foreground_style_operation,
                ctx,
            )?;
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "gi".to_owned(),
                        black_error_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(2)
                    ),
                    // "a" has both a black underline and blue foreground color.
                    TextRun::new(
                        "a".to_owned(),
                        black_error_underline_style
                            .with_foreground_color(ColorU::new(0, 0, 255, 0)),
                        ByteOffset::from(2)..ByteOffset::from(3)
                    ),
                    // "b" has both a green underline and blue foreground color.
                    TextRun::new(
                        "b".to_owned(),
                        green_underline_style.with_foreground_color(ColorU::new(0, 0, 255, 0)),
                        ByteOffset::from(3)..ByteOffset::from(4)
                    ),
                    TextRun::new(
                        "f".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(4)..ByteOffset::from(5)
                    ),
                ]
            );
            Ok(())
        })
    })
}

#[test]
fn test_update_text_style_partial_run() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize buffer with "abc" (no styles).
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());
            // Add "def" to end of buffer with a black highlight style.
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("def", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Ensure the last fragment has the correct text style.
            let text_styles = visible_text_styles(buffer);
            assert_eq!(
                text_styles,
                vec![None, None, Some(black_highlight_text_style)]
            );
            let black_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            // Update "ab" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;
            // Check state of buffer against what we expect.
            let text = buffer.text();
            assert_eq!(text, "abcdef");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "ab".to_owned(),
                        black_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(2)
                    ),
                    TextRun::new(
                        "c".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(2)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "def".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(3)..ByteOffset::from(6)
                    )
                ]
            );
            Ok(())
        })
    })
}

#[test]
fn test_update_text_style_multiple_style_updates() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize buffer with "abc" (no styles).
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());
            // Add "defghi" to end of buffer with black highlight style.
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("defghi", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            // Ensure the last fragment has the correct text style.
            let text_styles = visible_text_styles(buffer);
            assert_eq!(
                text_styles,
                vec![None, None, Some(black_highlight_text_style)]
            );
            let black_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            let red_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::new(255, 0, 0, 0));
            // Update "ab" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;
            // Update "fgh" to have a red underline.
            buffer.update_styles(
                vec![to_char_index_range(5..8)],
                red_underline_style_operation,
                ctx,
            )?;
            // Check the current state of the buffer against what we expect.
            let text = buffer.text();
            assert_eq!(text, "abcdefghi");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "ab".to_owned(),
                        black_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(2)
                    ),
                    TextRun::new(
                        "c".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(2)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "de".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(3)..ByteOffset::from(5)
                    ),
                    // "fgh" should have a black highlight style and red underline.
                    TextRun::new(
                        "fgh".to_owned(),
                        TextStyle::default()
                            .with_background_color(ColorU::black())
                            .with_error_underline_color(ColorU::new(255, 0, 0, 0)),
                        ByteOffset::from(5)..ByteOffset::from(8)
                    ),
                    TextRun::new(
                        "i".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(8)..ByteOffset::from(9)
                    )
                ]
            );

            Ok(())
        })
    })
}

#[test]
fn test_update_text_style_complex_multiple_ranges() -> Result<()> {
    App::test((), |mut app| async move {
        // Initialize buffer with "abc" (no styles).
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());
            // Add "defghijkl" to the end of the buffer with a black highlight style
            // (note highlight == background color).
            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("defghijkl", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(
                text_styles,
                vec![None, None, Some(black_highlight_text_style)]
            );

            let black_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::black());
            let black_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::black());
            let red_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::new(255, 0, 0, 0));

            // Update "ab" to have a black underline.
            buffer.update_styles(
                vec![to_char_index_range(0..2)],
                black_underline_style_operation,
                ctx,
            )?;

            // Update both "fgh" and "jk" to have a red underline i.e. multiple
            // ranges at once.
            buffer.update_styles(
                vec![to_char_index_range(5..8), to_char_index_range(9..11)],
                red_underline_style_operation,
                ctx,
            )?;
            // Check state of buffer against what we expect.
            let text = buffer.text();
            assert_eq!(text, "abcdefghijkl");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "ab".to_owned(),
                        black_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(2)
                    ),
                    TextRun::new(
                        "c".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(2)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "de".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(3)..ByteOffset::from(5)
                    ),
                    // "fgh" should have a black highlight and red underline.
                    TextRun::new(
                        "fgh".to_owned(),
                        TextStyle::default()
                            .with_background_color(ColorU::black())
                            .with_error_underline_color(ColorU::new(255, 0, 0, 0)),
                        ByteOffset::from(5)..ByteOffset::from(8)
                    ),
                    TextRun::new(
                        "i".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(8)..ByteOffset::from(9)
                    ),
                    // "jk" should have a black highlight and red underline.
                    TextRun::new(
                        "jk".to_owned(),
                        TextStyle::default()
                            .with_background_color(ColorU::black())
                            .with_error_underline_color(ColorU::new(255, 0, 0, 0)),
                        ByteOffset::from(9)..ByteOffset::from(11)
                    ),
                    TextRun::new(
                        "l".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(11)..ByteOffset::from(12)
                    )
                ]
            );

            let green_underline_style =
                TextStyle::default().with_error_underline_color(ColorU::new(0, 255, 0, 0));
            let green_underline_style_operation =
                TextStyleOperation::default().set_error_underline_color(ColorU::new(0, 255, 0, 0));
            // Update "abcdefghijkl" (entire buffer) to have a green underline.
            buffer.update_styles(
                vec![to_char_index_range(0..12)],
                green_underline_style_operation,
                ctx,
            )?;
            assert_eq!(text, "abcdefghijkl");
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    // "ab" had black underline overwritten to green underline.
                    // "c" also now has a green underline.
                    TextRun::new(
                        "abc".to_owned(),
                        green_underline_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    // "de" and "i" now also have a green underline, in addition
                    // to black highlight style.
                    // "fgh" and "jk" had black underline overwritten to green
                    // underline (along with keeping black highlight).
                    TextRun::new(
                        "defghijkl".to_owned(),
                        TextStyle::default()
                            .with_background_color(ColorU::black())
                            .with_error_underline_color(ColorU::new(0, 255, 0, 0),),
                        ByteOffset::from(3)..ByteOffset::from(12)
                    ),
                ]
            );

            Ok(())
        })
    })
}

#[test]
fn test_update_text_style_one_char() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "");

            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());
            let black_highlight_text_style_operation =
                TextStyleOperation::default().set_background_color(ColorU::black());
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                Text::new("a", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "a");
            assert_eq!(buffer.text().len(), buffer.len().as_usize());

            buffer.update_styles(
                vec![to_char_index_range(0..1)],
                black_highlight_text_style_operation,
                ctx,
            )?;
            assert_eq!(buffer.text(), "a");
            assert_eq!(1, buffer.len().as_usize());

            let text_styles = visible_text_styles(buffer);
            assert_eq!(text_styles, vec![None, Some(black_highlight_text_style)]);

            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![TextRun::new(
                    "a".to_owned(),
                    black_highlight_text_style,
                    ByteOffset::from(0)..ByteOffset::from(1)
                ),]
            );

            buffer.edit_for_test(
                vec![to_char_index_range(1..1)],
                Text::new("b", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "ab");
            assert_eq!(2, buffer.len().as_usize());

            Ok(())
        })
    })
}

#[test]
fn test_text_style_ranges() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("abc"));
        buffer_model.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "abc");

            let black_highlight_text_style =
                TextStyle::default().with_background_color(ColorU::black());

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("def", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            buffer.edit_for_test(
                vec![to_char_index_range(6..6)],
                Text::new("g", Some(black_highlight_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text = buffer.text();
            assert_eq!(text, "abcdefg");

            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "abc".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "defg".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(3)..ByteOffset::from(7)
                    )
                ]
            );

            buffer.edit_for_test(
                vec![to_char_index_range(6..6)],
                Text::new("123", Some(TextStyle::default())),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "abc".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "def".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(3)..ByteOffset::from(6)
                    ),
                    TextRun::new(
                        "123".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(6)..ByteOffset::from(9)
                    ),
                    TextRun::new(
                        "g".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(9)..ByteOffset::from(10)
                    )
                ]
            );

            buffer.edit_for_test(
                vec![to_char_index_range(0..3)],
                Text::new("", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![
                    TextRun::new(
                        "def".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(0)..ByteOffset::from(3)
                    ),
                    TextRun::new(
                        "123".to_owned(),
                        TextStyle::default(),
                        ByteOffset::from(3)..ByteOffset::from(6)
                    ),
                    TextRun::new(
                        "g".to_owned(),
                        black_highlight_text_style,
                        ByteOffset::from(6)..ByteOffset::from(7)
                    )
                ]
            );

            buffer.edit_for_test(
                vec![to_char_index_range(3..6)],
                Text::new("", None),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(
                buffer.text_style_runs().collect::<Vec<_>>(),
                vec![TextRun::new(
                    "defg".to_owned(),
                    black_highlight_text_style,
                    ByteOffset::from(0)..ByteOffset::from(4)
                )]
            );

            Ok(())
        })
    })
}

#[test]
fn test_text_styles_back_to_back() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new("ab"));
        buffer_model.update(&mut app, |buffer, ctx| {
            let red_text_style =
                TextStyle::default().with_background_color(ColorU::from_u32(0xFF0000FF));
            let green_text_style =
                TextStyle::default().with_background_color(ColorU::from_u32(0x00FF00FF));
            buffer.edit_for_test(
                vec![to_char_index_range(2..2)],
                Text::new("cd", Some(red_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            buffer.edit_for_test(
                vec![to_char_index_range(4..4)],
                Text::new("ef", Some(green_text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                text_styles,
                vec![None, None, Some(red_text_style), Some(green_text_style)]
            );

            Ok(())
        })
    })
}

#[test]
fn test_text_styles_delete_last_word() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            let text_style =
                TextStyle::default().with_background_color(ColorU::from_u32(0xFF0000FF));
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "foo",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            assert_eq!(buffer.text(), "foo");

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("bar", Some(text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobar");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            buffer.edit_for_test(
                vec![to_char_index_range(2..6)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "fo");
            assert_eq!(text_styles, vec![None, None,]);

            buffer.edit_for_test(
                vec![to_char_index_range(2..2)],
                "b",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "fob");
            assert_eq!(text_styles, vec![None, None, None]);

            Ok(())
        })
    })
}

#[test]
fn test_text_styles_delete_and_replace_whole_fragment() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            let text_style =
                TextStyle::default().with_background_color(ColorU::from_u32(0xFF0000FF));
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "foo",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foo");
            assert_eq!(text_styles, vec![None, None,]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("bar", Some(text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobar");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            buffer.edit_for_test(
                vec![to_char_index_range(4..4)],
                "bazz",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobbazzar");
            assert_eq!(
                text_styles,
                vec![
                    None,
                    None,
                    Some(text_style),
                    Some(text_style),
                    Some(text_style)
                ]
            );

            buffer.edit_for_test(vec![3..10], "a", EditOrigin::UserInitiated, ctx)?;
            let text_styles = visible_text_styles(buffer);

            assert_eq!(buffer.text(), "fooa");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            Ok(())
        })
    })
}

#[test]
fn test_text_styles_delete_until_end_of_highlight() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            let text_style =
                TextStyle::default().with_background_color(ColorU::from_u32(0xFF0000FF));
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "foo",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foo");
            assert_eq!(text_styles, vec![None, None,]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("bar", Some(text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobar");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..6)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foo");
            assert_eq!(text_styles, vec![None, None,]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                "b",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foob");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..4)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foo");
            assert_eq!(text_styles, vec![None, None]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                "a",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "fooa");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            buffer.edit_for_test(
                vec![to_char_index_range(2..4)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "fo");
            assert_eq!(text_styles, vec![None, None]);

            buffer.edit_for_test(
                vec![to_char_index_range(2..2)],
                "a",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foa");
            assert_eq!(text_styles, vec![None, None, None]);

            Ok(())
        })
    })
}

#[test]
fn test_text_styles_delete_until_highlight() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            let text_style =
                TextStyle::default().with_background_color(ColorU::from_u32(0xFF0000FF));
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "foo",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foo");
            assert_eq!(text_styles, vec![None, None,]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                "bar",
                EditOrigin::UserInitiated,
                ctx,
            )?;

            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobar");
            assert_eq!(text_styles, vec![None, None, None]);

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                Text::new("bazz", Some(text_style)),
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobazzbar");
            assert_eq!(text_styles, vec![None, None, Some(text_style), None,]);

            buffer.edit_for_test(vec![7..10], "", EditOrigin::UserInitiated, ctx)?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobazz");
            assert_eq!(text_styles, vec![None, None, Some(text_style),]);

            buffer.edit_for_test(
                vec![to_char_index_range(8..8)],
                "b",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let text_styles = visible_text_styles(buffer);
            assert_eq!(buffer.text(), "foobazzb");
            assert_eq!(text_styles, vec![None, None, Some(text_style), None,]);
            Ok(())
        })
    })
}

#[test]
fn test_edit_events() {
    App::test((), |mut app| async move {
        let base_text = "abcdef";
        let (buffer_1_tx, buffer_1_rx) = async_channel::unbounded();
        let (buffer_2_tx, buffer_2_rx) = async_channel::unbounded();

        let buffer1 = app.add_model(|_| Buffer::new_with_replica_id(ReplicaId::new(1), base_text));
        let buffer2 = app.add_model(|_| Buffer::new_with_replica_id(ReplicaId::new(2), base_text));

        app.update(|ctx| {
            ctx.subscribe_to_model(&buffer1, move |_, event, _| {
                buffer_1_tx
                    .try_send(event.to_owned())
                    .expect("Can send over buffer_2_tx")
            });

            ctx.subscribe_to_model(&buffer2, move |_, event, _| {
                buffer_2_tx
                    .try_send(event.to_owned())
                    .expect("Can send over buffer_2_tx")
            });
        });

        buffer1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(Some(2..4), "XYZ", EditOrigin::UserInitiated, ctx)
                .unwrap()
        });

        let mut buffer_1_rx = pin!(buffer_1_rx);
        let buffer_1_edit_event = buffer_1_rx
            .next()
            .await
            .expect("buffer 1 has an edited event");
        assert_eq!(
            buffer_1_edit_event,
            Event::Edited {
                edits: vec![Edit {
                    old_range: to_char_index_range(2..4),
                    new_range: to_char_index_range(2..5)
                }],
                edit_origin: EditOrigin::UserInitiated,
            }
        );

        let buffer_1_event = buffer_1_rx
            .next()
            .await
            .expect("buffer 1 has an update peers event");
        let operations = match buffer_1_event {
            Event::UpdatePeers { operations } => operations.to_vec(),
            _ => panic!("Expected to receive an UpdatePeers event but got {buffer_1_event:?}"),
        };

        buffer2.update(&mut app, |buffer, ctx| {
            buffer.apply_ops(operations, ctx).unwrap();
        });

        let mut buffer_2_rx = pin!(buffer_2_rx);
        let buffer_2_edit_event = buffer_2_rx
            .next()
            .await
            .expect("buffer 2 has an edited event");
        assert_eq!(
            buffer_2_edit_event,
            Event::Edited {
                edits: vec![Edit {
                    old_range: to_char_index_range(2..4),
                    new_range: to_char_index_range(2..5)
                }],
                edit_origin: EditOrigin::RemoteEdit,
            }
        );
    })
}

#[test]
fn test_random_edits() {
    App::test((), |mut app| async move {
        for seed in 0..100 {
            println!("{seed:?}");
            let mut rng = &mut StdRng::seed_from_u64(seed);

            let reference_string_len = rng.gen_range(0..3);
            let mut reference_string = RandomCharIter::new(&mut rng)
                .take(reference_string_len)
                .collect::<String>();
            let buffer = app.add_model(|_| Buffer::new(reference_string.as_str()));
            buffer.update(&mut app, |buffer, ctx| {
                let mut buffer_versions = Vec::new();

                for _i in 0..10 {
                    let (old_ranges, new_text) = buffer.randomly_edit(
                        rng,
                        RangesWhenEditing::UseRandomRanges { num_ranges: 5 },
                        ctx,
                    );
                    for old_range in old_ranges.iter().rev() {
                        reference_string = [
                            &reference_string[0..old_range.start.as_usize()],
                            new_text.as_str(),
                            &reference_string[old_range.end.as_usize()..],
                        ]
                        .concat();
                    }
                    assert_eq!(buffer.text(), reference_string);

                    if rng.gen_bool(0.3) {
                        buffer_versions.push(buffer.clone());
                    }
                }

                for mut old_buffer in buffer_versions {
                    let mut delta = 0_isize;
                    for Edit {
                        old_range,
                        new_range,
                    } in buffer.edits_since(old_buffer.versions.clone())
                    {
                        let old_len = old_range.end - old_range.start;
                        let new_len = new_range.end - new_range.start;
                        let old_start = CharOffset::from(
                            (old_range.start.as_usize() as isize + delta) as usize,
                        );

                        old_buffer
                            .edit_for_test(
                                Some(old_start..old_start + old_len),
                                buffer.text_for_range(new_range).unwrap(),
                                EditOrigin::UserInitiated,
                                ctx,
                            )
                            .unwrap();

                        delta += new_len.as_usize() as isize - old_len.as_usize() as isize;
                    }
                    assert_eq!(old_buffer.text(), buffer.text());
                }
            })
        }
    })
}

#[test]
fn test_line_len() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "abcd\nefg\nhij",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            buffer.edit_for_test(vec![12..12], "kl\nmno", EditOrigin::UserInitiated, ctx)?;
            buffer.edit_for_test(vec![18..18], "\npqrs\n", EditOrigin::UserInitiated, ctx)?;
            buffer.edit_for_test(vec![18..21], "\nPQ", EditOrigin::UserInitiated, ctx)?;

            assert_eq!(buffer.line_len(0)?, 4);
            assert_eq!(buffer.line_len(1)?, 3);
            assert_eq!(buffer.line_len(2)?, 5);
            assert_eq!(buffer.line_len(3)?, 3);
            assert_eq!(buffer.line_len(4)?, 4);
            assert_eq!(buffer.line_len(5)?, 0);
            assert!(buffer.line_len(6).is_err());

            Ok(())
        })
    })
}

#[test]
fn test_text_summary_for_range() {
    let buffer = Buffer::new("ab\nefg\nhklm\nnopqrs\ntuvwxyz");
    let text = Text::from(buffer.text());

    assert_eq!(
        buffer.text_summary_for_range(to_char_index_range(1..3)),
        text.slice(to_char_index_range(1..3)).summary()
    );
    assert_eq!(
        buffer.text_summary_for_range(to_char_index_range(1..12)),
        text.slice(to_char_index_range(1..12)).summary()
    );
    assert_eq!(
        buffer.text_summary_for_range(to_char_index_range(0..20)),
        text.slice(to_char_index_range(0..20)).summary()
    );
    assert_eq!(
        buffer.text_summary_for_range(to_char_index_range(0..22)),
        text.slice(to_char_index_range(0..22)).summary()
    );
    assert_eq!(
        buffer.text_summary_for_range(to_char_index_range(7..22)),
        text.slice(to_char_index_range(7..22)).summary()
    );
}

#[test]
fn test_chars_at() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "abcd\nefgh\nij",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            buffer.edit_for_test(vec![12..12], "kl\nmno", EditOrigin::UserInitiated, ctx)?;
            buffer.edit_for_test(vec![18..18], "\npqrs", EditOrigin::UserInitiated, ctx)?;
            buffer.edit_for_test(vec![18..21], "\nPQ", EditOrigin::UserInitiated, ctx)?;

            let chars = buffer.chars_at(Point::new(0, 0))?;
            assert_eq!(chars.collect::<String>(), "abcd\nefgh\nijkl\nmno\nPQrs");

            let chars = buffer.chars_at(Point::new(1, 0))?;
            assert_eq!(chars.collect::<String>(), "efgh\nijkl\nmno\nPQrs");

            let chars = buffer.chars_at(Point::new(2, 0))?;
            assert_eq!(chars.collect::<String>(), "ijkl\nmno\nPQrs");

            let chars = buffer.chars_at(Point::new(3, 0))?;
            assert_eq!(chars.collect::<String>(), "mno\nPQrs");

            let chars = buffer.chars_at(Point::new(4, 0))?;
            assert_eq!(chars.collect::<String>(), "PQrs");

            // Regression test:
            let mut buffer = Buffer::new("");
            buffer.edit_for_test(vec![to_char_index_range(0..0)], "[workspace]\nmembers = [\n    \"xray_core\",\n    \"xray_server\",\n    \"xray_cli\",\n    \"xray_wasm\",\n]\n", EditOrigin::UserInitiated, ctx)?;
            buffer.edit_for_test(vec![60..60], "\n", EditOrigin::UserInitiated, ctx)?;

            let chars = buffer.chars_at(Point::new(6, 0))?;
            assert_eq!(chars.collect::<String>(), "    \"xray_wasm\",\n]\n");

            Ok(())
        })
    })
}

#[test]
fn test_fragment_ids() {
    for seed in 0..10 {
        let rng = &mut StdRng::seed_from_u64(seed);

        let mut ids = vec![FragmentId(Box::new([0])), FragmentId(Box::new([4]))];
        for _i in 0..100 {
            let index = rng.gen_range(1..ids.len());

            let left = ids[index - 1].clone();
            let right = ids[index].clone();
            ids.insert(index, FragmentId::between_with_max(&left, &right, 4));

            let mut sorted_ids = ids.clone();
            sorted_ids.sort();
            assert_eq!(ids, sorted_ids);
        }
    }
}

#[test]
fn test_anchors() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "abc",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            let left_anchor = buffer.anchor_before(2).unwrap();
            let right_anchor = buffer.anchor_after(2).unwrap();

            buffer.edit_for_test(
                vec![to_char_index_range(1..1)],
                "def\n",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "adef\nbc");
            assert_eq!(left_anchor.to_char_offset(buffer).unwrap(), 6.into());
            assert_eq!(right_anchor.to_char_offset(buffer).unwrap(), 6.into());
            assert_eq!(
                left_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 1 }
            );
            assert_eq!(
                right_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 1 }
            );

            buffer.edit_for_test(
                vec![to_char_index_range(2..3)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "adf\nbc");
            assert_eq!(left_anchor.to_char_offset(buffer).unwrap(), 5.into());
            assert_eq!(right_anchor.to_char_offset(buffer).unwrap(), 5.into());
            assert_eq!(
                left_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 1 }
            );
            assert_eq!(
                right_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 1 }
            );

            buffer.edit_for_test(
                vec![to_char_index_range(5..5)],
                "ghi\n",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "adf\nbghi\nc");
            assert_eq!(left_anchor.to_char_offset(buffer).unwrap(), 5.into());
            assert_eq!(right_anchor.to_char_offset(buffer).unwrap(), 9.into());
            assert_eq!(
                left_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 1 }
            );
            assert_eq!(
                right_anchor.to_point(buffer).unwrap(),
                Point { row: 2, column: 0 }
            );

            buffer.edit_for_test(
                vec![to_char_index_range(7..9)],
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "adf\nbghc");
            assert_eq!(left_anchor.to_char_offset(buffer).unwrap(), 5.into());
            assert_eq!(right_anchor.to_char_offset(buffer).unwrap(), 7.into());
            assert_eq!(
                left_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 1 },
            );
            assert_eq!(
                right_anchor.to_point(buffer).unwrap(),
                Point { row: 1, column: 3 }
            );

            // Ensure anchoring to a point is equivalent to anchoring to an offset.
            assert_eq!(
                buffer.anchor_before(Point { row: 0, column: 0 })?,
                buffer.anchor_before(0)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 0, column: 1 })?,
                buffer.anchor_before(1)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 0, column: 2 })?,
                buffer.anchor_before(2)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 0, column: 3 })?,
                buffer.anchor_before(3)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 1, column: 0 })?,
                buffer.anchor_before(4)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 1, column: 1 })?,
                buffer.anchor_before(5)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 1, column: 2 })?,
                buffer.anchor_before(6)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 1, column: 3 })?,
                buffer.anchor_before(7)?
            );
            assert_eq!(
                buffer.anchor_before(Point { row: 1, column: 4 })?,
                buffer.anchor_before(8)?
            );

            // Comparison between anchors.
            let anchor_at_offset_0 = buffer.anchor_before(0).unwrap();
            let anchor_at_offset_1 = buffer.anchor_before(1).unwrap();
            let anchor_at_offset_2 = buffer.anchor_before(2).unwrap();

            assert_eq!(
                anchor_at_offset_0.cmp(&anchor_at_offset_0, buffer)?,
                Ordering::Equal
            );
            assert_eq!(
                anchor_at_offset_1.cmp(&anchor_at_offset_1, buffer)?,
                Ordering::Equal
            );
            assert_eq!(
                anchor_at_offset_2.cmp(&anchor_at_offset_2, buffer)?,
                Ordering::Equal
            );

            assert_eq!(
                anchor_at_offset_0.cmp(&anchor_at_offset_1, buffer)?,
                Ordering::Less
            );
            assert_eq!(
                anchor_at_offset_1.cmp(&anchor_at_offset_2, buffer)?,
                Ordering::Less
            );
            assert_eq!(
                anchor_at_offset_0.cmp(&anchor_at_offset_2, buffer)?,
                Ordering::Less
            );

            assert_eq!(
                anchor_at_offset_1.cmp(&anchor_at_offset_0, buffer)?,
                Ordering::Greater
            );
            assert_eq!(
                anchor_at_offset_2.cmp(&anchor_at_offset_1, buffer)?,
                Ordering::Greater
            );
            assert_eq!(
                anchor_at_offset_2.cmp(&anchor_at_offset_0, buffer)?,
                Ordering::Greater
            );
            Ok(())
        })
    })
}

#[test]
fn test_anchors_at_start_and_end() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer_model: &mut ModelHandle<Buffer> = &mut app.add_model(|_| Buffer::new(""));
        buffer_model.update(&mut app, |buffer, ctx| {
            let before_start_anchor = buffer.anchor_before(0).unwrap();
            let after_end_anchor = buffer.anchor_after(0).unwrap();

            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "abc",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                before_start_anchor.to_char_offset(buffer).unwrap(),
                0.into()
            );
            assert_eq!(after_end_anchor.to_char_offset(buffer).unwrap(), 3.into());

            let after_start_anchor = buffer.anchor_after(0).unwrap();
            let before_end_anchor = buffer.anchor_before(3).unwrap();

            buffer.edit_for_test(
                vec![to_char_index_range(3..3)],
                "def",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            buffer.edit_for_test(
                vec![to_char_index_range(0..0)],
                "ghi",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            assert_eq!(buffer.text(), "ghiabcdef");
            assert_eq!(
                before_start_anchor.to_char_offset(buffer).unwrap(),
                0.into()
            );
            assert_eq!(after_start_anchor.to_char_offset(buffer).unwrap(), 3.into());
            assert_eq!(before_end_anchor.to_char_offset(buffer).unwrap(), 6.into());
            assert_eq!(after_end_anchor.to_char_offset(buffer).unwrap(), 9.into());

            Ok(())
        })
    })
}

#[test]
fn test_concurrent_insertions() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "quick".
        // Concurrently, one replica adds "The " to the start
        // while the other replica adds " brown" to the end.
        let base_text = "quick";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "The ",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "The quick");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(5..5)],
                    " brown",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "quick brown");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec().to_vec();

        // Both replicas should converge to "The quick brown".
        let expected_text = "The quick brown";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_concurrent_insertions_at_same_location() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "quick".
        // Concurrently, one replica adds an "One" to the start
        // while the other replica adds "The" to the start.
        // We break the tie by using replica ID.
        let base_text = "quick";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "One ",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "One quick");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "The ",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "The quick");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        // The buffers should converge to "The One quick" because
        // we break ties using (lamport, replica ID). In this case, both ops
        // have equal lamport timestamps, and thus we break the tie using replica ID
        // (replica 2 wins).
        let expected_text = "The One quick";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });

        // Replica 1 should still be able to edit at the start of the buffer.
        // If we only relies on replica ID to break ties, replica 1 would never be able
        // to insert at the start of the buffer. But in this case, replica 1's edit will have a larger
        // lamport timestamp.
        let expected_text = "One day The One quick";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "One day ",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), expected_text);
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_concurrent_non_overlapping_deletions() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "The quick brown jumps".
        // Concurrently, one replica deletes " brown"
        // while the other replica adds " fox" after "brown".
        let base_text = "The quick brown jumps";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(9..15)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "The quick jumps");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(16..16)],
                    "fox ",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "The quick brown fox jumps");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        // Although the inserted text was relative to the deleted text,
        // the buffer should converge to "The quick fox jumps".
        let expected_text = "The quick fox jumps";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_concurrent_overlapping_deletions() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "The quick brown jumps".
        // Concurrently, one replica deletes "quick brown "
        // while the other replica delete "brown ".
        let base_text = "The quick brown jumps";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(4..16)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "The jumps");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(10..16)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "The quick jumps");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        // Replica 2's deletion is a subset of replica 1's deletion,
        // so the buffers should converge to "The jumps".
        let expected_text = "The jumps";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_insertion_within_deletion() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "The quick brown jumps".
        // Concurrently, one replica deletes " brown jumps"
        // while the other replica inserts "fox" after "brown".
        let base_text = "The quick brown jumps";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(9..21)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "The quick");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(16..16)],
                    "fox ",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "The quick brown fox jumps");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        // Since replica 2's insertion was made when it saw the base text,
        // it should be honored, so the buffers should converge to
        // "The quickfox " (the space before the 'f' was deleted as part of replica 1's deletion).
        let expected_text = "The quickfox ";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_concurrent_replace() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "The quick brown fox".
        // Concurrently, one replica replaces "brown" with "red"
        // while the other replica replaces "brown" with "white".
        let base_text: &str = "The quick brown fox";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(10..15)],
                    "red",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "The quick red fox");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(10..15)],
                    "white",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "The quick white fox");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        // The edits were concurrent, so we need to honor both.
        // The buffer should converge to "The quick whitered fox".
        // (Note: "white" before "red" because replica 2 > replica 1).
        let expected_text = "The quick whitered fox";
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_deferred_ops() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abcd".
        // Replica 1 makes 4 consecutive edits:
        // 1. insert "1" before "a"
        // 2. insert "2" before "b"
        // 3. insert "3" before "c"
        // 4. insert "4" before "d"
        // Replica 2 gets the edits out of order ("2" -> "4" -> "1" -> "3").
        // Since the edits are out of order, replica 2 needs to defer them until
        // it's seen all previous edits (e.g. can't apply "2" until "1" is received).
        // Despite this, replica 2 inserts "~" after "b" before any of replica 1's edits
        // are actually applied.
        let base_text = "abcd";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "1",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1abcd");
        });
        let buffer_1_edit_1 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(2..2)],
                    "2",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1a2bcd");
        });
        let buffer_1_edit_2 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(4..4)],
                    "3",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1a2b3cd");
        });
        let buffer_1_edit_3 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(6..6)],
                    "4",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1a2b3c4d");
        });
        let buffer_1_edit_4 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit_2, ctx)
                .expect("can apply replica 1's edits to replica 2");
            // Can't apply it so buffer text should not change.
            assert_eq!(buffer.text(), "abcd");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(2..2)],
                    "~",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "ab~cd");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit_4, ctx)
                .expect("can apply replica 1's edits to replica 2");
            // Can't apply it so buffer text should not change.
            assert_eq!(buffer.text(), "ab~cd");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit_1, ctx)
                .expect("can apply replica 1's edits to replica 2");
            // Since we've received edit 1, we can also flush edit 2
            // (but not edit 4 since edit 3 was not received).
            assert_eq!(buffer.text(), "1a2b~cd");
        });

        let expected_text = "1a2b3~c4d";
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit_3, ctx)
                .expect("can apply replica 1's edits to replica 2");
            // Since we've received edit 3, we can also flush edit 4.
            // Note: the '3' comes before '~' because the former edit had a larger
            // lamport timestamp.
            assert_eq!(buffer.text(), "1a2b3~c4d");
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_duplicate_ops() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "The quick brown jumps".
        // One replica adds "fox" before "jumps", but the edit is received
        // twice by the second replica. The second replica should de-dupe
        // and only apply it once.
        let base_text = "The quick brown jumps";
        let expected_text = "The quick brown fox jumps";
        let (buffer_1, mut buffer_1_ops_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, _) = new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(15..15)],
                    " fox",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), expected_text);
        });
        let buffer_1_edit = buffer_1_ops_rx.next().await.unwrap().to_vec();
        let buffer_1_edit_clone = buffer_1_edit.clone();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit_clone, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), expected_text);
        });
    })
}

#[test]
fn test_multiple_peers() {
    App::test((), |mut app| async move {
        // This test was adapted from an occurrence of `test_random_concurrent_operations`.
        // In this test, the buffer starts as "bar".
        // We have three peers where each peer makes an edit.
        let base_text = "bar";
        let expected_text = "applebaz";
        let (buffer_1, mut buffer_1_ops_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_ops_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);
        let (buffer_3, mut buffer_3_ops_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(3), base_text);

        // Buffer 3 edits the base text.
        buffer_3.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "foo",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 3");
            assert_eq!(buffer.text(), "foobar");
        });
        let buffer_3_edit_1 = buffer_3_ops_rx.next().await.unwrap().to_vec();

        // Buffer 1 applies buffer 3's edit.
        buffer_1.update(
            &mut app,
            enclose!((buffer_3_edit_1) move |buffer, ctx| {
                buffer
                    .apply_ops(buffer_3_edit_1, ctx)
                    .expect("can apply replica 3's edits to replica 1");
                assert_eq!(buffer.text(), "foobar");
            }),
        );

        // Buffer 2 edits the base text.
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "baz",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "bazbar");
        });
        let buffer_2_edit_1 = buffer_2_ops_rx.next().await.unwrap().to_vec();

        // Buffer 3 applies buffer 2's edit.
        buffer_3.update(
            &mut app,
            enclose!((buffer_2_edit_1) move|buffer, ctx| {
                buffer
                    .apply_ops(buffer_2_edit_1, ctx)
                    .expect("can apply replica 2's edits to replica 3");
                assert_eq!(buffer.text(), "foobazbar");
            }),
        );

        // Buffer 1 edits their buffer (recall buffer 1 observed buffer 3's first edit).
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..6)],
                    "apple",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "apple");
        });
        let buffer_1_edit_1 = buffer_1_ops_rx.next().await.unwrap().to_vec();

        // Buffer 3 applies buffer 1's edit.
        buffer_3.update(
            &mut app,
            enclose!((buffer_1_edit_1) move |buffer, ctx| {
                buffer
                    .apply_ops(buffer_1_edit_1, ctx)
                    .expect("can apply replica 1's edits to replica 3");
                assert_eq!(buffer.text(), expected_text);
            }),
        );

        // Buffer 2 applies buffer 3's edit.
        buffer_2.update(&mut app, move |buffer, ctx| {
            buffer
                .apply_ops(buffer_3_edit_1, ctx)
                .expect("can apply replica 1's edits to replica 3");
            assert_eq!(buffer.text(), "foobazbar");
        });

        // Buffer 2 applies buffer 1's edit.
        buffer_2.update(&mut app, move |buffer, ctx| {
            // Recall: buffer_1_edit_1 replaced "foobar" with "apple".
            buffer
                .apply_ops(buffer_1_edit_1, ctx)
                .expect("can apply replica 1's edits to replica 3");
            assert_eq!(buffer.text(), expected_text);
        });

        // Buffer 1 applies buffer 2's edit.
        buffer_1.update(&mut app, move |buffer, ctx| {
            // Recall: buffer_2_edit_1 added "baz" before "bar".
            buffer
                .apply_ops(buffer_2_edit_1, ctx)
                .expect("can apply replica 1's edits to replica 3");
            assert_eq!(buffer.text(), expected_text);
        });

        // At this point, all peers have applied each other's edits and converged to the same buffer.
    })
}

#[test]
fn test_random_concurrent_operations() {
    App::test((), |mut app| async move {
        const PEERS: usize = 10;

        for seed in 0..1000 {
            println!("seed={seed}");
            let mut rng = &mut StdRng::seed_from_u64(seed);

            let base_text_len = rng.gen_range(0..10);
            let base_text = RandomCharIter::new(&mut rng)
                .take(base_text_len)
                .collect::<String>();
            let mut replica_ids = Vec::new();
            let mut buffers = Vec::new();
            let mut network = Network::new();
            for i in 1..=PEERS {
                let replica_id = ReplicaId::new(i);
                let (buffer, ops_rx) =
                    new_buffer_with_ops_receiver(&mut app, replica_id.clone(), base_text.as_str());
                buffers.push((buffer, ops_rx));
                replica_ids.push(replica_id.clone());
                network.add_peer(replica_id);
            }

            let mut mutation_count = 10;
            let mut replicas_that_changed_selections = HashSet::new();
            loop {
                let replica_index = rng.gen_range(0..PEERS);
                let replica_id = replica_ids[replica_index].clone();
                let (buffer, ops_rx) = &buffers[replica_index];
                let mut ops_rx = ops_rx.clone();
                if mutation_count > 0 && rng.gen() {
                    let mutation_type = buffer.update(&mut app, |buffer, ctx| {
                        buffer.randomly_mutate(&mut rng, ctx)
                    });
                    if matches!(
                        mutation_type,
                        RandomMutationType::ChangeSelections { changed: true }
                    ) {
                        replicas_that_changed_selections.insert(replica_id.clone());
                    }

                    let ops = ops_rx.next().await.unwrap().to_vec();
                    network.broadcast(replica_id, ops, &mut rng);
                    mutation_count -= 1;
                } else if network.has_unreceived(&replica_id) {
                    let ops = network.receive(replica_id, &mut rng);
                    buffer.update(&mut app, |buffer, ctx| {
                        buffer.apply_ops(ops, ctx).unwrap();
                    });
                }

                if mutation_count == 0 && network.is_idle() {
                    break;
                }
            }

            let (expected_text, _expected_selections) = buffers[0].0.read(&app, |buffer, _ctx| {
                // Only include our own selections if they ever changed
                // (otherwise, we wouldn't expect peers to have a selection state for us).
                let all_selections = buffer.all_selections(
                    replicas_that_changed_selections.contains(&buffer.replica_id()),
                );

                (buffer.text(), all_selections)
            });

            for (buffer, _) in &buffers[1..] {
                buffer.read(&app, |buffer, _ctx| {
                    assert_eq!(
                        buffer.text(),
                        expected_text,
                        "Replica {:?}'s text differs from replica 1",
                        buffer.lamport_clock.replica_id
                    );

                    // TODO(suraj): enable these assertions once
                    // we fixed selection consistency.
                    //
                    // // Only include our own selections if they ever changed
                    // // (otherwise, we wouldn't expect peers to have a selection state for us).
                    // let actual_selections = buffer.all_selections(dbg!(
                    //     replicas_that_changed_selections.contains(&buffer.replica_id())
                    // ));

                    // assert_eq!(
                    //     actual_selections, expected_selections,
                    //     "Replica {:?}'s selections differs from replica 1",
                    //     buffer.lamport_clock.replica_id
                    // );
                });
            }
        }
    })
}

#[test]
fn test_get_word_near_point() {
    {
        let buffer = Buffer::new("echo 'foo     bar' text-data");
        for col in 0..=3 {
            assert_eq!(
                buffer.get_word_nearest_to_point(&Point::new(0, col)),
                Some("echo".to_owned())
            );
        }
        for col in 4..=8 {
            assert_eq!(
                buffer.get_word_nearest_to_point(&Point::new(0, col)),
                Some("foo".to_owned())
            );
        }
        for col in 9..=16 {
            assert_eq!(
                buffer.get_word_nearest_to_point(&Point::new(0, col)),
                Some("bar".to_owned())
            );
        }
        for col in 17..=22 {
            assert_eq!(
                buffer.get_word_nearest_to_point(&Point::new(0, col)),
                Some("text".to_owned())
            );
        }
        for col in 23..=27 {
            assert_eq!(
                buffer.get_word_nearest_to_point(&Point::new(0, col)),
                Some("data".to_owned())
            );
        }
    }
    {
        let buffer = Buffer::new("");
        assert_eq!(buffer.get_word_nearest_to_point(&Point::new(0, 0)), None);
    }
    {
        let buffer = Buffer::new("    :)  ");
        assert_eq!(buffer.get_word_nearest_to_point(&Point::new(0, 2)), None);
    }
    {
        let buffer = Buffer::new("foo\n   \nbar");

        assert_eq!(buffer.get_word_nearest_to_point(&Point::new(1, 1)), None);
        assert_eq!(
            buffer.get_word_nearest_to_point(&Point::new(2, 1)),
            Some("bar".to_owned())
        );
    }
    {
        let buffer = Buffer::new("\necho foo\n");

        assert_eq!(buffer.get_word_nearest_to_point(&Point::new(0, 0)), None);
        assert_eq!(
            buffer.get_word_nearest_to_point(&Point::new(1, 0)),
            Some("echo".to_owned())
        );
    }
}

#[test]
fn test_undo_redo() -> Result<()> {
    App::test((), |mut app| async move {
        // In this test, we rely on the fact that [`Buffer::edit_for_test`] records
        // edits with the [`Action::ReplaceBuffer`] action which is atomic.
        // Therefore, each edit corresponds to one undo stack entry.
        let buffer = app.add_model(|_| Buffer::new("hello"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "hello");

            // Undo to start should be a no-op.
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello");

            // Same for redo.
            buffer.redo(ctx);
            assert_eq!(buffer.text(), "hello");

            // Make an edit, undo it and then redo it.
            buffer
                .edit_for_test(
                    vec![to_char_index_range(5..5)],
                    " world",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .unwrap();
            assert_eq!(buffer.text(), "hello world");

            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello");

            buffer.redo(ctx);
            assert_eq!(buffer.text(), "hello world");

            // Make two consecutive edits (i.e. two undo stack entries).
            buffer
                .edit_for_test(
                    vec![to_char_index_range(5..5)],
                    "?",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .unwrap();
            assert_eq!(buffer.text(), "hello? world");

            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..1)],
                    "j",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .unwrap();
            assert_eq!(buffer.text(), "jello? world");

            // Undo both edits.
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello? world");

            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello world");

            // Redo the old edit.
            buffer.redo(ctx);
            assert_eq!(buffer.text(), "hello? world");

            // Make an edit while we're not at the top of the
            // undo stack (because there were two undos followed by
            // only one redo).
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..5)],
                    "bye",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .unwrap();
            assert_eq!(buffer.text(), "bye? world");

            // Since we edited while we were not at the top of the undo stack,
            // the stack should be truncated and redo should be a no-op.
            buffer.redo(ctx);
            assert_eq!(buffer.text(), "bye? world");

            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello? world");
        });

        Ok(())
    })
}

#[test]
fn test_only_one_undo_record_per_batch() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("hello"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "hello");

            buffer.start_edits_and_selection_changes_batch(
                EditOrigin::UserInitiated,
                PlainTextEditorViewAction::ReplaceBuffer,
                false,
            );
            buffer
                .edit(vec![to_char_index_range(5..5)], " world", ctx)
                .unwrap();
            assert_eq!(buffer.text(), "hello world");

            buffer
                .edit(vec![to_char_index_range(11..11)], "!", ctx)
                .unwrap();
            assert_eq!(buffer.text(), "hello world!");

            buffer.end_batch(ctx);
        });

        // The edits should be coalesced as one entry on the undo stack.
        buffer.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello");
        });
        Ok(())
    })
}

#[test]
fn test_record_edits() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("hello"));
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.text(), "hello");

            buffer.start_edits_and_selection_changes_batch(
                EditOrigin::UserInitiated,
                PlainTextEditorViewAction::ReplaceBuffer,
                false,
            );
            buffer
                .edit(vec![to_char_index_range(5..5)], " world", ctx)
                .unwrap();
            assert_eq!(buffer.text(), "hello world");

            // Force a record.
            buffer.record_edits(PlainTextEditorViewAction::ReplaceBuffer, ctx);

            buffer
                .edit(vec![to_char_index_range(11..11)], "!", ctx)
                .unwrap();
            assert_eq!(buffer.text(), "hello world!");

            buffer.end_batch(ctx);
        });

        // The edits should be two separate entries on the undo stack.
        buffer.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello world");

            buffer.redo(ctx);
            assert_eq!(buffer.text(), "hello world!");

            buffer.undo(ctx);
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "hello");
        });
        Ok(())
    })
}

#[test]
fn test_remote_undo_redo() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as empty.
        // One replica makes three insertions: "a", "b" and "c".
        // Upon receiving these edits, another replica changes "b" -> "d".
        // When replica one performs undo, it can only undo its own operations,
        // while respecting any changes made by peers.
        let base_text: &str = "";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        // Set up the edits.
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0)],
                    "a",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "a");
        });
        let buffer_1_edit_1 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..1)],
                    "b",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "ab");
        });
        let buffer_1_edit_2 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(2..2)],
                    "c",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "abc");
        });
        let buffer_1_edit_3 = buffer_1_rx.next().await.unwrap().to_vec();

        // Make sure replica 2 acknowledges the edits.
        let ops: Vec<Operation> = buffer_1_edit_1
            .into_iter()
            .chain(buffer_1_edit_2)
            .chain(buffer_1_edit_3)
            .collect();
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(ops, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), "abc");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..2)],
                    "d",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "adc");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        // Undo!
        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), "adc");
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            // The first undo should only undo buffer 1's local edit (not buffer 2's latest edit).
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "ad");
        });
        let buffer_1_undo_1 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            // The second undo should be a no-op because the "b" edit is no longer visible anyways.
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "ad");
        });
        let buffer_1_undo_2 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "d");
        });
        let buffer_1_undo_3 = buffer_1_rx.next().await.unwrap().to_vec();

        // Apply undo's to replica 2.
        let ops: Vec<Operation> = buffer_1_undo_1
            .into_iter()
            .chain(buffer_1_undo_2)
            .chain(buffer_1_undo_3)
            .collect();
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(ops, ctx)
                .expect("can apply replica 1's undos to replica 2");
            assert_eq!(buffer.text(), "d");
        });

        // Redo!
        buffer_1.update(&mut app, |buffer, ctx| {
            // The first redo should bring back the 'a'.
            buffer.redo(ctx);
            assert_eq!(buffer.text(), "ad");
        });
        let buffer_1_redo_1 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            // The second redo should be a no-op because the "d" edit is still present.
            buffer.redo(ctx);
            assert_eq!(buffer.text(), "ad");
        });
        let buffer_1_redo_2 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            // The third redo should bring back the 'c'.
            buffer.redo(ctx);
            assert_eq!(buffer.text(), "adc");
        });
        let buffer_1_redo_3 = buffer_1_rx.next().await.unwrap().to_vec();

        // Apply redo's to replica 2!
        let ops: Vec<Operation> = buffer_1_redo_1
            .into_iter()
            .chain(buffer_1_redo_2)
            .chain(buffer_1_redo_3)
            .collect();
        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(ops, ctx)
                .expect("can apply replica 1's redos to replica 2");
            assert_eq!(buffer.text(), "adc");
        });
    })
}

#[test]
fn test_concurrent_edit_with_undo() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "foobar".
        // Replica 1 adds '!' between 'foo' and 'bar' and replica 2 receives this.
        // Then, concurrently, replica 1 does an undo while replica 2
        // removes "foo".
        let base_text: &str = "foobar";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(3..3)],
                    "!",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "foo!bar");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), "foo!bar");
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "foobar");
        });
        let buffer_1_undo = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..3)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "!bar");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), "bar");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_undo, ctx)
                .expect("can apply replica 1's undo to replica 2");
            assert_eq!(buffer.text(), "bar");
        });
    })
}

#[test]
fn test_concurrent_edit_with_conflicting_undo() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "foobar".
        // Replica 1 adds 'baz' between 'foo' and 'bar' and replica 2 receives this.
        // Then, concurrently, replica 1 does an undo while replica 2
        // changes the 'z' to 'x'.
        let base_text: &str = "foobar";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(3..3)],
                    "baz",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "foobazbar");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), "foobazbar");
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "foobar");
        });
        let buffer_1_undo = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(5..6)],
                    "x",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "foobaxbar");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), "fooxbar");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_undo, ctx)
                .expect("can apply replica 1's undo to replica 2");
            assert_eq!(buffer.text(), "fooxbar");
        });
    })
}

#[test]
fn test_deletions_with_undo() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 changes 'a' to '1'.
        // Upon receiving replica 1's edit, replica 2 replaces "1bc" with "0".
        // Upon receiving replica 2's edit, replica 1 performs an undo.
        // Replica 1's undo should be a no-op because their edit is not visible anymore.
        // The final text should be "0".
        let base_text: &str = "abc";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..1)],
                    "1",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1bc");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), "1bc");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..3)],
                    "0",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "0");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), "0");
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "0");
        });
        let buffer_1_undo = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_undo, ctx)
                .expect("can apply replica 1's undo to replica 2");
            assert_eq!(buffer.text(), "0");
        });
    })
}

#[test]
fn test_concurrent_deletions_with_undo() {
    App::test((), |mut app| async move {
        // This test is similar to [`test_deletions_with_undo`] but
        // the edits here happen concurrently rather than sequentially.
        //
        // In this test, the buffer begins as "abc".
        // Concurrently:
        // - Replica 1 changes 'a' to '1'.
        // - Replica 2 changes "abc" to "0".
        // The expected text as this point should be "01" because
        // the deletion of "a" was made concurrent to the edit.
        //
        // Now, suppose replica 1 performs an undo.
        // The expected text should be "0" because while we
        // are reverting "1" -> "a", the edit for "a" has been deleted
        // by replica 2 (which has not been undone).
        //
        // If replica 2 undoes at this point, then the
        // buffer text should be "abc" (not "1bc" because
        // replica 1 already undid).
        let base_text: &str = "abc";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..1)],
                    "1",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1bc");
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..3)],
                    "0",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 2");
            assert_eq!(buffer.text(), "0");
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply replica 2's edits to replica 1");
            assert_eq!(buffer.text(), "01");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply replica 1's edits to replica 2");
            assert_eq!(buffer.text(), "01");
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "0");
        });
        let buffer_1_undo = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_undo, ctx)
                .expect("can apply replica 1's undo to replica 2");
            assert_eq!(buffer.text(), "0");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "abc");
        });
        let buffer_2_undo = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_undo, ctx)
                .expect("can apply replica 2's undo to replica 1");
            assert_eq!(buffer.text(), "abc");
        });
    })
}

#[test]
fn test_out_of_order_undo() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 makes the following consecutive changes:
        // - replace 'a' with 1
        // - undo
        // - replace 'c' to '3'
        // which produces the text 'ab2'.
        //
        // Suppose replica 2 receives these in reverse order.
        // Operations should be deferred and the final text should be the same.
        let base_text: &str = "abc";
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(1), base_text);
        let (buffer_2, _buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, ReplicaId::new(2), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..1)],
                    "1",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "1bc");
        });
        let buffer_1_edit_1 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);
            assert_eq!(buffer.text(), "abc");
        });
        let buffer_1_undo = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(2..3)],
                    "3",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit buffer 1");
            assert_eq!(buffer.text(), "ab3");
        });
        let buffer_1_edit_2 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit_2, ctx)
                .expect("can apply replica the last edit");
            assert_eq!(buffer.text(), "abc");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_undo, ctx)
                .expect("can apply replica the last edit");
            assert_eq!(buffer.text(), "abc");
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            // Once the first edit is applied, all ops should be flushed.
            buffer
                .apply_ops(buffer_1_edit_1, ctx)
                .expect("can apply replica the last edit");
            assert_eq!(buffer.text(), "ab3");
        });
    })
}

#[test]
fn test_stable_selections() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 moves their cursor to after 'b'.
        // Replica 2 makes edits before and after 'b'.
        // Replica 1's cursor should stay stable (not move).
        let base_text: &str = "abc";
        let (replica_1_id, replica_2_id) = (ReplicaId::new(1), ReplicaId::new(2));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(2..2)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(2..2)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(2..2)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..0), to_char_index_range(2..2)],
                    "1",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 2");

            assert_eq!(buffer.text(), "1ab1c");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(3..3)]
            );
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply edit to replica 1");

            assert_eq!(buffer.text(), "1ab1c");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id),
                vec![to_char_index_range(3..3)]
            );
        });
    })
}

#[test]
fn test_replace_selected_text() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 selects the text 'b'.
        // Replica 2 then removes the 'b'.
        // Replica 1's selections should collapsed to a cursor because the
        // edit that the selection was based on is no longer visible.
        // If replica 2 adds back the 'b', replica 1's selections should be unchanged.
        let base_text: &str = "abc";
        let (replica_1_id, replica_2_id) = (ReplicaId::new(1), ReplicaId::new(2));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(1..2)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..2)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..2)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..2)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 2");

            assert_eq!(buffer.text(), "ac");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..1)],
            );
        });
        let buffer_2_edit_1 = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit_1, ctx)
                .expect("can apply edit to replica 1");

            assert_eq!(buffer.text(), "ac");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..1)],
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..1)],
                    "b",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..1)],
            );
        });
        let buffer_2_edit_2 = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit_2, ctx)
                .expect("can apply edit to replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id),
                vec![to_char_index_range(1..1)],
            );
        });
    })
}

#[test]
fn test_receiving_selection_change_before_edit() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 adds '123' after 'a' and then selects it.
        // Replica 2 receives the selection change before the edit operation.
        // The selection change should be deferred until the edit is applied.
        let base_text: &str = "abc";
        let (replica_1_id, replica_2_id) = (ReplicaId::new(1), ReplicaId::new(2));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, _buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..1)],
                    "123",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 1");

            assert_eq!(buffer.text(), "a123bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(1..4)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "a123bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..4)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert!(buffer
                .selections_for_replica(replica_1_id.clone())
                .is_empty(),);
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply edit to replica 2");

            assert_eq!(buffer.text(), "a123bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(1..4)]
            );
        });
    })
}

#[test]
fn test_receiving_out_of_date_selection_updates() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 selects 'a'. And then replica 1 changes selections to 'c'.
        // Suppose replica 2 receives the second update first.
        // When it receives the first selection update, it should ignore it.
        let base_text: &str = "abc";
        let (replica_1_id, replica_2_id) = (ReplicaId::new(1), ReplicaId::new(2));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, _buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(0..1)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1)]
            );
        });
        let buffer_1_selection_update_1 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(2..3)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(2..3)]
            );
        });
        let buffer_1_selection_update_2 = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update_2, ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(2..3)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update_1, ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(2..3)]
            );
        });
    })
}

#[test]
fn test_undo_should_restore_selection() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 selects 'a' and then removes it.
        // If replica 1 undoes that action, the selection should be restored.
        let base_text: &str = "abc";
        let (replica_1_id, replica_2_id) = (ReplicaId::new(1), ReplicaId::new(2));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, _buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(0..1)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..1)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 1");

            assert_eq!(buffer.text(), "bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });
        let buffer_1_edit = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 2");

            buffer
                .apply_ops(buffer_1_edit, ctx)
                .expect("can apply edit to replica 2");

            assert_eq!(buffer.text(), "bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1)]
            );
        });
        let buffer_1_undo = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_undo, ctx)
                .expect("can apply undo to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1)]
            );
        });
    })
}

#[ignore = "The underlying issue here is the same as the one in test_merging_selections_with_concurrent_edits."]
#[test]
fn test_undo_should_not_restore_remote_selection() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abc".
        // Replica 1 selects 'a'.
        // Replica 2 then removes it.
        // Replica 2 then undoes its operation.
        // Replica 1's selection should _not_ be restored.
        let base_text: &str = "abc";
        let (replica_1_id, replica_2_id) = (ReplicaId::new(1), ReplicaId::new(2));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(vec![to_char_index_range(0..1)], ctx)
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(0..1)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 2");

            assert_eq!(buffer.text(), "bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply edit to replica 1");

            assert_eq!(buffer.text(), "bc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer.undo(ctx);

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });
        let buffer_2_undo = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_undo, ctx)
                .expect("can apply undo to replica 1");

            assert_eq!(buffer.text(), "abc");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..0)]
            );
        });
    })
}

#[test]
fn test_merging_selections_with_remote_edits() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abcdef".
        // Replica 1 selects two ranges: "a" and "cdef".
        // Then, replica 2 removes the 'b'.
        // Every replica should merge the selection ranges.
        let base_text: &str = "abcdef";
        let (replica_1_id, replica_2_id, replica_3_id) =
            (ReplicaId::new(1), ReplicaId::new(2), ReplicaId::new(3));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);
        let (buffer_3, _buffer_3_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_3_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(
                    vec![to_char_index_range(0..1), to_char_index_range(2..6)],
                    ctx,
                )
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update.clone(), ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });

        buffer_3.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 3");

            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..2)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 2");

            assert_eq!(buffer.text(), "acdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..5)],
            );
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit.clone(), ctx)
                .expect("can apply buffer 2's edit to buffer 1");

            assert_eq!(buffer.text(), "acdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..5)],
            );
        });

        buffer_3.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply buffer 2's edit to buffer 3");

            assert_eq!(buffer.text(), "acdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..5)]
            );
        });
    })
}

// See https://github.com/warpdotdev/warp-internal/pull/9249 for discussion
// about possible strategies to address this.
#[ignore = "The test points out an eventual consistency problem with selections."]
#[test]
fn test_merging_selections_with_concurrent_edits() {
    App::test((), |mut app| async move {
        // In this test, the buffer begins as "abcdef".
        // Replica 1 selects two ranges: "a" and "cdef".
        // Then, we have two concurrent edits:
        // - Replica 2 removes the 'b'
        // - Replica 3 inserts '1' after 'a'
        //
        // When replica 2 removes the 'b', replica 1's selection
        // range should be collapsed into one. When replica 2 processes
        // replica 3's edit, the new character would be part of the selected range.
        //
        // When replica 3 inesrts the '1' after 'a', replica 1's selections
        // are still split in two ranges. When replica 3 processes replica 2's edit,
        // the selections don't collapse.
        //
        // Now, replica 2 and replica 3 have different interpretations of replica 1's selection state.
        //
        // Suppose replica 1 processes replica 2's edit before replica 3's edit.
        // In this case, when we process replica 2's edit, we would collapse
        // the selection ranges into one and then when inserting replica 3's edit,
        // it would be part of the selected range. So Replica 1 would expect
        // "a1cdef" where the entire range is selected.
        //
        // We need to get replica 3 to be consistent now.
        let base_text: &str = "abcdef";
        let (replica_1_id, replica_2_id, replica_3_id) =
            (ReplicaId::new(1), ReplicaId::new(2), ReplicaId::new(3));
        let (buffer_1, mut buffer_1_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_1_id.clone(), base_text);
        let (buffer_2, mut buffer_2_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_2_id.clone(), base_text);
        let (buffer_3, mut buffer_3_rx) =
            new_buffer_with_ops_receiver(&mut app, replica_3_id.clone(), base_text);

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .change_selections_for_test(
                    vec![to_char_index_range(0..1), to_char_index_range(2..6)],
                    ctx,
                )
                .expect("can change selections for replica 1");

            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });
        let buffer_1_selection_update = buffer_1_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update.clone(), ctx)
                .expect("can apply selection change to replica 2");

            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });

        buffer_3.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_1_selection_update, ctx)
                .expect("can apply selection change to replica 3");

            assert_eq!(buffer.text(), "abcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..2)],
                    "",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 2");

            assert_eq!(buffer.text(), "acdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..5)],
            );
        });
        let buffer_2_edit = buffer_2_rx.next().await.unwrap().to_vec();

        buffer_3.update(&mut app, |buffer, ctx| {
            buffer
                .edit_for_test(
                    vec![to_char_index_range(1..1)],
                    "1",
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .expect("can edit replica 3");

            assert_eq!(buffer.text(), "a1bcdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(3..7)],
            );
        });
        let buffer_3_edit = buffer_3_rx.next().await.unwrap().to_vec();

        buffer_2.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_3_edit.clone(), ctx)
                .expect("can apply buffer 3's edit to buffer 2");

            assert_eq!(buffer.text(), "a1cdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..6)]
            );
        });

        buffer_3.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit.clone(), ctx)
                .expect("can apply buffer 2's edit to buffer 3");

            assert_eq!(buffer.text(), "a1cdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..1), to_char_index_range(2..6)]
            );
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_2_edit, ctx)
                .expect("can apply buffer 2's edit to buffer 1");

            assert_eq!(buffer.text(), "acdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..5)],
            );
        });

        buffer_1.update(&mut app, |buffer, ctx| {
            buffer
                .apply_ops(buffer_3_edit, ctx)
                .expect("can apply buffer 3's edit to buffer 1");

            assert_eq!(buffer.text(), "a1cdef");
            assert_eq!(
                buffer.selections_for_replica(replica_1_id.clone()),
                vec![to_char_index_range(0..6)],
            );
        });
    })
}

/// The type of random mutation [`Buffer::randomly_mutate`] applied.
pub enum RandomMutationType {
    Edit,
    ChangeSelections {
        /// True iff the selections before
        /// the change are not equal to the
        /// selections after.
        changed: bool,
    },
    Undo,
    Redo,
}

impl Buffer {
    pub fn randomly_edit<T>(
        &mut self,
        rng: &mut T,
        ranges: RangesWhenEditing,
        ctx: &mut ModelContext<Self>,
    ) -> (Vec<Range<CharOffset>>, String)
    where
        T: Rng,
    {
        let ranges = match ranges {
            RangesWhenEditing::UseExistingSelections => self
                .local_selections
                .selections
                .iter()
                .map(|selection| {
                    selection.start().to_char_offset(self).unwrap()
                        ..selection.end().to_char_offset(self).unwrap()
                })
                .collect(),
            RangesWhenEditing::UseRandomRanges { num_ranges } => {
                random_ranges(rng, num_ranges, self.len().as_usize())
            }
        };

        let new_text_len = rng.gen_range(0..10);
        let new_text: String = RandomCharIter::new(&mut *rng).take(new_text_len).collect();

        self.edit_for_test(
            ranges.clone(),
            new_text.as_str(),
            EditOrigin::UserInitiated,
            ctx,
        )
        .unwrap();

        (ranges, new_text)
    }

    pub fn randomly_change_selections<T>(
        &mut self,
        rng: &mut T,
        ctx: &mut ModelContext<Self>,
    ) -> bool
    where
        T: Rng,
    {
        // Select between 1 to 3 ranges.
        let max_num_ranges = rng.gen_range(1..=3);
        let text_len = self.len();

        let new_ranges = if text_len.as_usize() == 0 {
            vec![CharOffset::zero()..CharOffset::zero()]
        } else {
            random_ranges(rng, max_num_ranges, text_len.as_usize())
        };

        self.change_selections_for_test(new_ranges, ctx).unwrap()
    }

    /// Performs one mutation of the buffer which should
    /// produce one [`Event::UpdatePeers`] event.
    pub fn randomly_mutate<T>(
        &mut self,
        rng: &mut T,
        ctx: &mut ModelContext<Self>,
    ) -> RandomMutationType
    where
        T: Rng,
    {
        // Randomly mutate the buffer by editing, undo'ing, redo'ing, or changing selections.
        // 70% of the time we edit.
        // 10% of the time we update selections.
        // 10% of the time we undo.
        // 10% of the time we redo.
        match rng.gen_range(0..10) {
            0..=6 => {
                let num_ranges = rng.gen_range(0..=5);
                let ranges = if num_ranges == 0 {
                    RangesWhenEditing::UseExistingSelections
                } else {
                    RangesWhenEditing::UseRandomRanges { num_ranges }
                };
                self.randomly_edit(rng, ranges, ctx);
                RandomMutationType::Edit
            }
            7 => {
                let changed = self.randomly_change_selections(rng, ctx);
                RandomMutationType::ChangeSelections { changed }
            }
            8 => {
                self.undo(ctx);
                RandomMutationType::Undo
            }
            9 => {
                self.redo(ctx);
                RandomMutationType::Redo
            }
            _ => unreachable!(),
        }
    }
}

/// Randomly creates offset-based ranges (up to [`max_num_ranges`])
/// for a string of length `text_len`. The ranges are disjoint
/// and ordered by the start point.
fn random_ranges<T>(rng: &mut T, max_num_ranges: usize, text_len: usize) -> Vec<Range<CharOffset>>
where
    T: Rng,
{
    let mut new_ranges: Vec<Range<CharOffset>> = Vec::new();
    let mut start = 0;
    while new_ranges.len() < max_num_ranges && start <= text_len {
        let new_start = rng.gen_range(start..=text_len);
        let new_end = rng.gen_range(new_start..=text_len);
        new_ranges.push(new_start.into()..new_end.into());
        start = new_end + 1;
    }
    new_ranges
}
