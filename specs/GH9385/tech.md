# GH9385: Tech Spec

## Problem
Rich AI and CLI-agent output currently feeds link detection one formatted line at a time. If a CLI TUI hard-wraps a long URL by emitting newline-separated fragments, Warp detects only the fragment that independently looks like a URL. Cmd+click then opens that partial target.

The regular terminal grid has a different path that scans across soft-wrapped rows, so changing the grid path is not the primary fix. The technical challenge is to reconstruct only high-confidence logical links in rich output while preserving performance and avoiding false positives in arbitrary TUI layouts.

## Relevant code
- `app/src/util/link_detection.rs:131` — `detect_urls`, the shared URL detector based on `urlocator`.
- `app/src/util/link_detection.rs (532-590)` — `collect_output_data_for_link_detection`, which currently iterates `formatted_lines.lines()` and pushes each formatted line as a separate `(String, TextLocation)`.
- `app/src/util/link_detection.rs (594-650)` — `detect_all_links`, which runs URL and file-path detection independently for each `(text, location)` and stores links keyed by `TextLocation`.
- `app/src/util/link_detection.rs (301-394)` — rich-output file-path detection; it splits each text by whitespace, enumerates candidate path substrings, and may perform filesystem checks on a background thread.
- `app/src/ai/blocklist/block.rs (1492-1558)` — `AIBlock::spawn_link_detection`, which collects rich output text and runs `detect_all_links` on a background task for local filesystem builds.
- `app/src/ai/blocklist/block/view_impl/common.rs (1520-1585)` — `render_rich_text_output_text_section`, which registers per-formatted-line mouse handlers using `TextLocation::Output { section_index, line_index }`.
- `app/src/ai/agent/mod.rs (1233-1287)` — `FormattedTextWrapper` and `FormattedTextLineWrapper`; each formatted line stores stripped text and pre-extracted markdown hyperlinks.
- `app/src/terminal/model/grid/grid_handler.rs (596-747)` — terminal-grid URL hit-testing. It scans with `grapheme_cursor::Wrap::Soft`, caps URL scan length at `URL_SCAN_CHARACTER_MAX_COUNT`, and returns a `Link` range across rows.
- `app/src/terminal/model/grid/grid_handler_test.rs (618-845)` — existing grid tests for URLs across wrapped lines, hard line breaks, trailing punctuation, and scan caps.
- `app/src/terminal/model/ansi/mod.rs (809-1227)` — OSC dispatcher. Known OSCs are handled, but OSC 8 hyperlink sequences are not currently parsed into terminal link metadata.
- `crates/markdown_parser/src/lib.rs (184-249)` — formatted-line raw text and hyperlink extraction. `raw_text()` appends a newline to each formatted line.

## Current state
Rich output detection has two useful properties today:

1. It is simple and location-local. Every detected range maps directly to a single `TextLocation`, so rendering can register clickable ranges without cross-line state.
2. Expensive file-path work already runs off the UI thread on local filesystem builds.

The limitation is that a logical URL split across `FormattedTextLineWrapper` entries is not represented as one detection input. `detect_urls` never sees the full URL, so it cannot return the full range. Markdown hyperlinks are handled separately through `line.hyperlinks()` and are already target-aware, but only within a single formatted line.

The terminal-grid path is more mature for soft wrapping. `GridHandler::url_at_point` scans backward and forward across soft-wrapped cells, has URL separator and length caps, and its tests explicitly distinguish soft wrapping from real CRLF hard line breaks. That design should inform the rich-output fix, but the data model differs because rich output stores independent formatted lines rather than grid cells.

Warp does not appear to support OSC 8 hyperlinks in the terminal ANSI path today. Adding OSC 8 support would let compatible apps attach a URL target to arbitrary display text, but it would not fix existing plain-text output from tools that do not emit OSC 8.

## Options considered
### Option A: Keep per-line detection and special-case clicks
Detect links as today, then when opening a URL from the end of a line, look at neighboring rendered lines and append URL-safe text.

