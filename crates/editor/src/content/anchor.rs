use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use sum_tree::SumTree;

use string_offset::CharOffset;

use super::text::BufferText;

#[cfg(test)]
#[path = "anchor_tests.rs"]
mod test;

/// Handle to a particular anchor. As long as there is an active handle, the
/// anchor will be kept in sync with text edits. Once all handles are dropped,
/// the anchor is lazily disposed.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    id: AnchorId,
    /// Reference to keep this anchor alive. See [`AnchorReference`].
    reference: Arc<AnchorReference>,
}

/// Empty type for keeping anchor references alive.
///
/// We use `Arc<AnchorReference>` to implement reference-counting for anchors.
/// External anchor handles ([`Anchor`]) have strong references to their
/// `AnchorReference`, so it lives as long as there is an existing anchor.
/// Internally, we store a `Weak<AnchorReference>`, solely to be able to check
/// the strong reference count and clean up unused anchors.
///
/// ### Why not count references directly?
/// We could implement our own reference counting using an atomic integer. However,
/// in order to pass that atomic integer to callers and create [`Anchor`] handles,
/// we'd have to wrap it in an `Arc` or similar anyways - [`Anchor`] and [`Anchors`]
/// need a safe way to refer to the same memory!
///
/// ### Why not wrap [`AnchorState`] in an [`Arc`]?
/// We could have [`Anchor`]s strongly own their state, while keeping a [`Weak`]
/// reference here. When updating anchors, we'd filter out any that can't
/// be upgraded to a strong reference. However, we still need mutable access to
/// the character offset of each anchor - that's simpler if [`Anchors`] owns
/// all the mutable state. In addition, we want all anchor dereferencing to
/// go through the model, so that the UI framework enforces consistency around
/// when updates are applied. If [`Anchor`]s could be resolved without a model
/// handle, we don't know what state of the world they see.
type AnchorReference = ();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AnchorId(usize);

#[derive(Debug, Clone, Copy)]
pub struct AnchorUpdate {
    pub start: CharOffset,
    pub old_character_count: usize,
    pub new_character_count: usize,
    pub clamp: bool,
}

/// Used to tie-break when an update happens exactly at the position of the anchor.
/// When set to AnchorSide::Left, updates happening at the exact position won't shift
/// the anchor offset. When set to AnchorSide::Right, they will shift the offset.
///
/// For example, there is an anchor at CharOffset(2) with an incoming update update(
/// start=2, old_char_count=0, new_char_count=1). If the anchor has AnchorSide::Left, it
/// will stay at CharOffset(2). If the anchor has AnchorSide::Right, it will shift to CharOffset(3).
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum AnchorSide {
    Left,
    Right,
}

/// Internal state for a specific anchor.
struct AnchorState {
    /// Weak reference to detect when this anchor is no longer strongly
    /// referenced and should be removed. See [`AnchorReference`].
    live: Weak<AnchorReference>,
    /// The current character offset that this anchor points to.
    offset: CharOffset,
    side: AnchorSide,
}

/// Component of the buffer model that tracks relative anchors into the content.
///
/// Unlike point-in-time character offsets, anchors shift as text is added or
/// removed around them.
pub(crate) struct Anchors {
    next_id: usize,
    anchors: HashMap<AnchorId, AnchorState>,
}

impl Anchors {
    pub fn new() -> Anchors {
        Self {
            next_id: 0,
            anchors: HashMap::new(),
        }
    }

    /// Update an existing anchor to a new offset.
    pub fn update_anchor(&mut self, anchor: &Anchor, offset: CharOffset) {
        if let Some(anchor) = self.anchors.get_mut(&anchor.id) {
            anchor.offset = offset;
        }
    }

    /// Create a new anchor starting at the given offset.
    pub fn create_anchor(&mut self, offset: CharOffset, side: AnchorSide) -> Anchor {
        let id = AnchorId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);

        let reference = Arc::new(());
        self.anchors.insert(
            id,
            AnchorState {
                live: Arc::downgrade(&reference),
                offset,
                side,
            },
        );

