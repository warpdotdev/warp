# GH10259: Support `<details>`/`<summary>` in markdown rendering

Issue: https://github.com/warpdotdev/warp/issues/10259

## Summary

Warp's markdown renderer should support the HTML5 `<details>`/`<summary>` tags as collapsible sections, matching GitHub-flavored markdown (GFM) behavior. This primarily benefits agent-mode output, where long responses (logs, reasoning, code) can be collapsed behind a summary line.

## Problem

Today `<details>` and `<summary>` tags in markdown are rendered as plain text (the markdown parser only treats HTML entities and `<u>`/`</u>` specially). Agent output and README-style content that relies on GFM collapsible sections renders as noisy, tag-littered text.

## Goals / Non-goals

Goals:
- Render `<details>` blocks in interactive markdown surfaces as collapsible sections with a clickable summary line.
- Degrade deterministically on malformed input, deep nesting, and non-interactive surfaces — content is never dropped.
- Preserve round-trip structure: serializing a buffer containing a details block back to markdown reproduces the `<details>`/`<summary>` markup and the body's block structure. This is canonical re-serialization, not byte-exact source preservation, consistent with how the rich-text pipeline treats every other block type.

Non-goals:
- General inline-HTML rendering (other tags keep their current behavior).
- Supporting `<details>` in non-markdown terminal output (raw ANSI streams).
- Persisting open/collapsed state across app restarts.
- CSS/attribute styling of details blocks (`class`, `style`, etc. are ignored).

## Behavior

1. A block-level `<details>` element whose content is markdown renders as a collapsible section: a summary line with a disclosure indicator (▸/▾), followed by the body when expanded.
2. The section is collapsed by default. If the `<details>` tag carries the standard `open` attribute, it renders expanded initially.
3. Clicking the summary line (or pressing Enter/Space while it is focused) toggles the section. The toggle target is keyboard-focusable and exposed to accessibility APIs as an activatable disclosure control. Any accessibility IDs are renderer-generated; they are never derived from input attributes.
4. The first `<summary>` child provides the summary line, rendered with inline markdown styling. If no `<summary>` is present, the summary line is the localized literal "Details". Any additional `<summary>` siblings are treated as ordinary body content: their tags are rendered as visible plain text and their inner content as normal markdown — consistent with how stray summary tags render under 8(b) and 8(e), and with the principle that content (including markup the parser did not consume) is never silently dropped.
5. The body content between `<summary>` (or `<details>` when no summary exists) and `</details>` is parsed as normal markdown: paragraphs, code blocks, lists, tables, and nested `<details>` all render as they would at top level.
6. Nesting is supported up to a fixed depth of 8. This limit is deterministic: a `<details>` opening at depth 9 or greater is not rendered as a widget — its tags are rendered as plain text and its content rendered inline (current behavior), and this fallback applies consistently regardless of input size or timing.
7. A document renders at most 512 details widgets. From the 513th onward, the same deterministic plain-text fallback as (6) applies. Both limits exist to bound resource use against untrusted input; they are constants, not heuristics. The bounded resource is not the content itself (which costs the same as plain text either way) but what each *widget* adds on top of it: a level of parser recursion for the depth limit, and a keyboard-focusable interactive element with its accessibility-tree node and event bookkeeping for the count limit.
8. Malformed input degrades deterministically, never panics, and never drops content:
   - (a) An unclosed `<details>` extends to the end of the enclosing block context (end of document, or end of the parent details body).
   - (b) A stray `</details>` outside a details block, or a `<summary>`/`</summary>` outside a details body, renders as plain text (current behavior).
   - (c) `<details>` appearing mid-line (not at the start of a block) is not treated as a container and renders as plain text.
   - (d) An unclosed `<summary>` (no matching `</summary>` before the end of the details body) consumes the remainder of the details body as summary content; the body is then empty. Content is preserved verbatim inside the summary line.
   - (e) A `</summary>` with no matching opener inside a details body renders as plain text within the body.
   - (f) Attributes on `<summary>` are ignored. Summary content is rendered as inline markdown only: line breaks collapse to single spaces, and block-level constructs (code fences, lists, nested `<details>` tags) inside a summary are rendered as literal inline text, not as blocks or widgets.
   - (g) Details tags never take effect inside literal regions: a `<details>`, `</details>`, or `<summary>` appearing within a fenced code block is code content and neither opens nor closes a container. Outside fenced code, opening and closing tags are recognized only at the start of a block (consistent with (c)), so tags embedded mid-paragraph are plain text.
9. Collapsed state is view state, not content: copying the block, `raw_text()` extraction, and buffer conversion always include summary and full body regardless of the current collapsed state.
10. Round-trip: converting the rendered block to markdown (e.g. copying an agent message as markdown, or serializing an editor buffer) emits `<details>`/`<summary>` markup, preserving the `open` attribute only if the block was expanded by default in the source. Round-trip is canonical: the body is re-serialized from its parsed block structure through the standard markdown serialization, so structure and content are preserved but exact source bytes (whitespace, fence styles, list markers) are not — the same guarantee the rich-text pipeline gives every other block type.
11. Rendering is two-tier. The interactive disclosure widget (invariants 1-3) is implemented on two surfaces: the agent conversation block renderer (the surface the issue targets) and the notebook/plan rich-text editor (`RichTextEditorView`), where collapsible sections are directly useful for long plans and where the buffer-native representation (tech spec §4) makes the widget natural. Every other markdown surface uses a generic static fallback: the block renders expanded, in source order (summary line first, then body), with no disclosure indicator and no interaction. The fallback is the default for any renderer that has not opted into the widget, so no surface can render details in an undefined state. The concrete call sites for both tiers are enumerated in the tech spec.
12. Streaming agent output renders progressively: while a `<details>` block is still streaming (no closing tag yet), it renders under invariant (8a) semantics on each frame; lines above an in-progress details block do not visually shift when the closing tag arrives.
