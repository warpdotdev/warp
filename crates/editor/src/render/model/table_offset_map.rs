use markdown_parser::FormattedTextFragment;
use string_offset::CharOffset;

/// Maps between linear CharOffset positions and table cell coordinates.
///
/// The table content is represented as:
/// ```text
/// Header1\tHeader2\tHeader3\n
/// Cell1\tCell2\tCell3\n
/// Cell4\tCell5\tCell6\n
/// ```
///
/// This structure enables:
/// - Finding which cell contains a given CharOffset
/// - Getting the CharOffset range for a specific cell
/// - Determining if an offset is on a separator (tab or newline)
#[derive(Debug, Clone)]
pub struct TableOffsetMap {
    cell_ranges: Vec<CellRange>,
    row_ranges: Vec<RowRange>,
    cell_index_by_row_col: Vec<Vec<usize>>,
    total_length: CharOffset,
    num_rows: usize,
    num_cols: usize,
}

#[derive(Debug, Clone, Copy)]
struct RowRange {
    start: CharOffset,
    end: CharOffset,
}

/// A range representing a single cell's position in the linear character stream.
#[derive(Debug, Clone, Copy)]
pub struct CellRange {
    pub start: CharOffset,
    pub end: CharOffset,
    pub row: usize,
    pub col: usize,
}

/// The location of a character offset within a table cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellAtOffset {
    pub row: usize,
    pub col: usize,
    pub offset_in_cell: CharOffset,
}

/// The character offset range (start, end) of a cell in the linear content stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellOffsetRange {
    pub start: CharOffset,
    pub end: CharOffset,
}

/// Result of looking up a CharOffset in the table.
#[derive(Debug, Clone, Copy)]
pub enum TablePosition {
    /// Offset is within a cell's text content
    InCell {
        row: usize,
        col: usize,
        offset_in_cell: CharOffset,
    },
    /// Offset is on a tab separator between cells
    OnTab { row: usize, after_col: usize },
    /// Offset is on a newline at the end of a row
    OnNewline { row: usize },
}

#[derive(Debug, Clone)]
pub struct TableCellOffsetMap {
    fragment_ranges: Vec<TableCellFragmentRange>,
    rendered_length: CharOffset,
    source_length: CharOffset,
}

#[derive(Debug, Clone, Copy)]
struct TableCellFragmentRange {
    rendered_start: CharOffset,
    rendered_end: CharOffset,
    source_end: CharOffset,
    visible_source_start: CharOffset,
    visible_source_end: CharOffset,
}
impl TableOffsetMap {
    /// Build a new TableOffsetMap from cell text lengths.
    ///
    /// `cell_lengths` is a 2D array where cell_lengths[row][col] is the character
    /// count of that cell's text content.
    pub fn new(cell_lengths: Vec<Vec<usize>>) -> Self {
        let num_rows = cell_lengths.len();
        let num_cols = cell_lengths.first().map(|r| r.len()).unwrap_or(0);

        let mut cell_ranges = Vec::new();
        let mut row_ranges = Vec::with_capacity(num_rows);
        let mut cell_index_by_row_col = Vec::with_capacity(num_rows);
        let mut current_offset = CharOffset::zero();

        for (row_idx, row) in cell_lengths.iter().enumerate() {
            let row_start = current_offset;
            let mut row_cell_indices = Vec::with_capacity(row.len());

            for (col_idx, &cell_len) in row.iter().enumerate() {
                let start = current_offset;
                let end = start + cell_len;
                let cell_idx = cell_ranges.len();

                cell_ranges.push(CellRange {
                    start,
                    end,
                    row: row_idx,
                    col: col_idx,
                });
                row_cell_indices.push(cell_idx);
                current_offset = end;

                if col_idx < row.len() - 1 {
                    current_offset += 1;
                }
            }

            current_offset += 1;
            row_ranges.push(RowRange {
                start: row_start,
                end: current_offset,
            });
            cell_index_by_row_col.push(row_cell_indices);
        }

        Self {
            cell_ranges,
            row_ranges,
            cell_index_by_row_col,
            total_length: current_offset,
            num_rows,
            num_cols,
        }
    }

    /// Get the total content length including all cells, tabs, and newlines.
    pub fn total_length(&self) -> CharOffset {
        self.total_length
    }

