use std::path::Path;

use itertools::Itertools;
use line_span::{LineSpan, LineSpans};

use crate::index::full_source_code_embedding::chunker::{coalesce_fragments, Fragment};

/// Chunks the given file into [`Fragment`]s. Each chunk is at most `num_lines_per_chunk` lines long, and contains at most `max_bytes_per_chunk` bytes.
pub(super) fn chunk_code<'a>(
    code: &'a str,
    path: &'a Path,
    max_bytes_per_chunk: usize,
    num_lines_per_chunk: usize,
) -> Vec<Fragment<'a>> {
    let lines = code.line_spans().enumerate().collect_vec();
    let chunks = lines.chunks(num_lines_per_chunk);

    chunks
        .into_iter()
        .flat_map(|chunk| {
            let (start_line, start_range) = chunk[0];
            let (end_line, end_range) =
                chunk.last().expect("Chunks must have at least one element");

            if (end_range.end() - start_range.start()) > max_bytes_per_chunk {
                let chunked_fragments = chunk.iter().flat_map(|(line, line_span)| {
                    chunk_line_by_bytes(code, path, max_bytes_per_chunk, *line, line_span)
                });

                return coalesce_fragments(chunked_fragments, code, max_bytes_per_chunk);
            }

            vec![Fragment {
                content: &code[start_range.start()..end_range.end()],
                start_line,
                end_line: *end_line,
                file_path: path,
                start_byte_index: start_range.start().into(),
                end_byte_index: end_range.end().into(),
            }]
        })
        .collect()
}

/// Chunks the line represented by `line_span` into multiple fragments if it exceeds `max_bytes_per_chunk`.
fn chunk_line_by_bytes<'a>(
    code: &'a str,
    path: &'a Path,
    max_bytes_per_chunk: usize,
    line_number: usize,
    line_span: &LineSpan<'a>,
) -> Vec<Fragment<'a>> {
    let line_start = line_span.start();
    let line_end = line_span.end();
    let line_content = &code[line_start..line_end];
    let line_length = line_end - line_start;

    // If the line is smaller than max_bytes_per_chunk, return it as a single fragment
    if line_length <= max_bytes_per_chunk {
        return vec![Fragment {
            content: line_content,
            start_line: line_number,
            end_line: line_number,
            file_path: path,
            start_byte_index: line_start.into(),
            end_byte_index: line_end.into(),
        }];
    }

    // Otherwise, split the line into multiple fragments
    let mut fragments = Vec::new();
    let mut current_start = line_start;

    while current_start < line_end {
        let remaining_bytes = line_end - current_start;
        let chunk_size = std::cmp::min(remaining_bytes, max_bytes_per_chunk);
        let mut chunk_end = current_start + chunk_size;

        // Ensure chunk_end is on a UTF-8 character boundary
        while chunk_end > current_start && !code.is_char_boundary(chunk_end) {
            chunk_end -= 1;
        }

        // If we couldn't find a valid boundary within reasonable distance,
        // move forward to the next character boundary instead
        if chunk_end <= current_start {
            chunk_end = current_start + chunk_size;
            while chunk_end < line_end && !code.is_char_boundary(chunk_end) {
                chunk_end += 1;
            }
        }

        fragments.push(Fragment {
            content: &code[current_start..chunk_end],
            start_line: line_number,
            end_line: line_number,
            file_path: path,
            start_byte_index: current_start.into(),
            end_byte_index: chunk_end.into(),
        });

        current_start = chunk_end;
    }

    fragments
}

#[cfg(test)]
#[path = "naive_tests.rs"]
mod tests;
