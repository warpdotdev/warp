use std::collections::HashMap;
use std::ops::Range;

use arborium::tree_sitter::{Node, Query, QueryCursor, Tree};
use streaming_iterator::StreamingIterator;
use warp_editor::content::buffer::Buffer;
use warpui_core::text::point::Point;

use super::highlight_query::TextBuffer;

/// The absolute indentation unit from start of the line.
#[derive(Debug)]
pub struct IndentDelta {
    pub delta: u8,
}

/// Given the current syntax tree and a point in the buffer. Calculate the correct indentation level for that point.
pub fn indentation_delta(
    buffer: &Buffer,
    tree: &Tree,
    position: Point,
    query: &Query,
) -> Option<IndentDelta> {
    let mut cursor = QueryCursor::new();
    let tree_sitter_point = arborium::tree_sitter::Point {
        row: position.row as usize,
        column: position.column as usize,
    };
    let (mut node, byte_range) = find_indent_query_range(tree, tree_sitter_point)?;

    cursor.set_byte_range(byte_range);
    let mut captures = cursor.captures(query, tree.root_node(), TextBuffer(buffer));

    // We want to group indents and outdents by nodes. This is necessary given in lines where we have multiple
    // indents fragments, the absolute indent / outdent level should be capped to one. Take the following example line:
    // `if self.x > 0 {`. Here there are multiple indent sources [self.] (field_expression), [{] (block), yet
    // the absolute indentation level after the line should be 1.
    let mut delta_with_node: HashMap<usize, i8> = HashMap::new();
    while let Some(matches) = captures.next() {
        for capture in matches.0.captures {
            // Do not look at nodes that are after the current point. Note that we still want to look at the current node in case
            // it is an outdent node. In this case, we want to reduce the overall indentation level by 1.
            if capture.node.start_position() > tree_sitter_point {
                break;
            }

            let capture_name = query.capture_names()[capture.index as usize];
            let base = delta_with_node.entry(capture.node.id()).or_default();

            // Cap indent / outdent to 1.
            *base = match capture_name {
                "indent" if capture.node.start_position() != tree_sitter_point => {
                    (*base + 1).min(1)
                }
                "outdent" => (*base - 1).max(-1),
                _ => 0,
            };
        }
    }
    let mut sum = 0;
    let mut previous_line = None;
    let mut total_line_delta = 0;

    // Starting from the source syntax node, iterate over its parents and add the indent delta over every single node.
    // Take the following code example:
    //
    // impl Element { // Parent 2
    //      fn some_func_1() {...}
    //      fn some_func() { // Parent 1
    //          [syntax node]
    //      }
    // }
    //
    // The total indentation level should be 2 because (parent 1) has a delta of 1, and (parent 2) has a delta of 1.
    loop {
        // Similar to above, there could be multiple nodes in a single line, we also want to cap the max indent/outdent.
        // returned to 1.
        if let Some(line_delta) = delta_with_node.remove(&node.id()) {
            let current_line = node.start_position().row;

            if previous_line.is_none() || previous_line != Some(current_line) {
                sum += total_line_delta;
                previous_line = Some(current_line);
                total_line_delta = line_delta;
            } else {
                total_line_delta = (total_line_delta + line_delta).clamp(-1, 1);
            }
        }

        match node.parent() {
            Some(parent) => {
                node = parent;
            }
            None => {
                sum += total_line_delta;
                break;
            }
        }
    }

    Some(IndentDelta {
        delta: sum.max(0) as u8,
    })
}

/// Given a position in the buffer, find the corresponding syntax node and byte range we should
/// use for indentation query.
fn find_indent_query_range(
    tree: &Tree,
    tree_sitter_point: arborium::tree_sitter::Point,
) -> Option<(Node<'_>, Range<usize>)> {
    // Find the exact syntax node for the given position.
    let node = tree
        .root_node()
        .descendant_for_point_range(tree_sitter_point, tree_sitter_point)
        .and_then(|node| {
            // Handle edge case where the node we want to start traversing the tree from can't be
            // found with `descendant_for_point_range`. This happens because there can be "empty"
            // nodes that don't span any points. In that case, the fall back is always to the
            // `Tree`'s root node which always spans all valid points. We actually want the leaf
            // node that is spans the `tree_sitter::Point` one column to the left because we need to
            // start summing indentation levels from there.
            // TODO(INT-614): Remove this special case.
            if node == tree.root_node() {
                let new_ts_point = arborium::tree_sitter::Point {
                    row: tree_sitter_point.row,
                    column: tree_sitter_point.column.saturating_sub(1),
                };
                tree.root_node()
                    .descendant_for_point_range(new_ts_point, new_ts_point)
            } else {
                Some(node)
            }
        })?;

    let mut cursor = tree.walk();
    let mut last_child_row: Option<(usize, Range<usize>)> = None;

    // Find the line range right before the given position, this will be the source for determining
    // the correct syntax tree range to query.
    for child in node.children(&mut cursor) {
        if child.start_position() >= tree_sitter_point {
            break;
        }

        if let Some((row_idx, range)) = &mut last_child_row {
            if *row_idx < child.start_position().row {
                *row_idx = child.start_position().row;
                *range = child.byte_range();
            } else {
                range.end = child.end_byte();
            }
        } else {
            last_child_row = Some((child.start_position().row, child.byte_range()));
        }
    }

    // If node has no children, fallback to use node's byte range.
    let query_range = last_child_row.map(|row| row.1).unwrap_or(node.byte_range());
    Some((node, query_range))
}

#[cfg(test)]
#[path = "indent_query_tests.rs"]
mod tests;
