//! Natural (numeric-aware) ordering for strings such as file names.

use std::cmp::Ordering;

/// Compares two strings in natural (numeric-aware) order: runs of ASCII digits
/// are compared by numeric value, all other characters by `char` (so the
/// comparison is case-sensitive). The result is a total order, suitable for
/// `sort_by`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        let (char_a, char_b) = (a[i], b[j]);

        if char_a.is_ascii_digit() && char_b.is_ascii_digit() {
            let (start_a, start_b) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            match compare_number_runs(&a[start_a..i], &b[start_b..j]) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        } else if char_a == char_b {
            i += 1;
            j += 1;
        } else {
            return char_a.cmp(&char_b);
        }
    }

    // One string is a prefix of the other (or both ended): shorter sorts first.
    (a.len() - i).cmp(&(b.len() - j))
}

/// Compares two runs of digit characters by numeric value: by significant-digit
/// length, then digit by digit (never parsing into an integer, so runs of any
/// length cannot overflow). Equal values are ordered by raw length so leading
/// zeros (`1` vs `01`) stay deterministic.
fn compare_number_runs(a: &[char], b: &[char]) -> Ordering {
    let digits_a = trim_leading_zeros(a);
    let digits_b = trim_leading_zeros(b);

    digits_a
        .len()
        .cmp(&digits_b.len())
        .then_with(|| digits_a.iter().cmp(digits_b.iter()))
        .then_with(|| a.len().cmp(&b.len()))
}

/// Drops leading `'0'` characters from a digit run, keeping at least one digit.
fn trim_leading_zeros(digits: &[char]) -> &[char] {
    let mut start = 0;
    while start + 1 < digits.len() && digits[start] == '0' {
        start += 1;
    }
    &digits[start..]
}

#[cfg(test)]
#[path = "natural_sort_tests.rs"]
mod tests;