        Anchor { id, reference }
    }

    /// Update all live anchors to reflect replacing `old_character_count`
    /// characters at `start_offset` with `new_character_count` characters.
    ///
    /// This will also dispose of any anchors that are no longer live if clamp is false.
    /// If clamp is set to true, the anchors in an deleted range will be clamped to the new
    /// end instead.
    pub fn update(&mut self, update: AnchorUpdate) {
        let AnchorUpdate {
            start,
            old_character_count,
            new_character_count,
            clamp,
        } = update;

        let old_end = start + old_character_count;
        let new_end = start + new_character_count;

        self.anchors.retain(|_, state| {
            if !state.is_live() {
                return false;
            }

            // There are 3 cases for an anchor's location relative to an edit:
            // 1. It's after the original end location, and needs to be shifted by the edit delta.
            // 2. It's within the deleted text, and should be removed.
            // 3. It's not affected by the edit (either before it or within overwritten text).
            //
            // Consider the following net insertion over d..e (ignore spaces):
            //    a b c|d e|f g h   => a b c|x y z|f g h
            //     ^     ^     ^    =>  ^     ^       ^
            //     1     2     3    =>  1     2       3
            // * Anchor 1 should not move.
            // * Anchor 2 should not move, even though it's now between x..y
            //   instead of d..e
            // * Anchor 3 moves up by 1 character
            //
            // Likewise, consider a net deletion, going from def to z:
            //  a b c|d e f|g h i  => a b c|z|g h i
            //   ^     ^ ^   ^     =>  ^     ^ ^
            //   1     2 3   4     =>  1     2 4
            // * Again, anchor 1 does not move
            // * Anchor 2 does not move - it's within the affected range, now after
            //   z instead of d, but was not deleted.
            // * Anchor 3 referred to content that no longer exists at all, so it
            //   is removed.
            // * Anchor 4 moves down by 2 characters, since it's fully after the
            //   affected range.

            if state.offset > old_end
                || (matches!(state.side, AnchorSide::Right) && state.offset == old_end)
            {
                // Overall, we need to adjust the offset by the difference between
                // the old and new lengths: state.offset += new_end - range.end.
                // Since we're dealing with unsigned integers, regrouping as
                // state.offset = (state.offset + new_end) - range.end avoids
                // underflow. It will overflow if state.offset + new_end is
                // greater than usize::MAX, but we do not expect that in practice.
                state.offset = (state.offset + new_end) - old_end;

                true
            } else {
                // We want to clamp instead of invalidate anchors if:
                // 1) Clamp is set to true.
                // 2) If an anchor is exactly at the old_end offset and is pegged to the
                // left side, we should still retain the anchor.
                if clamp && state.offset > new_end
                    || (state.offset == old_end
                        && state.offset > new_end
                        && matches!(state.side, AnchorSide::Left))
                {
                    state.offset = new_end;
                    return true;
                }
                // If we're in this branch, the anchor is either unaffected or
                // should be removed.
                state.offset <= new_end
            }
        });
    }

    /// Resolve an anchor to the character offset it currently points to.
    pub fn resolve(&self, anchor: &Anchor) -> Option<CharOffset> {
        // The anchor may have been removed by an edit. However, we don't need
        // to check liveness, because the fact that an anchor exists to call this
        // function means that it is live.
        self.anchors.get(&anchor.id).map(|state| state.offset)
    }

    /// Validates all anchors against the content they reference.
    pub fn validate(&self, content: &SumTree<BufferText>) {
        let content_length: CharOffset = content.extent();
        for (id, anchor) in self.anchors.iter() {
            if anchor.is_live() {
                assert!(
                    anchor.offset <= content_length,
                    "{id:?} has offset {}, but buffer length is {content_length}",
                    anchor.offset
                );
            }
        }
    }
}

impl AnchorState {
    fn is_live(&self) -> bool {
        self.live.strong_count() > 0
    }
}

impl Default for Anchors {
    fn default() -> Self {
        Self::new()
    }
}
