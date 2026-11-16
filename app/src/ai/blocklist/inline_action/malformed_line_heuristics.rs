use ai::diff_validation::{DiffDelta, DiffType};

pub(super) fn proposed_terminal_line_range(diff_type: &DiffType) -> Option<std::ops::Range<usize>> {
    let (terminal_delta, cumulative_line_shift) = terminal_update_delta_with_shift(diff_type)?;
    let terminal_start = terminal_delta
        .replacement_line_range
        .start
        .saturating_sub(1);
    let terminal_start = terminal_start.saturating_add_signed(cumulative_line_shift);
    let inserted_lines = inserted_line_count(&terminal_delta.insertion);
    let terminal_end = terminal_start.saturating_add(inserted_lines);
    Some(terminal_start..terminal_end.max(terminal_start))
}

pub(super) fn proposed_terminal_line_text(diff_type: &DiffType) -> Option<String> {
    let terminal_delta = terminal_update_delta(diff_type)?;
    terminal_delta
        .insertion
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn looks_malformed_terminal_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    has_odd_unescaped_quote(trimmed, '"')
        || has_odd_unescaped_quote(trimmed, '\'')
        || has_odd_unescaped_quote(trimmed, '`')
        || trimmed.ends_with('\\')
        || has_trailing_incomplete_token(trimmed)
        || looks_like_function_declaration_without_body(trimmed)
}

pub(super) fn changed_lines_intersect_terminal_range(
    changed_lines: &[std::ops::Range<usize>],
    terminal_line_range: &std::ops::Range<usize>,
) -> bool {
    changed_lines
        .iter()
        .any(|changed| range_intersects_or_contains_point(changed, terminal_line_range))
}

pub(super) fn has_malformed_terminal_correction_signal(
    diff_type: &DiffType,
    changed_lines: &[std::ops::Range<usize>],
) -> bool {
    let Some(terminal_line_range) = proposed_terminal_line_range(diff_type) else {
        return false;
    };
    let Some(terminal_line_text) = proposed_terminal_line_text(diff_type) else {
        return false;
    };
    looks_malformed_terminal_line(&terminal_line_text)
        && changed_lines_intersect_terminal_range(changed_lines, &terminal_line_range)
}

fn terminal_update_delta(diff_type: &DiffType) -> Option<&DiffDelta> {
    match diff_type {
        DiffType::Update { deltas, .. } => deltas.iter().max_by_key(|delta| {
            (
                delta.replacement_line_range.end,
                delta.replacement_line_range.start,
            )
        }),
        DiffType::Create { .. } | DiffType::Delete { .. } => None,
    }
}

fn terminal_update_delta_with_shift(diff_type: &DiffType) -> Option<(&DiffDelta, isize)> {
    let DiffType::Update { deltas, .. } = diff_type else {
        return None;
    };
    let terminal_delta = terminal_update_delta(diff_type)?;
    let mut sorted_deltas: Vec<&DiffDelta> = deltas.iter().collect();
    sorted_deltas.sort_by_key(|delta| {
        (
            delta.replacement_line_range.start,
            delta.replacement_line_range.end,
        )
    });
    let mut cumulative_line_shift = 0isize;
    for delta in sorted_deltas {
        if std::ptr::eq(delta, terminal_delta) {
            return Some((terminal_delta, cumulative_line_shift));
        }
        let replaced_lines = delta
            .replacement_line_range
            .end
            .saturating_sub(delta.replacement_line_range.start);
        let inserted_lines = inserted_line_count(&delta.insertion);
        cumulative_line_shift += inserted_lines as isize - replaced_lines as isize;
    }
    unreachable!(
        "terminal_update_delta_with_shift: terminal delta not found in sorted update deltas"
    )
}

fn inserted_line_count(insertion: &str) -> usize {
    if insertion.is_empty() {
        0
    } else {
        insertion.lines().count()
    }
}

fn has_odd_unescaped_quote(line: &str, quote: char) -> bool {
    let mut escaped = false;
    let mut count = 0usize;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            count += 1;
        }
    }
    count % 2 == 1
}

fn has_trailing_incomplete_token(line: &str) -> bool {
    if line.starts_with("//") || line.starts_with('#') {
        return false;
    }
    // Keep this list biased toward operators that are strongly code-like at EOL.
    // Deliberately exclude ambiguous single-character tokens like ".", "-", "*", and "/"
    // because they frequently appear in prose/markdown/path-like text and can inflate false positives.
    [
        "&&", "||", "??", "=>", "->", "::", "==", "!=", "<=", ">=", "+", "%", "=", "<", ">",
    ]
    .iter()
    .any(|token| line.ends_with(token))
}

fn starts_like_function_declaration(line: &str) -> bool {
    [
        "fn ",
        "pub fn ",
        "pub(crate) fn ",
        "pub(super) fn ",
        "pub async fn ",
        "async fn ",
        "def ",
        "async def ",
        "function ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn looks_like_function_declaration_without_body(line: &str) -> bool {
    if !starts_like_function_declaration(line) {
        return false;
    }

    let has_params = line.contains('(') && line.contains(')');
    let has_body_delimiter = line.contains('{') || line.ends_with(':');
    let ends_like_declaration = line.ends_with(')') || line.contains("->");
    let terminates_signature = line.ends_with(';');

    has_params && ends_like_declaration && !has_body_delimiter && !terminates_signature
}

fn range_intersects_or_contains_point(
    changed: &std::ops::Range<usize>,
    terminal: &std::ops::Range<usize>,
) -> bool {
    let changed_is_point = changed.start == changed.end;
    let terminal_is_point = terminal.start == terminal.end;

    if changed_is_point && terminal_is_point {
        return changed.start == terminal.start;
    }
    if changed_is_point {
        return terminal.start <= changed.start && changed.start < terminal.end;
    }
    if terminal_is_point {
        return changed.start <= terminal.start && terminal.start < changed.end;
    }

    changed.start < terminal.end && terminal.start < changed.end
}

#[cfg(test)]
#[path = "malformed_line_heuristics_tests.rs"]
mod tests;
