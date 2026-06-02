use std::ops::Range;
use std::time::SystemTime;

/// A segment of text read from a file, with optional line-range metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFileSegment {
    pub file_name: String,
    pub content: String,
    pub line_range: Option<Range<usize>>,
    pub last_modified: Option<SystemTime>,
    /// Total number of lines in the source file (not just this segment).
    pub line_count: usize,
}

/// Result of attempting to read a file as text.
pub enum TextFileReadResult {
    /// Successfully read as text.
    Segments {
        segments: Vec<TextFileSegment>,
        bytes_read: usize,
    },
    /// Not valid UTF-8 — caller should try the binary path.
    NotText,
}

/// Accumulates lines into [`TextFileSegment`]s for a set of (possibly empty)
/// line ranges, enforcing a byte budget and tracking the total line count.
///
/// **Line ending normalization**: `\n` and `\r\n` line endings are normalized
/// to `\n` (LF) in the emitted [`TextFileSegment::content`]. Classic Mac
/// `\r`-only line endings are **not** recognized as line separators (matching
/// the behavior of `read_line()`, which only splits on `\n`). Lines are
/// expected to be pushed with their terminators already stripped (as produced
/// by `read_line()` + manual stripping). The trailing newline of the file, if
/// present, is preserved via the `has_trailing_newline` flag passed to
/// [`Self::push_line`].
pub(crate) struct TextFileAccumulator {
    file_name: String,
    last_modified: Option<SystemTime>,
    effective_ranges: Vec<Range<usize>>,
    whole_file: bool,
    max_bytes: usize,
    segments: Vec<TextFileSegment>,
    total_bytes_read: usize,
    range_idx: usize,
    buf: Vec<String>,
    buf_bytes: usize,
    truncated: bool,
    last_line: usize,
    current_line: usize,
    /// Whether the most recently pushed line had a trailing newline in the
    /// original file. Updated on every [`Self::push_line`] call so that after
    /// all lines are pushed, this reflects the final line's terminator state.
    last_line_had_newline: bool,
}

impl TextFileAccumulator {
    #[allow(clippy::single_range_in_vec_init)]
    pub(crate) fn new(
        file_name: String,
        last_modified: Option<SystemTime>,
        requested_ranges: &[Range<usize>],
        max_bytes: usize,
    ) -> Self {
        let whole_file = requested_ranges.is_empty();
        let effective_ranges = if whole_file {
            vec![1..usize::MAX]
        } else {
            let mut sorted = requested_ranges.to_vec();
            sorted.sort_by_key(|r| r.start);
            sorted
        };
        Self {
            file_name,
            last_modified,
            effective_ranges,
            whole_file,
            max_bytes,
            segments: Vec::new(),
            total_bytes_read: 0,
            range_idx: 0,
            buf: Vec::new(),
            buf_bytes: 0,
            truncated: false,
            last_line: 0,
            current_line: 0,
            last_line_had_newline: false,
        }
    }

    /// Pushes a single line (with its terminator already stripped) into the
    /// accumulator.
    ///
    /// `has_trailing_newline` indicates whether this line was terminated by a
    /// newline in the original file. For every line except possibly the last
    /// one in a file, this will be `true`.
    pub(crate) fn push_line(&mut self, line: String, has_trailing_newline: bool) {
        self.current_line += 1;
        self.last_line_had_newline = has_trailing_newline;

        if self.range_idx >= self.effective_ranges.len() {
            // Past all requested ranges — just count remaining lines.
            return;
        }

        // Past the current range — finalize it and advance.
        if self.current_line >= self.effective_ranges[self.range_idx].end {
            self.flush_range(false);
            self.range_idx += 1;
        }

        // Within the current range — accumulate.
        if self.range_idx < self.effective_ranges.len() {
            let range = &self.effective_ranges[self.range_idx];
            if self.current_line >= range.start && self.current_line < range.end && !self.truncated
            {
                let line_bytes = line.len() + if self.buf.is_empty() { 0 } else { 1 };
                if self.total_bytes_read + self.buf_bytes + line_bytes > self.max_bytes {
                    self.truncated = true;
                } else {
                    self.buf_bytes += line_bytes;
                    self.last_line = self.current_line;
                    self.buf.push(line);
                }
            }
        }
    }

    /// Emits a [`TextFileSegment`] for the current range and resets per-range
    /// state. Always produces a segment — even when the buffer is empty (e.g.
    /// the requested range was entirely past EOF).
    fn flush_range(&mut self, final_flush: bool) {
        let range = self.effective_ranges[self.range_idx].clone();
        let line_range = if self.whole_file && !self.truncated {
            None
        } else if self.truncated {
            Some(range.start..self.last_line)
        } else {
            Some(range)
        };

        self.total_bytes_read += self.buf_bytes;
        let mut content = std::mem::take(&mut self.buf).join("\n");

        // If this is the final flush of a non-truncated whole-file read
        // and the last line in the file had a trailing newline, preserve
        // it. This ensures round-tripping file content through the
        // accumulator doesn't silently drop a trailing newline (which
        // would otherwise cause data loss when the content is written
        // back to disk, e.g. during remote diff application).
        //
        // We skip this for truncated reads (the content is incomplete, so
        // appending a newline would be misleading) and for ranged reads
        // (which extract a slice, not the full file).
        if final_flush && self.whole_file && !self.truncated && self.last_line_had_newline {
            content.push('\n');
            self.total_bytes_read += 1;
        }

        self.segments.push(TextFileSegment {
            file_name: self.file_name.clone(),
            content,
            line_range,
            last_modified: self.last_modified,
            line_count: 0, // Set in finalize()
        });

        self.buf.clear();
        self.buf_bytes = 0;
        self.truncated = false;
        self.last_line = 0;
    }

    pub(crate) fn finalize(mut self) -> (Vec<TextFileSegment>, usize) {
        if self.range_idx < self.effective_ranges.len() {
            self.flush_range(true);
        }

        let total_line_count = self.current_line;
        for segment in &mut self.segments {
            segment.line_count = total_line_count;
        }

        (self.segments, self.total_bytes_read)
    }
}

#[cfg(test)]
#[path = "text_file_reader_tests.rs"]
mod tests;
