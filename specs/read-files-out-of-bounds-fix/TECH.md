# TECH.md — Fix read_files with out-of-bounds line ranges

## Context

When an LLM requests a `read_files` tool call with line ranges entirely beyond EOF (e.g., lines 1891–2090 of a 1237-line file), the client returns `ReadFilesResult::Success { files: [] }` — the file silently vanishes from the result. This renders as an empty box with a green checkmark in the blocklist UI, and the server receives `anyFilesSuccess: {}` with zero files.

The root cause is in `TextFileAccumulator::flush_range` (`crates/warp_files/src/text_file_reader.rs:131`). The condition `let should_emit = !self.buf.is_empty() || (final_flush && self.whole_file)` only emits a segment on final flush for whole-file reads. For ranged reads where no lines fall within the requested range, `buf` stays empty and no segment is emitted. The caller (`read_local_file_context` in `app/src/ai/blocklist/action_model/execute.rs:1029–1045`) extends `file_contexts` from an empty segments iterator, then `continue`s past the binary fallback — the file ends up in neither `file_contexts` nor `missing_files`.

Relevant files:
- `crates/warp_files/src/text_file_reader.rs:131` — `flush_range` with the `should_emit` guard
- `app/src/ai/blocklist/action_model/execute.rs:1029–1045` — `Segments` branch in `read_local_file_context`
- `app/src/ai/blocklist/block/view_impl/output.rs:424–450` — rendering logic for `ReadFilesResult::Success`

## Proposed changes

### 1. `TextFileAccumulator::flush_range` — always emit on final flush
Change the `should_emit` condition from `!self.buf.is_empty() || (final_flush && self.whole_file)` to `!self.buf.is_empty() || final_flush`. This makes the behavior consistent: every requested range always produces a `TextFileSegment`, even when the range is entirely past EOF. The emitted segment has `content: ""`, the original requested `line_range`, and `line_count` set to the total file lines (populated by `finalize`).

This is a one-line change. The `whole_file` field remains used for trailing-newline preservation logic, so it is not removed.

### 2. Defensive rendering for empty `file_contexts`
In the `ReadFilesResult::Success` rendering arm, add a match guard `if !file_contexts.is_empty()` for the normal path. Add a second arm `if file_contexts.is_empty()` that renders an error-styled action box with a red X icon and "Failed to read files" message, then `continue`s. This prevents an empty box regardless of upstream cause.

### 3. Tests
- Update existing `empty_file_with_ranges_produces_no_segment` → renamed to `empty_file_with_ranges_produces_empty_segment`, now expects 1 segment with empty content and the original range.
- Add `range_past_eof_produces_empty_segment` — 5-line file, range 10..15, expects 1 segment with empty content, `line_count: 5`.
- Add `multiple_ranges_some_past_eof` — 5-line file, ranges [1..3, 10..15], expects 2 segments: first with content, second empty.

## Testing and validation

The fix is verified by the three unit tests above, all in `crates/warp_files/src/text_file_reader_tests.rs`. They cover:
- Empty file with ranges (previously zero segments, now one)
- Non-empty file with range entirely past EOF (the exact bug scenario)
- Mix of valid and out-of-bounds ranges in the same request

The defensive rendering change is not unit-tested (it requires the full UI framework context) but prevents the user-visible symptom for any future edge case that produces empty `file_contexts`.
