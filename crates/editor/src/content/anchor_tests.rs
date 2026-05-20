use std::cmp::Ordering;

use sum_tree::SumTree;
use warpui::App;

use super::{AnchorSide, Anchors};

use crate::content::{
    anchor::{Anchor, AnchorUpdate},
    buffer::Buffer,
    cursor::BufferSumTree,
    selection_model::BufferSelectionModel,
    text::IndentBehavior,
};
use string_offset::CharOffset;

#[test]
fn test_anchor_cleanup() {
    let mut anchors = Anchors::new();
    let a = anchors.create_anchor(3.into(), AnchorSide::Right);
    let b = anchors.create_anchor(4.into(), AnchorSide::Right);

    // Both anchors are live at this point.
    assert_eq!(anchors.anchors.len(), 2);

    // If an anchor is dropped, it is garbage-collected.
    drop(a);
    anchors.update(AnchorUpdate {
        start: CharOffset::zero(),
        old_character_count: 0,
        new_character_count: 0,
        clamp: false,
    });
    assert_eq!(anchors.anchors.len(), 1);

    // However, the other anchor should still resolve.
    assert_eq!(anchors.resolve(&b), Some(4.into()));

    // If an anchor is cloned, the clone keeps it alive.
    let b2 = b.clone();
    drop(b);
    anchors.update(AnchorUpdate {
        start: CharOffset::zero(),
        old_character_count: 0,
        new_character_count: 0,
        clamp: false,
    });
    assert_eq!(anchors.resolve(&b2), Some(4.into()));
}

#[test]
fn test_insert() {
    let mut anchors = Anchors::new();
    let before = anchors.create_anchor(3.into(), AnchorSide::Right);
    let cursor = anchors.create_anchor(6.into(), AnchorSide::Right);
    let cursor_left = anchors.create_anchor(6.into(), AnchorSide::Left);
    let after = anchors.create_anchor(9.into(), AnchorSide::Right);

    // Simulate typing a character at the cursor.
    anchors.update(AnchorUpdate {
        start: CharOffset::from(6),
        old_character_count: 0,
        new_character_count: 1,
        clamp: false,
    });

    // The anchor before the edit is unaffected.
    assert_eq!(anchors.resolve(&before), Some(3.into()));

    // The anchor _at_ the cursor increases, to be at the new cursor location.
    assert_eq!(anchors.resolve(&cursor), Some(7.into()));

    // The anchor _at_ the cursor with AnchorSide::Left stays at its old location.
    assert_eq!(anchors.resolve(&cursor_left), Some(6.into()));

    // The anchor after the cursor is also shifted down.
    assert_eq!(anchors.resolve(&after), Some(10.into()));

    // We should be able to type more text, and the anchors continue to update.
    anchors.update(AnchorUpdate {
        start: CharOffset::from(7),
        old_character_count: 0,
        new_character_count: 3,
        clamp: false,
    });
    assert_eq!(anchors.resolve(&cursor), Some(10.into()));
    assert_eq!(anchors.resolve(&after), Some(13.into()));
}

#[test]
fn test_backspace() {
    let mut anchors = Anchors::new();
    let before = anchors.create_anchor(3.into(), AnchorSide::Right);
    let cursor = anchors.create_anchor(6.into(), AnchorSide::Right);
    let after = anchors.create_anchor(9.into(), AnchorSide::Right);

    // Simulate backspacing at the cursor.
    anchors.update(AnchorUpdate {
        start: CharOffset::from(5),
        old_character_count: 1,
        new_character_count: 0,
        clamp: false,
    });

    // The anchor before the edit is unaffected.
    assert_eq!(anchors.resolve(&before), Some(3.into()));

    // The cursor and the anchor after it both shift by 1.
    assert_eq!(anchors.resolve(&cursor), Some(5.into()));
    assert_eq!(anchors.resolve(&after), Some(8.into()));
}