    /// Find what's at the given offset.
    pub fn position_at_offset(&self, offset: CharOffset) -> Option<TablePosition> {
        if offset >= self.total_length {
            return None;
        }

        let row_idx = self
            .row_ranges
            .partition_point(|row_range| row_range.end <= offset);
        let row_range = self.row_ranges.get(row_idx)?;
        if offset < row_range.start {
            return None;
        }

        let row_cells = self.cell_index_by_row_col.get(row_idx)?;
        let mut previous_cell: Option<CellRange> = None;
        for &cell_idx in row_cells {
            let cell = *self.cell_ranges.get(cell_idx)?;

            if offset < cell.start {
                return previous_cell.map(|cell| self.separator_position(cell));
            }

            if offset < cell.end {
                return Some(TablePosition::InCell {
                    row: cell.row,
                    col: cell.col,
                    offset_in_cell: offset - cell.start,
                });
            }

            if offset == cell.end {
                return Some(self.separator_position(cell));
            }

            previous_cell = Some(cell);
        }

        Some(TablePosition::OnNewline { row: row_idx })
    }

    /// Find which cell contains the given offset.
    /// If the offset is on a separator, returns the cell before the separator.
    pub fn cell_at_offset(&self, offset: CharOffset) -> Option<CellAtOffset> {
        match self.position_at_offset(offset)? {
            TablePosition::InCell {
                row,
                col,
                offset_in_cell,
            } => Some(CellAtOffset {
                row,
                col,
                offset_in_cell,
            }),
            TablePosition::OnTab { row, after_col } => {
                let cell = self.cell_range(row, after_col)?;
                Some(CellAtOffset {
                    row,
                    col: after_col,
                    offset_in_cell: cell.end - cell.start,
                })
            }
            TablePosition::OnNewline { row } => {
                let last_col = self
                    .cell_index_by_row_col
                    .get(row)
                    .map(|cells| cells.len())
                    .unwrap_or(0)
                    .saturating_sub(1);
                let cell = self.cell_range(row, last_col)?;
                Some(CellAtOffset {
                    row,
                    col: last_col,
                    offset_in_cell: cell.end - cell.start,
                })
            }
        }
    }

    /// Get the character offset range for a specific cell by (row, col).
    pub fn cell_range(&self, row: usize, col: usize) -> Option<CellOffsetRange> {
        let cell_idx = *self.cell_index_by_row_col.get(row)?.get(col)?;
        let cell = self.cell_ranges.get(cell_idx)?;
        Some(CellOffsetRange {
            start: cell.start,
            end: cell.end,
        })
    }

    /// Check if an offset is on a tab or newline separator.
    pub fn is_separator(&self, offset: CharOffset) -> bool {
        matches!(
            self.position_at_offset(offset),
            Some(TablePosition::OnTab { .. } | TablePosition::OnNewline { .. })
        )
    }

    /// Get all cells that intersect with the given offset range.
    /// Returns cells in row-major order.
    pub fn cells_in_range(&self, start: CharOffset, end: CharOffset) -> Vec<CellRange> {
        self.cell_ranges
            .iter()
            .filter(|cell| cell.end > start && cell.start < end)
            .copied()
            .collect()
    }

    /// Get the number of rows in the table.
    pub fn num_rows(&self) -> usize {
        self.num_rows
    }

    /// Get the number of columns in the table.
    pub fn num_cols(&self) -> usize {
        self.num_cols
    }

    fn separator_position(&self, cell: CellRange) -> TablePosition {
        let row_len = self
            .cell_index_by_row_col
            .get(cell.row)
            .map(|cells| cells.len())
            .unwrap_or(0);
        if cell.col + 1 < row_len {
            TablePosition::OnTab {
                row: cell.row,
                after_col: cell.col,
            }
        } else {
            TablePosition::OnNewline { row: cell.row }
        }
    }
}

