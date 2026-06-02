use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use super::{ContentHash, Fragment, FragmentLocation, FragmentMetadata};
use crate::index::locations::{CodeContextLocation, FileFragmentLocation};

#[derive(Default)]
pub struct ReadFragmentResult {
    pub successfully_read: Vec<Fragment>,
    pub fail_to_read: Vec<ContentHash>,
    pub fail_to_read_path: Vec<PathBuf>,
}

pub fn build_fragments_from_file_contents(
    metadatas: impl IntoIterator<Item = (ContentHash, FragmentMetadata)>,
    file_contents: &HashMap<PathBuf, String>,
) -> ReadFragmentResult {
    let mut fragments = Vec::new();
    let mut fail_to_read = Vec::new();
    let mut fail_to_read_path = Vec::new();

    // Group fragments by file path.
    let mut fragments_by_path: HashMap<_, Vec<_>> = HashMap::new();
    for (content_hash, metadata) in metadatas {
        fragments_by_path
            .entry(metadata.absolute_path)
            .or_default()
            .push((content_hash, metadata.location.byte_range));
    }

    // Process each file and its fragments.
    for (file_path, file_fragments) in fragments_by_path {
        let mut has_failed_to_read_fragments = false;
        if let Some(file_content) = file_contents.get(&file_path) {
            // Process all fragments for this file.
            for (content_hash, fragment_ranges) in file_fragments {
                let start_idx = fragment_ranges.start.as_usize();
                let end_idx = fragment_ranges.end.as_usize();

                if start_idx <= end_idx
                    && end_idx <= file_content.len()
                    && file_content.is_char_boundary(start_idx)
                    && file_content.is_char_boundary(end_idx)
                {
                    let content = file_content[start_idx..end_idx].to_string();
                    if content.is_empty() {
                        log::trace!(
                            "Fragment for {:?} with range {:?} is empty",
                            file_path.display(),
                            fragment_ranges
                        );
                        fail_to_read.push(content_hash);
                        has_failed_to_read_fragments = true;
                    } else if ContentHash::from_content(&content) != content_hash {
                        log::trace!(
                            "Fragment for {:?} with range {:?} does not match its content hash",
                            file_path.display(),
                            fragment_ranges
                        );
                        fail_to_read.push(content_hash);
                        has_failed_to_read_fragments = true;
                    } else {
                        fragments.push(Fragment {
                            content,
                            content_hash,
                            location: FragmentLocation {
                                absolute_path: file_path.clone(),
                                byte_range: fragment_ranges,
                            },
                        });
                    }
                } else {
                    log::trace!("Invalid byte range {fragment_ranges:?} for file: {file_path:?}");
                    fail_to_read.push(content_hash);
                    has_failed_to_read_fragments = true;
                }
            }
        } else {
            log::trace!("Failed to read file: {file_path:?}");
            fail_to_read.extend(
                file_fragments
                    .into_iter()
                    .map(|(content_hash, _)| content_hash),
            );
            has_failed_to_read_fragments = true;
        }

        if has_failed_to_read_fragments {
            fail_to_read_path.push(file_path);
        }
    }

    ReadFragmentResult {
        successfully_read: fragments,
        fail_to_read,
        fail_to_read_path,
    }
}

// Convert fragments into CodeContextLocations. This function groups and dedupes fragments in the same file.
// It also allows the caller to define a context line number surrounding the relevant fragment.
pub fn fragments_to_context_locations<'a>(
    fragments: Vec<Fragment>,
    metadata_for_hash: impl Fn(&ContentHash) -> Option<&'a [FragmentMetadata]>,
    context_lines: usize,
) -> HashSet<CodeContextLocation> {
    // Map to collect fragments by file path.
    let mut fragments_by_path: HashMap<&PathBuf, Vec<Range<usize>>> = HashMap::new();
    let mut whole_files = HashSet::new();

    // First pass - collect all fragments and their line ranges by file path.
    for fragment in &fragments {
        if let Some(metadata) = metadata_for_hash(&fragment.content_hash).and_then(|metadatas| {
            metadatas.iter().find(|m| {
                m.absolute_path == fragment.location.absolute_path
                    && m.location.byte_range == fragment.location.byte_range
            })
        }) {
            // Add line range with context to the appropriate file's collection.
            let path = &fragment.location.absolute_path;
            let start = metadata.location.start_line.saturating_sub(context_lines);
            let end = metadata.location.end_line + 1 + context_lines;

            fragments_by_path.entry(path).or_default().push(start..end);
        } else {
            // Fallback to whole file if metadata not found.
            whole_files.insert(fragment.location.absolute_path.clone());
        }
    }

    // Second pass - process each file's fragments.
    let mut result = HashSet::new();

    // Process each file's fragments.
    for (path, mut line_ranges) in fragments_by_path {
        if line_ranges.is_empty() {
            continue;
        }

        // We can skip the fragments if the entire file is already included in the context.
        if whole_files.contains(path) {
            continue;
        }

        // Sort ranges by start position.
        line_ranges.sort_by_key(|range| range.start);

        // Merge overlapping or adjacent ranges.
        let mut merged_ranges: Vec<Range<usize>> = Vec::new();
        for range in line_ranges {
            if let Some(last) = merged_ranges.last_mut() {
                // If current range overlaps or is adjacent to the last one, merge them.
                if range.start <= last.end {
                    last.end = last.end.max(range.end);
                } else {
                    merged_ranges.push(range);
                }
            } else {
                merged_ranges.push(range);
            }
        }

        // Add file fragment location with all merged ranges.
        result.insert(CodeContextLocation::Fragment(FileFragmentLocation {
            path: path.clone(),
            line_ranges: merged_ranges,
        }));
    }

    // Add whole files to the result set.
    result.extend(whole_files.into_iter().map(CodeContextLocation::WholeFile));
    result
}

#[cfg(test)]
#[path = "search_shaping_tests.rs"]
mod tests;
