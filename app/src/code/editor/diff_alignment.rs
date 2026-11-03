//! Row alignment for the split (side-by-side) diff layout.
//!
//! The split layout renders the base file and the modified file in two panes. Lines that exist on
//! only one side face a blank gap row on the other side, so that unchanged lines, and paired
//! removed and added lines, sit at the same vertical position in both panes.

use std::ops::Range;

/// One contiguous change between the base and the modified text, in 0-based line indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSpan {
    /// The base lines removed or replaced. Empty for a pure addition.
    pub base: Range<usize>,
    /// The modified lines added. Empty for a pure deletion.
    pub modified: Range<usize>,
}

/// A row in the aligned layout. `None` on a side means that side shows a gap row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignedRow {
    pub base: Option<usize>,
    pub modified: Option<usize>,
    pub is_change: bool,
}

/// A run of consecutive gap rows in one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapRun {
    /// The 0-based pane line the run sits directly above; the pane's line count for a run at the
    /// end of the file.
    pub insert_before: usize,
    pub count: usize,
}

/// The aligned rows of a diff, plus lookups between rows and the lines of each side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffAlignment {
    rows: Vec<AlignedRow>,
    row_by_base_line: Vec<usize>,
    row_by_modified_line: Vec<usize>,
}

impl DiffAlignment {
    /// Aligns `base_line_count` base lines against `modified_line_count` modified lines.
    ///
    /// `spans` may be in any order. Within a span, removed and added lines pair up in order and
    /// the leftover lines of the longer side face gap rows after the paired rows.
    pub fn new(
        mut spans: Vec<ChangeSpan>,
        base_line_count: usize,
        modified_line_count: usize,
    ) -> Self {
        spans.sort_by_key(|span| (span.modified.start, span.base.start));

        let mut rows = Vec::with_capacity(base_line_count.max(modified_line_count));
        let mut base_cursor = 0;
        let mut modified_cursor = 0;

        for span in spans {
            let context = span.modified.start.saturating_sub(modified_cursor);
            push_context(&mut rows, &mut base_cursor, &mut modified_cursor, context);
            push_change(
                &mut rows,
                &mut base_cursor,
                &mut modified_cursor,
                span.base.len(),
                span.modified.len(),
            );
        }

        let remaining_base = base_line_count.saturating_sub(base_cursor);
        let remaining_modified = modified_line_count.saturating_sub(modified_cursor);
        let trailing_context = remaining_base.min(remaining_modified);
        push_context(
            &mut rows,
            &mut base_cursor,
            &mut modified_cursor,
            trailing_context,
        );
        push_change(
            &mut rows,
            &mut base_cursor,
            &mut modified_cursor,
            remaining_base - trailing_context,
            remaining_modified - trailing_context,
        );

        let mut row_by_base_line = vec![0; base_cursor];
        let mut row_by_modified_line = vec![0; modified_cursor];
        for (row_index, row) in rows.iter().enumerate() {
            if let Some(line) = row.base {
                row_by_base_line[line] = row_index;
            }
            if let Some(line) = row.modified {
                row_by_modified_line[line] = row_index;
            }
        }

        Self {
            rows,
            row_by_base_line,
            row_by_modified_line,
        }
    }

    pub fn rows(&self) -> &[AlignedRow] {
        &self.rows
    }

    /// Gap rows to insert into the base (left) pane.
    pub fn base_gaps(&self) -> Vec<GapRun> {
        self.gaps(|row| row.base)
    }

    /// Gap rows to insert into the modified (right) pane.
    pub fn modified_gaps(&self) -> Vec<GapRun> {
        self.gaps(|row| row.modified)
    }

    /// Ranges of base lines that were removed or replaced, merged where adjacent.
    pub fn changed_base_lines(&self) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for row in &self.rows {
            let (true, Some(line)) = (row.is_change, row.base) else {
                continue;
            };
            match ranges.last_mut() {
                Some(range) if range.end == line => range.end = line + 1,
                _ => ranges.push(line..line + 1),
            }
        }
        ranges
    }

    pub fn row_for_base_line(&self, line: usize) -> Option<usize> {
        self.row_by_base_line.get(line).copied()
    }

    pub fn row_for_modified_line(&self, line: usize) -> Option<usize> {
        self.row_by_modified_line.get(line).copied()
    }

    /// The modified line on the same row as a base line, if that row has one.
    pub fn modified_line_for_base_line(&self, line: usize) -> Option<usize> {
        self.rows.get(self.row_for_base_line(line)?)?.modified
    }

    /// The base line on the same row as a modified line, if that row has one.
    pub fn base_line_for_modified_line(&self, line: usize) -> Option<usize> {
        self.rows.get(self.row_for_modified_line(line)?)?.base
    }

    fn gaps(&self, side: impl Fn(&AlignedRow) -> Option<usize>) -> Vec<GapRun> {
        let mut gaps: Vec<GapRun> = Vec::new();
        let mut next_line = 0;
        for row in &self.rows {
            match side(row) {
                Some(line) => next_line = line + 1,
                None => match gaps.last_mut() {
                    Some(gap) if gap.insert_before == next_line => gap.count += 1,
                    _ => gaps.push(GapRun {
                        insert_before: next_line,
                        count: 1,
                    }),
                },
            }
        }
        gaps
    }
}

fn push_context(
    rows: &mut Vec<AlignedRow>,
    base_cursor: &mut usize,
    modified_cursor: &mut usize,
    count: usize,
) {
    for _ in 0..count {
        rows.push(AlignedRow {
            base: Some(*base_cursor),
            modified: Some(*modified_cursor),
            is_change: false,
        });
        *base_cursor += 1;
        *modified_cursor += 1;
    }
}

fn push_change(
    rows: &mut Vec<AlignedRow>,
    base_cursor: &mut usize,
    modified_cursor: &mut usize,
    removed: usize,
    added: usize,
) {
    for offset in 0..removed.max(added) {
        let base = (offset < removed).then(|| *base_cursor + offset);
        let modified = (offset < added).then(|| *modified_cursor + offset);
        rows.push(AlignedRow {
            base,
            modified,
            is_change: true,
        });
    }
    *base_cursor += removed;
    *modified_cursor += added;
}

#[cfg(test)]
#[path = "diff_alignment_tests.rs"]
mod tests;
