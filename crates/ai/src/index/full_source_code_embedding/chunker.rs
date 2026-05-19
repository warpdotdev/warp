use std::path::Path;

use string_offset::ByteOffset;
#[cfg(not(target_family = "wasm"))]
use warp_util::standardized_path::StandardizedPath;

mod naive;
#[cfg(not(target_family = "wasm"))]
mod semantic;

/// Number of lines per chunk when chunking naively. While there's no guarantee
/// that this is below the token limit of the embedding model used on the server,
/// this should give us more than enough buffer.
const LINES_PER_CHUNK: usize = 200;

/// The average number of characters per line.
const AVG_CHAR_PER_LINE: usize = 60;

/// Compute the max byte per chunk based on the average number of characters per line. We assume code is mostly ASCII,
/// which is why this max chunk makes sense even if we're using bytes instead of characters as our unit of chunking.
const MAX_BYTES_PER_CHUNK: usize = LINES_PER_CHUNK * AVG_CHAR_PER_LINE;

/// A code fragment with line range information.
#[derive(Debug, Clone)]
pub struct Fragment<'a> {
    /// The content of the fragment.
    pub content: &'a str,
    /// Start line number (inclusive).
    pub start_line: usize,
    /// End line number (inclusive).
    pub end_line: usize,
    /// The start byte index of the fragment in the original source code.
    pub start_byte_index: ByteOffset,
    /// The end byte index of the fragment (exclusive) in the original source code.
    pub end_byte_index: ByteOffset,
    /// File path of the fragment.
    pub file_path: &'a Path,
}

impl<'a> Fragment<'a> {
    fn size(&self) -> usize {
        self.content.len()
    }

    fn append(&mut self, other: &Fragment<'a>, content: &'a str) {
        self.end_line = other.end_line;
        self.end_byte_index = other.end_byte_index;
        self.content = &content[self.start_byte_index.as_usize()..other.end_byte_index.as_usize()];
    }
}

/// Coalesce small fragments into larger ones that still respect the `max_bytes_per_chunk`.
/// Treesitter often produces small fragments that splits function names from the actual function body,
/// we iterate in reverse to coalesce these chunks into fragments that are more meaningful.
fn coalesce_fragments<'a>(
    fragments: impl DoubleEndedIterator<Item = Fragment<'a>>,
    code: &'a str,
    max_bytes_per_chunk: usize,
) -> Vec<Fragment<'a>> {
    fragments
        .rev()
        .fold(
            Vec::new(),
            |mut acc: Vec<Fragment<'a>>, mut fragment| match acc.last_mut() {
                Some(last_item) => {
                    let new_fragment_size = code
                        [fragment.start_byte_index.as_usize()..last_item.end_byte_index.as_usize()]
                        .len();
                    if new_fragment_size <= max_bytes_per_chunk {
                        fragment.append(last_item, code);
                        *last_item = fragment;
                    } else {
                        acc.push(fragment);
                    }
                    acc
                }
                None => {
                    acc.push(fragment);
                    acc
                }
            },
        )
        .into_iter()
        .rev()
        .collect()
}

/// Chunks code into an ordered list of fragments.
///
/// The code is chunked "semantically" using treesitter.
/// If we are unable to generate semantic chunks for any reason, fragments are naively chunked by
/// lines.
pub fn chunk_code<'a>(code: &'a str, path: &'a Path) -> Vec<Fragment<'a>> {
    if let Some(fragments) = try_chunk_code_semantically(code, path) {
        return fragments;
    }
    naive::chunk_code(code, path, MAX_BYTES_PER_CHUNK, LINES_PER_CHUNK)
}

/// Attempts to chunk code semantically, returning [`None`] if the code
/// could not be chunked for any reason.
#[cfg(not(target_family = "wasm"))]
fn try_chunk_code_semantically<'a>(code: &'a str, path: &'a Path) -> Option<Vec<Fragment<'a>>> {
    let standardized_path = StandardizedPath::try_from_local(path).ok()?;
    let language = languages::language_by_filename(&standardized_path)?;
    semantic::chunk_code(code, path, MAX_BYTES_PER_CHUNK, &language.grammar).ok()
}

#[cfg(target_family = "wasm")]
fn try_chunk_code_semantically<'a>(_code: &'a str, _path: &'a Path) -> Option<Vec<Fragment<'a>>> {
    None
}
