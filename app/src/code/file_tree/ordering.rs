use std::cmp::Ordering;

/// Custom ordering function for items in file trees.
///
/// Directories are ordered first, sorted by natural (numeric-aware) order.
/// Files are ordered second, sorted by natural (numeric-aware) order.
/// Within each group, dotfiles (entries starting with a dot) are ordered first.
pub(crate) fn compare_file_tree_entries(
    is_dir_1: bool,
    name_1: Option<&str>,
    is_dir_2: bool,
    name_2: Option<&str>,
) -> Ordering {
    // Order directories before any files.
    match (is_dir_1, is_dir_2) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        // Both are same type, continue with name sort.
        _ => {}
    }

    // Missing names must compare antisymmetrically so sort implementations
    // cannot observe a total-order violation.
    let (name_1, name_2) = match (name_1, name_2) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Less,
        (Some(_), None) => return Ordering::Greater,
        (Some(n1), Some(n2)) => (n1, n2),
    };

    let starts_with_dot_1 = name_1.starts_with('.');
    let starts_with_dot_2 = name_2.starts_with('.');

    // Items starting with "." come first.
    match (starts_with_dot_1, starts_with_dot_2) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => alphanumeric_sort::compare_str(name_1, name_2),
    }
}
