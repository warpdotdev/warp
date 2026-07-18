use std::cmp::Ordering;
use std::sync::Arc;

use arrayvec::ArrayVec;
use warp_errors::report_error;

use super::*;

#[derive(Clone, Debug)]
struct StackEntry<'a, T: Item, S, U> {
    tree: &'a SumTree<T>,
    index: usize,
    seek_dimension: S,
    sum_dimension: U,
}

/// Cursors allow you to navigate across SumTrees and access items or subtrees
/// from a given point.
///
/// It works something like this...imagine you have a sumtree of ints with a simple
/// count dimension with 5 items:
///
/// | Item 1 | Item 2 | Item 3 | Item 4 | Item 5 |
/// 1        2        3        4        5        6
///
/// The cursor navigates along the *bars* in the above diagram, and there are
/// 6 positions it can be in, corresponding to the numbers below the bars.
///
/// It starts by default at position 1, and you can move it using the
/// seek API.
///
/// Think of seeking as positioning the cursor, but using a query along one
/// of the dimensions to figure out where it goes.  In the above example, assume
/// there was also a Sum dimension on the items so that the tree looks like:
///
/// | Item 1 | Item 2 | Item 3 | Item 4 | Item 5 |
/// 1        2        3        4        5        6 (Count)
/// 1        3        6        10       15       21 (Sum)
///
/// The cool thing about seek is that you can position the cursor based on
/// Sum, so if you seeked to where the sum is 12, the cursor would be at Item 4.
///
/// One other thing to note, is that if you happen to seek exactly to the border
/// of two items, you an control which one the cursor lands using SeekBias.
/// SeekBias::Left will position the cursor at the item to the left, and vice-versa
/// for SeekBias::Right.
///
/// Once the cursor is at a position, you can take various actions:
/// * You can access the item at that position
/// * You can access a summary from the sum tree at the *start* or *end* point of the cursor
/// * You can move forwards or backwards one item at a time
/// * You can make a new sumtree from everything after the cursor (the suffix)
/// * You can make a new sumtree by splicing from the current position to some arbitrary end point
#[derive(Clone, Debug)]
pub struct Cursor<'a, T: Item, S, U> {
    tree: &'a SumTree<T>,
    stack: ArrayVec<StackEntry<'a, T, S, U>, 16>,
    seek_dimension: S,
    sum_dimension: U,
    did_seek: bool,
    at_end: bool,
}

/// Whether to clamp the max of the dimension while seeking
enum ClampMax {
    /// Yes means that you will not seek past the end of the last item
    /// in the tree
    Yes,

    /// No means that you may seek past the last item, in which case the cursor
    /// will not be set.
    No,
}

