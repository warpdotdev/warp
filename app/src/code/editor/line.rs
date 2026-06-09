use std::ops::Range;

use warp_editor::render::model::{LineCount, RenderLineLocation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorLineLocation {
    Collapsed {
        line_range: Range<LineCount>,
    },
    Current {
        line_number: LineCount,
        line_range: Range<LineCount>,
    },
    Removed {
        line_number: LineCount,
        line_range: Range<LineCount>,
        // There can be many deleted lines in a removal hunk, so we track the index of this line within the hunk.
        index: usize,
    },
}

impl EditorLineLocation {
    pub fn line_range(&self) -> &Range<LineCount> {
        match self {
            EditorLineLocation::Current { line_range, .. } => line_range,
            EditorLineLocation::Removed { line_range, .. } => line_range,
            EditorLineLocation::Collapsed { line_range } => line_range,
        }
    }

    pub fn line_number(&self) -> Option<LineCount> {
        match self {
            EditorLineLocation::Current { line_number, .. } => Some(*line_number),
            EditorLineLocation::Removed { line_number, .. } => Some(*line_number),
            EditorLineLocation::Collapsed { .. } => None,
        }
    }

    /// Check if this line location represents the same line as another.
    /// This is not the same as equality, as the line range of the diff hunk may differ.
    pub fn is_same_line(&self, other: &Self) -> bool {
        match (self, other) {
            (
                EditorLineLocation::Current { line_number: a, .. },
                EditorLineLocation::Current { line_number: b, .. },
            ) if a == b => true,
            (
                EditorLineLocation::Removed {
                    line_number: al,
                    index: ai,
                    ..
                },
                EditorLineLocation::Removed {
                    line_number: bl,
                    index: bi,
                    ..
                },
            ) if al == bl && ai == bi => true,
            _ => false,
        }
    }

    pub fn into_render_line_location(self) -> RenderLineLocation {
        match self {
            EditorLineLocation::Current { line_number, .. } => {
                RenderLineLocation::Current(line_number)
            }
            EditorLineLocation::Removed {
                line_number, index, ..
            } => RenderLineLocation::Temporary {
                at_line: line_number,
                index_from_at_line: index,
            },
            EditorLineLocation::Collapsed { line_range } => {
                debug_assert!(false, "We don't support converting from collapsed line location to render line location yet");
                RenderLineLocation::Current(line_range.start)
            }
        }
    }
}