#[test]
fn test_invalidate_anchor() {
    let mut anchors = Anchors::new();
    let inside = anchors.create_anchor(4.into(), AnchorSide::Right);
    let outside = anchors.create_anchor(3.into(), AnchorSide::Right);

    let outside_anchor_right = anchors.create_anchor(5.into(), AnchorSide::Right);
    let outside_anchor_left = anchors.create_anchor(5.into(), AnchorSide::Left);

    // If we delete text including one of the anchors, it's invalidated.
    anchors.update(AnchorUpdate {
        start: CharOffset::from(3),
        old_character_count: 2,
        new_character_count: 0,
        clamp: false,
    });
    assert_eq!(anchors.resolve(&inside), None);

    // However, the anchor just before it is unaffected.
    assert_eq!(anchors.resolve(&outside), Some(3.into()));

    // The anchor on the right side is updated because it is equal to the old character range.
    assert_eq!(anchors.resolve(&outside_anchor_right), Some(3.into()));

    // The anchor on the left side should still be valid because it is equal to the old character range.
    assert_eq!(anchors.resolve(&outside_anchor_left), Some(3.into()));

    // If clamp is set to true, we want to clamp instead of invalidate anchors.
    let inside = anchors.create_anchor(4.into(), AnchorSide::Right);
    anchors.update(AnchorUpdate {
        start: CharOffset::from(3),
        old_character_count: 2,
        new_character_count: 0,
        clamp: true,
    });
    assert_eq!(anchors.resolve(&inside), Some(3.into()));
}

#[test]
fn test_update_anchor() {
    let mut anchors = Anchors::new();
    let anchor = anchors.create_anchor(4.into(), AnchorSide::Right);

    anchors.update_anchor(&anchor, CharOffset::from(3));
    assert_eq!(anchors.resolve(&anchor), Some(3.into()));
}

#[test]
#[should_panic(expected = "AnchorId(2) has offset 5, but buffer length is 4")]
fn test_validate_anchor_out_of_bounds() {
    let mut anchors = Anchors::new();
    let _valid_anchor = anchors.create_anchor(2.into(), AnchorSide::Right);
    // An invalid, but dead, anchor.
    let _ = anchors.create_anchor(100.into(), AnchorSide::Right);
    let _invalid_anchor = anchors.create_anchor(5.into(), AnchorSide::Right);

    let mut tree = SumTree::new();
    tree.append_str("abcd");

    anchors.validate(&tree);
}

#[test]
fn test_validate_anchors_ok() {
    let mut anchors = Anchors::new();
    let _valid_anchor = anchors.create_anchor(2.into(), AnchorSide::Right);
    // An invalid, but dead, anchor.
    let _ = anchors.create_anchor(100.into(), AnchorSide::Right);

    let mut tree = SumTree::new();
    tree.append_str("abcd");

    // This should not panic.
    anchors.validate(&tree);
}

#[test]
fn test_anchor_comparison() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::zero()..CharOffset::zero(),
                "some text",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let first = selection.update(ctx, |selection, _| {
                selection.create_anchor(CharOffset::from(1), AnchorSide::Right)
            });

            // Anchors should be equal to themselves.
            assert_eq!(
                first.cmp(&first.clone(), selection.as_ref(ctx)),
                Some(Ordering::Equal)
            );

            // Anchors should be equal to other anchors with the same offset.
            let first2 = selection.update(ctx, |selection, _| {
                selection.create_anchor(CharOffset::from(1), AnchorSide::Right)
            });
            assert_ne!(first.id, first2.id);
            assert_eq!(
                first.cmp(&first2, selection.as_ref(ctx)),
                Some(Ordering::Equal)
            );

            // Unequal anchors compare by offset.
            let second = selection.update(ctx, |selection, _| {
                selection.create_anchor(CharOffset::from(5), AnchorSide::Right)
            });
            assert_eq!(
                first.cmp(&second, selection.as_ref(ctx)),
                Some(Ordering::Less)
            );
            assert_eq!(
                second.cmp(&first, selection.as_ref(ctx)),
                Some(Ordering::Greater)
            );

            // Invalid anchors do not compare - delete the range containing `second`.
            buffer.edit_internal_first_selection(
                CharOffset::from(3)..CharOffset::from(7),
                "",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(second.cmp(&first, selection.as_ref(ctx)), None);
            assert_eq!(first.cmp(&second, selection.as_ref(ctx)), None);
        });
    });
}

impl Anchor {
    pub fn cmp(&self, other: &Anchor, selection: &BufferSelectionModel) -> Option<Ordering> {
        if self.id == other.id {
            Some(Ordering::Equal)
        } else {
            Some(
                selection
                    .resolve_anchor(self)?
                    .cmp(&selection.resolve_anchor(other)?),
            )
        }
    }
}
