use string_offset::CharOffset;
use vec1::Vec1;

use super::anchor::{Anchor, AnchorSide, Anchors};

#[derive(Clone)]
pub struct Selection {
    /// A head is where the cursor is and any arrow movement action only modifies the
    /// head of a selection.
    head: Anchor,
    tail: Anchor,
    bias: TextStyleBias,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStyleBias {
    InStyle,
    #[default]
    OutOfStyle,
}

impl Selection {
    pub fn new(head: Anchor, tail: Anchor) -> Self {
        Self {
            head,
            tail,
            bias: TextStyleBias::OutOfStyle,
        }
    }

    pub fn head(&self) -> &Anchor {
        &self.head
    }

    pub fn bias(&self) -> TextStyleBias {
        self.bias
    }

    pub fn tail(&self) -> &Anchor {
        &self.tail
    }

    pub(super) fn set_head(&mut self, anchors: &mut Anchors, head: CharOffset) {
        if anchors.resolve(&self.head).is_some() {
            anchors.update_anchor(&self.head, head);
        } else {
            let anchor = anchors.create_anchor(head, AnchorSide::Right);
            self.head = anchor;
        }
    }

    pub(super) fn set_tail(&mut self, anchors: &mut Anchors, tail: CharOffset) {
        if anchors.resolve(&self.tail).is_some() {
            anchors.update_anchor(&self.tail, tail);
        } else {
            let anchor = anchors.create_anchor(tail, AnchorSide::Right);
            self.tail = anchor;
        }
    }

    pub fn set_bias(&mut self, bias: TextStyleBias) {
        self.bias = bias;
    }
}

/// All active selections in the editor.
///
/// Create a new selection set with a single selection.  Note that there must
/// always be at least one selection in the set.
///
/// let selection_set = SelectionSet::new(selection);
#[derive(Clone)]
pub struct SelectionSet {
    selections: Vec1<Selection>,
}

impl SelectionSet {
    /// Create a new selection set with a single selection.  Note that there must
    /// always be at least one selection in the set.
    pub fn new(selection: Selection) -> Self {
        Self {
            selections: Vec1::new(selection),
        }
    }

    /// Return a reference to the first selection that was created.
    pub fn first(&self) -> &Selection {
        self.selections.first()
    }

    /// Return a mutable reference to the first selection that was created.
    pub fn first_mut(&mut self) -> &mut Selection {
        self.selections.first_mut()
    }

    pub fn last(&self) -> &Selection {
        self.selections.last()
    }

    pub fn last_mut(&mut self) -> &mut Selection {
        self.selections.last_mut()
    }

    /// Add a new selection to the set of selections.
    pub fn push(&mut self, selection: Selection) {
        self.selections.push(selection);
    }

    /// Remove all selections except the first one.
    pub fn truncate(&mut self) {
        self.selections
            .truncate(1)
            .expect("Truncating to literal 1 cannot fail");
    }

    pub fn len(&self) -> usize {
        self.selections.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }

    /// Map a function over the selections in the set, returning a new Vec1 of the results.
    pub fn selection_map<T, F>(&self, f: F) -> Vec1<T>
    where
        F: Fn(&Selection) -> T,
    {
        self.selections.mapped_ref(f)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.selections.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Selection> {
        self.selections.iter_mut()
    }
}

impl From<Vec1<Selection>> for SelectionSet {
    fn from(selections: Vec1<Selection>) -> SelectionSet {
        SelectionSet { selections }
    }
}

impl TryFrom<Vec<Selection>> for SelectionSet {
    type Error = vec1::Size0Error;

    fn try_from(selections: Vec<Selection>) -> Result<Self, Self::Error> {
        Vec1::try_from(selections).map(SelectionSet::from)
    }
}
