use std::cmp;

/// Takes a slice of strings and returns a string representing the common
/// prefix of every string in the vector
pub fn longest_common_prefix<'a>(strings: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut strings = strings.into_iter();
    let first = strings.next()?;
    let mut len = first.len();

    for string in strings {
        len = cmp::min(
            len,
            string
                .chars()
                .zip(first.chars())
                .take_while(|(c1, c2)| c1 == c2)
                .map(|(c, _)| c.len_utf8())
                .sum(),
        );
    }

    if len == 0 {
        None
    } else {
        Some(&first[0..len])
    }
}

#[cfg(test)]
#[path = "prefix_tests.rs"]
mod tests;