Pros:
- Small surface area.
- Avoids changing detection data structures.

Cons:
- Hover and highlighting remain partial or inconsistent.
- The clicked target can differ from the visual clickable range.
- Tooltips, context menus, copy affordances, and future features would need their own reconstruction logic.

This is not recommended.

### Option B: Reconstruct logical rich-output blocks before detection
Build a bounded logical text buffer from adjacent formatted lines before URL detection. Run URL detection on the reconstructed buffer, then map detected character ranges back to per-line `TextLocation` ranges. Store each segment as clickable but with the full reconstructed URL as the target.

Pros:
- Click, hover, tooltip, and future copy behavior share the same detected target.
- Keeps reconstruction in the existing background link-detection pipeline.
- Can use conservative, testable continuation heuristics.
- Does not disturb terminal-grid link detection.

Cons:
- Requires a richer internal representation than `HashMap<TextLocation, HashMap<Range<usize>, DetectedLinkType>>` if multiple per-line ranges need to share one target.
- Needs careful range mapping because formatted-line `raw_text()` includes trailing newlines.

This is the recommended approach.

### Option C: Add OSC 8 support and rely on upstream tools
Parse OSC 8 hyperlinks in the terminal ANSI model and use hyperlink metadata for click targets.

Pros:
- Best long-term protocol for explicit links.
- Avoids heuristic reconstruction when apps emit metadata.
- Helps links whose displayed text differs from the URL.

Cons:
- Claude Code and other current tools may not emit OSC 8, so it does not solve this report alone.
- Requires terminal-grid metadata storage, rendering, selection, and serialization design beyond the rich-output bug.

This should be tracked as a follow-up or parallel platform improvement, not the only fix for GH9385.

## Proposed changes
Implement Option B for URL detection in rich AI/CLI-agent output.

### 1. Introduce a rich-output logical link detection input
Add an internal structure in `app/src/util/link_detection.rs` that represents a group of adjacent formatted lines from one output section:

- source `section_index`
- ordered line entries containing `TextLocation`, raw line text, and char-count metadata
- a reconstructed string used only for URL detection
- a mapping from reconstructed character offsets back to `(TextLocation, line-local char range)`

For product safety, only build logical groups within the same `AIAgentTextSection::PlainText` section and only from formatted text lines. Do not join across separate sections, code blocks, tables, images, action rows, user queries, or different output messages.

### 2. Normalize only high-confidence URL continuations
When reconstructing adjacent formatted lines, treat a line boundary as a URL continuation only when all of these are true:

- The previous logical line currently ends inside a URL candidate or with URL-safe continuation characters.
- The next line begins with URL-safe continuation characters and does not begin with obvious prose, list markers, table delimiters, prompt prefixes, or indentation that suggests a separate layout cell.
- Joining without whitespace produces a URL accepted by `detect_urls`.
- The reconstructed candidate remains under a bounded maximum length. Reuse the grid path's 1000-character cap as the initial limit unless implementation testing shows a different cap is needed.

Preserve the current per-line detection for all other boundaries. The detector should prefer false negatives over false positives for ambiguous TUI output.

### 3. Return per-line clickable segments with full targets
Keep rendering keyed by `TextLocation`, but allow a detected URL target to differ from the line-local display text. For a reconstructed URL spanning multiple lines:

- Insert one clickable range for each participating line segment.
- Store `DetectedLinkType::Url(full_url.clone())` for every segment.
- Preserve existing `Range<usize>` semantics as line-local character ranges so `FormattedTextElement` and `Text` handlers do not need layout changes.

This lets Cmd+clicking any segment open the full URL through the existing `open_link` path while keeping hover registration per rendered line.

### 4. Preserve markdown hyperlink precedence
Markdown hyperlinks extracted from `FormattedTextLineWrapper::hyperlinks()` should continue to override or replace raw URL detection for their line-local ranges. Do not reconstruct display text across lines for explicit markdown hyperlinks unless the parser already exposes a single hyperlink target.

