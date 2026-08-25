# Support `<details>`/`<summary>` in markdown rendering — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/10259

Figma: none provided

## Summary

Render `<details>`/`<summary>` HTML blocks in Warp-rendered markdown as collapsible disclosure sections, matching GitHub-flavored markdown. Today both tags render as literal text with no toggle, so agent output, READMEs, and plans that use progressive disclosure show every hidden section expanded and inline.

## Goals

- Render a `<details>` block as a disclosure widget whose summary is always visible and whose body collapses and expands on user action.
- Support formatted markdown inside the summary and full markdown inside the body, including nested `<details>`.
- Degrade deterministically on malformed input, so no markdown input produces an undefined or inconsistent rendering.
- Make the disclosure interactive on the two surfaces where a user can act on it: the agent conversation view and the notebook/plan editor.

## Non-goals

- Supporting arbitrary HTML tags. This spec covers `<details>` and `<summary>` only; every other unconsumed tag keeps its current literal-text behavior.
- Preserving byte-exact markdown source through a round trip. Warp's rich-text pipeline re-serializes canonically, and this feature follows that policy.
- Adding a disclosure widget to Warp's headless TUI renderer, which has no pointer input and no per-view fold state. The TUI renders details content expanded.
- Authoring affordances for creating a `<details>` region from a toolbar or command. Users type the tags; the editor round-trips them.
- Persisting collapse state across sessions, panes, or restarts.

## Behavior

### Rendering

1. A `<details>` block whose opening tag starts a line renders as a disclosure section: a summary row followed by the body content. On an interactive surface the summary row carries a disclosure indicator; behavior 16 governs surfaces that are not interactive.

2. The body renders as ordinary markdown. Headings, lists, tables, code blocks, images, and nested `<details>` inside the body render exactly as they would outside it.

3. The summary renders as formatted inline markdown. Bold, italic, inline code, and links inside `<summary>` render with their normal styling and remain clickable.

4. A `<details>` block with the `open` attribute renders expanded on first render. Without `open`, it renders collapsed. `open` is a boolean attribute: its presence means expanded regardless of any value written on it, so `open="false"` renders expanded, matching HTML and GitHub. Other attributes on either tag are ignored and do not affect rendering.

5. Tag names match case-insensitively, so `<DETAILS>` and `<Summary>` behave as their lowercase spellings do. Whitespace is allowed between the tag name or its attributes and the closing `>`.

### Interaction

6. On an interactive surface, clicking the summary row toggles the section between collapsed and expanded. The section is keyboard focusable, and Enter or Space toggles it. The disclosure indicator reflects the current state.

7. Toggling a section changes only what is displayed. It never modifies document content, never marks a buffer dirty, and never adds an undo entry.

8. Copying a selection that spans a details section yields the summary text and the full body text, including the body of a section that is currently collapsed. Collapse is a view state, so it never removes content from a copy, a save, or a serialization.

9. Nested `<details>` sections render as independently toggleable sections. Collapsing an outer section hides its nested sections; expanding it restores each nested section to the state it already had.

### Limits

10. Nesting is supported to a depth of 8. A `<details>` opening tag at depth 9 or deeper renders as literal text, and its content renders as ordinary markdown. The now-unmatched `</details>` renders as literal text under behavior 12(b).

11. A single rendered document renders at most 512 disclosure widgets. Beyond that count, each further `<details>` renders as literal text with its content as ordinary markdown. A `<details>` that falls back under either limit does not consume a widget slot, since the count tracks widgets actually rendered. Both limits are fixed constants, so the same input always produces the same rendering.

### Malformed and unsupported input

12. Malformed and unsupported input degrades deterministically, and markup the parser does not consume renders as visible literal text rather than being silently dropped:

    a. A `<details>` with no matching `</details>` takes the rest of the enclosing content as its body. A nested unclosed `<details>` ends where its parent ends.

    b. A `</details>` with no open `<details>` renders as literal text.

    c. A `<details>` or `</details>` tag that does not start a line renders as literal text and does not open or close a section. Leading whitespace is permitted before an opening tag; any other preceding character on the line, including a backtick, disqualifies it.

    d. A self-closing `<details/>` opens a section with no distinct closing tag, so it degrades under 12(a).

    e. A `<details>` with no `<summary>` renders with the literal summary label `Details`.

    f. Only a `<summary>` that opens the details body is the summary. A `<summary>` appearing after body content renders as literal text, as does each `<summary>` after the first.

    g. A `<summary>` with no matching `</summary>` takes the rest of the details body as its summary, leaving the body empty. When the enclosing `<details>` is itself unclosed under 12(a), the summary ends at the end of the enclosing content, so a single unclosed `<summary>` can consume the remainder of the document into one summary row.

    h. A `</summary>` with no open `<summary>` renders as literal text.

    i. Markdown block structure inside `<summary>` is not honored: a code fence, a nested `<details>`, or a nested `<summary>` inside a summary renders as literal inline text, and a multi-line summary renders as one line.

13. Tags inside a code region are content, not markup. A `<details>`, `</details>`, `<summary>`, or `</summary>` line inside a fenced code block in the body renders verbatim in the code block and does not open or close any section. A tag inside an inline code span likewise never opens or closes a section, because it fails the line-start rule in 12(c).

14. A code fence opened inside a details body and never closed leaves the rest of the body inside that fence, so no later `</details>` closes the section and the section degrades under 12(a).

### Surfaces

15. On an interactive surface, a details section exposes disclosure semantics to assistive technology: the summary is an activatable control that reports its expanded or collapsed state, and the association between that control and the body it controls uses renderer-generated identifiers, never identifiers taken from the markdown input.

16. On surfaces that render markdown without user interaction — banners, modals, settings pages, changelog entries, the command palette, and the TUI — a details section renders as its summary row followed by its fully expanded body, with no disclosure indicator and no toggle. No content is hidden on a surface where the user cannot reveal it.

### Streaming and editing

17. While markdown streams in, a `<details>` whose closing tag has not yet arrived renders progressively under behavior 12(a), and content above it does not reflow as the block grows.

18. Editing inside or around a details region in the notebook/plan editor always leaves a well-defined document. Deleting one of the region's boundaries degrades under the same rules as behavior 12(a) and 12(b) rather than producing an undefined state.

19. Existing rendering is unchanged for documents containing neither tag. Markdown that contains no `<details>` and no `<summary>` renders exactly as it does today.

## Open questions

- Whether the agent conversation surface reuses the existing block-folding interaction used for command blocks, or gets a dedicated disclosure component. The tech spec proposes a dedicated opt-in widget on the shared formatted-text element; this spec constrains behaviors 6 and 15, not the component choice, and the proposal is open to being redirected. The editor surface has no equivalent question — it folds through the existing hidden-lines mechanism described in the tech spec.
- Whether a `<details>` opening inside a list item or a blockquote should nest inside that block or terminate it. This spec covers only a `<details>` at document or details-body block level; the tech spec notes the parser consequence.
