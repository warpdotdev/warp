use crate::code_review::diff_state::{DiffHunk, DiffLineType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneLineKind {
    Context,
    Add,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PaneLine {
    Line {
        line_number: usize,
        text: String,
        kind: PaneLineKind,
    },
    Gap {
        after_row: usize,
    },
}

impl PaneLine {
    pub fn text(&self) -> &str {
        match self {
            Self::Line { text, .. } => text,
            Self::Gap { .. } => "",
        }
    }

    pub fn kind(&self) -> Option<PaneLineKind> {
        match self {
            Self::Line { kind, .. } => Some(*kind),
            Self::Gap { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignedRow {
    pub baseline: PaneLine,
    pub modified: PaneLine,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HunkAlignment {
    rows: Vec<AlignedRow>,
}

impl HunkAlignment {
    pub fn from_diff_hunks(hunks: &[DiffHunk]) -> Self {
        let mut rows = Vec::new();

        for hunk in hunks {
            let mut pending_deletes = Vec::new();
            let mut pending_adds = Vec::new();

            for line in &hunk.lines {
                match line.line_type {
                    DiffLineType::Context => {
                        flush_pending_pairs(&mut rows, &mut pending_deletes, &mut pending_adds);
                        if let (Some(old_line), Some(new_line)) =
                            (line.old_line_number, line.new_line_number)
                        {
                            rows.push(AlignedRow {
                                baseline: PaneLine::Line {
                                    line_number: old_line,
                                    text: line.text.clone(),
                                    kind: PaneLineKind::Context,
                                },
                                modified: PaneLine::Line {
                                    line_number: new_line,
                                    text: line.text.clone(),
                                    kind: PaneLineKind::Context,
                                },
                            });
                        }
                    }
                    DiffLineType::Delete => {
                        if !pending_adds.is_empty() {
                            flush_pending_pairs(&mut rows, &mut pending_deletes, &mut pending_adds);
                        }
                        if let Some(line_number) = line.old_line_number {
                            pending_deletes.push(PaneLine::Line {
                                line_number,
                                text: line.text.clone(),
                                kind: PaneLineKind::Delete,
                            });
                        }
                    }
                    DiffLineType::Add => {
                        if let Some(line_number) = line.new_line_number {
                            pending_adds.push(PaneLine::Line {
                                line_number,
                                text: line.text.clone(),
                                kind: PaneLineKind::Add,
                            });
                        }
                    }
                    DiffLineType::HunkHeader => {}
                }
            }

            flush_pending_pairs(&mut rows, &mut pending_deletes, &mut pending_adds);
        }

        Self { rows }
    }

    pub fn rows(&self) -> &[AlignedRow] {
        &self.rows
    }
}

fn flush_pending_pairs(
    rows: &mut Vec<AlignedRow>,
    pending_deletes: &mut Vec<PaneLine>,
    pending_adds: &mut Vec<PaneLine>,
) {
    let pair_count = pending_deletes.len().min(pending_adds.len());

    for (delete, add) in pending_deletes
        .drain(..pair_count)
        .zip(pending_adds.drain(..pair_count))
    {
        rows.push(AlignedRow {
            baseline: delete,
            modified: add,
        });
    }

    for delete in pending_deletes.drain(..) {
        let after_row = rows.len().saturating_sub(1);
        rows.push(AlignedRow {
            baseline: delete,
            modified: PaneLine::Gap { after_row },
        });
    }

    for add in pending_adds.drain(..) {
        let after_row = rows.len().saturating_sub(1);
        rows.push(AlignedRow {
            baseline: PaneLine::Gap { after_row },
            modified: add,
        });
    }
}

#[cfg(test)]
#[path = "hunk_alignment_tests.rs"]
mod tests;