### 5. Keep file paths out of the first URL fix unless cheap and safe
Do not run cross-line filesystem path detection as part of the initial URL fix. Current file-path detection may perform filesystem checks and has different false-positive characteristics. If the same range-mapping helper is useful, make it reusable, but leave wrapped file-path reconstruction as a follow-up unless implementation review finds a low-risk way to support it behind the existing background task and safety caps.

### 6. Do not modify terminal-grid behavior
Leave `GridHandler::url_at_point` unchanged except for tests if needed to document parity. The terminal path already has soft-wrap-aware behavior and explicitly treats hard CRLF line breaks differently.

## End-to-end flow
1. `AIBlock::spawn_link_detection` obtains an `AIAgentOutput`.
2. `collect_output_data_for_link_detection` walks output text sections.
3. For formatted plain-text sections, it collects raw line text and markdown hyperlinks as today, and additionally builds rich-output logical URL groups for adjacent lines in the same section.
4. The background detection task runs URL detection against logical groups and existing per-line text inputs.
5. Reconstructed URL results are remapped into `HashMap<TextLocation, HashMap<Range<usize>, DetectedLinkType>>` as line-local clickable segments whose `DetectedLinkType::Url` contains the full URL.
6. `render_rich_text_output_text_section` registers the line-local clickable ranges exactly as it does today.
7. Cmd+click on any segment dispatches `AIBlockAction::OpenLink`, and `AIBlock::open_link` opens the stored full URL.

## Risks and mitigations
- False positives across TUI layouts: Join only within a single plain-text section, require URL-safe continuation, exclude table/list/prompt-looking boundaries, cap scan length, and add negative tests.
- Performance regressions on large outputs: Build bounded logical groups, reuse existing background detection, avoid filesystem checks for cross-line URL reconstruction, and avoid quadratic joins across many lines.
- Range mapping bugs: Add unit tests with multi-byte characters before and inside wrapped URLs because rich text ranges are character-based while raw strings are UTF-8.
- Markdown hyperlink regressions: Keep extracted markdown hyperlinks as a separate input and preserve their precedence over raw URL detection.
- Streaming churn: Continue aborting stale link-detection tasks through the existing `link_detection_handle`; do not introduce UI-thread scanning.
- Incomplete very long URLs: Match the grid path's preference not to offer incomplete links beyond the scan cap.

## Testing and validation
- Add unit tests in `app/src/util/link_detection_test.rs` for:
  - two-line rich formatted URL reconstruction
  - three-line rich formatted URL reconstruction
  - clicking ranges on each participating line resolving to the same full URL target
  - adjacent independent URLs remaining separate
  - URL followed by prose not being joined
  - table-like or bullet-list adjacent fragments not being joined
  - multi-byte text before a wrapped URL preserving correct character ranges
  - markdown hyperlink target precedence
- Keep existing `app/src/terminal/model/grid/grid_handler_test.rs` URL tests passing to confirm terminal-grid behavior does not change.
- Manually validate on macOS with a narrow pane and Claude Code output that contains a long URL split across hard-rendered lines.
- Manually validate a normal terminal command that prints a long URL and soft-wraps in the grid.
- During implementation, run the focused Rust tests for `link_detection` and grid URL detection. If local resources allow, run the repository's relevant presubmit or package-level test target documented by the Warp test workflow.

## Follow-ups
- Add OSC 8 hyperlink support in the terminal ANSI path. This likely requires a new `Handler` callback, grid cell or range metadata for active hyperlink targets, serialization/restoration decisions, rendering/hover integration, and tests for `OSC 8 ; params ; URI ST display text OSC 8 ;; ST`.
- Add "Copy full URL" for reconstructed rich-output links if product wants a copy affordance in addition to Cmd+click.
- Evaluate wrapped local filesystem references in rich CLI-agent output using the same range-mapping approach, but separately assess filesystem lookup cost and false positives.
- Consider sharing URL separator and scan-cap constants between terminal-grid and rich-output detection to keep behavior aligned.
