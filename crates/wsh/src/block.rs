use crate::cell::{Cell, CellAttr, CellFlags, Color};

pub struct CompletedBlock {
    pub rows: Vec<Vec<Cell>>,
}

pub struct BlockManager {
    completed: Vec<CompletedBlock>,
    scroll_offset: usize,
}

impl Default for BlockManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockManager {
    pub fn new() -> Self {
        Self {
            completed: Vec::new(),
            scroll_offset: 0,
        }
    }

    pub fn add_block(&mut self, rows: Vec<Vec<Cell>>) {
        // Trim trailing blank rows.
        let mut rows = rows;
        while let Some(last) = rows.last() {
            if last.iter().all(|c| c.ch == ' ' && c.attr == CellAttr::default()) {
                rows.pop();
            } else {
                break;
            }
        }
        if !rows.is_empty() {
            self.completed.push(CompletedBlock { rows });
        }
    }

    pub fn add_styled_line(&mut self, text: &str, fg: Color, flags: CellFlags, cols: usize) {
        let attr = CellAttr {
            fg,
            bg: Color::Default,
            flags,
        };
        let mut row: Vec<Cell> = text
            .chars()
            .take(cols)
            .map(|ch| Cell::with_char(ch, attr))
            .collect();
        // Pad to full width.
        while row.len() < cols {
            row.push(Cell::default());
        }
        self.completed.push(CompletedBlock { rows: vec![row] });
    }

    pub fn collected_rows(&self) -> Vec<Vec<Cell>> {
        self.completed
            .iter()
            .flat_map(|b| b.rows.iter().cloned())
            .collect()
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }
}
