use std::iter::Peekable;

use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy)]
enum ColumnAlignment {
    Left,
    Center,
    Right,
}

/// Split a table row into cells, handling escaped pipes (`\|`) as literal pipe characters.
fn split_cells_escaped(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_matches('|');
    let mut cells = Vec::new();
    let mut current_cell = String::new();
    let mut chars = trimmed.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            current_cell.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(current_cell.trim().to_string());
            current_cell = String::new();
        } else {
            current_cell.push(c);
        }
    }
    cells.push(current_cell.trim().to_string());
    cells
}

/// Returns true if the line looks like a GFM pipe-table separator row,
/// e.g. `| --- | ---: | :---: |`.
fn is_gfm_table_separator_row(row: &str) -> bool {
    let trimmed = row.trim();
    if trimmed.is_empty() || !trimmed.contains('|') {
        return false;
    }

    let mut contains_separator_cell = false;
    // Split row into individual cells.
    for cell in trimmed.split('|').map(|c| c.trim()) {
        if cell.is_empty() {
            continue;
        }

        // `:` are used to indicate a column's horizontal alignment (e.g. `:--:` for center).
        let dashes = cell.trim_matches(':').trim();
        if dashes.is_empty() {
            return false;
        }
        if !dashes.chars().all(|c| c == '-') {
            return false;
        }
        contains_separator_cell = true;
    }
    contains_separator_cell
}

/// Attempts to parse a GFM table starting from `header_line`.
///
/// If the next line in `lines` is a valid GFM separator row, this consumes all
/// subsequent table rows and returns the raw table lines.
/// The `should_stop` predicate is called on each candidate row to allow the caller
/// to halt parsing early (e.g., when encountering a fenced code block).
///
/// Returns `None` if `header_line` and the next line don't form a valid table start.
pub fn maybe_collect_gfm_table_lines<'a, I>(
    header_line: &str,
    lines: &mut Peekable<I>,
    should_stop: impl Fn(&str) -> bool,
) -> Option<Vec<String>>
where
    I: Iterator<Item = &'a str>,
{
    let header_trimmed = header_line.trim();
    let has_leading_or_trailing_pipe =
        header_trimmed.starts_with('|') || header_trimmed.ends_with('|');
    let has_at_least_two_pipes = header_trimmed.matches('|').count() >= 2;
    if !has_leading_or_trailing_pipe || !has_at_least_two_pipes {
        return None;
    }

    let separator = lines
        .next_if(|line| is_gfm_table_separator_row(line))?
        .to_owned();

    let header_column_count = split_cells_escaped(header_trimmed).len();
    let separator_column_count = split_cells_escaped(&separator).len();
    if header_column_count != separator_column_count {
        return None;
    }

    let mut table_lines = vec![header_line.to_owned(), separator];

    while let Some(next_line) = lines.peek() {
        let is_blank = next_line.trim().is_empty();
        let is_end_of_section = should_stop(next_line);
        let row_column_count = split_cells_escaped(next_line).len();
        let has_wrong_column_count = row_column_count != header_column_count;
        if is_blank || is_end_of_section || has_wrong_column_count {
            break;
        }
        table_lines.push(lines.next().expect("peeked line must exist").to_owned());
    }

    Some(table_lines)
}

/// Attempts to parse a GFM table starting from `header_line`.
///
/// If the next line in `lines` is a valid GFM separator row, this consumes all
/// subsequent table rows and returns the formatted table as a `String`.
/// The `should_stop` predicate is called on each candidate row to allow the caller
/// to halt parsing early (e.g., when encountering a fenced code block).
///
/// Returns `None` if `header_line` and the next line don't form a valid table start.
pub fn maybe_parse_gfm_table<'a, I>(
    header_line: &str,
    lines: &mut Peekable<I>,
    should_stop: impl Fn(&str) -> bool,
) -> Option<String>
where
    I: Iterator<Item = &'a str>,
{
    maybe_collect_gfm_table_lines(header_line, lines, should_stop)
        .map(|table_lines| format_gfm_table(&table_lines))
}

/// Formats a GFM table with normalized column widths.
pub fn format_gfm_table(rows: &[String]) -> String {
    // A valid GFM table must consist of at least two rows
    // (a header row and a separator row).
    if rows.len() < 2 {
        return rows.join("\n");
    }

    // Parse all rows into cells, handling leading/trailing pipes
    let parsed_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let trimmed = row.trim();
            if trimmed.is_empty() {
                return vec![];
            }

            // Split into cells, handling escaped pipes.
            split_cells_escaped(trimmed)
        })
        .collect();

    let num_columns = parsed_rows.first().map_or(0, |r| r.len());
    if num_columns == 0 {
        return rows.join("\n");
    }

    // Calculate max display width for each column
    // (3 is the minimum width for the separator row).
    let mut column_widths = vec![3usize; num_columns];
    for (row_idx, row) in parsed_rows.iter().enumerate() {
        if row_idx == 1 {
            continue;
        }
        for (col_idx, cell) in row.iter().enumerate() {
            if col_idx < num_columns {
                column_widths[col_idx] = column_widths[col_idx].max(cell.width());
            }
        }
    }

    // Parse alignments from separator row
    let alignments: Vec<ColumnAlignment> = parsed_rows
        .get(1)
        .map(|sep_row| {
            (0..num_columns)
                .map(|i| {
                    sep_row.get(i).map_or(ColumnAlignment::Left, |cell| {
                        let cell = cell.trim();
                        match (cell.starts_with(':'), cell.ends_with(':')) {
                            // :---: => center aligned
                            (true, true) => ColumnAlignment::Center,
                            // ---: => right aligned
                            (false, true) => ColumnAlignment::Right,
                            // :--- or --- => left aligned
                            _ => ColumnAlignment::Left,
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_else(|| vec![ColumnAlignment::Left; num_columns]);

    // Build formatted rows
    let mut result = Vec::with_capacity(rows.len());
    for (row_idx, row) in parsed_rows.iter().enumerate() {
        let formatted_cells: Vec<String> = (0..num_columns)
            .map(|col_idx| {
                let width = column_widths[col_idx];
                let alignment = alignments[col_idx];
                if row_idx == 1 {
                    // Use format padding to generate repeated dashes for the separator rows
                    // (e.g. `{:-<width$}` pads "-" on the right with `-` chars to reach `width`).
                    match alignment {
                        ColumnAlignment::Left => format!("{:-<width$}", "-"),
                        ColumnAlignment::Right => {
                            let dashes = width.saturating_sub(1);
                            format!("{:-<dashes$}:", "-")
                        }
                        ColumnAlignment::Center => {
                            let dashes = width.saturating_sub(2);
                            format!(":{:-<dashes$}:", "-")
                        }
                    }
                } else {
                    // Data row: pad manually since format! doesn't account for
                    // Unicode display width (e.g. emojis are wider than 1 char).
                    let cell = row.get(col_idx).map_or("", |s| s.as_str());
                    let display_width = cell.width();
                    let padding = width.saturating_sub(display_width);
                    match alignment {
                        ColumnAlignment::Left => format!("{cell}{:padding$}", ""),
                        ColumnAlignment::Right => format!("{:padding$}{cell}", ""),
                        ColumnAlignment::Center => {
                            let left_pad = padding / 2;
                            let right_pad = padding - left_pad;
                            format!("{:left_pad$}{cell}{:right_pad$}", "", "")
                        }
                    }
                }
            })
            .collect();
        result.push(format!("| {} |", formatted_cells.join(" | ")));
    }

    result.join("\n")
}

#[cfg(test)]
#[path = "gfm_table_tests.rs"]
mod tests;