// TODO: When we add editable tables or other complex table operations, consider moving
// cell/row boundaries into the `SumTree` with new `BufferText` marker types so that
// per-cell offsets can be derived by seeking to boundaries instead of re-parsing the
// whole table on every edit. The current embedded-text-plus-cached-parse model is
// sufficient for read-only tables; see PR #24326 discussion for context.
impl TableCellOffsetMap {
    /// Build a cell offset map from the raw cell `source` text and the parsed `inline`
    /// fragments produced by the Markdown parser.
    ///
    /// This walks `source` character-by-character alongside the rendered text of each fragment,
    /// which makes it robust to:
    /// - backslash escapes (e.g. `\*foo` consumes two source chars for one rendered char),
    /// - changes to Markdown marker syntax (we don't hardcode `**`, `*`, `<u>`, etc. here), and
    /// - nested styles where adjacent fragments share outer markers (e.g. `**a *b* c**`), since we
    ///   attribute each marker to the fragment whose rendered text follows it.
    pub fn from_inline_and_source(source: &str, inline: &[FormattedTextFragment]) -> Self {
        let source_chars: Vec<char> = source.chars().collect();
        let total_source_chars = source_chars.len();
        let mut fragment_ranges: Vec<TableCellFragmentRange> = Vec::new();
        let mut rendered_offset = CharOffset::zero();
        let mut source_idx: usize = 0;

        for fragment in inline {
            let rendered_chars: Vec<char> = fragment.text.chars().collect();
            if rendered_chars.is_empty() {
                continue;
            }

            let first_rendered = rendered_chars[0];
            while source_idx < total_source_chars {
                let sc = source_chars[source_idx];
                if sc == '\\'
                    && source_idx + 1 < total_source_chars
                    && source_chars[source_idx + 1] == first_rendered
                {
                    source_idx += 1;
                    break;
                }
                if sc == first_rendered {
                    break;
                }
                source_idx += 1;
            }

            let visible_source_start = CharOffset::from(source_idx);

            for &rendered_char in &rendered_chars {
                if source_idx >= total_source_chars {
                    break;
                }
                let sc = source_chars[source_idx];
                if sc == '\\'
                    && source_idx + 1 < total_source_chars
                    && source_chars[source_idx + 1] == rendered_char
                {
                    source_idx += 2;
                } else {
                    source_idx += 1;
                }
            }

            let visible_source_end = CharOffset::from(source_idx);
            let rendered_start = rendered_offset;
            let rendered_end = rendered_start + CharOffset::from(rendered_chars.len());

            fragment_ranges.push(TableCellFragmentRange {
                rendered_start,
                rendered_end,
                source_end: visible_source_end,
                visible_source_start,
                visible_source_end,
            });

            rendered_offset = rendered_end;
        }

        let total_source_offset = CharOffset::from(total_source_chars);
        let fragment_count = fragment_ranges.len();
        for i in 0..fragment_count {
            let next_start = if i + 1 < fragment_count {
                fragment_ranges[i + 1].visible_source_start
            } else {
                total_source_offset
            };
            fragment_ranges[i].source_end = next_start;
        }

        Self {
            fragment_ranges,
            rendered_length: rendered_offset,
            source_length: total_source_offset,
        }
    }

    pub fn rendered_length(&self) -> CharOffset {
        self.rendered_length
    }

    pub fn source_length(&self) -> CharOffset {
        self.source_length
    }

    pub fn rendered_to_source(&self, rendered_offset: CharOffset) -> CharOffset {
        if rendered_offset >= self.rendered_length {
            return self
                .fragment_ranges
                .last()
                .map(|fragment| fragment.visible_source_end)
                .unwrap_or(self.source_length);
        }

        for fragment in &self.fragment_ranges {
            if rendered_offset < fragment.rendered_end {
                return fragment.visible_source_start + (rendered_offset - fragment.rendered_start);
            }
        }

        self.source_length
    }

    pub fn source_to_rendered(&self, source_offset: CharOffset) -> CharOffset {
        if source_offset >= self.source_length {
            return self.rendered_length;
        }

        for fragment in &self.fragment_ranges {
            if source_offset < fragment.source_end {
                if source_offset <= fragment.visible_source_start {
                    return fragment.rendered_start;
                }
                if source_offset >= fragment.visible_source_end {
                    return fragment.rendered_end;
                }
                return fragment.rendered_start + (source_offset - fragment.visible_source_start);
            }
        }

        self.rendered_length
    }
}

#[cfg(test)]
#[path = "table_offset_map_tests.rs"]
mod tests;
