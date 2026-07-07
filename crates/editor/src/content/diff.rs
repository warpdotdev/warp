//! Text diff computation for incremental buffer updates.
//!
//! This module provides functionality for computing minimal diffs between two text strings,
//! which is used for auto-reloading files without wiping undo history or disrupting anchors.

use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput, Token};
use string_offset::{ByteOffset, CharOffset};

use super::buffer::{Buffer, ToBufferCharOffset};

/// A computed diff between two strings.
///
/// The edits are represented as byte ranges in the old text and their replacement strings.
/// These can be applied to transform the old text into the new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    /// List of edits: (old_byte_range, new_text)
    pub edits: Vec<(Range<usize>, String)>,
}

impl TextDiff {
    /// Returns true if this diff represents no changes.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Convert byte-range edits to CharOffset-range edits.
    ///
    /// The buffer uses 1-indexed coordinates (first editable character is at CharOffset(1)).
    /// The diff byte offsets are 0-indexed relative to the plain text, so we add 1 to
    /// convert to the buffer's 1-indexed system before using to_buffer_char_offset.
    pub fn to_char_offset_edits(&self, buffer: &Buffer) -> Vec<(Range<CharOffset>, String)> {
        self.edits
            .iter()
            .map(|(byte_range, new_text)| {
                // Add 1 to convert from 0-indexed plain text byte offset to 1-indexed buffer byte offset
                let start = ByteOffset::from(byte_range.start + 1).to_buffer_char_offset(buffer);
                let end = ByteOffset::from(byte_range.end + 1).to_buffer_char_offset(buffer);
                (start..end, new_text.clone())
            })
            .collect()
    }
}

/// Compute a diff between two strings.
///
/// This uses a line-based diff algorithm (Histogram) for efficiency.
/// Returns a list of edits as (byte_range_in_old, replacement_text) pairs.
pub async fn text_diff(old_text: &str, new_text: &str) -> TextDiff {
    let input = InternedInput::new(old_text, new_text);
    // Yield here to prevent doing more work if the task is aborted.
    futures_lite::future::yield_now().await;

    // Only compute line-based diff for now. Zed does more fine-grained word-level diffing for smaller hunks
    // but I don't think it's worth it for our use case.
    let edits = diff_internal(&input, new_text).await;

    TextDiff { edits }
}

async fn diff_internal(input: &InternedInput<&str>, new_text: &str) -> Vec<(Range<usize>, String)> {
    let mut old_offset = 0;
    let mut new_offset = 0;
    let mut old_token_ix = 0;
    let mut new_token_ix = 0;
    let mut edits = Vec::new();

    let diff = Diff::compute(Algorithm::Histogram, input);

    // Yield here to prevent doing more work if the task is aborted.
    futures_lite::future::yield_now().await;

    for hunk in diff.hunks() {
        // Calculate byte offsets for unchanged tokens before this hunk
        old_offset += token_len(
            input,
            &input.before[old_token_ix as usize..hunk.before.start as usize],
        );
        new_offset += token_len(
            input,
            &input.after[new_token_ix as usize..hunk.after.start as usize],
        );

        // Calculate byte lengths of the changed tokens
        let old_len = token_len(
            input,
            &input.before[hunk.before.start as usize..hunk.before.end as usize],
        );
        let new_len = token_len(
            input,
            &input.after[hunk.after.start as usize..hunk.after.end as usize],
        );

        let old_byte_range = old_offset..old_offset + old_len;
        let new_byte_range = new_offset..new_offset + new_len;

        old_token_ix = hunk.before.end;
        new_token_ix = hunk.after.end;
        old_offset = old_byte_range.end;
        new_offset = new_byte_range.end;

        let replacement = if new_byte_range.is_empty() {
            String::new()
        } else {
            new_text[new_byte_range].to_string()
        };
        edits.push((old_byte_range, replacement));
    }

    edits
}

/// Calculate total byte length of a sequence of tokens.
fn token_len(input: &InternedInput<&str>, tokens: &[Token]) -> usize {
    tokens
        .iter()
        .map(|token| input.interner[*token].len())
        .sum()
}
