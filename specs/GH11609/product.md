# PRODUCT.md — URL detection across TUI-rendered row breaks

Issue: https://github.com/warpdotdev/warp/issues/11609

## Summary

AI TUI tools (Kiro CLI, Claude Code, GSD) render responses with explicit `\r\n` line endings, producing hard-wrapped rows where the `WRAPLINE` flag is absent. When a URL overflows the terminal width in this output, `url_at_point` returns only the first-row fragment. CMD+click opens a truncated, invalid URL.

## Goals / Non-goals

**In scope (V1):**
- Single-row continuation (URL wraps exactly once across a hard-wrap boundary)
- Forward extension only (hover on row 0 returns the full URL)
- Hard-wrapped rows only (`WRAPLINE` absent, `row_wraps() == false`)

**Out of scope (V1):**
- Multi-row wraps (URL wraps 2+ times) — deferred to V2
- Backward extension (hover on continuation row returns full URL) — deferred
- OSC 8 hyperlinks — tracked in #4194; supersedes this heuristic when it ships

## Behavior invariants

1. When `url_at_point` detects a URL ending within 4 columns of the right edge of a hard-wrapped row, and the following row starts with a URL-continuation fragment, the returned `Link` range spans both rows and the opened URL is complete.
2. Existing terminal auto-wrap URL detection is unaffected — the fix only fires when `row_wraps()` returns `false`.
3. A continuation fragment starting with an uppercase letter is rejected (new-sentence guard).
4. A continuation fragment starting with whitespace is rejected (indented-block guard).
5. The URL opened via `link_at_range` is the fully concatenated URL with no embedded newlines.
6. Hovering on the continuation fragment returns `None` — backward scan limitation unchanged in V1.

## Known limitation

URLs whose continuation fragment begins with an uppercase letter will not be extended (e.g., `/OAuth2/callback`). This is a deliberate tradeoff: rejecting uppercase-start continuations prevents false joins with sentence starts. A V2 enhancement could relax this guard when the continuation immediately follows `/` on the preceding row.