impl<'a, T, S, U> Cursor<'a, T, S, U>
where
    T: Item,
    S: Dimension<'a, T::Summary>,
    U: Dimension<'a, T::Summary>,
{
    pub fn new(tree: &'a SumTree<T>) -> Self {
        Self {
            tree,
            stack: ArrayVec::new(),
            seek_dimension: S::default(),
            sum_dimension: U::default(),
            did_seek: false,
            at_end: false,
        }
    }

    fn reset(&mut self) {
        self.did_seek = false;
        self.at_end = false;
        self.stack.truncate(0);
        self.seek_dimension = S::default();
        self.sum_dimension = U::default();
    }

    /// Returns the seek dimension summary at the start of the current cursor position (right
    /// before whatever item it is at)
    pub fn seek_position(&self) -> &S {
        &self.seek_dimension
    }

    /// Returns the seek dimension summary at the end of the current cursor position (right
    /// after whatever item it is at)
    pub fn end_seek_position(&self) -> S {
        if let Some(item_summary) = self.item_summary() {
            let mut end = self.seek_position().clone();
            end.add_summary(item_summary);
            end
        } else {
            self.seek_position().clone()
        }
    }

    /// Returns a summary at the start of the current cursor position (right
    /// before whatever item it is at)
    pub fn start(&self) -> &U {
        &self.sum_dimension
    }

    /// Returns a summary at the end of the current cursor position (right
    /// after whatever item it is at)
    pub fn end(&self) -> U {
        if let Some(item_summary) = self.item_summary() {
            let mut end = self.start().clone();
            end.add_summary(item_summary);
            end
        } else {
            self.start().clone()
        }
    }

    /// Returns the item at the current cursor position if there is one.
    /// It's an error to call this without seeking.
    pub fn item(&self) -> Option<&'a T> {
        debug_assert!(self.did_seek, "Must seek before calling this method");
        if let Some(entry) = self.stack.last() {
            match *entry.tree.0 {
                Node::Leaf { ref items, .. } => {
                    if entry.index == items.len() {
                        None
                    } else {
                        Some(&items[entry.index])
                    }
                }
                _ => {
                    log::warn!("item: The last item in the cursor stack is not a leaf.",);
                    if cfg!(debug_assertions) {
                        log::warn!("Current cursor state: {self:#?}");
                    }
                    None
                }
            }
        } else {
            None
        }
    }

    fn item_summary(&self) -> Option<&'a T::Summary> {
        debug_assert!(self.did_seek, "Must seek before calling this method");
        if let Some(entry) = self.stack.last() {
            match *entry.tree.0 {
                Node::Leaf {
                    ref item_summaries, ..
                } => {
                    if entry.index == item_summaries.len() {
                        None
                    } else {
                        Some(&item_summaries[entry.index])
                    }
                }
                _ => {
                    report_error!("item_summary: The last item in the cursor stack is not a leaf");
                    None
                }
            }
        } else {
            None
        }
    }

    /// Returns the item prior to where the cursor is positioned.
    /// It is an error to call this method without first seeking.
    pub fn prev_item(&self) -> Option<&'a T> {
        debug_assert!(self.did_seek, "Must seek before calling this method");
        if let Some(entry) = self.stack.last() {
            if entry.index == 0 {
                self.prev_leaf()
                    .map(|prev_leaf| prev_leaf.0.items().last().unwrap())
            } else {
                match *entry.tree.0 {
                    Node::Leaf { ref items, .. } => Some(&items[entry.index - 1]),
                    _ => {
                        report_error!("The last item in the cursor stack is not a leaf");
                        None
                    }
                }
            }
        } else if self.at_end {
            self.tree.last()
        } else {
            None
        }
    }

    fn prev_leaf(&self) -> Option<&'a SumTree<T>> {
        for entry in self.stack.iter().rev().skip(1) {
            if entry.index != 0 {
                match *entry.tree.0 {
                    Node::Internal {
                        ref child_trees, ..
                    } => return Some(child_trees[entry.index - 1].rightmost_leaf()),
                    Node::Leaf { .. } => {
                        report_error!("A leaf is not the last item in the cursor stack");
                        return None;
                    }
                };
            }
        }
        None
    }

    /// Moves the cursor back one position.
    /// You must first seek before calling this method.
    #[allow(dead_code)]
    pub fn prev(&mut self) {
        debug_assert!(self.did_seek, "Must seek before calling this method");

        if self.at_end {
            self.seek_dimension = S::default();
            self.sum_dimension = U::default();
            self.descend_to_last_item(self.tree);
            self.at_end = false;
        } else {
            while let Some(entry) = self.stack.pop() {
                if entry.index > 0 {
                    let new_index = entry.index - 1;

                    if let Some(StackEntry {
                        seek_dimension,
                        sum_dimension,
                        ..
                    }) = self.stack.last()
                    {
                        self.seek_dimension = seek_dimension.clone();
                        self.sum_dimension = sum_dimension.clone();
                    } else {
                        self.seek_dimension = S::default();
                        self.sum_dimension = U::default();
                    }

                    match entry.tree.0.as_ref() {
                        Node::Internal {
                            child_trees,
                            child_summaries,
                            ..
                        } => {
                            for summary in &child_summaries[0..new_index] {
                                self.seek_dimension.add_summary(summary);
                                self.sum_dimension.add_summary(summary);
                            }
                            self.stack.push(StackEntry {
                                tree: entry.tree,
                                index: new_index,
                                seek_dimension: self.seek_dimension.clone(),
                                sum_dimension: self.sum_dimension.clone(),
                            });
                            self.descend_to_last_item(&child_trees[new_index]);
                        }
                        Node::Leaf { item_summaries, .. } => {
                            for item_summary in &item_summaries[0..new_index] {
                                self.seek_dimension.add_summary(item_summary);
                                self.sum_dimension.add_summary(item_summary);
                            }
                            self.stack.push(StackEntry {
                                tree: entry.tree,
                                index: new_index,
                                seek_dimension: self.seek_dimension.clone(),
                                sum_dimension: self.sum_dimension.clone(),
                            });
                        }
                    }

                    break;
                }
            }
        }
    }

    /// Moves the cursor forward one position.
    /// You must first seek before calling this method.
    pub fn next(&mut self) {
        self.next_internal(|_| true)
    }

    fn next_internal<F>(&mut self, filter_node: F)
    where
        F: Fn(&T::Summary) -> bool,
    {
        debug_assert!(self.did_seek, "Must seek before calling this method");

        if self.stack.is_empty() {
            if !self.at_end {
                self.descend_to_first_item(self.tree, filter_node);
            }
        } else {
            while !self.stack.is_empty() {
                let new_subtree = {
                    let entry = self.stack.last_mut().unwrap();
                    match entry.tree.0.as_ref() {
                        Node::Internal {
                            child_trees,
                            child_summaries,
                            ..
                        } => {
                            while entry.index < child_summaries.len() {
                                entry
                                    .seek_dimension
                                    .add_summary(&child_summaries[entry.index]);
                                entry
                                    .sum_dimension
                                    .add_summary(&child_summaries[entry.index]);

                                entry.index += 1;
                                if let Some(next_summary) = child_summaries.get(entry.index) {
                                    if filter_node(next_summary) {
                                        break;
                                    } else {
                                        self.seek_dimension.add_summary(next_summary);
                                        self.sum_dimension.add_summary(next_summary);
                                    }
                                }
                            }

                            child_trees.get(entry.index)
                        }
                        Node::Leaf { item_summaries, .. } => loop {
                            let item_summary = &item_summaries[entry.index];
                            self.seek_dimension.add_summary(item_summary);
                            entry.seek_dimension.add_summary(item_summary);
                            self.sum_dimension.add_summary(item_summary);
                            entry.sum_dimension.add_summary(item_summary);
                            entry.index += 1;
                            if let Some(next_item_summary) = item_summaries.get(entry.index) {
                                if filter_node(next_item_summary) {
                                    return;
                                }
                            } else {
                                break None;
                            }
                        },
                    }
                };

                if let Some(subtree) = new_subtree {
                    self.descend_to_first_item(subtree, filter_node);
                    break;
                } else {
                    self.stack.pop();
                }
            }
        }

        self.at_end = self.stack.is_empty();
    }

    pub fn descend_to_first_item<F>(&mut self, mut subtree: &'a SumTree<T>, filter_node: F)
    where
        F: Fn(&T::Summary) -> bool,
    {
        self.did_seek = true;
        loop {
            subtree = match *subtree.0 {
                Node::Internal {
                    ref child_trees,
                    ref child_summaries,
                    ..
                } => {
                    let mut new_index = None;
                    for (index, summary) in child_summaries.iter().enumerate() {
                        if filter_node(summary) {
                            new_index = Some(index);
                            break;
                        }
                        self.seek_dimension.add_summary(summary);
                        self.sum_dimension.add_summary(summary);
                    }

                    if let Some(new_index) = new_index {
                        self.stack.push(StackEntry {
                            tree: subtree,
                            index: new_index,
                            seek_dimension: self.seek_dimension.clone(),
                            sum_dimension: self.sum_dimension.clone(),
                        });
                        &child_trees[new_index]
                    } else {
                        break;
                    }
                }
                Node::Leaf {
                    ref item_summaries, ..
                } => {
                    let mut new_index = None;
                    for (index, item_summary) in item_summaries.iter().enumerate() {
                        if filter_node(item_summary) {
                            new_index = Some(index);
                            break;
                        }
                        self.seek_dimension.add_summary(item_summary);
                        self.sum_dimension.add_summary(item_summary);
                    }

                    if let Some(new_index) = new_index {
                        self.stack.push(StackEntry {
                            tree: subtree,
                            index: new_index,
                            seek_dimension: self.seek_dimension.clone(),
                            sum_dimension: self.sum_dimension.clone(),
                        });
                    }
                    break;
                }
            }
        }
    }

    pub fn descend_to_last_item(&mut self, mut subtree: &'a SumTree<T>) {
        self.did_seek = true;
        loop {
            match subtree.0.as_ref() {
                Node::Internal {
                    child_trees,
                    child_summaries,
                    ..
                } => {
                    for summary in &child_summaries[0..child_summaries.len() - 1] {
                        self.seek_dimension.add_summary(summary);
                        self.sum_dimension.add_summary(summary);
                    }

                    self.stack.push(StackEntry {
                        tree: subtree,
                        index: child_trees.len() - 1,
                        seek_dimension: self.seek_dimension.clone(),
                        sum_dimension: self.sum_dimension.clone(),
                    });
                    subtree = child_trees.last().unwrap();
                }
                Node::Leaf { item_summaries, .. } => {
                    let last_index = item_summaries.len().saturating_sub(1);
                    for item_summary in &item_summaries[0..last_index] {
                        self.seek_dimension.add_summary(item_summary);
                        self.sum_dimension.add_summary(item_summary);
                    }
                    self.stack.push(StackEntry {
                        tree: subtree,
                        index: last_index,
                        seek_dimension: self.seek_dimension.clone(),
                        sum_dimension: self.sum_dimension.clone(),
                    });
                    break;
                }
            }
        }
    }
}

