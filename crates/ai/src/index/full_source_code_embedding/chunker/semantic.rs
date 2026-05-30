use std::path::Path;

use arborium::tree_sitter::{Language, Node, Parser, TreeCursor};
use itertools::Itertools;

use super::{coalesce_fragments, Fragment};

/// Maximum depth for recursive tree traversal to prevent infinite recursion
/// or excessive depth in malformed/deeply nested code.
const MAX_TRAVERSAL_DEPTH: usize = 200;

/// Chunks code into an ordered list of fragments, where each fragment is at most
/// `max_bytes_per_chunk` bytes.
pub(super) fn chunk_code<'a>(
    code: &'a str,
    path: &'a Path,
    max_bytes_per_chunk: usize,
    language: &Language,
) -> anyhow::Result<Vec<Fragment<'a>>> {
    // Wrap this in a block to ensure the treesitter Parser / Tree are dropped
    // after creating the fragments.
    let fragments = {
        let mut parser = Parser::new();
        parser.set_language(language)?;

        let tree = parser
            .parse(code, None /* old_tree */)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse code"))?;

        let mut cursor = tree.walk();

        let nodes = split_node(
            tree.root_node(),
            code,
            max_bytes_per_chunk,
            path,
            &mut cursor,
            0, // initial depth
        )?;

        coalesce_fragments(nodes.into_iter(), code, max_bytes_per_chunk)
    };

    // Release extra unused memory from malloc to the system.  For some
    // reason, the memory obtained by the allocator is often not released
    // back to the OS after we're done with it, resulting in high memory
    // usage (from the perspective of the OS, though not from the perspective
    // of the allocator).
    //
    // See: https://github.com/tree-sitter/tree-sitter/issues/3129
    #[cfg(all(
        any(target_os = "linux", target_os = "freebsd"),
        target_env = "gnu",
        not(feature = "jemalloc")
    ))]
    unsafe {
        nix::libc::malloc_trim(0);
    }

    Ok(fragments)
}

/// Splits a [`Node`] into a series of [`Fragment`]s that are at most `max_bytes_per_chunk` bytes.
fn split_node<'a, 'b>(
    node: Node<'b>,
    code: &'a str,
    max_bytes_per_chunk: usize,
    path: &'a Path,
    cursor: &mut TreeCursor<'b>,
    depth: usize,
) -> anyhow::Result<Vec<Fragment<'a>>> {
    // Check if we've exceeded the maximum traversal depth
    if depth > MAX_TRAVERSAL_DEPTH {
        return Err(anyhow::anyhow!(
            "Maximum traversal depth {} exceeded, falling back to naive chunking",
            MAX_TRAVERSAL_DEPTH
        ));
    }

    let mut current_fragment = Fragment::from_node_start(node, path);
    let mut fragments = vec![];

    // Collect into a vec to avoid a double mutable borrow with `cursor` when we make
    // the recursive call below.
    for child in node.children(cursor).collect_vec() {
        let child_size = child.end_byte().saturating_sub(child.start_byte());

        // The child is larger than the max chunk size, so we need to split it recursively.
        if child_size > max_bytes_per_chunk {
            let mut new_fragment = Fragment::from_node_end(child, path);
            std::mem::swap(&mut current_fragment, &mut new_fragment);
            fragments.push(new_fragment);

            fragments.append(&mut split_node(
                child,
                code,
                max_bytes_per_chunk,
                path,
                cursor,
                depth + 1,
            )?);
        } else if child_size + current_fragment.size() > max_bytes_per_chunk {
            // The child would make the current fragment too large, so we finalize the current
            // fragment and create a new one.
            fragments.push(current_fragment);
            current_fragment = Fragment::from_node_start(child, path);
            current_fragment.append(&Fragment::from_node_end(child, path), code);
        } else {
            // The child fits within the current fragment.
            current_fragment.end_line = child.end_position().row;
            current_fragment.end_byte_index = child.end_byte().into();
            current_fragment.content =
                &code[current_fragment.start_byte_index.as_usize()..child.end_byte()];
        }
    }

    fragments.push(current_fragment);

    Ok(fragments)
}

impl<'a> Fragment<'a> {
    /// Creates an empty fragment.
    fn empty() -> Fragment<'a> {
        Fragment {
            content: "",
            start_line: 0,
            end_line: 0,
            start_byte_index: 0.into(),
            end_byte_index: 0.into(),
            file_path: Path::new(""),
        }
    }

    /// Creates a fragment comprised solely of the start of the given node.
    fn from_node_start(node: Node<'_>, path: &'a Path) -> Self {
        Fragment {
            content: "",
            start_line: node.start_position().row,
            end_line: node.start_position().row,
            start_byte_index: node.start_byte().into(),
            end_byte_index: node.start_byte().into(),
            file_path: path,
        }
    }

    /// Creates a fragment comprised solely of the end of the given node.
    fn from_node_end(node: Node<'_>, path: &'a Path) -> Self {
        Fragment {
            content: "",
            start_line: node.end_position().row,
            end_line: node.end_position().row,
            start_byte_index: node.end_byte().into(),
            end_byte_index: node.end_byte().into(),
            file_path: path,
        }
    }

    /// Creates a fragment comprised solely of the end of the given fragment.
    fn from_fragment_end(fragment: &Fragment<'a>) -> Self {
        Fragment {
            content: "",
            start_line: fragment.end_line,
            end_line: fragment.end_line,
            start_byte_index: fragment.end_byte_index,
            end_byte_index: fragment.end_byte_index,
            file_path: fragment.file_path,
        }
    }
}

#[cfg(test)]
#[path = "semantic_tests.rs"]
mod tests;
