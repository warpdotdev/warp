//! Utilities for editing lists.

use std::fmt;

use enum_iterator::Sequence;

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;

/// Technically we don't need to cap these numbers. But the list
/// would be hard to render and read when the text gets too long.
/// For now, we cap the alphabet list at 26*3 = 78 and roman list at
/// 30, which should be sufficient for most of the use cases.
const MAX_ALPHABET_NUM: usize = 78;
const MAX_ROMAN_NUM: usize = 30;

/// The indentation level we support in unordered and ordered list.
#[derive(Eq, PartialEq, Clone, Copy, Debug, Hash, Sequence, PartialOrd, Ord)]
pub enum ListIndentLevel {
    One,
    Two,
    Three,
}

impl ListIndentLevel {
    /// Only supports for indent level up to 2. If the indent level is greater than 2, it will snap
    /// to [`ListIndentLevel::Three`].
    pub fn from_usize(indent_level: usize) -> Self {
        match indent_level {
            0 => Self::One,
            1 => Self::Two,
            2 => Self::Three,
            _ => {
                log::warn!("Only support indent level up to 2");
                Self::Three
            }
        }
    }

    /// Supports for any indent level. If the indent level is greater than 2, it will return
    /// the result of [`Self::from_usize`] with the indentation level mod 3 (cyclic).
    pub fn from_usize_unbounded(indentation_level: usize) -> Self {
        match indentation_level {
            0 => Self::One,
            1 => Self::Two,
            2 => Self::Three,
            _ => Self::from_usize(indentation_level % 3),
        }
    }

    pub fn as_usize(&self) -> usize {
        match self {
            Self::One => 0,
            Self::Two => 1,
            Self::Three => 2,
        }
    }

    pub fn shift_right(self) -> Self {
        match self {
            Self::One => Self::Two,
            Self::Two | Self::Three => Self::Three,
        }
    }

    pub fn shift_left(self) -> Self {
        match self {
            Self::Three => Self::Two,
            Self::One | Self::Two => Self::One,
        }
    }

    /// Get the string representation of the number for the ordered list item of the given indentation.
    pub fn list_number_string(&self, number: usize) -> String {
        match self {
            ListIndentLevel::One => number.to_string(),
            ListIndentLevel::Two => number_to_alphabet(number.saturating_sub(1)),
            ListIndentLevel::Three => number_to_roman(number.saturating_sub(1)),
        }
    }
}

impl fmt::Display for ListIndentLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            ListIndentLevel::One => "1",
            ListIndentLevel::Two => "2",
            ListIndentLevel::Three => "3",
        })
    }
}

/// Tracker for ordered list numbers.
#[derive(Default)]
pub struct ListNumbering {
    /// The current list index at each indent level, from 0 to the current indent level.
    /// * When entering a new sublist (the indent level increases), we start its index at the first
    ///   item's number (or 1 for auto-numbered items).
    /// * When exiting a sublist (the indent level decreases), we truncate to the parent indent
    ///   level so that numbering from one sublist doesn't affect later sublists that happen to be
    ///   at the same indent level.
    /// * Within one list/sublist, we only use the first item's number, and auto-number all
    ///   subsequent items.
    indices_by_level: Vec<usize>,
}

#[derive(Debug, PartialEq)]
pub struct OrderedListLabel {
    /// The numerical value of the label.
    pub label_index: usize,
    /// The displayed string of the label.
    pub display_label: String,
}

impl ListNumbering {
    /// Construct a new numbering tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the next list item is explicitly numberable (i.e. if the `number`
    /// parameter to [`Self::advance`] will be respected).
    pub fn can_number(&self, indent: usize) -> bool {
        self.indices_by_level.len() <= indent
    }

    /// Advance to the next ordered list item, returning its index.
    ///
    /// ## Parameters
    /// * `indent` the current list indent level, starting at 0.
    /// * `number` the number assigned to the list, if present. If the item is not the first at its
    ///   indent level, this number is ignored. This matches Markdown's behavior and the semantics
    ///   of the HTML `start` attribute.
    pub fn advance(&mut self, indent: usize, number: Option<usize>) -> OrderedListLabel {
        let can_number = self.can_number(indent);
        self.indices_by_level.resize(indent + 1, 0);

        // Panic-safety: Due to the resize above, `self.indices` contains exactly `indent + 1`
        // items, so `indent` is a valid index.
        let slot = &mut self.indices_by_level[indent];

        match number {
            Some(number) if can_number => *slot = number,
            _ => *slot += 1,
        }

        let indentation_level = ListIndentLevel::from_usize_unbounded(indent);
        OrderedListLabel {
            label_index: *slot,
            display_label: indentation_level.list_number_string(*slot),
        }
    }

    /// Reset after encountering a non-ordered-list item.
    pub fn reset(&mut self) {
        self.indices_by_level.clear();
    }
}

/// Convert a number into a alphabet for ordered lists. We would repeat the alphabet to
/// represent any number larger than 26. For example, 27 would be "aa".
/// Note that this is 0-based so 0 -> 'a', 1 -> 'b'.
fn number_to_alphabet(num: usize) -> String {
    // Cap it to the max number of alphabet repeats.
    let capped_num = num % MAX_ALPHABET_NUM;

    let num_repeat = capped_num / 26;
    let remainder = (capped_num % 26) as u8;

    let alphabet = (remainder + 97) as char;
    alphabet.to_string().repeat(num_repeat + 1)
}

/// Convert a number into a roman number for ordered lists.
/// Note that this is 0-based so 0 -> 'i', 1 -> 'ii'.
fn number_to_roman(num: usize) -> String {
    // Cap it to the max number we want to represent.
    let mut capped_num = (num % MAX_ROMAN_NUM) + 1;

    let roman_pairs = [("x", 10), ("ix", 9), ("v", 5), ("iv", 4), ("i", 1)];
    let mut result = String::new();
    for (name, value) in roman_pairs.iter() {
        while capped_num >= *value {
            capped_num -= value;
            result.push_str(name);
        }
    }

    result
}