impl<'a, T, S, U> Cursor<'a, T, S, U>
where
    T: Item,
    S: Dimension<'a, T::Summary> + Ord,
    U: Dimension<'a, T::Summary>,
{
    /// Seeks the cursor to a specific location in the sumtree. If the location is at an exact
    /// boundary between two elements, `bias` is used to break ties.
    /// Returns whether we were able to successfully seek to the target position.
    pub fn seek(&mut self, pos: &S, bias: SeekBias) -> bool {
        self.reset();
        self.seek_internal::<()>(pos, bias, &mut SeekAggregate::None, ClampMax::No)
    }

    /// Seeks the cursor to a specific location in the sumtree. If the location is at an exact
    /// boundary between two elements, `bias` is used to break ties.  Clamps to the max
    /// of the dimension when seeking.
    pub fn seek_clamped(&mut self, pos: &S, bias: SeekBias) {
        self.reset();
        self.seek_internal::<()>(pos, bias, &mut SeekAggregate::None, ClampMax::Yes);
    }

    /// Seeks the cursor to `end`, returning a new subtree of all the items between the cursor up
    /// to, but not including, `end`.
    pub fn slice(&mut self, end: &S, bias: SeekBias) -> SumTree<T> {
        let mut slice = SeekAggregate::Slice(SumTree::new());
        self.seek_internal::<()>(end, bias, &mut slice, ClampMax::No);
        if let SeekAggregate::Slice(slice) = slice {
            slice
        } else {
            unreachable!("slice: seek aggregate must be a slice")
        }
    }

    /// Seeks the cursor to the very end of the sum tree, returning a new subtree of all the items
    /// from the cursor to the end of the tree.
    pub fn suffix(&mut self) -> SumTree<T> {
        let extent = self.tree.extent::<S>();
        let mut slice = SeekAggregate::Slice(SumTree::new());
        self.seek_internal::<()>(&extent, SeekBias::Right, &mut slice, ClampMax::No);
        if let SeekAggregate::Slice(slice) = slice {
            slice
        } else {
            unreachable!("suffix: seek aggregate must be a slice")
        }
    }

    /// Returns a summary from the current position to the given end dimension.
    pub fn summary<D>(&mut self, end: &S, bias: SeekBias) -> D
    where
        D: Dimension<'a, T::Summary>,
    {
        let mut summary = SeekAggregate::Summary(D::default());
        self.seek_internal(end, bias, &mut summary, ClampMax::No);
        if let SeekAggregate::Summary(summary) = summary {
            summary
        } else {
            unreachable!("summary: seek aggregate must be a summary")
        }
    }

    fn seek_internal<D>(
        &mut self,
        target: &S,
        bias: SeekBias,
        aggregate: &mut SeekAggregate<T, D>,
        clamp_max: ClampMax,
    ) -> bool
    where
        D: Dimension<'a, T::Summary>,
    {
        if cfg!(debug_assertions) && target < &self.seek_dimension {
            log::warn!(
                "Out-of-bounds target {:?} given seek_dimension {:?}",
                target,
                self.seek_dimension
            );
            panic!("target should be >= self.seek_dimension");
        }
        let mut containing_subtree = None;

        // This first path accounts for the case where a seek has already taken place.
        // If you are reading this code for the first time, it's more helpful to look
        // at the code in the not-already-seeked-case first (see below).
        //
        // In the already seeked case, we have a prebuilt stack that has at its end
        // some leaf node which represents a cursor position.
        //
        // Note that, confusingly, we only re-seek using this path in the case of slice,
        // suffix, or summary, which, importantly if you're trying to understand this code path,
        // are actions which can only move the cursor position rightward.
        //
        // For the rightward case, there are two options:
        //
        // First, it could be to the right in the current leaf node,
        // in which case you just adjust the entry index in the leaf to point to the right position.
        //
        // Second, it could be further to the right than is summarized in the current leaf.
        // In that case, you need to move up the tree, which
        // means you need to pop an element off the stack and search there, and that the
        // element is contained in a different subtree.  The internal node case will identify that subtree
        // and then pass it into the logic for the "never seeked" case, which will descend that subtree
        // until it finds the right element.
        //
        // Note that we don't need special handling for
        // https://linear.app/warpdotdev/issue/WAR-5942/sumtree-has-consistency-issues-with-floats
        // in the already seeked case - it's only an issue when descending the tree, not when
        // unwinding the stack.
        if self.did_seek {
            'outer: while let Some(entry) = self.stack.last_mut() {
                {
                    match *entry.tree.0 {
                        Node::Internal {
                            ref child_summaries,
                            ref child_trees,
                            ..
                        } => {
                            entry.index += 1;
                            for (child_tree, child_summary) in child_trees[entry.index..]
                                .iter()
                                .zip(&child_summaries[entry.index..])
                            {
                                let mut child_end = self.seek_dimension.clone();
                                child_end.add_summary(child_summary);

                                let at_last_item = entry.index == child_trees.len() - 1;
                                let comparison =
                                    if at_last_item && matches!(clamp_max, ClampMax::Yes) {
                                        target.min(&child_end).cmp(&child_end)
                                    } else {
                                        target.cmp(&child_end)
                                    };

                                if comparison == Ordering::Greater
                                    || (comparison == Ordering::Equal && bias == SeekBias::Right)
                                {
                                    self.seek_dimension.add_summary(child_summary);
                                    self.sum_dimension.add_summary(child_summary);
                                    match aggregate {
                                        SeekAggregate::None => {}
                                        SeekAggregate::Slice(slice) => {
                                            slice.push_tree(child_tree.clone());
                                        }
                                        SeekAggregate::Summary(summary) => {
                                            summary.add_summary(child_summary);
                                        }
                                    }
                                    entry.index += 1;
                                } else {
                                    containing_subtree = Some(child_tree);
                                    break 'outer;
                                }
                            }
                        }
                        Node::Leaf {
                            ref items,
                            ref item_summaries,
                            ..
                        } => {
                            let mut slice_items = ArrayVec::<T, { 2 * TREE_BASE }>::new();
                            let mut slice_item_summaries =
                                ArrayVec::<T::Summary, { 2 * TREE_BASE }>::new();
                            let mut slice_items_summary = match aggregate {
                                SeekAggregate::Slice(_) => Some(T::Summary::default()),
                                _ => None,
                            };

                            for (item, item_summary) in items[entry.index..]
                                .iter()
                                .zip(&item_summaries[entry.index..])
                            {
                                let mut item_end = self.seek_dimension.clone();
                                item_end.add_summary(item_summary);

                                let at_last_item = entry.index == items.len() - 1;
                                let comparison =
                                    if at_last_item && matches!(clamp_max, ClampMax::Yes) {
                                        target.min(&item_end).cmp(&item_end)
                                    } else {
                                        target.cmp(&item_end)
                                    };

                                if comparison == Ordering::Greater
                                    || (comparison == Ordering::Equal && bias == SeekBias::Right)
                                {
                                    self.seek_dimension.add_summary(item_summary);
                                    self.sum_dimension.add_summary(item_summary);
                                    match aggregate {
                                        SeekAggregate::None => {}
                                        SeekAggregate::Slice(_) => {
                                            slice_items.push(item.clone());
                                            slice_item_summaries.push(item_summary.clone());
                                            *slice_items_summary.as_mut().unwrap() += item_summary;
                                        }
                                        SeekAggregate::Summary(summary) => {
                                            summary.add_summary(item_summary);
                                        }
                                    }
                                    entry.index += 1;
                                } else {
                                    if let SeekAggregate::Slice(slice) = aggregate {
                                        slice.push_tree(SumTree(Arc::new(Node::Leaf {
                                            summary: slice_items_summary.unwrap(),
                                            items: slice_items,
                                            item_summaries: slice_item_summaries,
                                        })));
                                    }
                                    break 'outer;
                                }
                            }

                            if let SeekAggregate::Slice(slice) = aggregate {
                                if !slice_items.is_empty() {
                                    slice.push_tree(SumTree(Arc::new(Node::Leaf {
                                        summary: slice_items_summary.unwrap(),
                                        items: slice_items,
                                        item_summaries: slice_item_summaries,
                                    })));
                                }
                            }
                        }
                    }
                }

                self.stack.pop();
            }
        } else {
            self.did_seek = true;
            containing_subtree = Some(self.tree);
        }

        // At a high level, seeking works by navigating down the sumtree
        // searching the summaries at each level to find which one contains the
        // value along the dimension we are seeking for.  At every step in the descent, we
        // push the containing subtree onto a stack.  By the time we reach the
        // leaf node, the stack records the path of containing subtrees, along with a recording
        // of what index the contained value is at.
        // This allows a call to "item" to just look at the tree at the top of the stack
        // and use the index to return the correct value or summary at that point.
        //
        // If the seek is doing more than just positioning the cursor (e.g. if it's
        // summarizing the tree or splicing it, those summaries and splices are built
        // during the descent)
        if let Some(mut subtree) = containing_subtree {
            // This is a flag to check if the target dimension value is greater than the range of the current sub-tree.
            // This could happen when the parent node holds a larger total sum compared to the sum of all its
            // children nodes due to floating point precision error.
            let mut is_item_after_current_subtree = false;
            loop {
                let mut next_subtree = None;
                match *subtree.0 {
                    Node::Internal {
                        ref child_summaries,
                        ref child_trees,
                        ref summary,
                        ..
                    } => {
                        let mut max_end = self.seek_dimension.clone();
                        max_end.add_summary(summary);
                        for (index, (child_tree, child_summary)) in
                            child_trees.iter().zip(child_summaries).enumerate()
                        {
                            let mut child_end = self.seek_dimension.clone();
                            child_end.add_summary(child_summary);

                            let at_last_item = index == child_trees.len() - 1;
                            if at_last_item {
                                // Ensure that the child_end is at least as big as the summary's end.
                                // This handles https://linear.app/warpdotdev/issue/WAR-5942/sumtree-has-consistency-issues-with-floats
                                child_end = child_end.max(max_end.clone());
                            }
                            let comparison = if at_last_item && matches!(clamp_max, ClampMax::Yes) {
                                target.min(&child_end).cmp(&child_end)
                            } else {
                                target.cmp(&child_end)
                            };

                            // Whether the target is beyond the current internal node.
                            let target_beyond_node = comparison == Ordering::Greater
                                || (comparison == Ordering::Equal && bias == SeekBias::Right);

                            // When we have
                            // 1) Stack is not empty (there is a parent internal node with larger sum than target).
                            // 2) Seek target is beyond the last item of the current subtree.
                            //
                            // This means we have gotten to the floating point precision
                            // error state. In this case, we should descend to the right-most
                            // leaf and move our cursor to the next item.
                            if (self.stack.is_empty() || !at_last_item) && target_beyond_node {
                                self.seek_dimension.add_summary(child_summary);
                                self.sum_dimension.add_summary(child_summary);
                                match aggregate {
                                    SeekAggregate::None => {}
                                    SeekAggregate::Slice(slice) => {
                                        slice.push_tree(child_trees[index].clone());
                                    }
                                    SeekAggregate::Summary(summary) => {
                                        summary.add_summary(child_summary);
                                    }
                                }
                            } else {
                                // If we are at the last item and the target is actually
                                // beyond the current node, continue descending to the right
                                // most node and mark item_after_current_subtree to true.
                                if target_beyond_node {
                                    is_item_after_current_subtree = true;
                                }
                                self.stack.push(StackEntry {
                                    tree: subtree,
                                    index,
                                    seek_dimension: self.seek_dimension.clone(),
                                    sum_dimension: self.sum_dimension.clone(),
                                });
                                next_subtree = Some(child_tree);
                                break;
                            }
                        }
                    }
                    Node::Leaf {
                        ref items,
                        ref item_summaries,
                        ref summary,
                        ..
                    } => {
                        let mut slice_items = ArrayVec::<T, { 2 * TREE_BASE }>::new();
                        let mut slice_item_summaries =
                            ArrayVec::<T::Summary, { 2 * TREE_BASE }>::new();
                        let mut slice_items_summary = match aggregate {
                            SeekAggregate::Slice(_) => Some(T::Summary::default()),
                            _ => None,
                        };
                        let mut max_end = self.seek_dimension.clone();
                        max_end.add_summary(summary);

                        for (index, (item, item_summary)) in
                            items.iter().zip(item_summaries).enumerate()
                        {
                            let mut child_end = self.seek_dimension.clone();
                            child_end.add_summary(item_summary);

                            let at_last_item = index == items.len() - 1;
                            if at_last_item {
                                // Ensure that the child_end is at least as big as the summary's end.
                                // This handles https://linear.app/warpdotdev/issue/WAR-5942/sumtree-has-consistency-issues-with-floats
                                child_end = child_end.max(max_end.clone());
                            }
                            let comparison = if at_last_item && matches!(clamp_max, ClampMax::Yes) {
                                target.min(&child_end).cmp(&child_end)
                            } else {
                                target.cmp(&child_end)
                            };

                            // Whether the target is beyond the current leaf node.
                            let target_beyond_node = comparison == Ordering::Greater
                                || (comparison == Ordering::Equal && bias == SeekBias::Right);
                            if (self.stack.is_empty() || !at_last_item) && target_beyond_node {
                                self.seek_dimension.add_summary(item_summary);
                                self.sum_dimension.add_summary(item_summary);
                                match aggregate {
                                    SeekAggregate::None => {}
                                    SeekAggregate::Slice(_) => {
                                        slice_items.push(item.clone());
                                        *slice_items_summary.as_mut().unwrap() += item_summary;
                                        slice_item_summaries.push(item_summary.clone());
                                    }
                                    SeekAggregate::Summary(summary) => {
                                        summary.add_summary(item_summary);
                                    }
                                }
                            } else {
                                // If we are at the last item and the target is actually
                                // beyond the current leaf, add the item_summary to seek
                                // and sum dimension because the item is past the last leaf item.
                                // Mark item_after_current_subtree to true.
                                if target_beyond_node {
                                    self.seek_dimension.add_summary(item_summary);
                                    self.sum_dimension.add_summary(item_summary);
                                    is_item_after_current_subtree = true;
                                }
                                self.stack.push(StackEntry {
                                    tree: subtree,
                                    index,
                                    seek_dimension: self.seek_dimension.clone(),
                                    sum_dimension: self.sum_dimension.clone(),
                                });
                                break;
                            }
                        }

                        if let SeekAggregate::Slice(slice) = aggregate {
                            if !slice_items.is_empty() {
                                slice.push_tree(SumTree(Arc::new(Node::Leaf {
                                    summary: slice_items_summary.unwrap(),
                                    items: slice_items,
                                    item_summaries: slice_item_summaries,
                                })));
                            }
                        }
                    }
                };

                if let Some(next_subtree) = next_subtree {
                    subtree = next_subtree;
                } else {
                    break;
                }
            }

            // If is_item_after_current_subtree is true, this means the cursor is at
            // the right-most leaf node of the subtree and the target item is right after
            // it. Move the cursor to the next item to get the right target state.
            if is_item_after_current_subtree {
                self.next();
                return *target == self.seek_dimension;
            }
        }

        self.at_end = self.stack.is_empty();
        if bias == SeekBias::Left {
            let mut end = self.seek_dimension.clone();
            if let Some(summary) = self.item_summary() {
                end.add_summary(summary);
            }
            *target == end
        } else {
            *target == self.seek_dimension
        }
    }
}

