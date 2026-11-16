//! Module containing the definition of [`StrIndexMap`], allowing for repeated, efficient conversion
//! between a byte and/or char index from a backing `str`.

use std::collections::HashMap;

/// Map that provides efficient conversion from byte <-> char index from a backing `str`.
/// See [`StrIndexMap::byte_index`] and [`StrIndexMap::char_index`] for conversion functions to
/// convert from/to a byte index to a char index.   
pub(super) struct StrIndexMap {
    byte_to_char_index: HashMap<usize, usize>,
    char_to_byte_index: Vec<usize>,
}

impl StrIndexMap {
    /// Constructs a new [`StrIndexMap`] with byte <-> char indices based on the input `str`.
    /// NOTE this runs in O(n) time as it requires walking through each char index in the `str`.
    pub(super) fn new(str: impl AsRef<str>) -> Self {
        let char_indices = str.as_ref().char_indices();
        let (_, upper_bound) = char_indices.size_hint();

        let (mut byte_to_char_index, mut char_to_byte_index) = match upper_bound {
            None => (HashMap::new(), Vec::new()),
            Some(size) => (HashMap::with_capacity(size), Vec::with_capacity(size)),
        };

        for (char_index, (byte_index, _)) in char_indices.enumerate() {
            byte_to_char_index.insert(byte_index, char_index);
            char_to_byte_index.push(byte_index);
        }

        Self {
            byte_to_char_index,
            char_to_byte_index,
        }
    }

    /// Returns the _byte_ index of the string at the given `char_index`. If the `char_index` does
    /// not exist in the string, `None` is returned.
    pub(super) fn byte_index(&self, char_index: usize) -> Option<usize> {
        self.char_to_byte_index.get(char_index).copied()
    }

    /// Returns the _char_ index of the string at the given byte index. If the `byte_index` does not
    /// exist in the string or if it does not lie at a char boundary, `None` is returned.
    pub(super) fn char_index(&self, byte_index: usize) -> Option<usize> {
        self.byte_to_char_index.get(&byte_index).copied()
    }

    /// Returns the total number of characters in the string.
    pub(super) fn num_chars(&self) -> usize {
        self.char_to_byte_index.len()
    }
}

#[cfg(test)]
#[path = "str_index_map_tests.rs"]
mod tests;
