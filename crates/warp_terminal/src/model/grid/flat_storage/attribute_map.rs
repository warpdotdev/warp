use std::ops::RangeFrom;
use std::slice;

use get_size::GetSize;
use string_offset::ByteOffset;

/// A structure that efficiently stores and retrieves the value of some grid
/// attribute.
///
/// This internally coalesces ranges to store the data in a space-efficient
/// manner.
///
/// A sorted vector is used because we only ever append changes in order, and
/// iteration heavily outweighs random inserts on this path.
#[derive(Debug, Default, Clone)]
pub struct AttributeMap<A> {
    /// Stores a mapping between an _ending_ byte offset (inclusive) and the
    /// attribute value for the range ending at the given offset.
    map: Vec<(ByteOffset, A)>,
    /// The attribute value for all offsets beyond the last end offset stored
    /// in the map.
    tail_value: A,
}

impl<A> AttributeMap<A> {
    pub fn new(starting_value: A) -> Self {
        Self {
            map: Default::default(),
            tail_value: starting_value,
        }
    }

    /// Truncates the attribute map to the given content offset.
    pub fn truncate(&mut self, new_len: ByteOffset) {
        let split_idx = self.map.partition_point(|(offset, _)| *offset < new_len);
        let truncated_ranges = self.map.split_off(split_idx);
        if let Some((_, tail_value)) = truncated_ranges.into_iter().next() {
            self.tail_value = tail_value;
        }
    }

    /// Truncates the attribute map to start at the given content offset.
    pub fn truncate_front(&mut self, new_start_offset: ByteOffset) {
        let split_idx = self
            .map
            .partition_point(|(offset, _)| *offset < new_start_offset);
        self.map.drain(..split_idx);
    }

    /// Returns the end offset of the last range in the map.
    fn last_end_offset(&self) -> ByteOffset {
        if let Some((k, _v)) = self.map.last() {
            *k
        } else {
            ByteOffset::zero()
        }
    }
}

impl<A: PartialEq + std::fmt::Debug> AttributeMap<A> {
    /// Updates the map with the fact that the attribute value changes at the
    /// given byte offset.
    ///
    /// The start of the provided range must be after the end of the last range
    /// in the map.
    pub fn push_attribute_change(&mut self, range: RangeFrom<ByteOffset>, value: A) {
        if value == self.tail_value {
            return;
        }

        let prev_tail_value = std::mem::replace(&mut self.tail_value, value);

        if range.start == ByteOffset::zero() {
            debug_assert!(self.map.is_empty());
        } else {
            debug_assert!(
                range.start > self.last_end_offset(),
                "cannot push attribute change starting at {} when last end offset is {}.  attribute map: {:?}",
                range.start,
                self.last_end_offset(),
                self.map,
            );
            self.map.push((range.start - 1, prev_tail_value));
        }
    }
}

impl<A: GetSize> GetSize for AttributeMap<A> {
    fn get_heap_size(&self) -> usize {
        self.map.get_heap_size()
    }
}

impl<A: Copy> AttributeMap<A> {
    /// Returns an iterator over per-byte attribute values starting at the
    /// given byte offset.
    pub(super) fn iter_from(&self, start_offset: ByteOffset) -> Iter<'_, A> {
        Iter::new(self, start_offset)
    }

    /// Returns the tail (current) value of the given attribute.
    pub fn tail(&self) -> A {
        self.tail_value
    }
}

/// An iterator over an attribute map.
pub(super) struct Iter<'a, A> {
    cur_offset: ByteOffset,
    cur_range: (ByteOffset, A),
    inner: slice::Iter<'a, (ByteOffset, A)>,
    tail_value: A,
}

impl<'a, A: Copy> Iter<'a, A> {
    fn new(map: &'a AttributeMap<A>, start_offset: ByteOffset) -> Self {
        let start_idx = map
            .map
            .partition_point(|(offset, _)| *offset < start_offset);
        let mut inner = map.map[start_idx..].iter();
        let cur_range = Self::next_range(&mut inner, map.tail_value);

        Self {
            cur_offset: start_offset,
            cur_range,
            inner,
            tail_value: map.tail_value,
        }
    }

    /// Returns the end point and value for the next range.
    fn next_range(inner: &mut slice::Iter<'a, (ByteOffset, A)>, tail: A) -> (ByteOffset, A) {
        inner
            .next()
            .map(|(k, v)| (*k, *v))
            // If there are no more ranges in the map, return an "open" range
            // with the tail attribute value.
            .unwrap_or((ByteOffset::from(usize::MAX), tail))
    }

    fn advance_to_current_range(&mut self) {
        while self.cur_offset > self.cur_range.0 {
            self.cur_range = Self::next_range(&mut self.inner, self.tail_value);
        }
    }

    pub(super) fn skip_by(&mut self, n: usize) {
        self.cur_offset += n;
        self.advance_to_current_range();
    }
}

impl<A> Iterator for Iter<'_, A>
where
    A: Copy,
{
    type Item = A;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance_to_current_range();
        let val = self.cur_range.1;
        self.cur_offset += 1;
        self.advance_to_current_range();
        Some(val)
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.skip_by(n);
        self.next()
    }
}

#[cfg(test)]
#[path = "attribute_map_tests.rs"]
mod tests;