impl<'a, T, S, U> Iterator for Cursor<'a, T, S, U>
where
    T: Item,
    S: Dimension<'a, T::Summary>,
    U: Dimension<'a, T::Summary>,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.did_seek {
            self.descend_to_first_item(self.tree, |_| true);
        }

        if let Some(item) = self.item() {
            self.next();
            Some(item)
        } else {
            None
        }
    }
}

impl<'a, T, S, U> DoubleEndedIterator for Cursor<'a, T, S, U>
where
    T: Item,
    S: Dimension<'a, T::Summary>,
    U: Dimension<'a, T::Summary>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if !self.did_seek {
            self.descend_to_last_item(self.tree);
        }

        // If we are past the last element, move back one position.
        if self.at_end {
            self.prev();
        }

        if let Some(item) = self.item() {
            self.prev();
            Some(item)
        } else {
            None
        }
    }
}

pub struct FilterCursor<'a, F: Fn(&T::Summary) -> bool, T: Item, U> {
    cursor: Cursor<'a, T, (), U>,
    filter_node: F,
}

impl<'a, F, T, U> FilterCursor<'a, F, T, U>
where
    F: Fn(&T::Summary) -> bool,
    T: Item,
    U: Dimension<'a, T::Summary>,
{
    pub fn new(tree: &'a SumTree<T>, filter_node: F) -> Self {
        let mut cursor = tree.cursor::<(), U>();
        if filter_node(&tree.summary()) {
            cursor.descend_to_first_item(tree, &filter_node);
        } else {
            cursor.did_seek = true;
            cursor.at_end = true;
        }

        Self {
            cursor,
            filter_node,
        }
    }

    pub fn start(&self) -> &U {
        self.cursor.start()
    }

    pub fn item(&self) -> Option<&'a T> {
        self.cursor.item()
    }

    pub fn next(&mut self) {
        self.cursor.next_internal(&self.filter_node);
    }
}

impl<'a, F, T, U> Iterator for FilterCursor<'a, F, T, U>
where
    F: Fn(&T::Summary) -> bool,
    T: Item,
    U: Dimension<'a, T::Summary>,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.item() {
            self.cursor.next_internal(&self.filter_node);
            Some(item)
        } else {
            None
        }
    }
}

enum SeekAggregate<T: Item, D> {
    None,
    Slice(SumTree<T>),
    Summary(D),
}
