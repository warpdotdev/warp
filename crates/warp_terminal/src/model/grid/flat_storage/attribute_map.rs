use std::collections::{BTreeMap, btree_map};
use std::ops::RangeFrom;

use get_size::GetSize;
use string_offset::ByteOffset;

/// A structure that efficiently stores and retrieves the value of some grid
/// attribute.
///
/// This internally coalesces ranges to store the data in a space-efficient
/// manner.
///
/// A [`BTreeMap`] is used to achieve great performance both for looking up
/// a value in the map and scanning forward from that point.
#[derive(Debug, Default, Clone)]
pub struct AttributeMap<A> {
    /// Stores a mapping between an _ending_ byte offset (inclusive) and the
    /// attribute value for the range ending at the given offset.
    map: BTreeMap<ByteOffset, A>,
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
        // Split off any ranges that end after our new length.
        let mut truncated_ranges = self.map.split_off(&new_len);
        // If we split off any ranges, the first range defines our new tail value.
        if let Some((_, tail_value)) = truncated_ranges.pop_first() {
            self.tail_value = tail_value;
        }
    }

    /// Truncates the attribute map to start at the given content offset.
    pub fn truncate_front(&mut self, new_start_offset: ByteOffset) {
        self.map = self.map.split_off(&new_start_offset);
    }

    /// Returns the end offset of the last range in the map.
    fn last_end_offset(&self) -> ByteOffset {
        if let Some((k, _v)) = self.map.last_key_value() {
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
            debug_assert!(self.map.last_key_value().is_none());
        } else {
            debug_assert!(
                range.start > self.last_end_offset(),
                "cannot push attribute change starting at {} when last end offset is {}.  attribute map: {:?}",
                range.start,
                self.last_end_offset(),
                self.map,
            );
            self.map.insert(range.start - 1, prev_tail_value);
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
    pub fn iter_from(&self, start_offset: ByteOffset) -> impl Iterator<Item = A> + '_ {
        Iter::new(self, start_offset)
    }

    /// Returns the tail (current) value of the given attribute.
    pub fn tail(&self) -> A {
        self.tail_value
    }
}

/// An iterator over an attribute map.
struct Iter<'a, A> {
    cur_offset: ByteOffset,
    cur_range: (ByteOffset, A),
    inner: btree_map::Range<'a, ByteOffset, A>,
    tail_value: A,
}

impl<'a, A: Copy> Iter<'a, A> {
    fn new(map: &'a AttributeMap<A>, start_offset: ByteOffset) -> Self {
        let mut inner = map.map.range(start_offset..);
        let cur_range = Self::next_range(&mut inner, map.tail_value);

        Self {
            cur_offset: start_offset,
            cur_range,
            inner,
            tail_value: map.tail_value,
        }
    }

    /// Returns the end point and value for the next range.
    fn next_range(inner: &mut btree_map::Range<ByteOffset, A>, tail: A) -> (ByteOffset, A) {
        inner
            .next()
            .map(|(k, v)| (*k, *v))
            // If there are no more ranges in the map, return an "open" range
            // with the tail attribute value.
            .unwrap_or((ByteOffset::from(usize::MAX), tail))
    }
}

impl<A> Iterator for Iter<'_, A>
where
    A: Copy,
{
    type Item = A;

    fn next(&mut self) -> Option<Self::Item> {
        self.nth(0)
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        // Skip over the next n items.
        self.cur_offset += n;
        // While the offset of the next item is outside the current range, get
        // the next range from the iterator over our BTreeMap.
        while self.cur_offset > self.cur_range.0 {
            self.cur_range = Self::next_range(&mut self.inner, self.tail_value);
        }

        // Get the value and advance the iterator by one, in preparation for
        // the next call.
        let val = self.cur_range.1;
        self.cur_offset += 1;
        Some(val)
    }
}

#[cfg(test)]
#[path = "attribute_map_tests.rs"]
mod tests;
