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

1. A `<details>` block whose opening tag starts a line renders as a disclosure section: a summary row followed by the body content. On an interactive surface the summary row carries a disclosure indicator; behavior 15 governs surfaces that are not interactive.

2. The body renders as ordinary markdown. Headings, lists, tables, code blocks, images, and nested `<details>` inside the body render exactly as they would outside it.

3. The summary renders as formatted inline markdown. Bold, italic, inline code, and links inside `<summary>` render with their normal styling and remain clickable.

4. A `<details>` block with the `open` attribute renders expanded on first render. Without `open`, it renders collapsed. `open` is a boolean attribute: its presence means expanded regardless of any value written on it, so `open="false"` renders expanded, matching HTML and GitHub. Other attributes on either tag are ignored and do not affect rendering.

5. Tag names match case-insensitively, so `<DETAILS>` and `<Summary>` behave as their lowercase spellings do. Whitespace is allowed between the tag name or its attributes and the closing `>`.

### Interaction

6. On an interactive surface, clicking the summary row toggles the section between collapsed and expanded. The section is keyboard focusable, and Enter or Space toggles it. The disclosure indicator reflects the current state.

7. Toggling a section changes only what is displayed. It never modifies document content, never marks a buffer dirty, and never adds an undo entry.

8. Copying a selection that spans a details section yields the summary text and the full body text, including the body of a section that is currently collapsed. Collapse is a view state, so it never removes content from a copy, a save, or a serialization.

9. Nested `<details>` sections render as independently toggleable sections. Collapsing an outer section hides its nested sections; expanding it restores each nested section to the state it already had.

### Nesting guard

10. Nesting is supported to a depth of 64. A `<details>` opening tag at depth 65 or deeper renders as literal text, and its content renders as ordinary markdown. The now-unmatched `</details>` renders as literal text under behavior 11(b). The depth is a fixed constant, so the same input always produces the same rendering. This bound is a stack-safety guard on the recursive body parse, not a product limit: it sits far above any realistic content — content in the wild rarely nests past 3 — and far below the depth at which the recursion could exhaust a stack. The tech spec derives the number.

### Malformed and unsupported input

11. Malformed and unsupported input degrades deterministically, and markup the parser does not consume renders as visible literal text rather than being silently dropped:

    a. A `<details>` with no matching `</details>` takes the rest of the enclosing content as its body. A nested unclosed `<details>` ends where its parent ends.

    b. A `</details>` with no open `<details>` renders as literal text.

    c. A `<details>` or `</details>` tag that does not start a line renders as literal text and does not open or close a section. Leading whitespace is permitted before an opening tag; any other preceding character on the line, including a backtick, disqualifies it.

    d. A self-closing `<details/>` opens a section with no distinct closing tag, so it degrades under 11(a).

    e. A `<details>` with no `<summary>` renders with the literal summary label `Details`.

    f. Only a `<summary>` that opens the details body is the summary. A `<summary>` appearing after body content renders as literal text, as does each `<summary>` after the first.

    g. A `<summary>` with no matching `</summary>` takes the rest of the details body as its summary, leaving the body empty. When the enclosing `<details>` is itself unclosed under 11(a), the summary ends at the end of the enclosing content, so a single unclosed `<summary>` can consume the remainder of the document into one summary row.

    h. A `</summary>` with no open `<summary>` renders as literal text.

    i. Markdown block structure inside `<summary>` is not honored: a code fence, a nested `<details>`, or a nested `<summary>` inside a summary renders as literal inline text, and a multi-line summary renders as one line. GitHub renders block content inside a summary as real blocks; Warp models a summary as a single inline run, so this diverges — see "Divergences from GitHub rendering".

12. Tags inside a code region are content, not markup. A `<details>`, `</details>`, `<summary>`, or `</summary>` line inside a fenced code block in the body renders verbatim in the code block and does not open or close any section. A tag inside an inline code span likewise never opens or closes a section, because it fails the line-start rule in 11(c).

13. A code fence opened inside a details body and never closed leaves the rest of the body inside that fence, so no later `</details>` closes the section and the section degrades under 11(a).

### Block-level scope

14. A `<details>` opens a section only at document level or at the block level of a details body.

    a. A `<details>` indented under a list item terminates the list and opens a section as its own block. GitHub nests it inside the list item and continues the list; Warp diverges — see "Divergences from GitHub rendering".

    b. A `<details>` inside a blockquote does not open a section and renders as literal text, as does its `</details>`. GitHub nests the disclosure section inside the blockquote; Warp diverges — see "Divergences from GitHub rendering".

### Surfaces

15. On an interactive surface, a details section exposes disclosure semantics to assistive technology: the summary is an activatable control that reports its expanded or collapsed state, and the association between that control and the body it controls uses renderer-generated identifiers, never identifiers taken from the markdown input.

16. On surfaces that render markdown without user interaction — banners, modals, settings pages, changelog entries, the command palette, and the TUI — a details section renders as its summary row followed by its fully expanded body, with no disclosure indicator and no toggle. No content is hidden on a surface where the user cannot reveal it.

### Streaming and editing

17. While markdown streams in, a `<details>` whose closing tag has not yet arrived renders progressively under behavior 11(a), and content above it does not reflow as the block grows.

18. Editing inside or around a details region in the notebook/plan editor always leaves a well-defined document. The buffer accepts every edit — markdown is a plain-text format and the editor never rejects a keystroke because of what it would produce — so deleting one of the region's boundaries degrades under the same rules as behavior 11(a) and 11(b) rather than producing an undefined state. The nesting guard in behavior 10 applies where content is read back out, not to the edits themselves.

19. Existing rendering is unchanged for documents containing neither tag. Markdown that contains no `<details>` and no `<summary>` renders exactly as it does today.

## Divergences from GitHub rendering

GitHub-flavored markdown is the reference implementation for this feature, and Warp matches it everywhere except the three cases below. These are not decisions to remain divergent. Each one requires a structural change to the parser's intermediate representation, which is a flat sequence of lines with no container blocks; this spec scopes the implementation to the subset that representation supports today, and convergence on all three is future work.

| Case | GitHub | Warp | Structural reason |
|---|---|---|---|
| `<details>` indented under a list item (behavior 14(a)) | Nests inside the list item; the list continues afterwards | Terminates the list, then opens a section as its own block | List items are flat lines carrying an indent level, not containers with child blocks, so there is no list-item body for a section to nest inside |
| `<details>` inside a blockquote (behavior 14(b)) | Nests inside the blockquote | Renders as literal text | The intermediate representation has no blockquote variant at all, so there is nothing to nest into |
| Block content inside `<summary>` (behavior 11(i)) | Renders as real blocks, so a heading in a summary is a heading | Renders as literal inline text on one line | A summary is modeled as a single inline run, in both the parser IR and the editor buffer |

An unindented `<details>` following a list item terminates the list on both GitHub and Warp, and is not a divergence.
